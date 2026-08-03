//! Windows resident agent orchestration.

use crate::{
    AgentIntent, AppError,
    app::{
        PreparedPathContent, bind_error, finalize_path_transfer, load_identity,
        prepare_path_content, resolve_bind_ip, sender_policy, trust_store,
    },
    desktop_prompt::{
        AgentReceiver, DesktopOfferPrompt, SourceDirectoryStore, callback_notification,
        incoming_transfer_notification,
    },
};
use async_trait::async_trait;
use quick_share_core::{
    config::{AppConfig, TrustedPolicy},
    receive::{ReceiveBindingStore, ReceiveDestinationStore},
};
use quick_share_discovery::{Advertisement, MdnsRegistration};
use quick_share_platform::{
    AppDirs,
    desktop::{
        AuthorizationChoice, AuthorizationDialog, DesktopError, DesktopInteraction,
        DesktopNotification, SourceChoice, SourceDialog, windows::TrayAction,
    },
};
use quick_share_protocol::{
    Capability, DeviceInfo, InfoRequest, InfoResponse, ProtocolVersion, RequestId, TransferId,
};
use quick_share_transfer::{
    auth::PeerAuthContext,
    direct::{
        ClientConnector, DirectError, NoiseClientTransport, ServerContext, ServerSessionOutcome,
    },
    offer::{OfferManager, OfferPolicy},
    receiver::{ReceiverPolicy, ReceiverProgressEvent},
    receiver_router::ReceiverRouter,
    selection::{
        CallbackTarget, SelectionAuthorization, SelectionHandler, SelectionHandlerError,
        SelectionManager, SelectionPolicy,
    },
    sender::TransferSender,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    sync::{Arc, Mutex, MutexGuard, mpsc as std_mpsc},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct PreparedSelection {
    content: PreparedPathContent,
    callback_permit: tokio::sync::OwnedSemaphorePermit,
}

struct DesktopSelectionHandler {
    desktop: Arc<dyn DesktopInteraction>,
    callback_limit: Arc<Semaphore>,
    source_directories: SourceDirectoryStore,
    prepared: Mutex<BTreeMap<TransferId, PreparedSelection>>,
}

impl std::fmt::Debug for DesktopSelectionHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopSelectionHandler")
            .field("desktop", &"[DESKTOP]")
            .field("source_directories", &"[PREFERENCE STORE]")
            .field("prepared", &"[REDACTED]")
            .finish()
    }
}

impl DesktopSelectionHandler {
    fn new(
        desktop: Arc<dyn DesktopInteraction>,
        callback_limit: Arc<Semaphore>,
        source_directories: SourceDirectoryStore,
    ) -> Self {
        Self {
            desktop,
            callback_limit,
            source_directories,
            prepared: Mutex::new(BTreeMap::new()),
        }
    }

    fn take(
        &self,
        transfer_id: TransferId,
    ) -> Option<(PreparedPathContent, tokio::sync::OwnedSemaphorePermit)> {
        self.prepared
            .lock()
            .ok()?
            .remove(&transfer_id)
            .map(|selected| (selected.content, selected.callback_permit))
    }

    fn lock_prepared(
        &self,
    ) -> Result<MutexGuard<'_, BTreeMap<TransferId, PreparedSelection>>, SelectionHandlerError>
    {
        self.prepared
            .lock()
            .map_err(|_| SelectionHandlerError::Failed)
    }
}

#[async_trait]
impl SelectionHandler for DesktopSelectionHandler {
    async fn authorize(
        &self,
        peer: &PeerAuthContext,
        cancellation: CancellationToken,
    ) -> Result<SelectionAuthorization, SelectionHandlerError> {
        if cancellation.is_cancelled() {
            return Err(SelectionHandlerError::Cancelled);
        }
        let desktop = Arc::clone(&self.desktop);
        let dialog = AuthorizationDialog {
            device_name: peer.name().to_owned(),
            device_id: peer.device_id().to_string(),
            sas: peer.sas().to_string(),
            identity_changed: peer.change_reason().is_some(),
        };
        let choice = tokio::task::spawn_blocking(move || desktop.authorize_peer(&dialog))
            .await
            .map_err(|_| SelectionHandlerError::Failed)?
            .map_err(map_desktop_selection)?;
        if cancellation.is_cancelled() {
            return Err(SelectionHandlerError::Cancelled);
        }
        Ok(match choice {
            AuthorizationChoice::AcceptOnce => SelectionAuthorization::AcceptOnce,
            AuthorizationChoice::AcceptAndTrust => {
                SelectionAuthorization::AcceptAndTrust { sas_verified: true }
            }
            AuthorizationChoice::Reject => SelectionAuthorization::Reject,
            AuthorizationChoice::Cancelled => return Err(SelectionHandlerError::Cancelled),
        })
    }

    async fn select_source(
        &self,
        peer: &PeerAuthContext,
        cancellation: CancellationToken,
    ) -> Result<TransferId, SelectionHandlerError> {
        if cancellation.is_cancelled() {
            return Err(SelectionHandlerError::Cancelled);
        }
        let callback_permit = Arc::clone(&self.callback_limit)
            .try_acquire_owned()
            .map_err(|_| SelectionHandlerError::Busy)?;
        let desktop = Arc::clone(&self.desktop);
        let source_directories = self.source_directories.clone();
        let requester_name = peer.name().to_owned();
        let choice = tokio::task::spawn_blocking(move || {
            let initial_directory = source_directories
                .load()
                .ok()
                .flatten()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            let dialog = SourceDialog {
                requester_name,
                initial_directory,
            };
            let choice = desktop.choose_send_source(&dialog)?;
            if let Err(error) = source_directories.remember_choice(&choice) {
                eprintln!(
                    "Quick Share could not remember the source directory: {}",
                    crate::terminal::terminal_safe(&error)
                );
            }
            Ok::<SourceChoice, DesktopError>(choice)
        })
        .await
        .map_err(|_| SelectionHandlerError::Failed)?
        .map_err(map_desktop_selection)?;
        let paths = match choice {
            SourceChoice::Files(paths) if !paths.is_empty() => paths,
            SourceChoice::Folder(path) if !path.as_os_str().is_empty() => vec![path],
            SourceChoice::Files(_) | SourceChoice::Folder(_) => {
                return Err(SelectionHandlerError::Failed);
            }
            SourceChoice::Cancelled => return Err(SelectionHandlerError::Cancelled),
        };
        let prepared = tokio::task::spawn_blocking(move || prepare_path_content(&paths, false))
            .await
            .map_err(|_| SelectionHandlerError::Failed)?
            .map_err(|_| SelectionHandlerError::Failed)?;
        if cancellation.is_cancelled() {
            return Err(SelectionHandlerError::Cancelled);
        }
        let transfer_id = TransferId::new(Uuid::now_v7());
        self.lock_prepared()?.insert(
            transfer_id,
            PreparedSelection {
                content: prepared,
                callback_permit,
            },
        );
        Ok(transfer_id)
    }
}

fn map_desktop_selection(
    error: quick_share_platform::desktop::DesktopError,
) -> SelectionHandlerError {
    use quick_share_platform::desktop::DesktopError;
    match error {
        DesktopError::Unsupported
        | DesktopError::Unavailable
        | DesktopError::TimedOut
        | DesktopError::EventLoopExited => SelectionHandlerError::UiUnavailable,
        DesktopError::Busy => SelectionHandlerError::Busy,
        DesktopError::InvalidSelection
        | DesktopError::InvalidDirectory
        | DesktopError::DirectoryNotWritable
        | DesktopError::Backend => SelectionHandlerError::Failed,
    }
}

pub(crate) struct AgentBackground {
    pub dirs: AppDirs,
    pub config: AppConfig,
    pub intent: AgentIntent,
    pub desktop: Arc<dyn DesktopInteraction>,
    pub tray_events: std_mpsc::Receiver<TrayAction>,
    pub cancellation: CancellationToken,
}

pub(crate) async fn run_background(background: AgentBackground) -> Result<(), AppError> {
    let AgentBackground {
        dirs,
        config,
        intent,
        desktop,
        tray_events,
        cancellation,
    } = background;
    let identity = Arc::new(load_identity(&dirs)?);
    let trust = trust_store(&dirs);
    let offers = Arc::new(OfferManager::new(
        trust.clone(),
        OfferPolicy {
            trusted_policy: TrustedPolicy::Confirm,
            ..OfferPolicy::default()
        },
    ));
    let bindings = ReceiveBindingStore::new(dirs.data_dir().join("receive-bindings.json"));
    let router = Arc::new(
        ReceiverRouter::new(
            Arc::clone(&offers),
            bindings.clone(),
            ReceiverPolicy {
                conflict: quick_share_core::config::ConflictPolicy::Error,
                max_receive_tasks: config.transfer.max_receive_tasks,
                max_file_streams: config.transfer.concurrent_files,
            },
            None::<mpsc::Sender<ReceiverProgressEvent>>,
        )
        .map_err(|error| AppError::Filesystem(error.to_string()))?,
    );
    let receiver = Arc::new(AgentReceiver::new(router, bindings, Arc::clone(&desktop)));
    let prompt = Arc::new(DesktopOfferPrompt::new(
        Arc::clone(&desktop),
        trust.clone(),
        ReceiveDestinationStore::new(dirs.data_dir().join("receive-destinations.json")),
        dirs.clone(),
        Arc::clone(&receiver),
    ));
    let callback_limit = Arc::new(Semaphore::new(2));
    let selection_handler = Arc::new(DesktopSelectionHandler::new(
        Arc::clone(&desktop),
        Arc::clone(&callback_limit),
        SourceDirectoryStore::new(dirs.data_dir().join("source-selection.json")),
    ));
    let selection = Arc::new(
        SelectionManager::new(
            Arc::clone(&selection_handler),
            trust.clone(),
            SelectionPolicy::default(),
        )
        .map_err(|error| AppError::Config(error.to_string()))?,
    );

    let bind_ip = resolve_bind_ip(
        intent.bind.as_deref().unwrap_or(&config.network.bind),
        &config,
    )?;
    let port = intent.port.unwrap_or(config.network.port);
    let listener = TcpListener::bind(SocketAddr::new(bind_ip, port))
        .await
        .map_err(|error| bind_error("desktop agent", error))?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::Network(error.to_string()))?;
    let advertisement = Advertisement {
        device_id: identity.device_id(),
        name: config.device.name.clone(),
        version: ProtocolVersion::V1_1,
        capabilities: agent_capabilities(),
        static_key_fingerprint: *blake3::hash(&identity.public_key()).as_bytes(),
        port: address.port(),
    };
    let registration = match MdnsRegistration::start_for_ips(
        &advertisement,
        config.discovery.include_virtual,
        &[bind_ip],
    ) {
        Ok(registration) => Some(registration),
        Err(_) if bind_ip.is_loopback() => None,
        Err(error) => return Err(AppError::Network(error.to_string())),
    };
    let mdns_enabled = registration.is_some();
    eprintln!("Quick Share agent is ready.");
    eprintln!(
        "  Device: {} ({})",
        crate::terminal::terminal_safe(&advertisement.name),
        advertisement.device_id
    );
    eprintln!("  Listen: {address}");
    eprintln!(
        "  Discovery: {}",
        if mdns_enabled {
            "mDNS enabled"
        } else {
            "mDNS unavailable (configured peers and --peer still work)"
        }
    );
    eprintln!("  Tray: use Exit to stop the agent");

    let server = Arc::new(ServerContext {
        identity: Arc::clone(&identity),
        trust_store: trust,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_1,
            device: DeviceInfo {
                device_id: advertisement.device_id,
                name: advertisement.name,
                capabilities: agent_capabilities(),
            },
        },
        offers,
        receiver: Arc::clone(&receiver),
        prompt,
        selection: Some(selection),
        expected_offers: None,
        operation_timeout: Duration::from_secs(30),
    });

    let (tray_sender, mut tray_receiver) = mpsc::channel(8);
    std::thread::spawn(move || {
        while let Ok(event) = tray_events.recv() {
            if tray_sender.blocking_send(event).is_err() {
                break;
            }
        }
    });
    let session_limit = Arc::new(Semaphore::new(16));
    let (session_sender, mut session_receiver) =
        mpsc::channel::<Result<ServerSessionOutcome, DirectError>>(16);
    let mut callbacks = JoinSet::new();
    let session_cancellation = cancellation.child_token();

    let result = loop {
        tokio::select! {
            _ = cancellation.cancelled() => {
                eprintln!("Quick Share agent is stopping: desktop event loop closed.");
                break Ok(())
            },
            signal = tokio::signal::ctrl_c() => {
                if signal.is_ok() {
                    eprintln!("Quick Share agent is stopping: Ctrl+C.");
                    break Ok(());
                }
            }
            event = tray_receiver.recv() => match event {
                Some(TrayAction::Open) => {
                    eprintln!("Quick Share agent status requested from the tray.");
                    notify_desktop(
                        Arc::clone(&desktop),
                        "Quick Share agent is running and ready for requests.",
                    )
                    .await;
                }
                Some(TrayAction::Exit) | None => {
                    eprintln!("Quick Share agent is stopping: tray Exit.");
                    break Ok(())
                },
            },
            accepted = listener.accept() => {
                let (stream, peer_address) = match accepted {
                    Ok(value) => value,
                    Err(error) => break Err(AppError::Network(error.to_string())),
                };
                let permit = match Arc::clone(&session_limit).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        eprintln!("Quick Share agent is busy: connection limit reached.");
                        continue
                    },
                };
                let server = Arc::clone(&server);
                let sender = session_sender.clone();
                let connection_cancellation = session_cancellation.child_token();
                tokio::spawn(async move {
                    let _permit = permit;
                    let outcome = server
                        .serve_stream(stream, peer_address, connection_cancellation)
                        .await;
                    let _ = sender.send(outcome).await;
                });
            }
            outcome = session_receiver.recv() => {
                let Some(outcome) = outcome else {
                    break Err(AppError::Network("agent session dispatcher stopped".to_owned()));
                };
                match outcome {
                    Ok(ServerSessionOutcome::TransferCompleted { transfer_id, .. }) => {
                        receiver.cleanup(transfer_id)
                            .map_err(|error| AppError::Filesystem(error.to_string()))?;
                        eprintln!("Incoming transfer completed: {}", transfer_id.as_uuid());
                    }
                    Ok(ServerSessionOutcome::SelectionReady {
                        request_id,
                        transfer_id,
                        callback,
                        ..
                    }) => {
                        eprintln!(
                            "Remote source request accepted: transfer {}",
                            transfer_id.as_uuid()
                        );
                        let Some((content, permit)) = selection_handler.take(transfer_id) else {
                            eprintln!("Remote source request failed: prepared selection was unavailable.");
                            continue;
                        };
                        let identity = Arc::clone(&identity);
                        let config = config.clone();
                        let cancellation = cancellation.child_token();
                        let desktop = Arc::clone(&desktop);
                        callbacks.spawn(async move {
                            let _permit = permit;
                            let result = send_callback(
                                identity,
                                config,
                                callback,
                                request_id,
                                transfer_id,
                                content,
                                cancellation,
                            )
                            .await;
                            if let Some(notification) = callback_notification(result.is_ok()) {
                                notify_desktop(desktop, notification).await;
                            }
                            (transfer_id, result)
                        });
                    }
                    Ok(ServerSessionOutcome::TransferCancelled { transfer_id, .. }) => {
                        let _ = receiver.cleanup(transfer_id);
                        eprintln!("Transfer cancelled: {}", transfer_id.as_uuid());
                        if let Some(notification) = incoming_transfer_notification(false) {
                            notify_desktop(Arc::clone(&desktop), notification).await;
                        }
                    }
                    Ok(ServerSessionOutcome::SelectionFinished { status, .. }) => {
                        eprintln!("Remote source request finished: {status:?}");
                    }
                    Ok(ServerSessionOutcome::OfferRejected) => {
                        eprintln!("Incoming transfer offer was rejected.");
                    }
                    Ok(ServerSessionOutcome::Disconnected) => {}
                    Err(error) => {
                        eprintln!(
                            "Agent session failed: {}",
                            crate::terminal::terminal_safe(&error.to_string())
                        );
                    }
                }
            }
            joined = callbacks.join_next(), if !callbacks.is_empty() => {
                match joined {
                    Some(Ok((transfer_id, Ok(())))) => {
                        eprintln!(
                            "Selected content sent successfully: {}",
                            transfer_id.as_uuid()
                        );
                    }
                    Some(Ok((transfer_id, Err(error)))) => {
                        eprintln!(
                            "Selected content send failed ({}): {}",
                            transfer_id.as_uuid(),
                            crate::terminal::terminal_safe(&error.to_string())
                        );
                    }
                    Some(Err(error)) => {
                        eprintln!("Callback task failed: {error}");
                    }
                    None => {}
                }
            }
        }
    };
    session_cancellation.cancel();
    callbacks.abort_all();
    while callbacks.join_next().await.is_some() {}
    receiver
        .pause_all()
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
    eprintln!("Quick Share agent stopped.");
    result
}

async fn notify_desktop(desktop: Arc<dyn DesktopInteraction>, message: &str) {
    let notification = DesktopNotification {
        title: "Quick Share".to_owned(),
        message: message.to_owned(),
    };
    let _ = tokio::task::spawn_blocking(move || desktop.notify(&notification)).await;
}

async fn send_callback(
    identity: Arc<quick_share_core::identity::DeviceIdentity>,
    config: AppConfig,
    callback: CallbackTarget,
    request_id: RequestId,
    transfer_id: TransferId,
    content: PreparedPathContent,
    cancellation: CancellationToken,
) -> Result<(), AppError> {
    let connector = ClientConnector::new(
        callback.endpoint,
        Arc::clone(&identity),
        Some(callback.requester_device_id),
        Some(callback.requester_public_key),
        InfoRequest {
            protocol_version: ProtocolVersion::V1_1,
            capabilities: agent_capabilities(),
        },
        Duration::from_secs(30),
    );
    let connected = connector
        .connect()
        .await
        .map_err(|error| AppError::Network(error.to_string()))?;
    let transport = Arc::new(NoiseClientTransport::new(connector, connected));
    let sender_info = DeviceInfo {
        device_id: identity.device_id(),
        name: config.device.name.clone(),
        capabilities: agent_capabilities(),
    };
    let (offer, plan) = finalize_path_transfer(
        content,
        sender_info,
        config.transfer.chunk_size,
        transfer_id,
        Some(request_id),
    )?;
    let token = transport
        .create_offer(offer)
        .await
        .map_err(|error| AppError::Network(error.to_string()))?;
    let sender = TransferSender::new(Arc::clone(&transport), sender_policy(&config)?)
        .map_err(|error| AppError::Usage(error.to_string()))?;
    sender
        .send(plan, &token, cancellation, None)
        .await
        .map(|_| ())
        .map_err(|error| AppError::Network(error.to_string()))
}

pub(crate) fn agent_capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([
        Capability::Files,
        Capability::Directories,
        Capability::Symlinks,
        Capability::Resume,
        Capability::RemoteSelection,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_share_core::identity::{IdentityStore, TrustedDeviceStore};
    use quick_share_platform::desktop::{
        ConflictChoice, ConflictDialog, DirectoryChoice, ReceiveDirectoryDialog,
    };
    use quick_share_transfer::{
        auth::{PeerClaim, classify_peer},
        noise::NoiseHandshake,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeDesktop {
        source: SourceChoice,
        authorization: AuthorizationChoice,
        source_calls: AtomicUsize,
    }

    impl DesktopInteraction for FakeDesktop {
        fn authorize_peer(
            &self,
            _request: &AuthorizationDialog,
        ) -> Result<AuthorizationChoice, quick_share_platform::desktop::DesktopError> {
            Ok(self.authorization)
        }

        fn choose_send_source(
            &self,
            _request: &SourceDialog,
        ) -> Result<SourceChoice, quick_share_platform::desktop::DesktopError> {
            self.source_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.source.clone())
        }

        fn confirm_receive_directory(
            &self,
            _request: &ReceiveDirectoryDialog,
        ) -> Result<DirectoryChoice, quick_share_platform::desktop::DesktopError> {
            Ok(DirectoryChoice::Cancelled)
        }

        fn resolve_conflict(
            &self,
            _request: &ConflictDialog,
        ) -> Result<ConflictChoice, quick_share_platform::desktop::DesktopError> {
            Ok(ConflictChoice::Cancelled)
        }

        fn notify(
            &self,
            _notification: &DesktopNotification,
        ) -> Result<(), quick_share_platform::desktop::DesktopError> {
            Ok(())
        }
    }

    fn unknown_peer(root: &std::path::Path) -> PeerAuthContext {
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
        let evidence = initiator.finish(None).expect("finish").1;
        classify_peer(
            PeerClaim {
                device_id: remote.device_id(),
                name: "requester".to_owned(),
            },
            &evidence,
            &TrustedDeviceStore::new(root.join("trust.toml")),
        )
        .expect("peer")
    }

    #[tokio::test]
    async fn fake_desktop_authorizes_prepares_files_and_honors_pre_cancel() {
        let root = tempfile::tempdir().expect("root");
        let source = root.path().join("payload.txt");
        std::fs::write(&source, b"payload").expect("payload");
        let peer = unknown_peer(root.path());
        let desktop = Arc::new(FakeDesktop {
            source: SourceChoice::Files(vec![source]),
            authorization: AuthorizationChoice::AcceptAndTrust,
            source_calls: AtomicUsize::new(0),
        });
        let handler = DesktopSelectionHandler::new(
            desktop.clone(),
            Arc::new(Semaphore::new(1)),
            SourceDirectoryStore::new(root.path().join("source-selection.json")),
        );

        assert_eq!(
            handler
                .authorize(&peer, CancellationToken::new())
                .await
                .expect("authorization"),
            SelectionAuthorization::AcceptAndTrust { sas_verified: true }
        );
        let transfer_id = handler
            .select_source(&peer, CancellationToken::new())
            .await
            .expect("selection");
        assert!(handler.take(transfer_id).is_some());
        assert_eq!(desktop.source_calls.load(Ordering::SeqCst), 1);

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(
            handler.select_source(&peer, cancelled).await,
            Err(SelectionHandlerError::Cancelled)
        );
        assert_eq!(desktop.source_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn desktop_busy_maps_to_stable_selection_busy() {
        assert_eq!(
            map_desktop_selection(quick_share_platform::desktop::DesktopError::Busy),
            SelectionHandlerError::Busy
        );
    }
}
