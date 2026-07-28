use async_trait::async_trait;
use quick_share_core::{
    config::ConflictPolicy,
    identity::{IdentityStore, TrustedDeviceStore},
};
use quick_share_protocol::{
    Capability, ContentKind, DeviceInfo, EntryId, InfoRequest, InfoResponse, ManifestEntry,
    ManifestEntryKind, ProtocolVersion, RequestId, SourceSelectionRequest, SourceSelectionStatus,
    TransferId, TransferOffer,
};
use quick_share_transfer::{
    auth::PeerAuthContext,
    direct::{
        ClientConnector, DirectError, NoiseClientTransport, OfferPrompt, SelectionDispatcher,
        ServerContext, ServerSessionOutcome,
    },
    expected_offer::{ExpectedCallback, ExpectedOfferPolicy, ExpectedOfferRegistry},
    offer::{OfferManager, OfferPolicy, OfferView},
    receiver::{ReceiverPolicy, ReceiverService},
    selection::{
        SelectionAuthorization, SelectionHandler, SelectionHandlerError, SelectionManager,
        SelectionPolicy,
    },
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct RejectOffers;

#[async_trait]
impl OfferPrompt for RejectOffers {
    async fn prepare_expected(
        &self,
        _peer: &PeerAuthContext,
        _view: &OfferView,
        _offer: &TransferOffer,
    ) -> Result<(), DirectError> {
        Ok(())
    }

    async fn decide(
        &self,
        _peer: &PeerAuthContext,
        _view: &OfferView,
        _offer: &TransferOffer,
    ) -> Result<(quick_share_protocol::OfferDecision, bool), DirectError> {
        Ok((
            quick_share_protocol::OfferDecision::Reject {
                reason: quick_share_protocol::RejectionReason::UserRejected,
            },
            false,
        ))
    }
}

struct CountingPrompt(AtomicUsize);

#[async_trait]
impl OfferPrompt for CountingPrompt {
    async fn prepare_expected(
        &self,
        _peer: &PeerAuthContext,
        _view: &OfferView,
        _offer: &TransferOffer,
    ) -> Result<(), DirectError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn decide(
        &self,
        _peer: &PeerAuthContext,
        _view: &OfferView,
        _offer: &TransferOffer,
    ) -> Result<(quick_share_protocol::OfferDecision, bool), DirectError> {
        panic!("an expected callback must not enter the ordinary offer prompt")
    }
}

struct ReadySelection;

#[async_trait]
impl SelectionHandler for ReadySelection {
    async fn authorize(
        &self,
        _peer: &PeerAuthContext,
        _cancellation: CancellationToken,
    ) -> Result<SelectionAuthorization, SelectionHandlerError> {
        Ok(SelectionAuthorization::AcceptOnce)
    }

    async fn select_source(
        &self,
        _peer: &PeerAuthContext,
        _cancellation: CancellationToken,
    ) -> Result<TransferId, SelectionHandlerError> {
        Ok(TransferId::new(Uuid::now_v7()))
    }
}

fn capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([Capability::Files, Capability::RemoteSelection])
}

#[tokio::test]
async fn direct_qsp_1_1_dispatches_selection_and_uses_authenticated_source_ip() {
    let root = tempdir().expect("root");
    let requester = Arc::new(
        IdentityStore::new(root.path().join("requester.json"))
            .load_or_create()
            .expect("requester"),
    );
    let agent = Arc::new(
        IdentityStore::new(root.path().join("agent.json"))
            .load_or_create()
            .expect("agent"),
    );
    let trust = TrustedDeviceStore::new(root.path().join("trust.json"));
    let offers = Arc::new(OfferManager::new(trust.clone(), OfferPolicy::default()));
    let receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&offers),
            root.path().join("output"),
            ReceiverPolicy {
                conflict: ConflictPolicy::Error,
                ..ReceiverPolicy::default()
            },
        )
        .expect("receiver"),
    );
    let selection: Arc<dyn SelectionDispatcher> = Arc::new(
        SelectionManager::new(
            Arc::new(ReadySelection),
            trust.clone(),
            SelectionPolicy::default(),
        )
        .expect("selection manager"),
    );
    let server = Arc::new(ServerContext {
        identity: Arc::clone(&agent),
        trust_store: trust,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_1,
            device: DeviceInfo {
                device_id: agent.device_id(),
                name: "agent".to_owned(),
                capabilities: capabilities(),
            },
        },
        offers,
        receiver,
        prompt: Arc::new(RejectOffers),
        selection: Some(selection),
        expected_offers: None,
        operation_timeout: Duration::from_secs(5),
    });
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.expect("accept");
        server
            .serve_stream(stream, peer, CancellationToken::new())
            .await
    });

    let connector = ClientConnector::new(
        address,
        Arc::clone(&requester),
        Some(agent.device_id()),
        Some(agent.public_key()),
        InfoRequest {
            protocol_version: ProtocolVersion::V1_1,
            capabilities: capabilities(),
        },
        Duration::from_secs(5),
    );
    let connected = connector.connect().await.expect("connect");
    assert_eq!(connected.negotiated.version, ProtocolVersion::V1_1);
    let transport = NoiseClientTransport::new(connector, connected);
    let exchange = transport
        .request_source_selection(SourceSelectionRequest {
            requester: DeviceInfo {
                device_id: requester.device_id(),
                name: "requester".to_owned(),
                capabilities: capabilities(),
            },
            callback_port: 43123,
        })
        .await
        .expect("selection exchange");
    assert_eq!(exchange.response.status, SourceSelectionStatus::Ready);

    let outcome = task.await.expect("task").expect("server");
    assert!(matches!(
        outcome,
        ServerSessionOutcome::SelectionReady {
            peer,
            request_id,
            transfer_id,
            callback,
        } if peer == requester.device_id()
            && request_id == exchange.request_id
            && Some(transfer_id) == exchange.response.transfer_id
            && callback.endpoint.ip().is_loopback()
            && callback.endpoint.port() == 43123
            && callback.requester_public_key == requester.public_key()
    ));
}

#[tokio::test]
async fn exact_expected_callback_offer_is_auto_accepted_once_without_prompt() {
    let root = tempdir().expect("root");
    let callback_sender = Arc::new(
        IdentityStore::new(root.path().join("callback-sender.json"))
            .load_or_create()
            .expect("callback sender"),
    );
    let requester = Arc::new(
        IdentityStore::new(root.path().join("requester.json"))
            .load_or_create()
            .expect("requester"),
    );
    let request_id = RequestId::new(Uuid::now_v7());
    let transfer_id = TransferId::new(Uuid::now_v7());
    let expected = Arc::new(
        ExpectedOfferRegistry::new(ExpectedOfferPolicy::default()).expect("expected registry"),
    );
    expected
        .register(
            ExpectedCallback {
                request_id,
                transfer_id,
                sender_device_id: callback_sender.device_id(),
                sender_public_key: callback_sender.public_key(),
                source_ip: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                expires_at: Instant::now() + Duration::from_secs(30),
            },
            Instant::now(),
        )
        .expect("register expectation");
    let trust = TrustedDeviceStore::new(root.path().join("trust.json"));
    let offers = Arc::new(OfferManager::new(trust.clone(), OfferPolicy::default()));
    let receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&offers),
            root.path().join("output"),
            ReceiverPolicy::default(),
        )
        .expect("receiver"),
    );
    let prompt = Arc::new(CountingPrompt(AtomicUsize::new(0)));
    let server = Arc::new(ServerContext {
        identity: Arc::clone(&requester),
        trust_store: trust,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_1,
            device: DeviceInfo {
                device_id: requester.device_id(),
                name: "requester".to_owned(),
                capabilities: capabilities(),
            },
        },
        offers,
        receiver,
        prompt: Arc::clone(&prompt),
        selection: None,
        expected_offers: Some(expected),
        operation_timeout: Duration::from_secs(5),
    });
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.expect("accept");
        server
            .serve_stream(stream, peer, CancellationToken::new())
            .await
    });
    let connector = ClientConnector::new(
        address,
        Arc::clone(&callback_sender),
        Some(requester.device_id()),
        Some(requester.public_key()),
        InfoRequest {
            protocol_version: ProtocolVersion::V1_1,
            capabilities: capabilities(),
        },
        Duration::from_secs(5),
    );
    let connected = connector.connect().await.expect("connect");
    let transport = NoiseClientTransport::new(connector, connected);
    let offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_1,
        transfer_id,
        initiated_by: Some(request_id),
        sender: DeviceInfo {
            device_id: callback_sender.device_id(),
            name: "callback sender".to_owned(),
            capabilities: capabilities(),
        },
        content_kind: ContentKind::Files,
        chunk_size: 256 * 1024,
        total_bytes: 0,
        entries: vec![ManifestEntry {
            id: EntryId::new(1).expect("entry"),
            relative_path: "empty.txt".to_owned(),
            kind: ManifestEntryKind::File,
            size: 0,
            digest: Some(*blake3::hash(b"").as_bytes()),
        }],
    };
    transport
        .create_offer(offer)
        .await
        .expect("expected callback accepted");
    assert_eq!(prompt.0.load(Ordering::SeqCst), 1);
    transport.disconnect().await;
    assert_eq!(
        task.await.expect("task").expect("server"),
        ServerSessionOutcome::Disconnected
    );
}

#[tokio::test]
async fn qsp_1_0_info_hides_selection_and_client_cannot_send_message_11() {
    let root = tempdir().expect("root");
    let requester = Arc::new(
        IdentityStore::new(root.path().join("requester.json"))
            .load_or_create()
            .expect("requester"),
    );
    let agent = Arc::new(
        IdentityStore::new(root.path().join("agent.json"))
            .load_or_create()
            .expect("agent"),
    );
    let trust = TrustedDeviceStore::new(root.path().join("trust.json"));
    let offers = Arc::new(OfferManager::new(trust.clone(), OfferPolicy::default()));
    let receiver = Arc::new(
        ReceiverService::new(
            Arc::clone(&offers),
            root.path().join("output"),
            ReceiverPolicy::default(),
        )
        .expect("receiver"),
    );
    let server = Arc::new(ServerContext {
        identity: Arc::clone(&agent),
        trust_store: trust,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_1,
            device: DeviceInfo {
                device_id: agent.device_id(),
                name: "agent".to_owned(),
                capabilities: capabilities(),
            },
        },
        offers,
        receiver,
        prompt: Arc::new(RejectOffers),
        selection: None,
        expected_offers: None,
        operation_timeout: Duration::from_secs(5),
    });
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.expect("accept");
        server
            .serve_stream(stream, peer, CancellationToken::new())
            .await
    });
    let connector = ClientConnector::new(
        address,
        Arc::clone(&requester),
        Some(agent.device_id()),
        None,
        InfoRequest {
            protocol_version: ProtocolVersion::V1_0,
            capabilities: BTreeSet::from([Capability::Files]),
        },
        Duration::from_secs(5),
    );
    let connected = connector.connect().await.expect("connect");
    assert_eq!(
        connected.remote_info.protocol_version,
        ProtocolVersion::V1_0
    );
    assert!(
        !connected
            .remote_info
            .device
            .capabilities
            .contains(&Capability::RemoteSelection)
    );
    let transport = NoiseClientTransport::new(connector, connected);
    assert!(matches!(
        transport
            .request_source_selection(SourceSelectionRequest {
                requester: DeviceInfo {
                    device_id: requester.device_id(),
                    name: "requester".to_owned(),
                    capabilities: BTreeSet::from([Capability::Files]),
                },
                callback_port: 4242,
            })
            .await,
        Err(DirectError::Noise(_))
    ));
    transport.disconnect().await;
    assert_eq!(
        task.await.expect("task").expect("server"),
        ServerSessionOutcome::Disconnected
    );
}
