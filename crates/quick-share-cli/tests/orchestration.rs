use async_trait::async_trait;
use quick_share_cli::{
    AppError, InteractionPolicy, ReceiveIntent, SendIntent,
    orchestration::{
        DirectSendAdapter, DirectTarget, OfferChoice, ReceiveOptions, ReceiveStartup,
        ReceiveTerminal, SendMode, SendOrchestrator, SendRequest, SendTerminal, ShutdownAction,
        ShutdownController, WebShareAdapter, decide_offer, deliver_received_text,
        prepare_send_request,
    },
};
use quick_share_discovery::{Discovery, DiscoveryError, ScanRequest, ScanResult, UnverifiedPeer};
use quick_share_platform::clipboard::{Clipboard, ClipboardError};
use quick_share_protocol::{
    Capability, ContentKind, DeviceId, OfferDecision, ProtocolVersion, TransferId,
};
use quick_share_transfer::{
    noise::SasCode,
    offer::{OfferEntryView, OfferView},
    sender::ProgressEvent,
    text::{TextDeliveryTarget, TextPayload, TextSource},
};
use std::{
    collections::{BTreeSet, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

fn device(value: char) -> DeviceId {
    DeviceId::parse(format!("qs_{}", value.to_string().repeat(32))).expect("device")
}

fn peer(value: char, name: &str) -> UnverifiedPeer {
    UnverifiedPeer {
        device_id: device(value),
        name: name.to_owned(),
        version: ProtocolVersion::V1_0,
        capabilities: BTreeSet::from([Capability::Files]),
        static_key_fingerprint: [value as u8; 32],
        endpoints: BTreeSet::from([SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, value as u8)),
            4242,
        )]),
        unverified: true,
    }
}

struct CountingDiscovery {
    outcomes: Mutex<VecDeque<Result<ScanResult, DiscoveryError>>>,
    calls: AtomicUsize,
}

impl CountingDiscovery {
    fn new(outcome: Result<ScanResult, DiscoveryError>) -> Self {
        Self {
            outcomes: Mutex::new(VecDeque::from([outcome])),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl Discovery for CountingDiscovery {
    async fn scan(&self, _request: ScanRequest) -> Result<ScanResult, DiscoveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.outcomes
            .lock()
            .expect("outcomes")
            .pop_front()
            .expect("outcome")
    }
}

#[derive(Clone, Copy)]
enum DirectResult {
    Ok,
    Rejected,
    Unavailable,
    Network,
}

struct FakeDirect {
    result: DirectResult,
    calls: AtomicUsize,
    targets: Mutex<Vec<DirectTarget>>,
}

impl FakeDirect {
    fn new(result: DirectResult) -> Self {
        Self {
            result,
            calls: AtomicUsize::new(0),
            targets: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl DirectSendAdapter for FakeDirect {
    async fn send(&self, target: DirectTarget, _request: &SendRequest) -> Result<(), AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.targets.lock().expect("targets").push(target);
        match self.result {
            DirectResult::Ok => Ok(()),
            DirectResult::Rejected => Err(AppError::Rejected("peer declined".to_owned())),
            DirectResult::Unavailable => Err(AppError::PeerUnavailable("offline".to_owned())),
            DirectResult::Network => Err(AppError::Network("connection failed".to_owned())),
        }
    }
}

struct FakeWeb {
    calls: AtomicUsize,
}

#[async_trait]
impl WebShareAdapter for FakeWeb {
    async fn serve(&self, _request: &SendRequest) -> Result<(), AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct FakeSendTerminal {
    selected: usize,
    selection_calls: AtomicUsize,
    modes: Mutex<Vec<SendMode>>,
    warnings: Mutex<Vec<String>>,
}

impl SendTerminal for FakeSendTerminal {
    fn warning(&self, message: &str) {
        self.warnings
            .lock()
            .expect("warnings")
            .push(message.to_owned());
    }

    fn mode(&self, mode: SendMode) {
        self.modes.lock().expect("modes").push(mode);
    }

    fn select_peer(
        &self,
        _peers: &[UnverifiedPeer],
        interaction: InteractionPolicy,
    ) -> Result<usize, AppError> {
        interaction.require_confirmation("send to selected receiver")?;
        self.selection_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.selected)
    }
}

fn request(route: quick_share_cli::orchestration::SendRoute) -> SendRequest {
    SendRequest {
        content: quick_share_cli::orchestration::SendContent::Text(
            TextPayload::new(TextSource::Literal, "hello").expect("text"),
        ),
        route,
        resume_transfer_id: None,
        allow_http: false,
        assume_yes: true,
    }
}

#[tokio::test]
async fn only_definitive_zero_peer_scan_uses_web_fallback() {
    let discovery = CountingDiscovery::new(Ok(ScanResult::complete(Vec::new())));
    let direct = FakeDirect::new(DirectResult::Ok);
    let web = FakeWeb {
        calls: AtomicUsize::new(0),
    };
    let terminal = FakeSendTerminal::default();
    let orchestrator = SendOrchestrator {
        discovery: &discovery,
        direct: &direct,
        web: &web,
        terminal: &terminal,
        local_device_id: device('a'),
        discovery_timeout: Duration::from_millis(1),
        interactive: false,
    };
    orchestrator
        .run(&request(quick_share_cli::orchestration::SendRoute::Auto))
        .await
        .expect("Web fallback");
    assert_eq!(discovery.calls.load(Ordering::SeqCst), 1);
    assert_eq!(direct.calls.load(Ordering::SeqCst), 0);
    assert_eq!(web.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        terminal.modes.lock().expect("modes").as_slice(),
        &[SendMode::WebHttps]
    );
}

#[tokio::test]
async fn resume_with_zero_peers_never_falls_back_to_web() {
    let discovery = CountingDiscovery::new(Ok(ScanResult::complete(Vec::new())));
    let direct = FakeDirect::new(DirectResult::Ok);
    let web = FakeWeb {
        calls: AtomicUsize::new(0),
    };
    let terminal = FakeSendTerminal::default();
    let mut request = request(quick_share_cli::orchestration::SendRoute::Auto);
    request.resume_transfer_id = Some(quick_share_protocol::TransferId::new(Uuid::now_v7()));
    let result = SendOrchestrator {
        discovery: &discovery,
        direct: &direct,
        web: &web,
        terminal: &terminal,
        local_device_id: device('a'),
        discovery_timeout: Duration::from_millis(1),
        interactive: false,
    }
    .run(&request)
    .await;
    assert!(matches!(result, Err(AppError::PeerUnavailable(_))));
    assert_eq!(web.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn discovery_error_or_partial_empty_never_falls_back_to_web() {
    for outcome in [
        Err(DiscoveryError::Backend("multicast failed".to_owned())),
        Ok(ScanResult::partial(
            Vec::new(),
            vec!["interface failed".to_owned()],
        )),
    ] {
        let discovery = CountingDiscovery::new(outcome);
        let direct = FakeDirect::new(DirectResult::Ok);
        let web = FakeWeb {
            calls: AtomicUsize::new(0),
        };
        let terminal = FakeSendTerminal::default();
        let result = SendOrchestrator {
            discovery: &discovery,
            direct: &direct,
            web: &web,
            terminal: &terminal,
            local_device_id: device('a'),
            discovery_timeout: Duration::from_millis(1),
            interactive: false,
        }
        .run(&request(quick_share_cli::orchestration::SendRoute::Auto))
        .await;
        assert!(matches!(result, Err(AppError::Network(_))));
        assert_eq!(web.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn selected_peer_rejection_timeout_or_transfer_failure_never_falls_back() {
    for direct_result in [DirectResult::Rejected, DirectResult::Network] {
        let discovery = CountingDiscovery::new(Ok(ScanResult::complete(vec![peer('b', "peer")])));
        let direct = FakeDirect::new(direct_result);
        let web = FakeWeb {
            calls: AtomicUsize::new(0),
        };
        let terminal = FakeSendTerminal::default();
        let result = SendOrchestrator {
            discovery: &discovery,
            direct: &direct,
            web: &web,
            terminal: &terminal,
            local_device_id: device('a'),
            discovery_timeout: Duration::from_millis(1),
            interactive: false,
        }
        .run(&request(quick_share_cli::orchestration::SendRoute::Auto))
        .await;
        assert!(result.is_err());
        assert_eq!(direct.calls.load(Ordering::SeqCst), 1);
        assert_eq!(web.calls.load(Ordering::SeqCst), 0);
        assert_eq!(terminal.selection_calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn explicit_peer_and_web_routes_skip_discovery_and_keep_stable_error_classes() {
    for (result, expected) in [(DirectResult::Unavailable, 3), (DirectResult::Network, 7)] {
        let discovery = CountingDiscovery::new(Err(DiscoveryError::NoInterfaces));
        let direct = FakeDirect::new(result);
        let web = FakeWeb {
            calls: AtomicUsize::new(0),
        };
        let terminal = FakeSendTerminal::default();
        let error = SendOrchestrator {
            discovery: &discovery,
            direct: &direct,
            web: &web,
            terminal: &terminal,
            local_device_id: device('a'),
            discovery_timeout: Duration::from_millis(1),
            interactive: false,
        }
        .run(&request(
            quick_share_cli::orchestration::SendRoute::ExplicitPeer("192.0.2.1:4242".to_owned()),
        ))
        .await
        .expect_err("explicit peer fails");
        assert_eq!(error.exit_code(), expected);
        assert_eq!(discovery.calls.load(Ordering::SeqCst), 0);
        assert_eq!(web.calls.load(Ordering::SeqCst), 0);
    }

    let discovery = CountingDiscovery::new(Err(DiscoveryError::NoInterfaces));
    let direct = FakeDirect::new(DirectResult::Ok);
    let web = FakeWeb {
        calls: AtomicUsize::new(0),
    };
    let terminal = FakeSendTerminal::default();
    SendOrchestrator {
        discovery: &discovery,
        direct: &direct,
        web: &web,
        terminal: &terminal,
        local_device_id: device('a'),
        discovery_timeout: Duration::from_millis(1),
        interactive: false,
    }
    .run(&request(quick_share_cli::orchestration::SendRoute::Web))
    .await
    .expect("explicit Web");
    assert_eq!(discovery.calls.load(Ordering::SeqCst), 0);
    assert_eq!(direct.calls.load(Ordering::SeqCst), 0);
    assert_eq!(web.calls.load(Ordering::SeqCst), 1);
}

struct FakeClipboard {
    read: Result<String, ClipboardError>,
    writes: Vec<String>,
}

impl Clipboard for FakeClipboard {
    fn read_text(&mut self) -> Result<String, ClipboardError> {
        self.read.clone()
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.writes.push(text.to_owned());
        Err(ClipboardError::Unsupported)
    }
}

#[test]
fn literal_clipboard_unicode_empty_and_safe_delivery_are_bounded_end_to_end() {
    let literal = SendIntent {
        paths: Vec::new(),
        text: Some("你好🚀\u{1b}[31m".to_owned()),
        clipboard: false,
        peer: None,
        web: false,
        resume: None,
        follow_links: false,
        allow_http: false,
        assume_yes: false,
    };
    let prepared = prepare_send_request(&literal, None).expect("literal");
    let debug = format!("{prepared:?}");
    assert!(!debug.contains("你好"));
    assert!(!debug.contains("[31m"));

    let mut clipboard = FakeClipboard {
        read: Ok(String::new()),
        writes: Vec::new(),
    };
    let clipboard_intent = SendIntent {
        text: None,
        clipboard: true,
        ..literal
    };
    let prepared = prepare_send_request(&clipboard_intent, Some(&mut clipboard)).expect("empty");
    assert!(format!("{prepared:?}").contains("TextPayload"));

    let payload = TextPayload::new(TextSource::Literal, "$(touch should-not-run);\u{1b}[2J")
        .expect("payload");
    let mut stdout = Vec::new();
    let delivery = deliver_received_text(&payload, &mut clipboard, None, &mut stdout)
        .expect("stdout fallback");
    assert_eq!(delivery.target, TextDeliveryTarget::Stdout);
    assert_eq!(stdout, payload.as_str().as_bytes());
    assert_eq!(clipboard.writes, vec![payload.as_str()]);
}

#[derive(Default)]
struct FakeReceiveTerminal {
    offers: AtomicUsize,
    choice: Mutex<Option<OfferChoice>>,
}

impl ReceiveTerminal for FakeReceiveTerminal {
    fn startup(&self, _startup: &ReceiveStartup) {}
    fn offer(&self, _view: &OfferView) {
        self.offers.fetch_add(1, Ordering::SeqCst);
    }
    fn choose_offer(&self, _view: &OfferView) -> Result<OfferChoice, AppError> {
        Ok(self
            .choice
            .lock()
            .expect("choice")
            .expect("configured choice"))
    }
    fn progress(&self, _event: ProgressEvent) {}
    fn warning(&self, _message: &str) {}
}

fn offer_view() -> OfferView {
    OfferView {
        transfer_id: TransferId::new(Uuid::now_v7()),
        sender_device_id: device('b'),
        sender_name: "sender".to_owned(),
        peer_address: Some("192.168.1.2:4242".parse().expect("address")),
        sas: SasCode::from_value(123_456).expect("SAS"),
        content_kind: ContentKind::Files,
        entry_count: 1,
        total_bytes: 5,
        entries: vec![OfferEntryView {
            relative_path: "file.txt".to_owned(),
            kind: "file",
            size: 5,
        }],
        identity_changed: false,
        trusted: false,
    }
}

#[test]
fn receive_defaults_output_and_non_tty_unknown_offer_is_never_silently_accepted() {
    let intent = ReceiveIntent {
        output: None,
        port: None,
        bind: None,
        once: false,
        confirm_trusted: None,
        assume_yes: false,
    };
    let options = ReceiveOptions::resolve(&intent, &PathBuf::from("Downloads"), false, false);
    assert_eq!(options.output, PathBuf::from("Downloads"));
    let terminal = FakeReceiveTerminal::default();
    assert!(matches!(
        decide_offer(&terminal, &offer_view(), &options).expect("safe default"),
        OfferDecision::Reject { .. }
    ));
    assert_eq!(terminal.offers.load(Ordering::SeqCst), 1);

    let override_options = ReceiveOptions::resolve(
        &ReceiveIntent {
            output: Some(PathBuf::from("custom")),
            assume_yes: true,
            ..intent
        },
        &PathBuf::from("Downloads"),
        true,
        false,
    );
    assert_eq!(override_options.output, PathBuf::from("custom"));
    assert!(override_options.confirm_trusted);
    assert_eq!(
        decide_offer(&terminal, &offer_view(), &override_options).expect("explicit yes"),
        OfferDecision::AcceptOnce
    );
}

#[test]
fn accept_and_trust_requires_explicit_sas_and_ctrl_c_is_two_stage() {
    let terminal = FakeReceiveTerminal {
        offers: AtomicUsize::new(0),
        choice: Mutex::new(Some(OfferChoice::AcceptAndTrust {
            sas_verified: false,
        })),
    };
    let options = ReceiveOptions {
        output: PathBuf::from("Downloads"),
        once: false,
        confirm_trusted: true,
        assume_yes: false,
        interactive: true,
    };
    assert!(matches!(
        decide_offer(&terminal, &offer_view(), &options),
        Err(AppError::Identity(_))
    ));

    let shutdown = ShutdownController::default();
    assert_eq!(shutdown.request(), ShutdownAction::GracefulSnapshot);
    assert!(shutdown.cancellation().is_cancelled());
    assert_eq!(shutdown.request(), ShutdownAction::Force);
}
