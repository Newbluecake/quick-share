//! Production CLI wiring for Batch 6 direct send/receive and trusted-device operations.

use crate::{
    AgentIntent, AppError, CommandIntent, ConfigIntent, ConfigKey, DevicesIntent, IntentCommand,
    InteractionPolicy, ReceiveIntent, SendIntent, ServeIntent, UpdateIntent, UploadPassword,
    devices::run_devices,
    orchestration::{
        DirectSendAdapter, DirectTarget, ReceiveOptions, ReceiveStartup, ReceiveTerminal,
        SendContent, SendOrchestrator, SendRequest, WebShareAdapter, decide_offer_with_sas,
        deliver_received_text, prepare_send_request,
    },
    terminal::{ConsoleTerminal, terminal_safe},
};
use async_trait::async_trait;
use quick_share_core::{
    config::{AppConfig, ConfigLoader, ConfigOverrides, ConflictPolicy, TrustedPolicy},
    identity::{DeviceIdentity, IdentityStore, TrustStatus, TrustedDeviceStore},
    manifest::{ManifestBuilder, ManifestEntryKind as SourceEntryKind},
};
use quick_share_discovery::{
    Advertisement, Discovery, MdnsDiscovery, MdnsRegistration, ScanRequest, filter_lan_interfaces,
    system_interfaces,
};
use quick_share_platform::{
    AppDirs, FileSensitivity, atomic_write,
    clipboard::{Clipboard, ClipboardError, NativeClipboard},
    network::receive_network_diagnostic,
};
use quick_share_protocol::{
    Capability, ContentKind, DeviceInfo, EntryId, InfoRequest, InfoResponse, ManifestEntry,
    ManifestEntryKind, OfferDecision, ProtocolVersion, TransferId, TransferOffer,
};
use quick_share_transfer::{
    direct::{
        ClientConnector, DirectError, NoiseClientTransport, OfferPrompt, ServerContext,
        ServerSessionOutcome,
    },
    offer::{OfferManager, OfferPolicy, OfferView},
    receiver::{ReceiverPolicy, ReceiverProgressEvent, ReceiverService},
    sender::{
        ProgressEvent, RetryPolicy, SendFile, SenderPolicy, TextSendPlan, TransferPlan,
        TransferSender,
    },
    text::{TextDeliveryTarget, TextSource},
};
use quick_share_update::{
    Ed25519ReleaseVerifier, GitHubReleaseProvider, SafeSelfReplacer, UpdateCheck, UpdateError,
    UpdateService, current_target,
};
use quick_share_web::{
    ExplicitHttp, PreparedWebSecurity, ShareCatalog, UploadConfig, UploadProgress, UploadService,
    WebAccess, WebProgressEvent, WebSecurity, ZipService, build_browser_router_with_progress,
    start_web_server,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{IsTerminal, Write},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub async fn run(intent: CommandIntent) -> Result<(), AppError> {
    let dirs = AppDirs::discover().map_err(|error| AppError::Filesystem(error.to_string()))?;
    let config = load_config(&dirs, intent.global.config_path.as_deref())?;
    let terminal = ConsoleTerminal::default();
    match intent.command {
        IntentCommand::Devices(command) => {
            let assume_yes = matches!(
                command,
                DevicesIntent::Remove {
                    assume_yes: true,
                    ..
                }
            );
            let store = trust_store(&dirs);
            run_devices(
                &store,
                &command,
                InteractionPolicy::new(terminal.is_interactive(), assume_yes),
                &mut std::io::stdout().lock(),
            )?;
            Ok(())
        }
        IntentCommand::Send(send) => run_send(&dirs, &config, terminal, send).await,
        IntentCommand::Receive(receive) => run_receive(&dirs, &config, terminal, receive).await,
        IntentCommand::Agent(agent) => run_agent(agent).await,
        IntentCommand::Config(command) => {
            run_config(&dirs, intent.global.config_path.as_deref(), config, command)
        }
        IntentCommand::Serve(serve) => run_serve(&config, serve).await,
        IntentCommand::Update(update) => run_update(terminal, update).await,
    }
}

async fn run_agent(_intent: AgentIntent) -> Result<(), AppError> {
    Err(AppError::Usage(if cfg!(windows) {
        "the Windows desktop agent must be launched through the main-thread agent entry point"
            .to_owned()
    } else {
        "the desktop agent is currently supported only by the Windows implementation".to_owned()
    }))
}

#[cfg(windows)]
pub fn run_windows_agent(intent: CommandIntent) -> Result<(), AppError> {
    let dirs = AppDirs::discover().map_err(|error| AppError::Filesystem(error.to_string()))?;
    let config = load_config(&dirs, intent.global.config_path.as_deref())?;
    let IntentCommand::Agent(agent_intent) = intent.command else {
        return Err(AppError::Usage(
            "the Windows agent entry point requires the agent command".to_owned(),
        ));
    };
    let (desktop, control, tray_events, desktop_runtime) =
        quick_share_platform::desktop::windows::windows_desktop(Duration::from_secs(11 * 60))?;
    let desktop: Arc<dyn quick_share_platform::desktop::DesktopInteraction> = Arc::new(desktop);
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let worker_control = control.clone();
    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| AppError::Config(error.to_string()))
            .and_then(|runtime| {
                runtime.block_on(crate::agent::run_background(
                    crate::agent::AgentBackground {
                        dirs,
                        config,
                        intent: agent_intent,
                        desktop,
                        tray_events,
                        cancellation: worker_cancellation,
                    },
                ))
            });
        let _ = result_sender.try_send(result);
        let _ = worker_control.exit();
    });
    let desktop_result = desktop_runtime.run().map_err(AppError::from);
    cancellation.cancel();
    let worker_result = result_receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| AppError::Network("desktop agent shutdown timed out".to_owned()))?;
    desktop_result?;
    worker_result
}

async fn run_update(terminal: ConsoleTerminal, intent: UpdateIntent) -> Result<(), AppError> {
    let target = current_target().ok_or_else(|| {
        AppError::Update("the current platform has no published update target".to_owned())
    })?;
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| AppError::Update(error.to_string()))?;
    let requested = intent
        .version
        .as_deref()
        .map(|value| semver::Version::parse(value.strip_prefix('v').unwrap_or(value)))
        .transpose()
        .map_err(|error| AppError::Usage(format!("invalid update version: {error}")))?;
    eprintln!("Checking signed Quick Share releases for {target}...");
    let provider = Arc::new(
        GitHubReleaseProvider::new().map_err(|error| AppError::Update(error.to_string()))?,
    );
    let service = UpdateService::new(
        provider,
        Arc::new(Ed25519ReleaseVerifier),
        Arc::new(SafeSelfReplacer),
        current,
        std::env::current_exe().map_err(|error| AppError::Update(error.to_string()))?,
    );
    let check = service
        .check(requested.as_ref())
        .await
        .map_err(map_update)?;
    let UpdateCheck::Available(plan) = check else {
        eprintln!("Quick Share is already up to date.");
        return Ok(());
    };
    if intent.check_only {
        eprintln!(
            "Signed update available: {} -> {}",
            plan.current, plan.release.version
        );
        return Ok(());
    }
    terminal.confirm_update(&plan.current, &plan.release.version, intent.assume_yes)?;
    eprintln!("Downloading and verifying the signed SHA-256 release manifest...");
    let cancellation = CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancellation.cancel();
        }
    });
    let result = service.install(&plan, &cancellation).await;
    signal_task.abort();
    let receipt = result.map_err(map_update)?;
    eprintln!(
        "Updated Quick Share from {} to {}.",
        receipt.previous, receipt.installed
    );
    Ok(())
}

fn map_update(error: UpdateError) -> AppError {
    match error {
        UpdateError::Cancelled => AppError::Cancelled,
        other => AppError::Update(other.to_string()),
    }
}

async fn run_send(
    dirs: &AppDirs,
    config: &AppConfig,
    terminal: ConsoleTerminal,
    intent: SendIntent,
) -> Result<(), AppError> {
    let identity = Arc::new(load_identity(dirs)?);
    let mut clipboard = if intent.clipboard {
        Some(NativeClipboard::connect().map_err(|error| {
            AppError::Filesystem(format!(
                "{error}; use --text when clipboard access is unavailable"
            ))
        })?)
    } else {
        None
    };
    let mut request = prepare_send_request(
        &intent,
        clipboard.as_mut().map(|value| value as &mut dyn Clipboard),
    )?;
    request.allow_http |= config.web.allow_http;
    let discovery = MdnsDiscovery::new(config.discovery.include_virtual);
    let direct = ProductionDirect {
        identity: Arc::clone(&identity),
        trust_store: trust_store(dirs),
        config: config.clone(),
        terminal,
    };
    let web = ProductionWeb {
        config: config.clone(),
    };
    let orchestrator = SendOrchestrator {
        discovery: &discovery,
        direct: &direct,
        web: &web,
        terminal: &terminal,
        local_device_id: identity.device_id(),
        discovery_timeout: Duration::from_millis(config.discovery.timeout_ms),
        interactive: terminal.is_interactive(),
    };
    let _ = orchestrator.run(&request).await?;
    Ok(())
}

struct ProductionWeb {
    config: AppConfig,
}

#[async_trait]
impl WebShareAdapter for ProductionWeb {
    async fn serve(&self, request: &SendRequest) -> Result<(), AppError> {
        let catalog = match &request.content {
            SendContent::Paths { paths, .. } => ShareCatalog::from_paths(paths),
            SendContent::Text(payload) => ShareCatalog::from_text(
                "quick-share-text.txt",
                Arc::from(payload.as_str().as_bytes()),
            ),
        }
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
        run_web_catalog(
            &self.config,
            catalog,
            WebRunOptions {
                upload: false,
                output: None,
                upload_password: None,
                max_downloads: Some(self.config.web.max_downloads),
                timeout: self.config.web.timeout.clone(),
                allow_http: request.allow_http,
                certificate: None,
                private_key: None,
            },
        )
        .await
    }
}

#[derive(Debug)]
struct WebRunOptions {
    upload: bool,
    output: Option<PathBuf>,
    upload_password: Option<UploadPassword>,
    max_downloads: Option<u32>,
    timeout: String,
    allow_http: bool,
    certificate: Option<PathBuf>,
    private_key: Option<PathBuf>,
}

async fn run_serve(config: &AppConfig, intent: ServeIntent) -> Result<(), AppError> {
    let catalog = if intent.paths.is_empty() {
        ShareCatalog::empty()
    } else {
        ShareCatalog::from_paths(&intent.paths)
            .map_err(|error| AppError::Filesystem(error.to_string()))?
    };
    run_web_catalog(
        config,
        catalog,
        WebRunOptions {
            upload: intent.upload || config.web.upload,
            output: intent.output,
            upload_password: intent.upload_password,
            max_downloads: intent.max_downloads.or(Some(config.web.max_downloads)),
            timeout: intent.timeout.unwrap_or_else(|| config.web.timeout.clone()),
            allow_http: intent.allow_http || config.web.allow_http,
            certificate: intent.certificate,
            private_key: intent.private_key,
        },
    )
    .await
}

async fn run_web_catalog(
    config: &AppConfig,
    catalog: ShareCatalog,
    options: WebRunOptions,
) -> Result<(), AppError> {
    let ttl = parse_web_duration(&options.timeout)?;
    let (access, token) = WebAccess::generate(ttl, options.max_downloads)
        .map_err(|error| AppError::Config(error.to_string()))?;
    let access = Arc::new(access);
    let (web_progress_tx, mut web_progress_rx) = mpsc::channel(64);
    let (upload_progress_tx, mut upload_progress_rx) = mpsc::channel(64);
    let upload = if options.upload {
        let output = options
            .output
            .unwrap_or_else(|| config.receive.output.clone());
        let service = UploadService::new_with_progress(
            UploadConfig {
                output_root: output,
                max_file_bytes: 1024 * 1024 * 1024,
                max_total_bytes: 4 * 1024 * 1024 * 1024,
                max_body_bytes: 4 * 1024 * 1024 * 1024_usize + 16 * 1024 * 1024,
                conflict: config.receive.conflict,
                max_uploads_per_window: 20,
                rate_window: Duration::from_secs(60),
                max_concurrent_uploads: 2,
            },
            options.upload_password.as_ref().map(UploadPassword::expose),
            Some(upload_progress_tx),
        )
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
        Some(Arc::new(service))
    } else {
        None
    };
    let router = build_browser_router_with_progress(
        Arc::new(catalog),
        Arc::clone(&access),
        upload,
        Arc::new(ZipService::new(1, 8).map_err(|error| AppError::Config(error.to_string()))?),
        web_progress_tx,
    );
    let bind_ip = resolve_bind_ip(&config.network.bind, config)?;
    let security = match (options.certificate, options.private_key, options.allow_http) {
        (Some(certificate), Some(private_key), false) => WebSecurity::UserProvided {
            certificate,
            private_key,
        },
        (None, None, true) => WebSecurity::Http(
            ExplicitHttp::new(true).map_err(|error| AppError::Config(error.to_string()))?,
        ),
        (None, None, false) => WebSecurity::SelfSigned,
        _ => {
            return Err(AppError::Usage(
                "certificate and private key must be supplied together and cannot be combined with HTTP"
                    .to_owned(),
            ));
        }
    };
    let security = PreparedWebSecurity::prepare(security, &[bind_ip])
        .await
        .map_err(|error| AppError::Network(error.to_string()))?;
    let cancellation = CancellationToken::new();
    let server = start_web_server(
        router,
        SocketAddr::new(bind_ip, config.network.port),
        bind_ip,
        security,
        token.expose(),
        ttl,
        options.max_downloads,
        cancellation.clone(),
        access.quota_shutdown(),
    )
    .map_err(|error| AppError::Network(error.to_string()))?;
    print_web_endpoint(&server.endpoint, server.local_addr());
    let web_progress_task = tokio::spawn(async move {
        while let Some(event) = web_progress_rx.recv().await {
            print_web_progress(event);
        }
    });
    let upload_progress_task = tokio::spawn(async move {
        while let Some(event) = upload_progress_rx.recv().await {
            print_upload_progress(event);
        }
    });

    let shutdown = Arc::new(crate::orchestration::ShutdownController::default());
    let signal_shutdown = Arc::clone(&shutdown);
    let signal_cancellation = cancellation.clone();
    let signal_task = tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            match signal_shutdown.request() {
                crate::orchestration::ShutdownAction::GracefulSnapshot => {
                    eprintln!("Graceful Web shutdown requested...");
                    signal_cancellation.cancel();
                }
                crate::orchestration::ShutdownAction::Force => {
                    eprintln!("Second interrupt received; forcing termination.");
                    std::process::exit(130);
                }
            }
        }
    });
    let result = server
        .wait()
        .await
        .map_err(|error| AppError::Network(error.to_string()));
    signal_task.abort();
    web_progress_task.abort();
    upload_progress_task.abort();
    result
}

fn print_web_progress(event: WebProgressEvent) {
    match event {
        WebProgressEvent::DownloadStarted { name, total_bytes } => eprintln!(
            "Web download started: {} ({} bytes)",
            terminal_safe(&name),
            total_bytes
        ),
        WebProgressEvent::DownloadProgress {
            name,
            transferred_bytes,
            total_bytes,
        } => eprintln!(
            "Web download progress: {} — {} / {} bytes",
            terminal_safe(&name),
            transferred_bytes,
            total_bytes
        ),
        WebProgressEvent::DownloadCompleted { name, total_bytes } => eprintln!(
            "Web download completed: {} ({} bytes)",
            terminal_safe(&name),
            total_bytes
        ),
        WebProgressEvent::DownloadInterrupted {
            name,
            transferred_bytes,
        } => eprintln!(
            "Web download interrupted: {} after {} bytes",
            terminal_safe(&name),
            transferred_bytes
        ),
        WebProgressEvent::UploadCompleted { files, total_bytes } => {
            eprintln!("Web upload completed: {files} file(s), {total_bytes} bytes");
        }
    }
}

fn print_upload_progress(event: UploadProgress) {
    eprintln!(
        "Web upload progress: {} — {} bytes ({} request bytes)",
        terminal_safe(&event.file_name),
        event.file_bytes,
        event.request_bytes
    );
}

fn parse_web_duration(source: &str) -> Result<Duration, AppError> {
    if source.len() < 2 {
        return Err(AppError::Config("invalid Web timeout".to_owned()));
    }
    let (number, suffix) = source.split_at(source.len() - 1);
    let value = number
        .parse::<u64>()
        .map_err(|_| AppError::Config("invalid Web timeout".to_owned()))?;
    let seconds = match suffix {
        "s" => value,
        "m" => value
            .checked_mul(60)
            .ok_or_else(|| AppError::Config("Web timeout is too large".to_owned()))?,
        "h" => value
            .checked_mul(60 * 60)
            .ok_or_else(|| AppError::Config("Web timeout is too large".to_owned()))?,
        _ => {
            return Err(AppError::Config(
                "Web timeout must end in s, m, or h".to_owned(),
            ));
        }
    };
    if seconds == 0 || seconds > 24 * 60 * 60 {
        return Err(AppError::Config(
            "Web timeout must be between 1 second and 24 hours".to_owned(),
        ));
    }
    Ok(Duration::from_secs(seconds))
}

fn print_web_endpoint(endpoint: &quick_share_web::EndpointInfo, listener: SocketAddr) {
    eprintln!("Traditional Web sharing is ready.");
    eprintln!("  Listen: {listener}");
    eprintln!("  URL: {}", endpoint.url);
    eprintln!("  Expires in: {} seconds", endpoint.expires_in.as_secs());
    match endpoint.max_downloads {
        Some(limit) => eprintln!("  Download sessions: {limit}"),
        None => eprintln!("  Download sessions: unlimited until timeout"),
    }
    if endpoint.self_signed {
        eprintln!(
            "  Security: self-signed HTTPS; browsers will show a certificate warning. Do not install this temporary certificate as a CA."
        );
        if let Some(fingerprint) = &endpoint.certificate_fingerprint {
            eprintln!("  Certificate SHA-256: {fingerprint}");
        }
    } else if endpoint.url.starts_with("http://") {
        eprintln!(
            "  WARNING: plaintext HTTP was explicitly enabled; anyone on the network path may observe the token and content."
        );
    } else {
        eprintln!("  Security: HTTPS with the supplied certificate.");
    }
    eprintln!("  curl: {}", endpoint.curl);
    eprintln!("  wget: {}", endpoint.wget);
    eprintln!("\n{}", endpoint.qr);
}

struct ProductionDirect {
    identity: Arc<DeviceIdentity>,
    trust_store: TrustedDeviceStore,
    config: AppConfig,
    terminal: ConsoleTerminal,
}

#[async_trait]
impl DirectSendAdapter for ProductionDirect {
    async fn send(&self, target: DirectTarget, request: &SendRequest) -> Result<(), AppError> {
        let (endpoint, expected_id, advertised_fingerprint, advertised_name) = match target {
            DirectTarget::Discovered(peer) => (
                *peer.endpoints.iter().next().ok_or_else(|| {
                    AppError::PeerUnavailable("selected receiver has no endpoint".to_owned())
                })?,
                Some(peer.device_id),
                Some(peer.static_key_fingerprint),
                peer.name,
            ),
            DirectTarget::Explicit(value) if value.starts_with("qs_") => {
                let selected_id = quick_share_protocol::DeviceId::parse(&value)
                    .map_err(|error| AppError::Usage(error.to_string()))?;
                let scan = MdnsDiscovery::new(self.config.discovery.include_virtual)
                    .scan(ScanRequest {
                        local_device_id: self.identity.device_id(),
                        timeout: Duration::from_millis(self.config.discovery.timeout_ms),
                    })
                    .await
                    .map_err(|error| AppError::Network(error.to_string()))?;
                let peer = scan
                    .peers
                    .into_iter()
                    .find(|peer| peer.device_id == selected_id)
                    .ok_or_else(|| {
                        AppError::PeerUnavailable(format!(
                            "forced receiver {selected_id} was not discovered"
                        ))
                    })?;
                (
                    *peer.endpoints.iter().next().ok_or_else(|| {
                        AppError::PeerUnavailable("selected receiver has no endpoint".to_owned())
                    })?,
                    Some(peer.device_id),
                    Some(peer.static_key_fingerprint),
                    peer.name,
                )
            }
            DirectTarget::Explicit(value) => {
                let endpoint = resolve_explicit_endpoint(&value).await?;
                (endpoint, None, None, value)
            }
        };
        let expected_key = expected_id.as_ref().and_then(|id| {
            self.trust_store
                .list()
                .ok()?
                .into_iter()
                .find(|device| &device.device_id == id)
                .map(|device| device.public_key)
        });
        let connector = ClientConnector::new(
            endpoint,
            Arc::clone(&self.identity),
            expected_id,
            expected_key,
            InfoRequest {
                protocol_version: ProtocolVersion::V1_0,
                capabilities: capabilities(),
            },
            Duration::from_secs(10),
        );
        let connected = connector.connect().await.map_err(map_direct)?;
        let required_capability = match &request.content {
            SendContent::Text(_) => Capability::Text,
            SendContent::Paths { .. } => Capability::Files,
        };
        if !connected
            .remote_info
            .device
            .capabilities
            .contains(&required_capability)
        {
            return Err(AppError::PeerUnavailable(format!(
                "receiver does not support {required_capability:?} content"
            )));
        }
        if advertised_fingerprint
            .is_some_and(|fingerprint| fingerprint != connected.evidence.static_key_fingerprint)
        {
            return Err(AppError::Identity(
                "authenticated receiver fingerprint differs from the discovery advertisement"
                    .to_owned(),
            ));
        }
        let trusted = match self
            .trust_store
            .check(
                &connected.evidence.remote_device_id,
                &connected.evidence.remote_static(),
            )
            .map_err(|error| AppError::Identity(error.to_string()))?
        {
            TrustStatus::Trusted(_) => true,
            TrustStatus::KeyMismatch => {
                return Err(AppError::Identity(
                    "remote static identity does not match the pinned key".to_owned(),
                ));
            }
            TrustStatus::Unknown => false,
        };
        if !trusted {
            self.terminal.confirm_tofu(
                &connected.remote_info.device.name,
                &connected.evidence.remote_device_id,
                connected.evidence.sas,
                request.assume_yes,
            )?;
        }
        eprintln!(
            "Receiver authenticated: {} ({}) via {}",
            terminal_safe(&connected.remote_info.device.name),
            connected.remote_info.device.device_id,
            terminal_safe(&advertised_name)
        );
        let transport = Arc::new(NoiseClientTransport::new(connector, connected));
        let sender_info = DeviceInfo {
            device_id: self.identity.device_id(),
            name: self.config.device.name.clone(),
            capabilities: capabilities(),
        };
        let policy = sender_policy(&self.config)?;
        let sender = TransferSender::new(Arc::clone(&transport), policy)
            .map_err(|error| AppError::Usage(error.to_string()))?;
        let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressEvent>(64);
        let progress_terminal = self.terminal;
        let progress_task = tokio::spawn(async move {
            while let Some(event) = progress_rx.recv().await {
                quick_share_cli_progress(progress_terminal, event);
            }
        });
        match &request.content {
            SendContent::Paths {
                paths,
                follow_links,
            } => {
                let (offer, plan) = prepare_path_transfer(
                    paths,
                    *follow_links,
                    sender_info,
                    self.config.transfer.chunk_size,
                    request.resume_transfer_id,
                )?;
                print_transfer_resume_hint(plan.transfer_id, request.resume_transfer_id.is_some());
                let token = if request.resume_transfer_id.is_some() {
                    transport.resume_offer(offer).await
                } else {
                    transport.create_offer(offer).await
                }
                .map_err(map_direct)?;
                if let Err(error) = sender
                    .send(
                        plan.clone(),
                        &token,
                        CancellationToken::new(),
                        Some(progress_tx),
                    )
                    .await
                {
                    print_retryable_resume_hint(plan.transfer_id, &error);
                    return Err(map_sender(error));
                }
            }
            SendContent::Text(payload) => {
                let (offer, plan) = prepare_text_transfer(
                    payload,
                    sender_info,
                    self.config.transfer.chunk_size,
                    request.resume_transfer_id,
                )?;
                print_transfer_resume_hint(plan.transfer_id, request.resume_transfer_id.is_some());
                let token = if request.resume_transfer_id.is_some() {
                    transport.resume_offer(offer).await
                } else {
                    transport.create_offer(offer).await
                }
                .map_err(map_direct)?;
                if let Err(error) = sender
                    .send_text(
                        plan.clone(),
                        &token,
                        CancellationToken::new(),
                        Some(progress_tx),
                    )
                    .await
                {
                    print_retryable_resume_hint(plan.transfer_id, &error);
                    return Err(map_sender(error));
                }
            }
        }
        progress_task
            .await
            .map_err(|_| AppError::Network("progress task failed".to_owned()))?;
        Ok(())
    }
}

fn quick_share_cli_progress(terminal: ConsoleTerminal, event: ProgressEvent) {
    terminal.progress(event);
}

fn print_transfer_resume_hint(transfer_id: TransferId, resumed: bool) {
    if resumed {
        eprintln!("Resuming transfer {}.", transfer_id.as_uuid());
    } else {
        eprintln!("Transfer ID: {}", transfer_id.as_uuid());
    }
}

fn print_retryable_resume_hint(
    transfer_id: TransferId,
    error: &quick_share_transfer::sender::SenderError,
) {
    if matches!(
        error,
        quick_share_transfer::sender::SenderError::Transport(
            quick_share_transfer::sender::TransportError::Retryable
                | quick_share_transfer::sender::TransportError::Unauthorized
        )
    ) {
        eprintln!(
            "Transfer {} remains resumable. Re-run the same send command with --resume {}.",
            transfer_id.as_uuid(),
            transfer_id.as_uuid()
        );
    }
}

pub(crate) struct PreparedPathContent {
    entries: Vec<ManifestEntry>,
    files: Vec<SendFile>,
    total_bytes: u64,
}

pub(crate) fn prepare_path_content(
    paths: &[PathBuf],
    follow_links: bool,
) -> Result<PreparedPathContent, AppError> {
    let manifest = ManifestBuilder::new()
        .follow_links(follow_links)
        .build(paths)
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
    let mut prepared = BTreeMap::new();
    for entry in &manifest.entries {
        if matches!(entry.kind, SourceEntryKind::File) {
            let file = SendFile::prepare(entry.clone()).map_err(map_sender)?;
            prepared.insert(entry.id, file);
        }
    }
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
            digest: prepared.get(&entry.id).map(|file| file.final_digest),
        })
        .collect::<Vec<_>>();
    Ok(PreparedPathContent {
        entries,
        files: prepared.into_values().collect(),
        total_bytes: manifest.total_bytes,
    })
}

pub(crate) fn finalize_path_transfer(
    prepared: PreparedPathContent,
    sender: DeviceInfo,
    chunk_size: u32,
    transfer_id: TransferId,
    initiated_by: Option<quick_share_protocol::RequestId>,
) -> Result<(TransferOffer, TransferPlan), AppError> {
    let protocol_version = if initiated_by.is_some() {
        ProtocolVersion::V1_1
    } else {
        ProtocolVersion::V1_0
    };
    let offer = TransferOffer {
        protocol_version,
        transfer_id,
        initiated_by,
        sender,
        content_kind: ContentKind::Files,
        chunk_size,
        total_bytes: prepared.total_bytes,
        entries: prepared.entries,
    };
    offer
        .validate()
        .map_err(|error| AppError::Usage(error.to_string()))?;
    let manifest_digest = offer_digest(&offer)?;
    let plan = TransferPlan {
        transfer_id,
        manifest_digest,
        chunk_size,
        total_bytes: prepared.total_bytes,
        files: prepared.files,
    };
    Ok((offer, plan))
}

fn prepare_path_transfer(
    paths: &[PathBuf],
    follow_links: bool,
    sender: DeviceInfo,
    chunk_size: u32,
    resume_transfer_id: Option<TransferId>,
) -> Result<(TransferOffer, TransferPlan), AppError> {
    let prepared = prepare_path_content(paths, follow_links)?;
    finalize_path_transfer(
        prepared,
        sender,
        chunk_size,
        resume_transfer_id.unwrap_or_else(|| TransferId::new(Uuid::now_v7())),
        None,
    )
}

fn prepare_text_transfer(
    payload: &quick_share_transfer::text::TextPayload,
    sender: DeviceInfo,
    chunk_size: u32,
    resume_transfer_id: Option<TransferId>,
) -> Result<(TransferOffer, TextSendPlan), AppError> {
    let transfer_id = resume_transfer_id.unwrap_or_else(|| TransferId::new(Uuid::now_v7()));
    let entry_id = EntryId::new(1)
        .ok_or_else(|| AppError::Usage("text entry ID must be non-zero".to_owned()))?;
    let media_type = match payload.source() {
        TextSource::Literal => "text/plain;charset=utf-8;source=literal",
        TextSource::Clipboard => "text/plain;charset=utf-8;source=clipboard",
    };
    let offer = TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id,
        initiated_by: None,
        sender,
        content_kind: ContentKind::Text,
        chunk_size,
        total_bytes: payload.as_str().len() as u64,
        entries: vec![ManifestEntry {
            id: entry_id,
            relative_path: "quick-share-text.txt".to_owned(),
            kind: ManifestEntryKind::Text {
                media_type: media_type.to_owned(),
            },
            size: payload.as_str().len() as u64,
            digest: Some(*blake3::hash(payload.as_str().as_bytes()).as_bytes()),
        }],
    };
    offer
        .validate()
        .map_err(|error| AppError::Usage(error.to_string()))?;
    let manifest_digest = offer_digest(&offer)?;
    Ok((
        offer,
        TextSendPlan {
            transfer_id,
            manifest_digest,
            entry_id,
            bytes: Arc::from(payload.as_str().as_bytes()),
        },
    ))
}

async fn resolve_explicit_endpoint(value: &str) -> Result<SocketAddr, AppError> {
    tokio::net::lookup_host(value)
        .await
        .map_err(|error| AppError::PeerUnavailable(error.to_string()))?
        .next()
        .ok_or_else(|| AppError::PeerUnavailable("address resolved to no endpoint".to_owned()))
}

async fn run_receive(
    dirs: &AppDirs,
    config: &AppConfig,
    terminal: ConsoleTerminal,
    intent: ReceiveIntent,
) -> Result<(), AppError> {
    intent.validate_interaction(terminal.is_interactive())?;
    if intent.request_remote {
        return crate::remote_receive::run_remote_receive(dirs, config, terminal, intent).await;
    }
    let text_output = intent
        .output
        .as_ref()
        .filter(|path| looks_like_file(path))
        .cloned();
    if let Some(path) = text_output.as_ref() {
        if !intent.once {
            return Err(AppError::Usage(
                "a text output file requires --once; use a directory for a persistent receiver"
                    .to_owned(),
            ));
        }
        if path.exists() {
            return Err(AppError::Filesystem(format!(
                "refusing to overwrite existing text output {}",
                path.display()
            )));
        }
    }
    let receive_root = if text_output.is_some() {
        dirs.cache_dir().join("text-receive")
    } else {
        intent
            .output
            .clone()
            .unwrap_or_else(|| config.receive.output.clone())
    };
    fs::create_dir_all(&receive_root).map_err(|error| AppError::Filesystem(error.to_string()))?;
    let options = ReceiveOptions::resolve(
        &ReceiveIntent {
            output: Some(receive_root.clone()),
            ..intent.clone()
        },
        &config.receive.output,
        config.receive.trusted_policy == TrustedPolicy::Confirm,
        terminal.is_interactive(),
    );
    let identity = Arc::new(load_identity(dirs)?);
    let trust = trust_store(dirs);
    let policy = OfferPolicy {
        trusted_policy: if options.confirm_trusted {
            TrustedPolicy::Confirm
        } else {
            TrustedPolicy::Auto
        },
        ..OfferPolicy::default()
    };
    let offers = Arc::new(OfferManager::new(trust.clone(), policy));
    let (progress_tx, mut progress_rx) = mpsc::channel::<ReceiverProgressEvent>(64);
    let receiver = Arc::new(
        ReceiverService::new_with_progress(
            Arc::clone(&offers),
            &receive_root,
            ReceiverPolicy {
                conflict: config.receive.conflict,
                max_receive_tasks: config.transfer.max_receive_tasks,
                max_file_streams: config.transfer.concurrent_files,
            },
            Some(progress_tx),
        )
        .map_err(|error| AppError::Filesystem(error.to_string()))?,
    );
    let bind_ip = resolve_bind_ip(
        intent.bind.as_deref().unwrap_or(&config.network.bind),
        config,
    )?;
    let port = intent.port.unwrap_or(config.network.port);
    let listener = TcpListener::bind(SocketAddr::new(bind_ip, port))
        .await
        .map_err(|error| AppError::Network(format!("cannot bind receiver: {error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::Network(error.to_string()))?;
    let advertisement = Advertisement {
        device_id: identity.device_id(),
        name: config.device.name.clone(),
        version: ProtocolVersion::V1_0,
        capabilities: capabilities(),
        static_key_fingerprint: *blake3::hash(&identity.public_key()).as_bytes(),
        port: address.port(),
    };
    let _registration = match MdnsRegistration::start_for_ips(
        &advertisement,
        config.discovery.include_virtual,
        &[bind_ip],
    ) {
        Ok(registration) => Some(registration),
        Err(_) if bind_ip.is_loopback() => {
            terminal.warning("loopback listener is not advertised over mDNS; use --peer host:port");
            None
        }
        Err(error) => return Err(AppError::Network(error.to_string())),
    };
    if let Ok(diagnostic) = receive_network_diagnostic()
        && let Some(warning) = diagnostic.actionable_warning
    {
        terminal.warning(&warning);
    }
    terminal.startup(&ReceiveStartup {
        device_name: config.device.name.clone(),
        output: receive_root.clone(),
        bind_description: address.to_string(),
        encrypted: true,
        trusted_auto_accept: !options.confirm_trusted,
    });
    let prompt = Arc::new(CliPrompt {
        terminal,
        options: options.clone(),
        text_only: text_output.is_some(),
        prompt_lock: tokio::sync::Mutex::new(()),
    });
    let server = Arc::new(ServerContext {
        identity,
        trust_store: trust,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_0,
            device: DeviceInfo {
                device_id: advertisement.device_id,
                name: advertisement.name,
                capabilities: capabilities(),
            },
        },
        offers,
        receiver: Arc::clone(&receiver),
        prompt,
        selection: None,
        expected_offers: None,
        operation_timeout: Duration::from_secs(30),
    });
    let shutdown = Arc::new(crate::orchestration::ShutdownController::default());
    let signal_shutdown = Arc::clone(&shutdown);
    let signal_task = tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            match signal_shutdown.request() {
                crate::orchestration::ShutdownAction::GracefulSnapshot => {
                    eprintln!("Graceful shutdown requested; persisting resumable state...");
                }
                crate::orchestration::ShutdownAction::Force => {
                    eprintln!("Second interrupt received; forcing termination.");
                    std::process::exit(130);
                }
            }
        }
    });
    let cancellation = shutdown.cancellation();
    let progress_terminal = terminal;
    let progress_task = tokio::spawn(async move {
        while let Some(event) = progress_rx.recv().await {
            let converted = ProgressEvent {
                current_bytes: event.received_bytes,
                total_bytes: event.total_bytes,
                bytes_per_second: 0.0,
                eta: None,
            };
            progress_terminal.progress(converted);
        }
    });

    let session_cancellation = cancellation.child_token();
    let session_limit = Arc::new(Semaphore::new(16));
    let (session_tx, mut session_rx) =
        mpsc::channel::<Result<ServerSessionOutcome, DirectError>>(16);
    let result = loop {
        tokio::select! {
            _ = cancellation.cancelled() => break Ok(()),
            accepted = listener.accept() => {
                let (stream, peer_address) = match accepted {
                    Ok(value) => value,
                    Err(error) => break Err(AppError::Network(error.to_string())),
                };
                let permit = match Arc::clone(&session_limit).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        terminal.warning("too many concurrent handshake sessions; connection dropped");
                        continue;
                    }
                };
                let server = Arc::clone(&server);
                let sender = session_tx.clone();
                let connection_cancel = session_cancellation.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let outcome = server
                        .serve_stream(stream, peer_address, connection_cancel)
                        .await;
                    let _ = sender.send(outcome).await;
                });
            }
            outcome = session_rx.recv() => {
                let Some(outcome) = outcome else {
                    break Err(AppError::Network("receiver session dispatcher stopped".to_owned()));
                };
                match outcome {
                    Ok(ServerSessionOutcome::TransferCompleted { peer, transfer_id }) => {
                        if let Some(payload) = receiver
                            .completed_text(&peer, transfer_id)
                            .map_err(|error| AppError::Filesystem(error.to_string()))?
                        {
                            deliver_cli_text(&payload, text_output.as_deref())?;
                        }
                        receiver
                            .cleanup(transfer_id)
                            .map_err(|error| AppError::Filesystem(error.to_string()))?;
                        if options.once {
                            break Ok(());
                        }
                    }
                    Ok(ServerSessionOutcome::OfferRejected | ServerSessionOutcome::TransferCancelled { .. }) => {
                        if options.once {
                            break Ok(());
                        }
                    }
                    Ok(
                        ServerSessionOutcome::SelectionReady { .. }
                        | ServerSessionOutcome::SelectionFinished { .. },
                    ) => terminal.warning(
                        "ignored an unexpected desktop selection outcome in terminal receive mode",
                    ),
                    Ok(ServerSessionOutcome::Disconnected) => {}
                    Err(error) if options.once => break Err(map_direct(error)),
                    Err(error) => terminal.warning(&map_direct(error).to_string()),
                }
            }
        }
    };
    session_cancellation.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while session_limit.available_permits() != 16 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    receiver
        .pause_all()
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
    signal_task.abort();
    drop(server);
    drop(receiver);
    progress_task.abort();
    result
}

struct CliPrompt {
    terminal: ConsoleTerminal,
    options: ReceiveOptions,
    text_only: bool,
    prompt_lock: tokio::sync::Mutex<()>,
}

#[async_trait]
impl OfferPrompt for CliPrompt {
    async fn prepare_expected(
        &self,
        _peer: &quick_share_transfer::auth::PeerAuthContext,
        _view: &OfferView,
        _offer: &quick_share_protocol::TransferOffer,
    ) -> Result<(), DirectError> {
        Ok(())
    }

    async fn decide(
        &self,
        _peer: &quick_share_transfer::auth::PeerAuthContext,
        view: &OfferView,
        _offer: &quick_share_protocol::TransferOffer,
    ) -> Result<(OfferDecision, bool), DirectError> {
        let _prompt_guard = self.prompt_lock.lock().await;
        if self.text_only && view.content_kind != ContentKind::Text {
            return Ok((
                OfferDecision::Reject {
                    reason: quick_share_protocol::RejectionReason::Policy,
                },
                false,
            ));
        }
        let terminal = self.terminal;
        let options = self.options.clone();
        let view = view.clone();
        tokio::task::spawn_blocking(move || {
            decide_offer_with_sas(&terminal, &view, &options)
                .map_err(|_| DirectError::InvalidResponse)
        })
        .await
        .map_err(|_| DirectError::InvalidResponse)?
    }
}

fn deliver_cli_text(
    payload: &quick_share_transfer::text::TextPayload,
    output: Option<&Path>,
) -> Result<(), AppError> {
    let summary = payload.summary();
    eprintln!(
        "Received {:?} text: {} bytes, {} characters, digest {}",
        summary.source, summary.bytes, summary.characters, summary.digest_prefix
    );
    let mut clipboard: Box<dyn Clipboard> = match NativeClipboard::connect() {
        Ok(clipboard) => Box::new(clipboard),
        Err(error) => Box::new(UnavailableClipboard(error)),
    };
    let stdout = std::io::stdout();
    let delivery = if stdout.is_terminal() {
        let mut lock = stdout.lock();
        let mut safe = SafeTerminalOutput(&mut lock);
        deliver_received_text(payload, clipboard.as_mut(), output, &mut safe)?
    } else {
        deliver_received_text(payload, clipboard.as_mut(), output, &mut stdout.lock())?
    };
    match delivery.target {
        TextDeliveryTarget::Clipboard => eprintln!("Text copied to the clipboard."),
        TextDeliveryTarget::File(path) => {
            eprintln!(
                "Text saved to {}.",
                terminal_safe(&path.display().to_string())
            );
        }
        TextDeliveryTarget::Stdout => eprintln!("\nClipboard unavailable; text written to stdout."),
    }
    if let Some(warning) = delivery.clipboard_warning {
        eprintln!("warning: {}", terminal_safe(&warning.to_string()));
    }
    Ok(())
}

struct SafeTerminalOutput<'a, W: Write>(&'a mut W);

impl<W: Write> Write for SafeTerminalOutput<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "text is not valid UTF-8")
        })?;
        self.0.write_all(terminal_safe(text).as_bytes())?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

struct UnavailableClipboard(ClipboardError);

impl Clipboard for UnavailableClipboard {
    fn read_text(&mut self) -> Result<String, ClipboardError> {
        Err(self.0.clone())
    }

    fn write_text(&mut self, _text: &str) -> Result<(), ClipboardError> {
        Err(self.0.clone())
    }
}

pub(crate) fn resolve_bind_ip(value: &str, config: &AppConfig) -> Result<IpAddr, AppError> {
    if value != "lan" {
        return value.parse().map_err(|_| {
            AppError::Usage("network bind must be 'lan' or an IP address".to_owned())
        });
    }
    let interfaces = filter_lan_interfaces(
        system_interfaces().map_err(|error| AppError::Network(error.to_string()))?,
        config.discovery.include_virtual,
    );
    interfaces
        .iter()
        .find(|item| matches!(item.ip, IpAddr::V4(address) if address.is_private()))
        .or_else(|| interfaces.first())
        .map(|item| item.ip)
        .ok_or_else(|| AppError::Network("no eligible LAN interface is available".to_owned()))
}

fn looks_like_file(path: &Path) -> bool {
    path.is_file() || (!path.exists() && path.extension().is_some())
}

pub(crate) fn sender_policy(config: &AppConfig) -> Result<SenderPolicy, AppError> {
    Ok(SenderPolicy {
        concurrent_files: config.transfer.concurrent_files,
        queue_capacity: config.transfer.concurrent_files.saturating_mul(4).max(1),
        retry: RetryPolicy {
            max_attempts: u32::from(config.transfer.retry_count),
            ..RetryPolicy::default()
        },
    })
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

fn offer_digest(offer: &TransferOffer) -> Result<[u8; 32], AppError> {
    serde_json::to_vec(offer)
        .map(|bytes| *blake3::hash(&bytes).as_bytes())
        .map_err(|error| AppError::Usage(error.to_string()))
}

pub(crate) fn load_identity(dirs: &AppDirs) -> Result<DeviceIdentity, AppError> {
    IdentityStore::new(dirs.data_dir().join("identity.json"))
        .load_or_create()
        .map_err(|error| AppError::Identity(error.to_string()))
}

pub(crate) fn trust_store(dirs: &AppDirs) -> TrustedDeviceStore {
    TrustedDeviceStore::new(dirs.data_dir().join("trusted-devices.toml"))
}

fn load_config(dirs: &AppDirs, explicit: Option<&Path>) -> Result<AppConfig, AppError> {
    let loader = ConfigLoader::new(dirs.clone());
    let environment = std::env::vars().collect::<BTreeMap<_, _>>();
    if let Some(path) = explicit {
        let source =
            fs::read_to_string(path).map_err(|error| AppError::Config(error.to_string()))?;
        loader
            .load_from_str(Some(&source), &environment, &ConfigOverrides::default())
            .map_err(|error| AppError::Config(error.to_string()))
    } else {
        loader
            .load(&environment, &ConfigOverrides::default())
            .map_err(|error| AppError::Config(error.to_string()))
    }
}

fn run_config(
    dirs: &AppDirs,
    explicit: Option<&Path>,
    mut config: AppConfig,
    command: ConfigIntent,
) -> Result<(), AppError> {
    let path = explicit
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dirs.config_dir().join("config.toml"));
    match command {
        ConfigIntent::Path => {
            println!("{}", path.display());
            Ok(())
        }
        ConfigIntent::Show { toml: _ } => {
            let rendered = toml::to_string_pretty(&config)
                .map_err(|error| AppError::Config(error.to_string()))?;
            print!("{rendered}");
            Ok(())
        }
        ConfigIntent::Set { key, value } => {
            set_config_value(&mut config, key, &value)?;
            let rendered = toml::to_string_pretty(&config)
                .map_err(|error| AppError::Config(error.to_string()))?;
            ConfigLoader::new(dirs.clone())
                .load_from_str(
                    Some(&rendered),
                    &BTreeMap::new(),
                    &ConfigOverrides::default(),
                )
                .map_err(|error| AppError::Config(error.to_string()))?;
            atomic_write(path, rendered.as_bytes(), FileSensitivity::Normal)
                .map_err(|error| AppError::Filesystem(error.to_string()))?;
            Ok(())
        }
    }
}

fn set_config_value(config: &mut AppConfig, key: ConfigKey, value: &str) -> Result<(), AppError> {
    match key {
        ConfigKey::DeviceName => config.device.name = value.to_owned(),
        ConfigKey::ReceiveOutput => config.receive.output = PathBuf::from(value),
        ConfigKey::ReceiveTrustedPolicy => {
            config.receive.trusted_policy = match value {
                "auto" => TrustedPolicy::Auto,
                "confirm" => TrustedPolicy::Confirm,
                _ => return Err(AppError::Config("expected auto or confirm".to_owned())),
            };
        }
        ConfigKey::ReceiveConflict => {
            config.receive.conflict = match value {
                "rename" => ConflictPolicy::Rename,
                "ask" => ConflictPolicy::Ask,
                "skip" => ConflictPolicy::Skip,
                "overwrite" => ConflictPolicy::Overwrite,
                "error" => ConflictPolicy::Error,
                _ => return Err(AppError::Config("invalid conflict policy".to_owned())),
            };
        }
        ConfigKey::DiscoveryTimeoutMs => {
            config.discovery.timeout_ms = parse_config_value(value, "timeout")?
        }
        ConfigKey::DiscoveryIncludeVirtual => {
            config.discovery.include_virtual = parse_config_value(value, "boolean")?
        }
        ConfigKey::NetworkPort => config.network.port = parse_config_value(value, "port")?,
        ConfigKey::NetworkBind => config.network.bind = value.to_owned(),
        ConfigKey::TransferChunkSize => {
            config.transfer.chunk_size = parse_config_value(value, "chunk size")?
        }
        ConfigKey::TransferConcurrentFiles => {
            config.transfer.concurrent_files = parse_config_value(value, "concurrency")?
        }
        ConfigKey::TransferMaxReceiveTasks => {
            config.transfer.max_receive_tasks = parse_config_value(value, "task limit")?
        }
        ConfigKey::TransferRetryCount => {
            config.transfer.retry_count = parse_config_value(value, "retry count")?
        }
        ConfigKey::WebTimeout => config.web.timeout = value.to_owned(),
        ConfigKey::WebMaxDownloads => {
            config.web.max_downloads = parse_config_value(value, "download limit")?
        }
        ConfigKey::WebUpload => config.web.upload = parse_config_value(value, "boolean")?,
        ConfigKey::WebAllowHttp => config.web.allow_http = parse_config_value(value, "boolean")?,
    }
    Ok(())
}

fn parse_config_value<T>(value: &str, name: &str) -> Result<T, AppError>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| AppError::Config(format!("invalid value for {name}")))
}

fn map_direct(error: DirectError) -> AppError {
    match error {
        DirectError::Rejected => AppError::Rejected(error.to_string()),
        DirectError::IdentityMismatch
        | DirectError::IncompatibleProtocol
        | DirectError::Authentication(_)
        | DirectError::Noise(quick_share_transfer::noise::NoiseError::RemoteIdentityMismatch) => {
            AppError::Identity(error.to_string())
        }
        DirectError::Connect(_) | DirectError::ConnectTimeout => {
            AppError::PeerUnavailable(error.to_string())
        }
        DirectError::Receiver(_) => AppError::Filesystem(error.to_string()),
        DirectError::Remote(remote) => match remote.code {
            quick_share_protocol::ErrorCode::IntegrityFailed => {
                AppError::Integrity(remote.to_string())
            }
            quick_share_protocol::ErrorCode::Unauthorized => AppError::Identity(remote.to_string()),
            quick_share_protocol::ErrorCode::Rejected => AppError::Rejected(remote.to_string()),
            quick_share_protocol::ErrorCode::ResourceLimit
            | quick_share_protocol::ErrorCode::IncompatibleProtocol
            | quick_share_protocol::ErrorCode::InvalidMessage
            | quick_share_protocol::ErrorCode::NotFound
            | quick_share_protocol::ErrorCode::Internal => AppError::Network(remote.to_string()),
        },
        DirectError::Noise(_) | DirectError::Network(_) => AppError::Network(error.to_string()),
        DirectError::Offer(_)
        | DirectError::Selection(_)
        | DirectError::ExpectedOffer(_)
        | DirectError::InvalidResponse
        | DirectError::Unauthorized
        | DirectError::OfferChannel
        | DirectError::Random => AppError::Identity(error.to_string()),
    }
}

fn map_sender(error: quick_share_transfer::sender::SenderError) -> AppError {
    use quick_share_transfer::sender::SenderError;
    match error {
        SenderError::SourceChanged(_) | SenderError::Io(_) => {
            AppError::Filesystem(error.to_string())
        }
        SenderError::RemoteTerminal(_) => AppError::Rejected(error.to_string()),
        SenderError::ResumeMismatch | SenderError::InvalidResponse => {
            AppError::Integrity(error.to_string())
        }
        SenderError::Cancelled => AppError::Cancelled,
        SenderError::Transport(quick_share_transfer::sender::TransportError::Integrity) => {
            AppError::Integrity(error.to_string())
        }
        SenderError::Transport(quick_share_transfer::sender::TransportError::Unauthorized) => {
            AppError::Identity(error.to_string())
        }
        SenderError::Transport(quick_share_transfer::sender::TransportError::Cancelled) => {
            AppError::Rejected(error.to_string())
        }
        SenderError::Transport(
            quick_share_transfer::sender::TransportError::Retryable
            | quick_share_transfer::sender::TransportError::Fatal
            | quick_share_transfer::sender::TransportError::ResourceLimit,
        )
        | SenderError::WorkerFailed => AppError::Network(error.to_string()),
        SenderError::InvalidPlan(_) | SenderError::Store(_) => AppError::Usage(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{looks_like_file, parse_web_duration};
    use std::fs;

    #[test]
    fn receive_output_classification_is_deterministic() {
        let root = tempfile::tempdir().expect("temporary output root");
        assert!(!looks_like_file(root.path()));
        assert!(!looks_like_file(&root.path().join("new-directory")));
        assert!(looks_like_file(&root.path().join("message.txt")));

        let extensionless_file = root.path().join("existing-file");
        fs::write(&extensionless_file, b"occupied").expect("existing file");
        assert!(looks_like_file(&extensionless_file));
    }

    #[test]
    fn web_timeout_parser_is_bounded_and_uses_explicit_units() {
        assert_eq!(parse_web_duration("30s").expect("seconds").as_secs(), 30);
        assert_eq!(parse_web_duration("5m").expect("minutes").as_secs(), 300);
        assert_eq!(parse_web_duration("1h").expect("hours").as_secs(), 3600);
        for invalid in ["", "0s", "30", "1d", "25h", "999999999999999999999h"] {
            assert!(parse_web_duration(invalid).is_err(), "{invalid}");
        }
    }
}
