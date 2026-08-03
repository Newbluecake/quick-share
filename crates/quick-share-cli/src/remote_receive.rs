//! Linux/terminal remote source request and exact expected-callback receiver.

use crate::{
    AppError, InteractionPolicy, ReceiveIntent,
    app::{bind_error, load_identity, resolve_bind_ip, trust_store},
    configured_discovery::HybridDiscovery,
    orchestration::{ReceiveStartup, ReceiveTerminal, SendTerminal},
    terminal::ConsoleTerminal,
};
use async_trait::async_trait;
use quick_share_core::{
    config::AppConfig,
    identity::{DeviceIdentity, TrustStatus},
};
use quick_share_discovery::{Discovery, ScanRequest, UnverifiedPeer};
use quick_share_platform::AppDirs;
use quick_share_protocol::{
    Capability, DeviceId, DeviceInfo, InfoRequest, InfoResponse, OfferDecision, ProtocolVersion,
    RejectionReason, SourceSelectionRequest, SourceSelectionStatus,
};
use quick_share_transfer::{
    auth::PeerAuthContext,
    direct::{
        ClientConnector, DirectError, NoiseClientTransport, OfferPrompt, ServerContext,
        ServerSessionOutcome,
    },
    expected_offer::{ExpectedCallback, ExpectedOfferPolicy, ExpectedOfferRegistry},
    offer::{OfferManager, OfferPolicy, OfferView},
    receiver::{ReceiverPolicy, ReceiverProgressEvent, ReceiverService},
};
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct RemoteAgentTarget {
    endpoint: SocketAddr,
    expected_id: Option<DeviceId>,
    expected_key: Option<[u8; 32]>,
    advertised_fingerprint: Option<[u8; 32]>,
}

#[derive(Debug)]
struct ExpectedOnlyPrompt;

#[async_trait]
impl OfferPrompt for ExpectedOnlyPrompt {
    async fn prepare_expected(
        &self,
        _peer: &PeerAuthContext,
        _view: &OfferView,
        _offer: &quick_share_protocol::TransferOffer,
    ) -> Result<(), DirectError> {
        Ok(())
    }

    async fn decide(
        &self,
        _peer: &PeerAuthContext,
        _view: &OfferView,
        _offer: &quick_share_protocol::TransferOffer,
    ) -> Result<(OfferDecision, bool), DirectError> {
        Ok((
            OfferDecision::Reject {
                reason: RejectionReason::Policy,
            },
            false,
        ))
    }
}

pub(crate) async fn run_remote_receive(
    dirs: &AppDirs,
    config: &AppConfig,
    terminal: ConsoleTerminal,
    intent: ReceiveIntent,
) -> Result<(), AppError> {
    let output_root = remote_output_root(intent.output.clone(), std::env::current_dir)?;
    std::fs::create_dir_all(&output_root)
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
    if !output_root.is_dir() {
        return Err(AppError::Filesystem(
            "remote receive output must be a directory".to_owned(),
        ));
    }
    let identity = Arc::new(load_identity(dirs)?);
    let trust = trust_store(dirs);
    let bind_ip = resolve_bind_ip(
        intent.bind.as_deref().unwrap_or(&config.network.bind),
        config,
    )?;
    let port = intent.port.unwrap_or(config.network.port);
    let listener = TcpListener::bind(SocketAddr::new(bind_ip, port))
        .await
        .map_err(|error| bind_error("callback receiver", error))?;
    let callback_address = listener
        .local_addr()
        .map_err(|error| AppError::Network(error.to_string()))?;

    let target = resolve_target(
        intent.peer.as_deref(),
        Arc::clone(&identity),
        &trust,
        config,
        terminal,
        intent.assume_yes,
    )
    .await?;
    let connector = ClientConnector::new(
        target.endpoint,
        Arc::clone(&identity),
        target.expected_id,
        target.expected_key,
        InfoRequest {
            protocol_version: ProtocolVersion::V1_1,
            capabilities: remote_receive_capabilities(),
        },
        Duration::from_secs(30),
    );
    let connected = connector.connect().await.map_err(map_direct)?;
    if target
        .advertised_fingerprint
        .is_some_and(|fingerprint| fingerprint != connected.evidence.static_key_fingerprint)
    {
        return Err(AppError::Identity(
            "authenticated agent fingerprint differs from discovery".to_owned(),
        ));
    }
    if !connected
        .negotiated
        .capabilities
        .contains(&Capability::RemoteSelection)
    {
        return Err(AppError::PeerUnavailable(
            "selected peer does not support remote source selection".to_owned(),
        ));
    }
    match trust
        .check(
            &connected.evidence.remote_device_id,
            &connected.evidence.remote_static(),
        )
        .map_err(|error| AppError::Identity(error.to_string()))?
    {
        TrustStatus::Trusted(_) => {}
        TrustStatus::KeyMismatch => {
            return Err(AppError::Identity(
                "remote agent static identity does not match the pinned key".to_owned(),
            ));
        }
        TrustStatus::Unknown => {
            let persist = terminal.confirm_tofu(
                &connected.remote_info.device.name,
                &connected.evidence.remote_device_id,
                connected.evidence.sas,
                intent.assume_yes,
            )?;
            if persist {
                trust
                    .trust_peer(
                        connected.evidence.remote_device_id.clone(),
                        &connected.remote_info.device.name,
                        connected.evidence.remote_static(),
                    )
                    .map_err(|error| AppError::Identity(error.to_string()))?;
                eprintln!("Receiver identity saved; future connections will not ask again.");
            }
        }
    }
    let sender_device_id = connected.evidence.remote_device_id.clone();
    let sender_public_key = connected.evidence.remote_static();
    let control_source_ip = target.endpoint.ip();
    let transport = NoiseClientTransport::new(connector, connected);
    let exchange = transport
        .request_source_selection(SourceSelectionRequest {
            requester: DeviceInfo {
                device_id: identity.device_id(),
                name: config.device.name.clone(),
                capabilities: remote_receive_capabilities(),
            },
            callback_port: callback_address.port(),
        })
        .await
        .map_err(map_direct)?;
    let transfer_id = selected_transfer(&exchange.response)?;

    let expected = Arc::new(
        ExpectedOfferRegistry::new(ExpectedOfferPolicy::default())
            .map_err(|error| AppError::Config(error.to_string()))?,
    );
    expected
        .register(
            ExpectedCallback {
                request_id: exchange.request_id,
                transfer_id,
                sender_device_id,
                sender_public_key,
                source_ip: control_source_ip,
                expires_at: Instant::now() + Duration::from_secs(10 * 60),
            },
            Instant::now(),
        )
        .map_err(|error| AppError::Identity(error.to_string()))?;

    let offers = Arc::new(OfferManager::new(
        trust.clone(),
        OfferPolicy {
            trusted_policy: quick_share_core::config::TrustedPolicy::Confirm,
            ..OfferPolicy::default()
        },
    ));
    let (progress_sender, mut progress_receiver) = mpsc::channel::<ReceiverProgressEvent>(64);
    let receiver = Arc::new(
        ReceiverService::new_with_progress(
            Arc::clone(&offers),
            &output_root,
            ReceiverPolicy {
                conflict: config.receive.conflict,
                max_receive_tasks: config.transfer.max_receive_tasks,
                max_file_streams: config.transfer.concurrent_files,
            },
            Some(progress_sender),
        )
        .map_err(|error| AppError::Filesystem(error.to_string()))?,
    );
    terminal.startup(&ReceiveStartup {
        device_name: config.device.name.clone(),
        output: output_root,
        bind_description: callback_address.to_string(),
        encrypted: true,
        trusted_auto_accept: false,
    });
    let local_device_id = identity.device_id();
    let server = Arc::new(ServerContext {
        identity,
        trust_store: trust,
        local_info: InfoResponse {
            protocol_version: ProtocolVersion::V1_1,
            device: DeviceInfo {
                device_id: local_device_id,
                name: config.device.name.clone(),
                capabilities: remote_receive_capabilities(),
            },
        },
        offers,
        receiver: Arc::clone(&receiver),
        prompt: Arc::new(ExpectedOnlyPrompt),
        selection: None,
        expected_offers: Some(expected),
        operation_timeout: Duration::from_secs(30),
    });

    let cancellation = CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancellation.cancel();
        }
    });
    let progress_terminal = terminal;
    let progress = tokio::spawn(async move {
        while let Some(event) = progress_receiver.recv().await {
            progress_terminal.progress(quick_share_transfer::sender::ProgressEvent {
                current_bytes: event.received_bytes,
                total_bytes: event.total_bytes,
                bytes_per_second: 0.0,
                eta: None,
            });
        }
    });
    let session_limit = Arc::new(Semaphore::new(8));
    let (outcome_sender, mut outcome_receiver) =
        mpsc::channel::<Result<ServerSessionOutcome, DirectError>>(8);
    let result = loop {
        tokio::select! {
            _ = cancellation.cancelled() => break Err(AppError::Cancelled),
            accepted = listener.accept() => {
                let (stream, peer_address) = accepted
                    .map_err(|error| AppError::Network(error.to_string()))?;
                let permit = match Arc::clone(&session_limit).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => continue,
                };
                let server = Arc::clone(&server);
                let sender = outcome_sender.clone();
                let session_cancel = cancellation.child_token();
                tokio::spawn(async move {
                    let _permit = permit;
                    let outcome = server.serve_stream(stream, peer_address, session_cancel).await;
                    let _ = sender.send(outcome).await;
                });
            }
            outcome = outcome_receiver.recv() => {
                let Some(outcome) = outcome else {
                    break Err(AppError::Network("callback dispatcher stopped".to_owned()));
                };
                match outcome {
                    Ok(ServerSessionOutcome::TransferCompleted { transfer_id: completed, .. })
                        if completed == transfer_id =>
                    {
                        receiver.cleanup(completed)
                            .map_err(|error| AppError::Filesystem(error.to_string()))?;
                        break Ok(());
                    }
                    Ok(ServerSessionOutcome::TransferCancelled { transfer_id: cancelled, .. })
                        if cancelled == transfer_id =>
                    {
                        let _ = receiver.cleanup(cancelled);
                        break Err(AppError::Cancelled);
                    }
                    Ok(ServerSessionOutcome::OfferRejected) => {
                        break Err(AppError::Rejected("expected callback offer was rejected".to_owned()));
                    }
                    Ok(_) => {}
                    Err(error) => break Err(map_direct(error)),
                }
            }
        }
    };
    cancellation.cancel();
    receiver
        .pause_all()
        .map_err(|error| AppError::Filesystem(error.to_string()))?;
    signal.abort();
    progress.abort();
    result
}

fn remote_output_root<F>(explicit: Option<PathBuf>, current_dir: F) -> Result<PathBuf, AppError>
where
    F: FnOnce() -> std::io::Result<PathBuf>,
{
    explicit.map_or_else(
        || current_dir().map_err(|error| AppError::Filesystem(error.to_string())),
        Ok,
    )
}

async fn resolve_target(
    requested: Option<&str>,
    identity: Arc<DeviceIdentity>,
    trust: &quick_share_core::identity::TrustedDeviceStore,
    config: &AppConfig,
    terminal: ConsoleTerminal,
    assume_yes: bool,
) -> Result<RemoteAgentTarget, AppError> {
    if let Some(value) = requested
        && !value.starts_with("qs_")
    {
        let endpoint = tokio::net::lookup_host(value)
            .await
            .map_err(|error| AppError::PeerUnavailable(error.to_string()))?
            .next()
            .ok_or_else(|| {
                AppError::PeerUnavailable("agent address resolved to no endpoint".to_owned())
            })?;
        return Ok(RemoteAgentTarget {
            endpoint,
            expected_id: None,
            expected_key: None,
            advertised_fingerprint: None,
        });
    }
    let discovery = HybridDiscovery::new(
        config.discovery.include_virtual,
        config.discovery.peers.clone(),
        identity.clone(),
        InfoRequest {
            protocol_version: ProtocolVersion::V1_1,
            capabilities: remote_receive_capabilities(),
        },
        Duration::from_millis(config.discovery.timeout_ms),
    );
    let scan = discovery
        .scan(ScanRequest {
            local_device_id: identity.device_id(),
            timeout: Duration::from_millis(config.discovery.timeout_ms),
        })
        .await
        .map_err(|error| AppError::Network(error.to_string()))?;
    for warning in scan.warnings {
        ReceiveTerminal::warning(&terminal, &warning);
    }
    let mut peers = scan
        .peers
        .into_iter()
        .filter(|peer| {
            peer.version >= ProtocolVersion::V1_1
                && peer.capabilities.contains(&Capability::RemoteSelection)
        })
        .collect::<Vec<_>>();
    if let Some(value) = requested {
        let expected =
            DeviceId::parse(value).map_err(|error| AppError::Usage(error.to_string()))?;
        peers.retain(|peer| peer.device_id == expected);
    }
    if peers.is_empty() {
        return Err(AppError::PeerUnavailable(
            "no QSP/1.1 remote-selection agent was discovered".to_owned(),
        ));
    }
    let index = if requested.is_some() {
        0
    } else {
        terminal.select_peer(
            &peers,
            InteractionPolicy::new(terminal.is_interactive(), assume_yes),
        )?
    };
    target_from_peer(
        peers
            .get(index)
            .ok_or_else(|| AppError::Usage("selected agent index is invalid".to_owned()))?,
        trust,
    )
}

fn target_from_peer(
    peer: &UnverifiedPeer,
    trust: &quick_share_core::identity::TrustedDeviceStore,
) -> Result<RemoteAgentTarget, AppError> {
    let endpoint =
        *peer.endpoints.iter().next().ok_or_else(|| {
            AppError::PeerUnavailable("selected agent has no endpoint".to_owned())
        })?;
    let expected_key = trust
        .list()
        .map_err(|error| AppError::Identity(error.to_string()))?
        .into_iter()
        .find(|device| device.device_id == peer.device_id)
        .map(|device| device.public_key);
    Ok(RemoteAgentTarget {
        endpoint,
        expected_id: Some(peer.device_id.clone()),
        expected_key,
        advertised_fingerprint: Some(peer.static_key_fingerprint),
    })
}

fn selected_transfer(
    response: &quick_share_protocol::SourceSelectionResponse,
) -> Result<quick_share_protocol::TransferId, AppError> {
    match response.status {
        SourceSelectionStatus::Ready => response
            .transfer_id
            .ok_or_else(|| AppError::Network("ready selection omitted transfer ID".to_owned())),
        SourceSelectionStatus::Cancelled => Err(AppError::Cancelled),
        SourceSelectionStatus::Rejected => Err(AppError::Rejected(
            "remote source request was rejected".to_owned(),
        )),
        SourceSelectionStatus::Busy => Err(AppError::PeerUnavailable(
            "remote desktop UI is busy".to_owned(),
        )),
        SourceSelectionStatus::UiUnavailable => Err(AppError::PeerUnavailable(
            "remote desktop UI is unavailable".to_owned(),
        )),
        SourceSelectionStatus::Expired => Err(AppError::Rejected(
            "remote source request expired".to_owned(),
        )),
        SourceSelectionStatus::Failed => Err(AppError::Network(
            "remote source preparation failed".to_owned(),
        )),
    }
}

fn remote_receive_capabilities() -> BTreeSet<Capability> {
    BTreeSet::from([
        Capability::Files,
        Capability::Directories,
        Capability::Symlinks,
        Capability::Resume,
        Capability::RemoteSelection,
    ])
}

fn map_direct(error: DirectError) -> AppError {
    match error {
        DirectError::ConnectTimeout | DirectError::Connect(_) => {
            AppError::PeerUnavailable(error.to_string())
        }
        DirectError::IdentityMismatch | DirectError::Authentication(_) => {
            AppError::Identity(error.to_string())
        }
        DirectError::Rejected | DirectError::Unauthorized => AppError::Rejected(error.to_string()),
        DirectError::Network(_)
        | DirectError::Noise(_)
        | DirectError::IncompatibleProtocol
        | DirectError::Remote(_)
        | DirectError::InvalidResponse
        | DirectError::OfferChannel
        | DirectError::Random
        | DirectError::Offer(_)
        | DirectError::Receiver(_)
        | DirectError::ReceiverRequest { .. }
        | DirectError::Selection(_)
        | DirectError::ExpectedOffer(_) => AppError::Network(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_share_protocol::{SourceSelectionResponse, TransferId};
    use uuid::Uuid;

    #[test]
    fn remote_request_defaults_to_current_directory_but_honors_explicit_output() {
        let current = PathBuf::from("working-directory");
        assert_eq!(
            remote_output_root(None, || Ok(current.clone())).expect("current directory"),
            current
        );
        assert_eq!(
            remote_output_root(Some(PathBuf::from("chosen")), || {
                Err(std::io::Error::other("must not be called"))
            })
            .expect("explicit output"),
            PathBuf::from("chosen")
        );
    }

    #[test]
    fn selection_terminal_statuses_keep_stable_cli_meaning() {
        let transfer_id = TransferId::new(Uuid::now_v7());
        assert_eq!(
            selected_transfer(&SourceSelectionResponse {
                status: SourceSelectionStatus::Ready,
                transfer_id: Some(transfer_id),
            })
            .expect("ready"),
            transfer_id
        );
        for (status, expected_exit) in [
            (SourceSelectionStatus::Cancelled, 0),
            (SourceSelectionStatus::Rejected, 4),
            (SourceSelectionStatus::Busy, 3),
            (SourceSelectionStatus::UiUnavailable, 3),
            (SourceSelectionStatus::Expired, 4),
            (SourceSelectionStatus::Failed, 7),
        ] {
            let error = selected_transfer(&SourceSelectionResponse {
                status,
                transfer_id: None,
            })
            .expect_err("terminal status");
            assert_eq!(error.exit_code(), expected_exit);
        }
        assert_eq!(
            selected_transfer(&SourceSelectionResponse {
                status: SourceSelectionStatus::Ready,
                transfer_id: None,
            })
            .expect_err("invalid ready")
            .exit_code(),
            7
        );
    }

    #[test]
    fn remote_receive_capabilities_require_qsp_1_1_callback_correlation() {
        let capabilities = remote_receive_capabilities();
        assert!(capabilities.contains(&Capability::RemoteSelection));
        assert!(capabilities.contains(&Capability::Files));
        assert!(!capabilities.contains(&Capability::Text));
    }
}
