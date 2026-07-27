use quick_share_core::{
    config::{ConflictPolicy, TrustedPolicy},
    identity::{IdentityStore, TrustedDeviceStore},
};
use quick_share_protocol::{
    AuthorizationProof, Capability, ChunkData, ChunkDescriptor, ContentKind, DeviceInfo, EntryId,
    ManifestEntry, ManifestEntryKind, OfferDecision, ProtocolVersion, RequestId, TransferCancel,
    TransferComplete, TransferId, TransferOffer, TransferStatus, TransferStatusRequest,
};
use quick_share_transfer::{
    noise::NoiseHandshake,
    offer::{AuthorizationToken, OfferManager, OfferPolicy},
    receiver::{ReceiverError, ReceiverPolicy, ReceiverService},
};
use std::{
    collections::BTreeSet,
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::tempdir;
use uuid::Uuid;

const CHUNK_SIZE: u32 = 256 * 1024;

fn peer_context(
    root: &std::path::Path,
) -> (
    quick_share_transfer::auth::PeerAuthContext,
    TrustedDeviceStore,
) {
    let local = IdentityStore::new(root.join("local.json"))
        .load_or_create()
        .expect("local identity");
    let remote = IdentityStore::new(root.join("remote.json"))
        .load_or_create()
        .expect("remote identity");
    let mut initiator =
        NoiseHandshake::initiator(&local, Duration::from_secs(5)).expect("initiator");
    let mut responder =
        NoiseHandshake::responder(&remote, Duration::from_secs(5)).expect("responder");
    let one = initiator.write_message().expect("one");
    responder.read_message(&one).expect("read one");
    let two = responder.write_message().expect("two");
    initiator.read_message(&two).expect("read two");
    let three = initiator.write_message().expect("three");
    responder.read_message(&three).expect("read three");
    let (_, evidence) = initiator.finish(None).expect("finish");
    let trust = TrustedDeviceStore::new(root.join("trusted.json"));
    let peer = quick_share_transfer::auth::classify_peer(
        quick_share_transfer::auth::PeerClaim {
            device_id: remote.device_id(),
            name: "sender".to_owned(),
        },
        &evidence,
        &trust,
    )
    .expect("peer context");
    (peer, trust)
}

fn file_entry(id: u32, path: &str, bytes: &[u8]) -> ManifestEntry {
    ManifestEntry {
        id: EntryId::new(id).expect("entry"),
        relative_path: path.to_owned(),
        kind: ManifestEntryKind::File,
        size: bytes.len() as u64,
        digest: Some(*blake3::hash(bytes).as_bytes()),
    }
}

fn offer(
    peer: &quick_share_transfer::auth::PeerAuthContext,
    transfer_id: TransferId,
    entries: Vec<ManifestEntry>,
) -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id,
        sender: DeviceInfo {
            device_id: peer.device_id().clone(),
            name: peer.claimed_name().to_owned(),
            capabilities: BTreeSet::from([
                Capability::Files,
                Capability::Directories,
                Capability::Symlinks,
                Capability::Resume,
            ]),
        },
        content_kind: ContentKind::Files,
        chunk_size: CHUNK_SIZE,
        total_bytes: entries.iter().map(|entry| entry.size).sum(),
        entries,
    }
}

async fn accepted(
    manager: &OfferManager,
    peer: &quick_share_transfer::auth::PeerAuthContext,
    offer: TransferOffer,
    now: Instant,
) -> AuthorizationToken {
    let submission = manager
        .create_offer(peer, None, offer, now)
        .expect("offer submission");
    manager
        .decide(
            submission.view.transfer_id,
            OfferDecision::AcceptOnce,
            false,
            now,
        )
        .expect("accept");
    submission
        .resolution
        .await
        .expect("resolution")
        .authorization
        .expect("token")
}

fn proof(token: &AuthorizationToken) -> AuthorizationProof {
    token.with_bytes(|bytes| AuthorizationProof::from_bytes(*bytes))
}

fn descriptor(
    transfer_id: TransferId,
    entry_id: EntryId,
    index: u32,
    offset: u64,
    bytes: &[u8],
) -> ChunkDescriptor {
    ChunkDescriptor {
        transfer_id,
        entry_id,
        index,
        offset,
        length: bytes.len() as u32,
        digest: *blake3::hash(bytes).as_bytes(),
    }
}

#[tokio::test]
async fn authorized_fragmented_chunks_resume_and_commit_files_directories_and_safe_links() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path());
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let transfer_id = TransferId::new(Uuid::now_v7());
    let payload = b"fragmented payload";
    let entries = vec![
        ManifestEntry {
            id: EntryId::new(1).expect("dir"),
            relative_path: "docs".to_owned(),
            kind: ManifestEntryKind::Directory,
            size: 0,
            digest: None,
        },
        file_entry(2, "docs/data.bin", payload),
        ManifestEntry {
            id: EntryId::new(3).expect("link"),
            relative_path: "latest".to_owned(),
            kind: ManifestEntryKind::Symlink {
                target: "docs/data.bin".to_owned(),
            },
            size: 0,
            digest: None,
        },
    ];
    let transfer_offer = offer(&peer, transfer_id, entries);
    let manifest_digest =
        *blake3::hash(&serde_json::to_vec(&transfer_offer).expect("offer JSON")).as_bytes();
    let now = Instant::now();
    let token = accepted(&manager, &peer, transfer_offer, now).await;
    let output = root.path().join("output");
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
    let receiver = ReceiverService::new_with_progress(
        manager,
        &output,
        ReceiverPolicy {
            conflict: ConflictPolicy::Error,
            ..ReceiverPolicy::default()
        },
        Some(progress_tx),
    )
    .expect("receiver");

    let initial = receiver
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id,
                authorization: proof(&token),
            },
            now,
        )
        .expect("initial status");
    assert_eq!(initial.status, TransferStatus::Accepted);
    assert!(initial.entries[0].chunks.is_missing(0));

    let chunk = descriptor(transfer_id, EntryId::new(2).expect("file"), 0, 0, payload);
    let request_id = RequestId::new(Uuid::now_v7());
    assert!(
        receiver
            .receive_chunk(
                peer.device_id(),
                request_id,
                ChunkData {
                    authorization: proof(&token),
                    descriptor: chunk.clone(),
                    fragment_offset: 0,
                    final_fragment: false,
                    payload: payload[..5].to_vec(),
                },
                now,
            )
            .expect("first fragment")
            .is_none()
    );
    let ack = receiver
        .receive_chunk(
            peer.device_id(),
            request_id,
            ChunkData {
                authorization: proof(&token),
                descriptor: chunk,
                fragment_offset: 5,
                final_fragment: true,
                payload: payload[5..].to_vec(),
            },
            now,
        )
        .expect("last fragment")
        .expect("ACK");
    assert!(!ack.duplicate);

    let complete = receiver
        .complete(
            peer.device_id(),
            TransferComplete {
                transfer_id,
                authorization: proof(&token),
                manifest_digest,
            },
            now,
        )
        .expect("complete");
    assert_eq!(complete.status, TransferStatus::Completed);
    let duplicate_complete = receiver
        .complete(
            peer.device_id(),
            TransferComplete {
                transfer_id,
                authorization: proof(&token),
                manifest_digest,
            },
            now,
        )
        .expect("idempotent completion retry");
    assert_eq!(duplicate_complete.status, TransferStatus::Completed);
    assert_eq!(
        fs::read(output.join("docs/data.bin")).expect("payload"),
        payload
    );
    assert!(output.join("docs").is_dir());
    if fs::symlink_metadata(output.join("latest")).is_ok() {
        let stored_target = fs::read_link(output.join("latest")).expect("stored link target");
        assert_eq!(
            fs::read(output.join("latest"))
                .unwrap_or_else(|error| panic!("link target {stored_target:?}: {error}")),
            payload
        );
    } else {
        assert!(output.join("latest.quick-share-symlink.txt").exists());
    }

    let completed = receiver
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id,
                authorization: proof(&token),
            },
            now,
        )
        .expect("completed status");
    assert_eq!(completed.status, TransferStatus::Completed);
    assert!(!completed.entries[0].chunks.is_missing(0));
    let cancellation = receiver
        .cancellation_token(transfer_id)
        .expect("child cancellation token");
    assert!(!cancellation.is_cancelled());
    let mut progress = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        progress.push(event);
    }
    assert!(progress.windows(2).all(|events| {
        events[0].received_bytes <= events[1].received_bytes
            && events[1].received_bytes <= events[1].total_bytes
    }));
    assert!(
        progress
            .iter()
            .any(|event| event.status == TransferStatus::Completed)
    );
}

#[tokio::test]
async fn unauthorized_expired_conflicting_and_cancelled_uploads_fail_closed() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path());
    let policy = OfferPolicy {
        grant_ttl: Duration::from_millis(10),
        ..OfferPolicy::default()
    };
    let manager = Arc::new(OfferManager::new(trust, policy));
    let transfer_id = TransferId::new(Uuid::now_v7());
    let payload = b"payload";
    let transfer_offer = offer(&peer, transfer_id, vec![file_entry(1, "data", payload)]);
    let now = Instant::now();
    let token = accepted(&manager, &peer, transfer_offer, now).await;
    let receiver =
        ReceiverService::new(manager, root.path().join("out"), ReceiverPolicy::default())
            .expect("receiver");
    let chunk = descriptor(transfer_id, EntryId::new(1).expect("entry"), 0, 0, payload);

    let wrong_peer = quick_share_protocol::DeviceId::parse("qs_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        .expect("wrong peer");
    assert!(matches!(
        receiver.status(
            &wrong_peer,
            TransferStatusRequest {
                transfer_id,
                authorization: proof(&token),
            },
            now,
        ),
        Err(ReceiverError::Offer(_))
    ));
    let random = receiver.receive_chunk(
        peer.device_id(),
        RequestId::new(Uuid::now_v7()),
        ChunkData {
            authorization: AuthorizationProof::from_bytes([0; 32]),
            descriptor: chunk.clone(),
            fragment_offset: 0,
            final_fragment: true,
            payload: payload.to_vec(),
        },
        now,
    );
    assert!(matches!(random, Err(ReceiverError::Offer(_))));
    let expired = receiver.receive_chunk(
        peer.device_id(),
        RequestId::new(Uuid::now_v7()),
        ChunkData {
            authorization: proof(&token),
            descriptor: chunk.clone(),
            fragment_offset: 0,
            final_fragment: true,
            payload: payload.to_vec(),
        },
        now + Duration::from_millis(10),
    );
    assert!(matches!(expired, Err(ReceiverError::Offer(_))));

    // A fresh manager is used to exercise cancellation without an expired grant.
    let (peer, trust) = peer_context(&root.path().join("fresh"));
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let transfer_id = TransferId::new(Uuid::now_v7());
    let transfer_offer = offer(
        &peer,
        transfer_id,
        vec![file_entry(1, "cancelled", payload)],
    );
    let token = accepted(&manager, &peer, transfer_offer, now).await;
    let receiver = ReceiverService::new(
        manager,
        root.path().join("fresh-out"),
        ReceiverPolicy::default(),
    )
    .expect("receiver");
    receiver
        .cancel(
            peer.device_id(),
            TransferCancel {
                transfer_id,
                authorization: proof(&token),
                reason: quick_share_protocol::CancelReason::User,
            },
            now,
        )
        .expect("cancel");
    receiver
        .cancel(
            peer.device_id(),
            TransferCancel {
                transfer_id,
                authorization: proof(&token),
                reason: quick_share_protocol::CancelReason::User,
            },
            now,
        )
        .expect("idempotent cancel");
    assert!(
        receiver
            .cancellation_token(transfer_id)
            .expect("cancellation token")
            .is_cancelled()
    );
    let rejected = receiver.receive_chunk(
        peer.device_id(),
        RequestId::new(Uuid::now_v7()),
        ChunkData {
            authorization: proof(&token),
            descriptor: descriptor(transfer_id, EntryId::new(1).expect("entry"), 0, 0, payload),
            fragment_offset: 0,
            final_fragment: true,
            payload: payload.to_vec(),
        },
        now,
    );
    assert!(matches!(
        rejected,
        Err(ReceiverError::Terminal(TransferStatus::Cancelled))
    ));
    assert!(!root.path().join("fresh-out/cancelled").exists());
}

#[tokio::test]
async fn duplicate_chunks_ack_idempotently_and_full_digest_mismatch_never_exposes_final() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path());
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let transfer_id = TransferId::new(Uuid::now_v7());
    let payload = b"wire payload";
    let mut entry = file_entry(1, "bad-final.bin", payload);
    entry.digest = Some([0; 32]);
    let transfer_offer = offer(&peer, transfer_id, vec![entry]);
    let manifest_digest =
        *blake3::hash(&serde_json::to_vec(&transfer_offer).expect("offer JSON")).as_bytes();
    let now = Instant::now();
    let token = accepted(&manager, &peer, transfer_offer, now).await;
    let output = root.path().join("out");
    let receiver =
        ReceiverService::new(manager, &output, ReceiverPolicy::default()).expect("receiver");
    let descriptor = descriptor(transfer_id, EntryId::new(1).expect("entry"), 0, 0, payload);
    let send = |receiver: &ReceiverService| {
        receiver.receive_chunk(
            peer.device_id(),
            RequestId::new(Uuid::now_v7()),
            ChunkData {
                authorization: proof(&token),
                descriptor: descriptor.clone(),
                fragment_offset: 0,
                final_fragment: true,
                payload: payload.to_vec(),
            },
            now,
        )
    };
    assert!(!send(&receiver).expect("first").expect("ACK").duplicate);
    assert!(send(&receiver).expect("duplicate").expect("ACK").duplicate);
    let mut conflicting = descriptor.clone();
    conflicting.digest = [3; 32];
    assert!(matches!(
        receiver.receive_chunk(
            peer.device_id(),
            RequestId::new(Uuid::now_v7()),
            ChunkData {
                authorization: proof(&token),
                descriptor: conflicting,
                fragment_offset: 0,
                final_fragment: true,
                payload: payload.to_vec(),
            },
            now,
        ),
        Err(ReceiverError::Store(
            quick_share_transfer::StoreError::ConflictingChunk { .. }
        ))
    ));
    assert!(
        receiver
            .complete(
                peer.device_id(),
                TransferComplete {
                    transfer_id,
                    authorization: proof(&token),
                    manifest_digest,
                },
                now,
            )
            .is_err()
    );
    assert!(!output.join("bad-final.bin").exists());
    let status = receiver
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id,
                authorization: proof(&token),
            },
            now,
        )
        .expect("failed status");
    assert_eq!(status.status, TransferStatus::Failed);
}

#[tokio::test]
async fn receiver_task_and_file_stream_limits_apply_backpressure_and_disconnect_releases_slots() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path());
    let manager = Arc::new(OfferManager::new(
        trust,
        OfferPolicy {
            trusted_policy: TrustedPolicy::Confirm,
            ..OfferPolicy::default()
        },
    ));
    let now = Instant::now();
    let first_id = TransferId::new(Uuid::now_v7());
    let first_offer = offer(
        &peer,
        first_id,
        vec![file_entry(1, "one", b"ab"), file_entry(2, "two", b"cd")],
    );
    let first_token = accepted(&manager, &peer, first_offer, now).await;
    let second_id = TransferId::new(Uuid::now_v7());
    let second_offer = offer(&peer, second_id, vec![file_entry(1, "other", b"x")]);
    let second_token = accepted(&manager, &peer, second_offer, now).await;
    let receiver = ReceiverService::new(
        manager,
        root.path().join("out"),
        ReceiverPolicy {
            max_receive_tasks: 1,
            max_file_streams: 1,
            ..ReceiverPolicy::default()
        },
    )
    .expect("receiver");
    receiver
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: first_id,
                authorization: proof(&first_token),
            },
            now,
        )
        .expect("activate first");
    assert!(matches!(
        receiver.status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: second_id,
                authorization: proof(&second_token),
            },
            now,
        ),
        Err(ReceiverError::ReceiveTaskLimit)
    ));

    let first_request = RequestId::new(Uuid::now_v7());
    receiver
        .receive_chunk(
            peer.device_id(),
            first_request,
            ChunkData {
                authorization: proof(&first_token),
                descriptor: descriptor(first_id, EntryId::new(1).expect("one"), 0, 0, b"ab"),
                fragment_offset: 0,
                final_fragment: false,
                payload: b"a".to_vec(),
            },
            now,
        )
        .expect("active first stream");
    assert!(matches!(
        receiver.receive_chunk(
            peer.device_id(),
            RequestId::new(Uuid::now_v7()),
            ChunkData {
                authorization: proof(&first_token),
                descriptor: descriptor(first_id, EntryId::new(2).expect("two"), 0, 0, b"cd"),
                fragment_offset: 0,
                final_fragment: false,
                payload: b"c".to_vec(),
            },
            now,
        ),
        Err(ReceiverError::FileStreamLimit)
    ));
    assert_eq!(
        receiver.disconnect(peer.device_id()).expect("disconnect"),
        1
    );
    let status = receiver
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: first_id,
                authorization: proof(&first_token),
            },
            now,
        )
        .expect("paused status");
    assert_eq!(status.status, TransferStatus::Paused);
    assert!(
        status
            .entries
            .iter()
            .all(|entry| entry.chunks.is_missing(0))
    );
    receiver
        .receive_chunk(
            peer.device_id(),
            RequestId::new(Uuid::now_v7()),
            ChunkData {
                authorization: proof(&first_token),
                descriptor: descriptor(first_id, EntryId::new(1).expect("one"), 0, 0, b"ab"),
                fragment_offset: 0,
                final_fragment: false,
                payload: b"a".to_vec(),
            },
            now,
        )
        .expect("reactivated stream");
    assert_eq!(receiver.pause_all().expect("graceful snapshot"), 1);
    assert_eq!(
        receiver
            .status(
                peer.device_id(),
                TransferStatusRequest {
                    transfer_id: first_id,
                    authorization: proof(&first_token),
                },
                now,
            )
            .expect("snapshot status")
            .status,
        TransferStatus::Paused
    );
}
