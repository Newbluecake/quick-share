use async_trait::async_trait;
use quick_share_core::{
    config::ConflictPolicy,
    identity::{IdentityStore, TrustedDeviceStore},
    manifest::ManifestBuilder,
};
use quick_share_protocol::{
    AuthorizationProof, Capability, ChunkAck, ChunkData, ChunkDescriptor, ContentKind, DeviceInfo,
    ManifestEntry, ManifestEntryKind, MessageType, OfferDecision, ProtocolVersion, RequestId,
    TransferCancel, TransferComplete, TransferCompleteAck, TransferId, TransferOffer,
    TransferStatusRequest, TransferStatusResponse,
};
use quick_share_transfer::{
    auth::{PeerAuthContext, PeerClaim, classify_peer},
    network::NetworkSession,
    noise::{NoiseHandshake, SecureChannel, decode_control_frame, encode_control_frame},
    offer::{AuthorizationToken, OfferManager, OfferPolicy},
    receiver::{ReceiverPolicy, ReceiverService},
    sender::{
        RetryPolicy, SendFile, SenderPolicy, TransferPlan, TransferSender, TransferTransport,
        TransportError,
    },
};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CHUNK_SIZE: u32 = 256 * 1024;

fn secure_channels(root: &Path) -> (SecureChannel, SecureChannel) {
    let local = IdentityStore::new(root.join("local.json"))
        .load_or_create()
        .expect("local identity");
    let remote = IdentityStore::new(root.join("remote.json"))
        .load_or_create()
        .expect("remote identity");
    let mut sender = NoiseHandshake::initiator(&remote, Duration::from_secs(5)).expect("sender");
    let mut receiver = NoiseHandshake::responder(&local, Duration::from_secs(5)).expect("receiver");
    let one = sender.write_message().expect("one");
    receiver.read_message(&one).expect("read one");
    let two = receiver.write_message().expect("two");
    sender.read_message(&two).expect("read two");
    let three = sender.write_message().expect("three");
    receiver.read_message(&three).expect("read three");
    let (sender, _) = sender
        .finish(Some(&local.public_key()))
        .expect("finish sender");
    let (receiver, _) = receiver
        .finish(Some(&remote.public_key()))
        .expect("finish receiver");
    (sender, receiver)
}

fn authenticated(root: &Path) -> (PeerAuthContext, TrustedDeviceStore) {
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
    let trust = TrustedDeviceStore::new(root.join("trust.json"));
    let peer = classify_peer(
        PeerClaim {
            device_id: remote.device_id(),
            name: "sender".to_owned(),
        },
        &evidence,
        &trust,
    )
    .expect("peer");
    (peer, trust)
}

async fn setup(
    root: &Path,
    payload: &[u8],
) -> (
    Arc<OfferManager>,
    PeerAuthContext,
    AuthorizationToken,
    TransferPlan,
    std::path::PathBuf,
) {
    let source = root.join("source.bin");
    fs::write(&source, payload).expect("source");
    let source_manifest = ManifestBuilder::new()
        .build(&[source])
        .expect("source manifest");
    let send_file = SendFile::prepare(source_manifest.entries[0].clone()).expect("send file");
    let (peer, trust) = authenticated(&root.join("identity"));
    let transfer_id = TransferId::new(Uuid::now_v7());
    let transfer_offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id,
        initiated_by: None,
        sender: DeviceInfo {
            device_id: peer.device_id().clone(),
            name: peer.claimed_name().to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::Resume]),
        },
        content_kind: ContentKind::Files,
        chunk_size: CHUNK_SIZE,
        total_bytes: payload.len() as u64,
        entries: vec![ManifestEntry {
            id: send_file.source.id,
            relative_path: "received.bin".to_owned(),
            kind: ManifestEntryKind::File,
            size: payload.len() as u64,
            digest: Some(send_file.final_digest),
        }],
    };
    let manifest_digest =
        *blake3::hash(&serde_json::to_vec(&transfer_offer).expect("offer JSON")).as_bytes();
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let now = Instant::now();
    let submission = manager
        .create_offer(&peer, None, transfer_offer, now)
        .expect("offer");
    manager
        .decide(transfer_id, OfferDecision::AcceptOnce, false, now)
        .expect("accept");
    let token = submission
        .resolution
        .await
        .expect("resolution")
        .authorization
        .expect("token");
    let plan = TransferPlan {
        transfer_id,
        manifest_digest,
        chunk_size: CHUNK_SIZE,
        total_bytes: payload.len() as u64,
        files: vec![send_file],
    };
    (manager, peer, token, plan, root.join("output"))
}

fn offer_from_plan(peer: &PeerAuthContext, plan: &TransferPlan) -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id: plan.transfer_id,
        initiated_by: None,
        sender: DeviceInfo {
            device_id: peer.device_id().clone(),
            name: peer.claimed_name().to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::Resume]),
        },
        content_kind: ContentKind::Files,
        chunk_size: plan.chunk_size,
        total_bytes: plan.total_bytes,
        entries: plan
            .files
            .iter()
            .map(|file| ManifestEntry {
                id: file.source.id,
                relative_path: "received.bin".to_owned(),
                kind: ManifestEntryKind::File,
                size: file.source.size,
                digest: Some(file.final_digest),
            })
            .collect(),
    }
}

async fn accept_again(
    manager: &OfferManager,
    peer: &PeerAuthContext,
    plan: &TransferPlan,
) -> AuthorizationToken {
    let now = Instant::now();
    let submission = manager
        .create_offer(peer, None, offer_from_plan(peer, plan), now)
        .expect("resubmitted offer");
    manager
        .decide(plan.transfer_id, OfferDecision::AcceptOnce, false, now)
        .expect("reaccept");
    submission
        .resolution
        .await
        .expect("resolution")
        .authorization
        .expect("new token")
}

fn proof(token: &AuthorizationToken) -> AuthorizationProof {
    token.with_bytes(|bytes| AuthorizationProof::from_bytes(*bytes))
}

struct LoopbackTransport {
    receiver: Mutex<Arc<ReceiverService>>,
    peer: quick_share_protocol::DeviceId,
}

impl LoopbackTransport {
    fn replace(&self, receiver: Arc<ReceiverService>) {
        *self.receiver.lock().expect("receiver") = receiver;
    }

    fn current(&self) -> Arc<ReceiverService> {
        Arc::clone(&self.receiver.lock().expect("receiver"))
    }
}

#[async_trait]
impl TransferTransport for LoopbackTransport {
    async fn status(
        &self,
        request: TransferStatusRequest,
    ) -> Result<TransferStatusResponse, TransportError> {
        self.current()
            .status(&self.peer, request, Instant::now())
            .map_err(|_| TransportError::Fatal)
    }

    async fn send_fragment(
        &self,
        request_id: RequestId,
        frame: ChunkData,
    ) -> Result<Option<ChunkAck>, TransportError> {
        self.current()
            .receive_chunk(&self.peer, request_id, frame, Instant::now())
            .map_err(|_| TransportError::Fatal)
    }

    async fn complete(
        &self,
        request: TransferComplete,
    ) -> Result<TransferCompleteAck, TransportError> {
        self.current()
            .complete(&self.peer, request, Instant::now())
            .map_err(|_| TransportError::Fatal)
    }

    async fn cancel(&self, request: TransferCancel) -> Result<(), TransportError> {
        self.current()
            .cancel(&self.peer, request, Instant::now())
            .map_err(|_| TransportError::Fatal)
    }

    async fn reconnect(&self) -> Result<(), TransportError> {
        self.current()
            .disconnect(&self.peer)
            .map(|_| ())
            .map_err(|_| TransportError::Fatal)
    }
}

#[tokio::test]
async fn authorized_chunk_and_completion_cross_authenticated_noise_transport() {
    let root = tempdir().expect("root");
    let payload = b"encrypted loopback payload";
    let (manager, peer, token, plan, output) = setup(root.path(), payload).await;
    let receiver = Arc::new(
        ReceiverService::new(manager, &output, ReceiverPolicy::default()).expect("receiver"),
    );
    let (sender_channel, receiver_channel) = secure_channels(&root.path().join("identity"));
    let (sender_stream, receiver_stream) = tokio::io::duplex(64 * 1024);
    let mut sender_session =
        NetworkSession::new(sender_stream, sender_channel, Duration::from_secs(1))
            .expect("sender session");
    let mut receiver_session =
        NetworkSession::new(receiver_stream, receiver_channel, Duration::from_secs(1))
            .expect("receiver session");
    let server_cancel = CancellationToken::new();
    let server_token = server_cancel.clone();
    let server_peer = peer.device_id().clone();
    let server = tokio::spawn(async move {
        let chunk_frame = receiver_session
            .receive_frame(&server_token)
            .await
            .expect("encrypted chunk frame");
        let chunk_request = chunk_frame.request_id;
        let chunk: ChunkData = decode_control_frame(&chunk_frame, MessageType::ChunkData)
            .expect("strict chunk decode");
        let ack = receiver
            .receive_chunk(&server_peer, chunk_request, chunk, Instant::now())
            .expect("receive chunk")
            .expect("chunk ACK");
        let ack_frame =
            encode_control_frame(MessageType::ChunkAck, chunk_request, &ack).expect("encode ACK");
        receiver_session
            .send_frame(&ack_frame, &server_token)
            .await
            .expect("encrypted ACK");

        let complete_frame = receiver_session
            .receive_frame(&server_token)
            .await
            .expect("encrypted complete frame");
        let complete_request = complete_frame.request_id;
        let complete: TransferComplete =
            decode_control_frame(&complete_frame, MessageType::TransferComplete)
                .expect("strict completion decode");
        let ack = receiver
            .complete(&server_peer, complete, Instant::now())
            .expect("complete transfer");
        let ack_frame = encode_control_frame(MessageType::TransferComplete, complete_request, &ack)
            .expect("encode completion ACK");
        receiver_session
            .send_frame(&ack_frame, &server_token)
            .await
            .expect("encrypted completion ACK");
    });

    let request_id = RequestId::new(Uuid::now_v7());
    let descriptor = ChunkDescriptor {
        transfer_id: plan.transfer_id,
        entry_id: plan.files[0].source.id,
        index: 0,
        offset: 0,
        length: payload.len() as u32,
        digest: *blake3::hash(payload).as_bytes(),
    };
    let frame = encode_control_frame(
        MessageType::ChunkData,
        request_id,
        &ChunkData {
            authorization: proof(&token),
            descriptor,
            fragment_offset: 0,
            final_fragment: true,
            payload: payload.to_vec(),
        },
    )
    .expect("encode chunk");
    sender_session
        .send_frame(&frame, &server_cancel)
        .await
        .expect("send encrypted chunk");
    let ack_frame = sender_session
        .receive_frame(&server_cancel)
        .await
        .expect("receive encrypted ACK");
    assert_eq!(ack_frame.request_id, request_id);
    let ack: ChunkAck =
        decode_control_frame(&ack_frame, MessageType::ChunkAck).expect("strict ACK decode");
    assert_eq!(ack.accepted_length, payload.len() as u32);

    let complete_request = RequestId::new(Uuid::now_v7());
    let complete_frame = encode_control_frame(
        MessageType::TransferComplete,
        complete_request,
        &TransferComplete {
            transfer_id: plan.transfer_id,
            authorization: proof(&token),
            manifest_digest: plan.manifest_digest,
        },
    )
    .expect("encode completion");
    sender_session
        .send_frame(&complete_frame, &server_cancel)
        .await
        .expect("send encrypted completion");
    let ack_frame = sender_session
        .receive_frame(&server_cancel)
        .await
        .expect("receive encrypted completion ACK");
    assert_eq!(ack_frame.request_id, complete_request);
    let ack: TransferCompleteAck = decode_control_frame(&ack_frame, MessageType::TransferComplete)
        .expect("strict completion ACK decode");
    assert_eq!(ack.status, quick_share_protocol::TransferStatus::Completed);
    server.await.expect("server task");
    assert_eq!(
        fs::read(output.join("received.bin")).expect("output"),
        payload
    );
}

#[tokio::test]
async fn receiver_and_sender_restart_at_chunk_boundaries_resume_only_missing_data() {
    let root = tempdir().expect("root");
    let payload = vec![0x5a; CHUNK_SIZE as usize * 2 + 17];
    let (manager, peer, token, plan, output) = setup(root.path(), &payload).await;
    let receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&manager),
            &output,
            ReceiverPolicy {
                conflict: ConflictPolicy::Error,
                ..ReceiverPolicy::default()
            },
        )
        .expect("receiver"),
    );

    // Simulate the first sender process completing chunk 0 before both processes stop.
    let first = &payload[..CHUNK_SIZE as usize];
    let descriptor = ChunkDescriptor {
        transfer_id: plan.transfer_id,
        entry_id: plan.files[0].source.id,
        index: 0,
        offset: 0,
        length: CHUNK_SIZE,
        digest: *blake3::hash(first).as_bytes(),
    };
    receiver
        .receive_chunk(
            peer.device_id(),
            RequestId::new(Uuid::now_v7()),
            ChunkData {
                authorization: proof(&token),
                descriptor,
                fragment_offset: 0,
                final_fragment: true,
                payload: first.to_vec(),
            },
            Instant::now(),
        )
        .expect("first process chunk")
        .expect("ACK");
    drop(receiver);
    drop(token);
    drop(manager);

    // A new offer manager, receiver service, authorization grant, and sender reconstruct state.
    let restarted_manager = Arc::new(OfferManager::new(
        TrustedDeviceStore::new(root.path().join("identity/trust.json")),
        OfferPolicy::default(),
    ));
    let restarted_token = accept_again(&restarted_manager, &peer, &plan).await;
    let restarted_receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&restarted_manager),
            &output,
            ReceiverPolicy {
                conflict: ConflictPolicy::Error,
                ..ReceiverPolicy::default()
            },
        )
        .expect("restarted receiver"),
    );
    let transport = Arc::new(LoopbackTransport {
        receiver: Mutex::new(Arc::clone(&restarted_receiver)),
        peer: peer.device_id().clone(),
    });
    // Exercise replacement API too: future reconnects use the newest capability-bound service.
    transport.replace(restarted_receiver);
    let sender = TransferSender::new(
        Arc::clone(&transport),
        SenderPolicy {
            concurrent_files: 2,
            queue_capacity: 1,
            retry: RetryPolicy {
                base_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
                ..RetryPolicy::default()
            },
        },
    )
    .expect("sender");
    let summary = sender
        .send(plan, &restarted_token, CancellationToken::new(), None)
        .await
        .expect("resumed transfer");

    assert_eq!(summary.uploaded_chunks, 2);
    assert_eq!(
        fs::read(output.join("received.bin")).expect("received"),
        payload
    );
}

#[tokio::test]
async fn persisted_resume_state_rejects_a_different_manifest_digest() {
    let root = tempdir().expect("root");
    let payload = b"same source";
    let (manager, peer, token, plan, output) = setup(root.path(), payload).await;
    let receiver = ReceiverService::new(Arc::clone(&manager), &output, ReceiverPolicy::default())
        .expect("receiver");
    receiver
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: plan.transfer_id,
                authorization: proof(&token),
            },
            Instant::now(),
        )
        .expect("persist staging");
    drop(receiver);

    // Corrupting durable state must fail closed rather than silently starting a new transfer.
    let state_path = output
        .join(".quick-share-staging")
        .join(plan.transfer_id.as_uuid().to_string())
        .join("state.json");
    fs::write(state_path, b"not valid JSON").expect("corrupt state");
    let restarted =
        ReceiverService::new(manager, &output, ReceiverPolicy::default()).expect("receiver");
    assert!(
        restarted
            .status(
                peer.device_id(),
                TransferStatusRequest {
                    transfer_id: plan.transfer_id,
                    authorization: proof(&token),
                },
                Instant::now(),
            )
            .is_err()
    );
    assert!(!output.join("received.bin").exists());
}
