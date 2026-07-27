use async_trait::async_trait;
use quick_share_core::{
    config::ConflictPolicy,
    identity::{IdentityStore, TrustedDeviceStore},
    manifest::ManifestBuilder,
};
use quick_share_protocol::{
    Capability, ContentKind, DeviceInfo, InfoRequest, InfoResponse, ManifestEntry,
    ManifestEntryKind, OfferDecision, ProtocolVersion, TransferId, TransferOffer,
};
use quick_share_transfer::{
    direct::{
        ClientConnector, DirectError, NoiseClientTransport, OfferPrompt, ServerContext,
        ServerSessionOutcome,
    },
    offer::{OfferManager, OfferPolicy, OfferView},
    receiver::{ReceiverPolicy, ReceiverService},
    sender::{
        SendFile, SenderError, SenderPolicy, TextSendPlan, TransferPlan, TransferSender,
        TransportError,
    },
};
use std::{collections::BTreeSet, fs, sync::Arc, time::Duration};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct FixedPrompt(OfferDecision, bool);

#[async_trait]
impl OfferPrompt for FixedPrompt {
    async fn decide(&self, _view: &OfferView) -> Result<(OfferDecision, bool), DirectError> {
        Ok((self.0, self.1))
    }
}

struct SlowPrompt;

#[async_trait]
impl OfferPrompt for SlowPrompt {
    async fn decide(&self, _view: &OfferView) -> Result<(OfferDecision, bool), DirectError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok((OfferDecision::AcceptOnce, false))
    }
}

fn capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([
        Capability::Files,
        Capability::Directories,
        Capability::Symlinks,
        Capability::Text,
        Capability::Resume,
    ])
}

#[tokio::test]
async fn tcp_noise_offer_status_chunk_and_complete_reach_the_receiver() {
    let root = tempdir().expect("root");
    let sender_identity = Arc::new(
        IdentityStore::new(root.path().join("sender-identity.json"))
            .load_or_create()
            .expect("sender identity"),
    );
    let receiver_identity = Arc::new(
        IdentityStore::new(root.path().join("receiver-identity.json"))
            .load_or_create()
            .expect("receiver identity"),
    );
    let sender_info = DeviceInfo {
        device_id: sender_identity.device_id(),
        name: "sender".to_owned(),
        capabilities: capabilities(),
    };
    let receiver_info = DeviceInfo {
        device_id: receiver_identity.device_id(),
        name: "receiver".to_owned(),
        capabilities: capabilities(),
    };

    let source_path = root.path().join("payload.bin");
    let payload = vec![0x5a; 300_000];
    fs::write(&source_path, &payload).expect("source");
    let manifest = ManifestBuilder::new()
        .build(&[source_path])
        .expect("manifest");
    let send_file = SendFile::prepare(manifest.entries[0].clone()).expect("send file");
    let transfer_id = TransferId::new(Uuid::now_v7());
    let offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id,
        sender: sender_info.clone(),
        content_kind: ContentKind::Files,
        chunk_size: 256 * 1024,
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
        *blake3::hash(&serde_json::to_vec(&offer).expect("offer JSON")).as_bytes();
    let plan = TransferPlan {
        transfer_id,
        manifest_digest,
        chunk_size: offer.chunk_size,
        total_bytes: offer.total_bytes,
        files: vec![send_file],
    };

    let trust_store = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let offers = Arc::new(OfferManager::new(
        trust_store.clone(),
        OfferPolicy::default(),
    ));
    let output = root.path().join("output");
    let receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&offers),
            &output,
            ReceiverPolicy {
                conflict: ConflictPolicy::Error,
                ..ReceiverPolicy::default()
            },
        )
        .expect("receiver service"),
    );
    let server = Arc::new(ServerContext {
        identity: Arc::clone(&receiver_identity),
        trust_store,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_0,
            device: receiver_info,
        },
        offers,
        receiver,
        prompt: Arc::new(FixedPrompt(OfferDecision::AcceptOnce, false)),
        operation_timeout: Duration::from_secs(5),
    });
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        loop {
            let (stream, peer_address) = listener.accept().await.expect("accept");
            let outcome = server
                .serve_stream(stream, peer_address, CancellationToken::new())
                .await?;
            if matches!(outcome, ServerSessionOutcome::TransferCompleted { .. }) {
                return Ok::<_, DirectError>(outcome);
            }
        }
    });

    let connector = ClientConnector::new(
        address,
        sender_identity,
        Some(receiver_identity.device_id()),
        None,
        InfoRequest {
            protocol_version: ProtocolVersion::V1_0,
            capabilities: capabilities(),
        },
        Duration::from_secs(5),
    );
    let connected = connector.connect().await.expect("connect and INFO");
    let transport = Arc::new(NoiseClientTransport::new(connector, connected));
    let token = transport.create_offer(offer).await.expect("accepted offer");
    // Resume over a new authenticated socket without replaying the accepted offer.
    transport.disconnect().await;
    let summary = TransferSender::new(Arc::clone(&transport), SenderPolicy::default())
        .expect("sender")
        .send(plan, &token, CancellationToken::new(), None)
        .await
        .expect("transfer");
    assert_eq!(summary.total_bytes, payload.len() as u64);
    assert!(matches!(
        server_task.await.expect("server task").expect("server"),
        ServerSessionOutcome::TransferCompleted { transfer_id: completed, .. }
            if completed == transfer_id
    ));
    assert_eq!(
        fs::read(output.join("received.bin")).expect("received"),
        payload
    );
}

#[tokio::test]
async fn confirmation_timeout_expires_the_offer_without_authorization() {
    let root = tempdir().expect("root");
    let sender_identity = Arc::new(
        IdentityStore::new(root.path().join("sender.json"))
            .load_or_create()
            .expect("sender"),
    );
    let receiver_identity = Arc::new(
        IdentityStore::new(root.path().join("receiver.json"))
            .load_or_create()
            .expect("receiver"),
    );
    let transfer_id = TransferId::new(Uuid::now_v7());
    let entry_id = quick_share_protocol::EntryId::new(1).expect("entry");
    let offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id,
        sender: DeviceInfo {
            device_id: sender_identity.device_id(),
            name: "sender".to_owned(),
            capabilities: capabilities(),
        },
        content_kind: ContentKind::Text,
        chunk_size: 4 * 1024 * 1024,
        total_bytes: 0,
        entries: vec![ManifestEntry {
            id: entry_id,
            relative_path: "quick-share-text.txt".to_owned(),
            kind: ManifestEntryKind::Text {
                media_type: "text/plain;charset=utf-8;source=literal".to_owned(),
            },
            size: 0,
            digest: Some(*blake3::hash(b"").as_bytes()),
        }],
    };
    let trust_store = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let offers = Arc::new(OfferManager::new(
        trust_store.clone(),
        OfferPolicy {
            confirmation_timeout: Duration::from_millis(20),
            ..OfferPolicy::default()
        },
    ));
    let receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&offers),
            root.path().join("output"),
            ReceiverPolicy::default(),
        )
        .expect("receiver"),
    );
    let server = Arc::new(ServerContext {
        identity: Arc::clone(&receiver_identity),
        trust_store,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_0,
            device: DeviceInfo {
                device_id: receiver_identity.device_id(),
                name: "receiver".to_owned(),
                capabilities: capabilities(),
            },
        },
        offers,
        receiver,
        prompt: Arc::new(SlowPrompt),
        operation_timeout: Duration::from_secs(2),
    });
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        let (stream, peer_address) = listener.accept().await.expect("accept");
        server
            .serve_stream(stream, peer_address, CancellationToken::new())
            .await
    });
    let connector = ClientConnector::new(
        address,
        sender_identity,
        Some(receiver_identity.device_id()),
        None,
        InfoRequest {
            protocol_version: ProtocolVersion::V1_0,
            capabilities: capabilities(),
        },
        Duration::from_secs(2),
    );
    let connected = connector.connect().await.expect("connect");
    let transport = NoiseClientTransport::new(connector, connected);
    assert!(matches!(
        transport.create_offer(offer).await,
        Err(DirectError::Rejected)
    ));
    assert_eq!(
        server_task.await.expect("task").expect("server"),
        ServerSessionOutcome::OfferRejected
    );
}

#[tokio::test]
async fn unicode_and_empty_text_cross_noise_without_a_plaintext_temp_file() {
    for (text, corrupt_digest) in [
        ("你好🚀\nnot executed: $(touch nope)", false),
        ("", false),
        ("integrity must fail", true),
    ] {
        let root = tempdir().expect("root");
        let sender_identity = Arc::new(
            IdentityStore::new(root.path().join("sender.json"))
                .load_or_create()
                .expect("sender"),
        );
        let receiver_identity = Arc::new(
            IdentityStore::new(root.path().join("receiver.json"))
                .load_or_create()
                .expect("receiver"),
        );
        let sender_device_id = sender_identity.device_id();
        let sender_info = DeviceInfo {
            device_id: sender_device_id.clone(),
            name: "sender".to_owned(),
            capabilities: capabilities(),
        };
        let entry_id = quick_share_protocol::EntryId::new(1).expect("entry");
        let transfer_id = TransferId::new(Uuid::now_v7());
        let offer = TransferOffer {
            protocol_version: ProtocolVersion::V1_0,
            transfer_id,
            sender: sender_info,
            content_kind: ContentKind::Text,
            chunk_size: 4 * 1024 * 1024,
            total_bytes: text.len() as u64,
            entries: vec![ManifestEntry {
                id: entry_id,
                relative_path: "quick-share-text.txt".to_owned(),
                kind: ManifestEntryKind::Text {
                    media_type: "text/plain;charset=utf-8;source=clipboard".to_owned(),
                },
                size: text.len() as u64,
                digest: Some(if corrupt_digest {
                    [0; 32]
                } else {
                    *blake3::hash(text.as_bytes()).as_bytes()
                }),
            }],
        };
        let manifest_digest =
            *blake3::hash(&serde_json::to_vec(&offer).expect("offer JSON")).as_bytes();
        let trust_store = TrustedDeviceStore::new(root.path().join("trusted.toml"));
        let offers = Arc::new(OfferManager::new(
            trust_store.clone(),
            OfferPolicy::default(),
        ));
        let output = root.path().join("output");
        let receiver = Arc::new(
            ReceiverService::new(Arc::clone(&offers), &output, ReceiverPolicy::default())
                .expect("receiver"),
        );
        let server = Arc::new(ServerContext {
            identity: Arc::clone(&receiver_identity),
            trust_store,
            local_info: InfoResponse {
                protocol_version: ProtocolVersion::V1_0,
                device: DeviceInfo {
                    device_id: receiver_identity.device_id(),
                    name: "receiver".to_owned(),
                    capabilities: capabilities(),
                },
            },
            offers,
            receiver: Arc::clone(&receiver),
            prompt: Arc::new(FixedPrompt(OfferDecision::AcceptOnce, false)),
            operation_timeout: Duration::from_secs(5),
        });
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let server_task = tokio::spawn(async move {
            let (stream, address) = listener.accept().await.expect("accept");
            server
                .serve_stream(stream, address, CancellationToken::new())
                .await
        });
        let connector = ClientConnector::new(
            address,
            sender_identity,
            Some(receiver_identity.device_id()),
            None,
            InfoRequest {
                protocol_version: ProtocolVersion::V1_0,
                capabilities: capabilities(),
            },
            Duration::from_secs(5),
        );
        let connected = connector.connect().await.expect("connect");
        let transport = Arc::new(NoiseClientTransport::new(connector, connected));
        let token = transport
            .create_offer(offer)
            .await
            .expect("accepted text offer");
        let result = TransferSender::new(Arc::clone(&transport), SenderPolicy::default())
            .expect("sender")
            .send_text(
                TextSendPlan {
                    transfer_id,
                    manifest_digest,
                    entry_id,
                    bytes: Arc::from(text.as_bytes()),
                },
                &token,
                CancellationToken::new(),
                None,
            )
            .await;
        if corrupt_digest {
            assert!(matches!(
                result,
                Err(SenderError::Transport(TransportError::Integrity))
            ));
            transport.disconnect().await;
            assert_eq!(
                server_task.await.expect("task").expect("server"),
                ServerSessionOutcome::Disconnected
            );
            assert!(!output.join("quick-share-text.txt").exists());
            continue;
        }
        result.expect("text transfer");
        assert!(matches!(
            server_task.await.expect("task").expect("server"),
            ServerSessionOutcome::TransferCompleted { transfer_id: completed, .. }
                if completed == transfer_id
        ));
        let wrong_peer =
            quick_share_protocol::DeviceId::parse("qs_cccccccccccccccccccccccccccccccc")
                .expect("wrong peer");
        assert!(receiver.completed_text(&wrong_peer, transfer_id).is_err());
        let received = receiver
            .completed_text(&sender_device_id, transfer_id)
            .expect("authorized text")
            .expect("text payload");
        assert_eq!(received.as_str(), text);
        assert_eq!(
            received.source(),
            quick_share_transfer::text::TextSource::Clipboard
        );
        assert!(!output.join("quick-share-text.txt").exists());
    }
}
