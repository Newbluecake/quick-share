use async_trait::async_trait;
use quick_share_core::identity::{IdentityStore, TrustStatus, TrustedDeviceStore};
use quick_share_protocol::{
    Capability, DeviceInfo, RequestId, SourceSelectionRequest, SourceSelectionStatus, TransferId,
};
use quick_share_transfer::{
    auth::{PeerAuthContext, PeerClaim, classify_peer},
    noise::NoiseHandshake,
    selection::{
        SelectionAuthorization, SelectionError, SelectionHandler, SelectionHandlerError,
        SelectionManager, SelectionPolicy,
    },
};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum PeerMode {
    Unknown,
    Trusted,
    Changed,
}

fn peer_context(root: &Path, mode: PeerMode) -> (PeerAuthContext, TrustedDeviceStore) {
    let local = IdentityStore::new(root.join("local.json"))
        .load_or_create()
        .expect("local");
    let pinned = IdentityStore::new(root.join("pinned.json"))
        .load_or_create()
        .expect("pinned");
    let pinned_device_id = pinned.device_id();
    let pinned_public_key = pinned.public_key();
    let presented = match mode {
        PeerMode::Unknown | PeerMode::Trusted => pinned,
        PeerMode::Changed => IdentityStore::new(root.join("changed.json"))
            .load_or_create()
            .expect("changed"),
    };
    let trust = TrustedDeviceStore::new(root.join("trust.json"));
    if matches!(mode, PeerMode::Trusted | PeerMode::Changed) {
        trust
            .trust_peer(pinned_device_id.clone(), "requester", pinned_public_key)
            .expect("pin");
    }
    let mut initiator = NoiseHandshake::initiator(&local, Duration::from_secs(5)).expect("init");
    let mut responder =
        NoiseHandshake::responder(&presented, Duration::from_secs(5)).expect("resp");
    let one = initiator.write_message().expect("one");
    responder.read_message(&one).expect("read one");
    let two = responder.write_message().expect("two");
    initiator.read_message(&two).expect("read two");
    let three = initiator.write_message().expect("three");
    responder.read_message(&three).expect("read three");
    let (_, evidence) = initiator.finish(None).expect("finish");
    let peer = classify_peer(
        PeerClaim {
            device_id: pinned_device_id,
            name: "requester".to_owned(),
        },
        &evidence,
        &trust,
    )
    .expect("classify");
    (peer, trust)
}

struct FakeHandler {
    authorization: SelectionAuthorization,
    selection_result: Result<TransferId, SelectionHandlerError>,
    delay: Duration,
    authorization_calls: AtomicUsize,
    selection_calls: AtomicUsize,
    completed: AtomicUsize,
}

impl FakeHandler {
    fn ready(authorization: SelectionAuthorization, delay: Duration) -> Self {
        Self {
            authorization,
            selection_result: Ok(TransferId::new(Uuid::now_v7())),
            delay,
            authorization_calls: AtomicUsize::new(0),
            selection_calls: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SelectionHandler for FakeHandler {
    async fn authorize(
        &self,
        _peer: &PeerAuthContext,
        cancellation: CancellationToken,
    ) -> Result<SelectionAuthorization, SelectionHandlerError> {
        self.authorization_calls.fetch_add(1, Ordering::SeqCst);
        tokio::select! {
            () = cancellation.cancelled() => Err(SelectionHandlerError::Failed),
            () = tokio::time::sleep(self.delay) => Ok(self.authorization),
        }
    }

    async fn select_source(
        &self,
        _peer: &PeerAuthContext,
        cancellation: CancellationToken,
    ) -> Result<TransferId, SelectionHandlerError> {
        self.selection_calls.fetch_add(1, Ordering::SeqCst);
        tokio::select! {
            () = cancellation.cancelled() => Err(SelectionHandlerError::Failed),
            () = tokio::time::sleep(self.delay) => {
                self.completed.fetch_add(1, Ordering::SeqCst);
                self.selection_result
            },
        }
    }
}

fn request(peer: &PeerAuthContext, port: u16) -> SourceSelectionRequest {
    SourceSelectionRequest {
        requester: DeviceInfo {
            device_id: peer.device_id().clone(),
            name: peer.claimed_name().to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::RemoteSelection]),
        },
        callback_port: port,
    }
}

#[tokio::test]
async fn trusted_duplicate_request_is_idempotent_and_uses_observed_callback_ip() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path(), PeerMode::Trusted);
    let handler = Arc::new(FakeHandler::ready(
        SelectionAuthorization::AcceptOnce,
        Duration::from_millis(20),
    ));
    let manager = Arc::new(
        SelectionManager::new(Arc::clone(&handler), trust, SelectionPolicy::default())
            .expect("manager"),
    );
    let request_id = RequestId::new(Uuid::now_v7());
    let observed = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8));
    let first = {
        let manager = Arc::clone(&manager);
        let peer = peer.clone();
        tokio::spawn(async move {
            manager
                .handle(
                    request_id,
                    peer.clone(),
                    observed,
                    request(&peer, 4242),
                    Instant::now(),
                )
                .await
        })
    };
    let second = manager.handle(
        request_id,
        peer.clone(),
        observed,
        request(&peer, 4242),
        Instant::now(),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("task").expect("first");
    let second = second.expect("second");

    assert_eq!(first, second);
    assert_eq!(first.response.status, SourceSelectionStatus::Ready);
    assert_eq!(
        first.callback.expect("callback").endpoint.to_string(),
        "10.0.0.8:4242"
    );
    assert_eq!(handler.authorization_calls.load(Ordering::SeqCst), 0);
    assert_eq!(handler.selection_calls.load(Ordering::SeqCst), 1);
    let cached = manager
        .handle(
            request_id,
            peer.clone(),
            observed,
            request(&peer, 4242),
            Instant::now(),
        )
        .await
        .expect("cached");
    assert_eq!(cached, second);
    assert_eq!(handler.selection_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        manager
            .handle(
                request_id,
                peer.clone(),
                observed,
                request(&peer, 4243),
                Instant::now(),
            )
            .await,
        Err(SelectionError::Replay)
    ));
}

#[tokio::test]
async fn unknown_accept_once_does_not_trust_accept_and_trust_does_and_changed_rejects() {
    for (authorization, should_trust) in [
        (SelectionAuthorization::AcceptOnce, false),
        (
            SelectionAuthorization::AcceptAndTrust { sas_verified: true },
            true,
        ),
    ] {
        let root = tempdir().expect("root");
        let (peer, trust) = peer_context(root.path(), PeerMode::Unknown);
        let handler = Arc::new(FakeHandler::ready(authorization, Duration::ZERO));
        let manager = SelectionManager::new(
            Arc::clone(&handler),
            trust.clone(),
            SelectionPolicy::default(),
        )
        .expect("manager");
        let result = manager
            .handle(
                RequestId::new(Uuid::now_v7()),
                peer.clone(),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                request(&peer, 4242),
                Instant::now(),
            )
            .await
            .expect("selection");
        assert_eq!(result.response.status, SourceSelectionStatus::Ready);
        assert_eq!(handler.authorization_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            matches!(
                trust
                    .check(peer.device_id(), &peer.public_key())
                    .expect("trust check"),
                TrustStatus::Trusted(_)
            ),
            should_trust
        );
    }

    let root = tempdir().expect("changed root");
    let (peer, trust) = peer_context(root.path(), PeerMode::Changed);
    let handler = Arc::new(FakeHandler::ready(
        SelectionAuthorization::AcceptOnce,
        Duration::ZERO,
    ));
    let manager = SelectionManager::new(Arc::clone(&handler), trust, SelectionPolicy::default())
        .expect("manager");
    let result = manager
        .handle(
            RequestId::new(Uuid::now_v7()),
            peer.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            request(&peer, 4242),
            Instant::now(),
        )
        .await
        .expect("changed result");
    assert_eq!(result.response.status, SourceSelectionStatus::Rejected);
    assert_eq!(handler.authorization_calls.load(Ordering::SeqCst), 0);
    assert_eq!(handler.selection_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn per_device_rate_limit_returns_a_cached_busy_result() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path(), PeerMode::Trusted);
    let handler = Arc::new(FakeHandler::ready(
        SelectionAuthorization::AcceptOnce,
        Duration::ZERO,
    ));
    let manager = SelectionManager::new(
        Arc::clone(&handler),
        trust,
        SelectionPolicy {
            maximum_requests_per_device: 1,
            ..SelectionPolicy::default()
        },
    )
    .expect("manager");
    let first_id = RequestId::new(Uuid::now_v7());
    let second_id = RequestId::new(Uuid::now_v7());
    assert_eq!(
        manager
            .handle(
                first_id,
                peer.clone(),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                request(&peer, 4242),
                Instant::now(),
            )
            .await
            .expect("first")
            .response
            .status,
        SourceSelectionStatus::Ready
    );
    for _ in 0..2 {
        assert_eq!(
            manager
                .handle(
                    second_id,
                    peer.clone(),
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    request(&peer, 4243),
                    Instant::now(),
                )
                .await
                .expect("busy")
                .response
                .status,
            SourceSelectionStatus::Busy
        );
    }
    assert_eq!(handler.selection_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn global_pending_limit_rejects_a_second_request_without_starting_it() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path(), PeerMode::Trusted);
    let handler = Arc::new(FakeHandler::ready(
        SelectionAuthorization::AcceptOnce,
        Duration::from_millis(50),
    ));
    let manager = Arc::new(
        SelectionManager::new(
            Arc::clone(&handler),
            trust,
            SelectionPolicy {
                maximum_pending: 1,
                ..SelectionPolicy::default()
            },
        )
        .expect("manager"),
    );
    let first = {
        let manager = Arc::clone(&manager);
        let peer = peer.clone();
        tokio::spawn(async move {
            manager
                .handle(
                    RequestId::new(Uuid::now_v7()),
                    peer.clone(),
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    request(&peer, 4242),
                    Instant::now(),
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(
        manager
            .handle(
                RequestId::new(Uuid::now_v7()),
                peer.clone(),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                request(&peer, 4243),
                Instant::now(),
            )
            .await
            .expect("busy")
            .response
            .status,
        SourceSelectionStatus::Busy
    );
    assert_eq!(
        first.await.expect("task").expect("first").response.status,
        SourceSelectionStatus::Ready
    );
    assert_eq!(handler.selection_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn overlapping_ui_is_busy_and_timeout_cannot_finish_a_delayed_selection() {
    let root = tempdir().expect("root");
    let (peer, trust) = peer_context(root.path(), PeerMode::Trusted);
    let handler = Arc::new(FakeHandler::ready(
        SelectionAuthorization::AcceptOnce,
        Duration::from_millis(100),
    ));
    let manager = Arc::new(
        SelectionManager::new(
            Arc::clone(&handler),
            trust,
            SelectionPolicy {
                request_timeout: Duration::from_millis(30),
                ..SelectionPolicy::default()
            },
        )
        .expect("manager"),
    );
    let first = {
        let manager = Arc::clone(&manager);
        let peer = peer.clone();
        tokio::spawn(async move {
            manager
                .handle(
                    RequestId::new(Uuid::now_v7()),
                    peer.clone(),
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    request(&peer, 4242),
                    Instant::now(),
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(5)).await;
    let busy = manager
        .handle(
            RequestId::new(Uuid::now_v7()),
            peer.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            request(&peer, 4243),
            Instant::now(),
        )
        .await
        .expect("busy");
    assert_eq!(busy.response.status, SourceSelectionStatus::Busy);
    let expired = first.await.expect("task").expect("expired");
    assert_eq!(expired.response.status, SourceSelectionStatus::Expired);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(handler.completed.load(Ordering::SeqCst), 0);
}
