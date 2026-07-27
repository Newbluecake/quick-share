use async_trait::async_trait;
use quick_share_core::{
    manifest::{ManifestBuilder, ManifestEntryKind as SourceEntryKind},
    paths::RelativePath,
};
use quick_share_protocol::{
    Capability, ChunkAck, ChunkData, ContentKind, DeviceId, DeviceInfo, EntryTransferStatus,
    ManifestEntry, ManifestEntryKind, MissingChunkBitmap, ProtocolVersion, RequestId,
    TransferCancel, TransferComplete, TransferCompleteAck, TransferId, TransferOffer,
    TransferStatus, TransferStatusRequest, TransferStatusResponse,
};
use quick_share_transfer::{
    offer::AuthorizationToken,
    sender::{
        ProgressEvent, RetryPolicy, SendFile, SenderError, SenderPolicy, TransferPlan,
        TransferSender, TransferTransport, TransportError,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CHUNK_SIZE: u32 = 256 * 1024;

fn plan(root: &Path, files: &[(&str, &[u8])]) -> TransferPlan {
    let mut paths = Vec::new();
    for (name, bytes) in files {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("source parent");
        }
        fs::write(&path, bytes).expect("source");
        paths.push(path);
    }
    let manifest = ManifestBuilder::new().build(&paths).expect("manifest");
    let send_files = manifest
        .entries
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                quick_share_core::manifest::ManifestEntryKind::File
            )
        })
        .map(|entry| SendFile::prepare(entry).expect("prepared source"))
        .collect::<Vec<_>>();
    let transfer_id = TransferId::new(Uuid::now_v7());
    let mut binding = Vec::new();
    binding.extend_from_slice(&transfer_id.to_bytes());
    for file in &send_files {
        binding.extend_from_slice(&file.source.id.get().to_be_bytes());
        binding.extend_from_slice(&file.final_digest);
    }
    TransferPlan {
        transfer_id,
        manifest_digest: *blake3::hash(&binding).as_bytes(),
        chunk_size: CHUNK_SIZE,
        total_bytes: send_files.iter().map(|file| file.source.size).sum(),
        files: send_files,
    }
}

fn status_for(plan: &TransferPlan, received: &[(u32, u32)]) -> TransferStatusResponse {
    let received: BTreeMap<u32, u32> = received.iter().copied().collect();
    TransferStatusResponse {
        transfer_id: plan.transfer_id,
        status: TransferStatus::Accepted,
        manifest_digest: plan.manifest_digest,
        entries: plan
            .files
            .iter()
            .map(|file| {
                let count = u32::try_from(file.source.size.div_ceil(u64::from(plan.chunk_size)))
                    .expect("chunk count");
                let completed = received.get(&file.source.id.get()).copied();
                EntryTransferStatus {
                    entry_id: file.source.id,
                    chunks: MissingChunkBitmap::from_missing(
                        count,
                        (0..count).filter(|index| Some(*index) != completed),
                    )
                    .expect("bitmap"),
                }
            })
            .collect(),
    }
}

#[derive(Default)]
struct FakeState {
    fragments: usize,
    completed: bool,
    cancelled: usize,
}

struct FakeTransport {
    status: Mutex<TransferStatusResponse>,
    state: Mutex<FakeState>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    status_failures: AtomicUsize,
    retry_failures: AtomicUsize,
    fatal_failures: AtomicUsize,
    cancelled_failures: AtomicUsize,
    complete_failures: AtomicUsize,
    reconnects: AtomicUsize,
    request_ids: Mutex<Vec<RequestId>>,
    logical_request_ids: Mutex<BTreeMap<(quick_share_protocol::EntryId, u32), RequestId>>,
    delay: Duration,
    mutate_on_status: Option<std::path::PathBuf>,
}

impl FakeTransport {
    fn new(status: TransferStatusResponse) -> Self {
        Self {
            status: Mutex::new(status),
            state: Mutex::new(FakeState::default()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            status_failures: AtomicUsize::new(0),
            retry_failures: AtomicUsize::new(0),
            fatal_failures: AtomicUsize::new(0),
            cancelled_failures: AtomicUsize::new(0),
            complete_failures: AtomicUsize::new(0),
            reconnects: AtomicUsize::new(0),
            request_ids: Mutex::new(Vec::new()),
            logical_request_ids: Mutex::new(BTreeMap::new()),
            delay: Duration::from_millis(5),
            mutate_on_status: None,
        }
    }

    fn with_retry_failures(self, failures: usize) -> Self {
        self.retry_failures.store(failures, Ordering::SeqCst);
        self
    }
}

#[async_trait]
impl TransferTransport for FakeTransport {
    async fn status(
        &self,
        _request: TransferStatusRequest,
    ) -> Result<TransferStatusResponse, TransportError> {
        if self
            .status_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(TransportError::Retryable);
        }
        if let Some(path) = &self.mutate_on_status {
            fs::write(path, b"changed-and-longer").expect("mutate source");
        }
        Ok(self.status.lock().expect("status").clone())
    }

    async fn send_fragment(
        &self,
        request_id: RequestId,
        frame: ChunkData,
    ) -> Result<Option<ChunkAck>, TransportError> {
        self.request_ids
            .lock()
            .expect("request IDs")
            .push(request_id);
        {
            let mut logical_requests = self
                .logical_request_ids
                .lock()
                .expect("logical request IDs");
            let stable_id = logical_requests
                .entry((frame.descriptor.entry_id, frame.descriptor.index))
                .or_insert(request_id);
            assert_eq!(*stable_id, request_id, "chunk retry changed request ID");
        }
        if self
            .cancelled_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(TransportError::Cancelled);
        }
        if self
            .fatal_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(TransportError::Fatal);
        }
        if self
            .retry_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(TransportError::Retryable);
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.state.lock().expect("state").fragments += 1;
        Ok(frame.final_fragment.then_some(ChunkAck {
            transfer_id: frame.descriptor.transfer_id,
            entry_id: frame.descriptor.entry_id,
            index: frame.descriptor.index,
            accepted_length: frame.descriptor.length,
            duplicate: false,
        }))
    }

    async fn complete(
        &self,
        request: TransferComplete,
    ) -> Result<TransferCompleteAck, TransportError> {
        if self
            .complete_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
        {
            return Err(TransportError::Retryable);
        }
        self.state.lock().expect("state").completed = true;
        Ok(TransferCompleteAck {
            transfer_id: request.transfer_id,
            status: TransferStatus::Completed,
        })
    }

    async fn cancel(&self, _request: TransferCancel) -> Result<(), TransportError> {
        self.state.lock().expect("state").cancelled += 1;
        Ok(())
    }

    async fn reconnect(&self) -> Result<(), TransportError> {
        self.reconnects.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn transfer_plan_binds_wire_manifest_to_local_source_snapshot() {
    let root = tempdir().expect("root");
    let source = root.path().join("bound.bin");
    fs::write(&source, b"bound").expect("source");
    let manifest = ManifestBuilder::new().build(&[source]).expect("manifest");
    let local = manifest.entries[0].clone();
    let transfer_offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id: TransferId::new(Uuid::now_v7()),
        sender: DeviceInfo {
            device_id: DeviceId::parse("qs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("device"),
            name: "sender".to_owned(),
            capabilities: BTreeSet::from([Capability::Files]),
        },
        content_kind: ContentKind::Files,
        chunk_size: CHUNK_SIZE,
        total_bytes: local.size,
        entries: vec![ManifestEntry {
            id: local.id,
            relative_path: local.relative_path.as_str().to_owned(),
            kind: ManifestEntryKind::File,
            size: local.size,
            digest: Some(*blake3::hash(b"bound").as_bytes()),
        }],
    };
    let bound = TransferPlan::from_offer(&transfer_offer, manifest).expect("bound plan");
    assert_eq!(bound.transfer_id, transfer_offer.transfer_id);
    assert_eq!(bound.files.len(), 1);

    let changed_manifest = ManifestBuilder::new()
        .build(&[root.path().join("bound.bin")])
        .expect("manifest");
    let mut changed_offer = transfer_offer;
    changed_offer.entries[0].relative_path = "different.bin".to_owned();
    assert!(TransferPlan::from_offer(&changed_offer, changed_manifest).is_err());
}

#[test]
fn directory_plan_preserves_empty_directories_and_queues_only_regular_payloads() {
    let root = tempdir().expect("root");
    let directory = root.path().join("folder");
    fs::create_dir_all(directory.join("empty")).expect("empty directory");
    fs::create_dir_all(directory.join("nested")).expect("nested directory");
    fs::write(directory.join("nested/data.bin"), b"directory payload").expect("payload");
    fs::write(directory.join("zero.bin"), b"").expect("empty file");
    let manifest = ManifestBuilder::new()
        .build(&[directory])
        .expect("directory manifest");
    let entries = manifest
        .entries
        .iter()
        .map(|entry| ManifestEntry {
            id: entry.id,
            relative_path: entry.relative_path.as_str().to_owned(),
            kind: match &entry.kind {
                SourceEntryKind::File => ManifestEntryKind::File,
                SourceEntryKind::Directory => ManifestEntryKind::Directory,
                SourceEntryKind::Symlink { target } => ManifestEntryKind::Symlink {
                    target: target.clone(),
                },
            },
            size: entry.size,
            digest: matches!(&entry.kind, SourceEntryKind::File)
                .then(|| *blake3::hash(&fs::read(&entry.source_path).expect("source")).as_bytes()),
        })
        .collect();
    let offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id: TransferId::new(Uuid::now_v7()),
        sender: DeviceInfo {
            device_id: DeviceId::parse("qs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("device"),
            name: "sender".to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::Directories]),
        },
        content_kind: ContentKind::Files,
        chunk_size: CHUNK_SIZE,
        total_bytes: manifest.total_bytes,
        entries,
    };
    let plan = TransferPlan::from_offer(&offer, manifest).expect("directory plan");
    assert_eq!(plan.files.len(), 2);
    assert!(plan.files.iter().any(|file| file.source.size == 0));
}

#[tokio::test]
async fn sender_uses_four_bounded_workers_and_emits_monotonic_progress_for_files_and_empty_files() {
    let root = tempdir().expect("root");
    let payload = vec![7_u8; CHUNK_SIZE as usize + 3];
    let transfer_plan = plan(
        root.path(),
        &[
            ("one.bin", &payload),
            ("two.bin", &payload),
            ("three.bin", &payload),
            ("four.bin", &payload),
            ("empty.bin", b""),
        ],
    );
    let transport = Arc::new(FakeTransport::new(status_for(&transfer_plan, &[])));
    let sender = TransferSender::new(
        Arc::clone(&transport),
        SenderPolicy {
            concurrent_files: 4,
            queue_capacity: 2,
            retry: RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                ..RetryPolicy::default()
            },
        },
    )
    .expect("sender");
    let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressEvent>(100);
    let summary = sender
        .send(
            transfer_plan.clone(),
            &AuthorizationToken::from_bytes([5; 32]),
            CancellationToken::new(),
            Some(progress_tx),
        )
        .await
        .expect("send");

    assert_eq!(summary.status, TransferStatus::Completed);
    assert_eq!(summary.transferred_bytes, transfer_plan.total_bytes);
    assert!(transport.max_active.load(Ordering::SeqCst) <= 4);
    assert!(transport.max_active.load(Ordering::SeqCst) >= 2);
    assert!(transport.state.lock().expect("state").completed);
    let mut events = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        events.push(event);
    }
    assert!(!events.is_empty());
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].current_bytes <= pair[1].current_bytes)
    );
    assert!(
        events
            .iter()
            .all(|event| event.current_bytes <= event.total_bytes)
    );
}

#[tokio::test]
async fn resume_sends_only_missing_chunks_and_retry_is_exponential_bounded_to_three_attempts() {
    let root = tempdir().expect("root");
    let payload = vec![1_u8; CHUNK_SIZE as usize * 2 + 7];
    let transfer_plan = plan(root.path(), &[("resume.bin", &payload)]);
    let entry = transfer_plan.files[0].source.id.get();
    let fake = FakeTransport::new(status_for(&transfer_plan, &[(entry, 0)])).with_retry_failures(2);
    fake.status_failures.store(1, Ordering::SeqCst);
    fake.complete_failures.store(1, Ordering::SeqCst);
    let transport = Arc::new(fake);
    let sender = TransferSender::new(
        Arc::clone(&transport),
        SenderPolicy {
            concurrent_files: 2,
            queue_capacity: 1,
            retry: RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                jitter_fraction: 0.0,
            },
        },
    )
    .expect("sender");
    let summary = sender
        .send(
            transfer_plan,
            &AuthorizationToken::from_bytes([6; 32]),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("resumed send");

    assert_eq!(summary.uploaded_chunks, 2);
    assert_eq!(transport.reconnects.load(Ordering::SeqCst), 4);
    assert_eq!(transport.retry_failures.load(Ordering::SeqCst), 0);
    assert_eq!(transport.complete_failures.load(Ordering::SeqCst), 0);
    let request_ids = transport.request_ids.lock().expect("request IDs");
    assert_eq!(request_ids.len(), 4);
    let mut logical_requests = Vec::new();
    for request_id in request_ids.iter().copied() {
        if !logical_requests.contains(&request_id) {
            logical_requests.push(request_id);
        }
    }
    assert_eq!(logical_requests.len(), 2);
    let retry = RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_secs(1),
        jitter_fraction: 0.0,
    };
    assert_eq!(retry.delay_for(1, 0), Duration::from_millis(100));
    assert_eq!(retry.delay_for(2, 0), Duration::from_millis(200));
    assert_eq!(retry.delay_for(5, 0), Duration::from_secs(1));
}

#[tokio::test]
async fn sparse_file_above_four_gib_uses_u64_offsets_and_sends_only_the_missing_tail() {
    let root = tempdir().expect("root");
    let source = root.path().join("large-sparse.bin");
    let file = fs::File::create(&source).expect("sparse file");
    let size = u64::from(u32::MAX) + 9;
    file.set_len(size).expect("sparse length");
    drop(file);
    let manifest = ManifestBuilder::new().build(&[source]).expect("manifest");
    let send_file = SendFile::new(manifest.entries[0].clone(), [0; 32]).expect("send file");
    let transfer_plan = TransferPlan {
        transfer_id: TransferId::new(Uuid::now_v7()),
        manifest_digest: [8; 32],
        chunk_size: CHUNK_SIZE,
        total_bytes: size,
        files: vec![send_file],
    };
    let chunk_count = u32::try_from(size.div_ceil(u64::from(CHUNK_SIZE))).expect("chunk count");
    let status = TransferStatusResponse {
        transfer_id: transfer_plan.transfer_id,
        status: TransferStatus::Paused,
        manifest_digest: transfer_plan.manifest_digest,
        entries: vec![EntryTransferStatus {
            entry_id: transfer_plan.files[0].source.id,
            chunks: MissingChunkBitmap::from_missing(chunk_count, [chunk_count - 1])
                .expect("tail bitmap"),
        }],
    };
    let transport = Arc::new(FakeTransport::new(status));
    let sender =
        TransferSender::new(Arc::clone(&transport), SenderPolicy::default()).expect("sender");
    let summary = sender
        .send(
            transfer_plan,
            &AuthorizationToken::from_bytes([1; 32]),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("sparse tail send");
    assert_eq!(summary.uploaded_chunks, 1);
    assert_eq!(transport.state.lock().expect("state").fragments, 1);
}

#[tokio::test]
async fn ten_thousand_small_files_use_a_fixed_worker_count_and_bounded_queue() {
    let root = tempdir().expect("root");
    let base = plan(root.path(), &[("shared-byte", b"x")]);
    let template = base.files[0].clone();
    let mut files = Vec::with_capacity(10_000);
    for index in 1..=10_000_u32 {
        let mut file = template.clone();
        file.source.id = quick_share_protocol::EntryId::new(index).expect("entry");
        file.source.relative_path =
            RelativePath::parse(format!("small/{index}.bin")).expect("relative");
        files.push(file);
    }
    let transfer_plan = TransferPlan {
        transfer_id: TransferId::new(Uuid::now_v7()),
        manifest_digest: [9; 32],
        chunk_size: CHUNK_SIZE,
        total_bytes: 10_000,
        files,
    };
    let mut fake = FakeTransport::new(status_for(&transfer_plan, &[]));
    fake.delay = Duration::ZERO;
    let transport = Arc::new(fake);
    let sender = TransferSender::new(
        Arc::clone(&transport),
        SenderPolicy {
            concurrent_files: 4,
            queue_capacity: 8,
            retry: RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                ..RetryPolicy::default()
            },
        },
    )
    .expect("sender");
    let summary = sender
        .send(
            transfer_plan,
            &AuthorizationToken::from_bytes([2; 32]),
            CancellationToken::new(),
            None,
        )
        .await
        .expect("bounded small-file transfer");
    assert_eq!(summary.uploaded_chunks, 10_000);
    assert!(transport.max_active.load(Ordering::SeqCst) <= 4);
    assert_eq!(transport.state.lock().expect("state").fragments, 10_000);
}

#[tokio::test]
async fn cancellation_propagates_to_workers_and_remote_without_final_completion() {
    let root = tempdir().expect("root");
    let payload = vec![2_u8; CHUNK_SIZE as usize + 1];
    let transfer_plan = plan(
        root.path(),
        &[
            ("a", &payload),
            ("b", &payload),
            ("c", &payload),
            ("d", &payload),
        ],
    );
    let mut fake = FakeTransport::new(status_for(&transfer_plan, &[]));
    fake.delay = Duration::from_millis(100);
    let transport = Arc::new(fake);
    let sender =
        TransferSender::new(Arc::clone(&transport), SenderPolicy::default()).expect("sender");
    let cancellation = CancellationToken::new();
    let cancel_from_test = cancellation.clone();
    let task = tokio::spawn(async move {
        sender
            .send(
                transfer_plan,
                &AuthorizationToken::from_bytes([4; 32]),
                cancellation,
                None,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    cancel_from_test.cancel();
    assert!(matches!(
        task.await.expect("sender task"),
        Err(SenderError::Cancelled)
    ));
    assert_eq!(transport.state.lock().expect("state").cancelled, 1);
    assert!(!transport.state.lock().expect("state").completed);
}

#[tokio::test]
async fn fatal_error_source_change_and_cancellation_do_not_retry_forever() {
    let root = tempdir().expect("root");
    let transfer_plan = plan(root.path(), &[("fatal.bin", b"payload")]);
    let transport = Arc::new(FakeTransport::new(status_for(&transfer_plan, &[])));
    transport.fatal_failures.store(1, Ordering::SeqCst);
    let sender = TransferSender::new(
        Arc::clone(&transport),
        SenderPolicy {
            retry: RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                ..RetryPolicy::default()
            },
            ..SenderPolicy::default()
        },
    )
    .expect("sender");
    assert!(matches!(
        sender
            .send(
                transfer_plan,
                &AuthorizationToken::from_bytes([7; 32]),
                CancellationToken::new(),
                None,
            )
            .await,
        Err(SenderError::Transport(TransportError::Fatal))
    ));
    assert_eq!(transport.reconnects.load(Ordering::SeqCst), 0);
    assert_eq!(transport.state.lock().expect("state").cancelled, 1);

    let remote_cancel_plan = plan(root.path(), &[("remote-cancel.bin", b"payload")]);
    let remote_cancel_transport =
        Arc::new(FakeTransport::new(status_for(&remote_cancel_plan, &[])));
    remote_cancel_transport
        .cancelled_failures
        .store(1, Ordering::SeqCst);
    let remote_cancel_sender = TransferSender::new(
        Arc::clone(&remote_cancel_transport),
        SenderPolicy::default(),
    )
    .expect("sender");
    assert!(matches!(
        remote_cancel_sender
            .send(
                remote_cancel_plan,
                &AuthorizationToken::from_bytes([17; 32]),
                CancellationToken::new(),
                None,
            )
            .await,
        Err(SenderError::Transport(TransportError::Cancelled))
    ));
    assert_eq!(remote_cancel_transport.reconnects.load(Ordering::SeqCst), 0);

    let changed_plan = plan(root.path(), &[("changed.bin", b"before!")]);
    let changed_path = changed_plan.files[0].source.source_path.clone();
    let mut changed_transport = FakeTransport::new(status_for(&changed_plan, &[]));
    changed_transport.mutate_on_status = Some(changed_path);
    let changed_transport = Arc::new(changed_transport);
    let changed_sender =
        TransferSender::new(Arc::clone(&changed_transport), SenderPolicy::default())
            .expect("sender");
    let changed_result = changed_sender
        .send(
            changed_plan,
            &AuthorizationToken::from_bytes([8; 32]),
            CancellationToken::new(),
            None,
        )
        .await;
    assert!(
        matches!(changed_result, Err(SenderError::SourceChanged(_))),
        "unexpected result: {changed_result:?}"
    );
    assert_eq!(changed_transport.reconnects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retry_stops_after_three_total_attempts() {
    let root = tempdir().expect("root");
    let transfer_plan = plan(root.path(), &[("retry.bin", b"payload")]);
    let transport =
        Arc::new(FakeTransport::new(status_for(&transfer_plan, &[])).with_retry_failures(3));
    let sender = TransferSender::new(
        Arc::clone(&transport),
        SenderPolicy {
            retry: RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                jitter_fraction: 0.0,
            },
            ..SenderPolicy::default()
        },
    )
    .expect("sender");
    assert!(matches!(
        sender
            .send(
                transfer_plan,
                &AuthorizationToken::from_bytes([3; 32]),
                CancellationToken::new(),
                None,
            )
            .await,
        Err(SenderError::Transport(TransportError::Retryable))
    ));
    assert_eq!(transport.reconnects.load(Ordering::SeqCst), 2);
    assert_eq!(transport.retry_failures.load(Ordering::SeqCst), 0);
}
