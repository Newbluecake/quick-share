//! Testable send/receive orchestration with closed fallback states and safe terminal defaults.

use crate::{AppError, InteractionPolicy, ReceiveIntent, SendIntent};
use async_trait::async_trait;
use quick_share_discovery::{Discovery, ScanRequest, UnverifiedPeer};
use quick_share_platform::clipboard::Clipboard;
use quick_share_protocol::{DeviceId, OfferDecision, RejectionReason, TransferId};
use quick_share_transfer::{
    offer::OfferView,
    sender::ProgressEvent,
    text::{TextDelivery, TextError, TextPayload, TextSource, capture_clipboard, deliver_text},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// Prepared content. `TextPayload` Debug output redacts its body.
#[derive(Debug, Clone)]
pub enum SendContent {
    Paths {
        paths: Vec<PathBuf>,
        follow_links: bool,
    },
    Text(TextPayload),
}

/// Routing is a closed enum, so discovery failure can never be represented as an empty list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendRoute {
    Auto,
    ExplicitPeer(String),
    Web,
}

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub content: SendContent,
    pub route: SendRoute,
    pub resume_transfer_id: Option<TransferId>,
    pub allow_http: bool,
    pub assume_yes: bool,
}

/// Explicitly selected direct target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectTarget {
    Discovered(UnverifiedPeer),
    Explicit(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendMode {
    DirectEncrypted,
    WebHttps,
    WebHttpExplicit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Direct { device_id: Option<DeviceId> },
    Web,
}

pub trait SendTerminal: Send + Sync {
    fn warning(&self, message: &str);
    fn mode(&self, mode: SendMode);
    fn select_peer(
        &self,
        peers: &[UnverifiedPeer],
        interaction: InteractionPolicy,
    ) -> Result<usize, AppError>;
}

#[async_trait]
pub trait DirectSendAdapter: Send + Sync {
    async fn send(&self, target: DirectTarget, request: &SendRequest) -> Result<(), AppError>;
}

#[async_trait]
pub trait WebShareAdapter: Send + Sync {
    async fn serve(&self, request: &SendRequest) -> Result<(), AppError>;
}

/// Enforces the one and only automatic Web fallback transition.
pub struct SendOrchestrator<'a, D, T, W, U> {
    pub discovery: &'a D,
    pub direct: &'a T,
    pub web: &'a W,
    pub terminal: &'a U,
    pub local_device_id: DeviceId,
    pub discovery_timeout: Duration,
    pub interactive: bool,
}

impl<D, T, W, U> SendOrchestrator<'_, D, T, W, U>
where
    D: Discovery,
    T: DirectSendAdapter,
    W: WebShareAdapter,
    U: SendTerminal,
{
    pub async fn run(&self, request: &SendRequest) -> Result<SendOutcome, AppError> {
        match &request.route {
            SendRoute::Web => return self.run_web(request).await,
            SendRoute::ExplicitPeer(peer) => {
                self.terminal.mode(SendMode::DirectEncrypted);
                self.direct
                    .send(DirectTarget::Explicit(peer.clone()), request)
                    .await?;
                return Ok(SendOutcome::Direct { device_id: None });
            }
            SendRoute::Auto => {}
        }

        let result = self
            .discovery
            .scan(ScanRequest {
                local_device_id: self.local_device_id.clone(),
                timeout: self.discovery_timeout,
            })
            .await
            .map_err(|error| {
                AppError::Network(format!(
                    "{error}; automatic Web fallback was not used because discovery failed; use --web explicitly"
                ))
            })?;
        for warning in &result.warnings {
            self.terminal.warning(warning);
        }
        if result.is_definitive_empty() {
            if request.resume_transfer_id.is_some() {
                return Err(AppError::PeerUnavailable(
                    "the interrupted receiver was not discovered; resume never falls back to Web sharing"
                        .to_owned(),
                ));
            }
            return self.run_web(request).await;
        }
        if result.peers.is_empty() {
            return Err(AppError::Network(
                "discovery ended without a definitive empty result; use --peer or --web explicitly"
                    .to_owned(),
            ));
        }

        let interaction = InteractionPolicy::new(self.interactive, request.assume_yes);
        let index = self.terminal.select_peer(&result.peers, interaction)?;
        let peer = result.peers.get(index).cloned().ok_or_else(|| {
            AppError::Usage(
                "selected receiver index is outside the discovered peer list".to_owned(),
            )
        })?;
        let device_id = peer.device_id.clone();
        self.terminal.mode(SendMode::DirectEncrypted);
        // No error after this point may transition to Web mode.
        self.direct
            .send(DirectTarget::Discovered(peer), request)
            .await?;
        Ok(SendOutcome::Direct {
            device_id: Some(device_id),
        })
    }

    async fn run_web(&self, request: &SendRequest) -> Result<SendOutcome, AppError> {
        self.terminal.mode(if request.allow_http {
            SendMode::WebHttpExplicit
        } else {
            SendMode::WebHttps
        });
        self.web.serve(request).await?;
        Ok(SendOutcome::Web)
    }
}

pub fn prepare_send_request(
    intent: &SendIntent,
    clipboard: Option<&mut dyn Clipboard>,
) -> Result<SendRequest, AppError> {
    let content = if let Some(text) = &intent.text {
        SendContent::Text(
            TextPayload::new(TextSource::Literal, text.clone()).map_err(map_text_error)?,
        )
    } else if intent.clipboard {
        let clipboard = clipboard.ok_or_else(|| {
            AppError::Filesystem(
                "clipboard access is unavailable; use --text or provide a desktop session"
                    .to_owned(),
            )
        })?;
        SendContent::Text(capture_clipboard(clipboard).map_err(map_text_error)?)
    } else {
        SendContent::Paths {
            paths: intent.paths.clone(),
            follow_links: intent.follow_links,
        }
    };
    let route = if intent.web {
        SendRoute::Web
    } else if let Some(peer) = &intent.peer {
        SendRoute::ExplicitPeer(peer.clone())
    } else {
        SendRoute::Auto
    };
    let resume_transfer_id = intent
        .resume
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|error| AppError::Usage(format!("invalid resume transfer ID: {error}")))?
        .map(TransferId::new);
    Ok(SendRequest {
        content,
        route,
        resume_transfer_id,
        allow_http: intent.allow_http,
        assume_yes: intent.assume_yes,
    })
}

fn map_text_error(error: TextError) -> AppError {
    match error {
        TextError::TooLarge { .. } => AppError::Usage(error.to_string()),
        TextError::Clipboard(_) | TextError::Storage(_) | TextError::Output(_) => {
            AppError::Filesystem(error.to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveOptions {
    pub output: PathBuf,
    pub once: bool,
    pub confirm_trusted: bool,
    pub assume_yes: bool,
    pub interactive: bool,
}

impl ReceiveOptions {
    #[must_use]
    pub fn resolve(
        intent: &ReceiveIntent,
        default_output: &Path,
        trusted_confirm_default: bool,
        interactive: bool,
    ) -> Self {
        Self {
            output: intent
                .output
                .clone()
                .unwrap_or_else(|| default_output.to_path_buf()),
            once: intent.once,
            confirm_trusted: intent.confirm_trusted.unwrap_or(trusted_confirm_default),
            assume_yes: intent.assume_yes,
            interactive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveStartup {
    pub device_name: String,
    pub output: PathBuf,
    pub bind_description: String,
    pub encrypted: bool,
    pub trusted_auto_accept: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferChoice {
    AcceptOnce,
    AcceptAndTrust { sas_verified: bool },
    Reject,
}

pub trait ReceiveTerminal: Send + Sync {
    fn startup(&self, startup: &ReceiveStartup);
    fn offer(&self, view: &OfferView);
    fn choose_offer(&self, view: &OfferView) -> Result<OfferChoice, AppError>;
    fn progress(&self, event: ProgressEvent);
    fn warning(&self, message: &str);
}

/// Applies safe non-TTY defaults before an offer reaches the authorization state machine.
pub fn decide_offer(
    terminal: &dyn ReceiveTerminal,
    view: &OfferView,
    options: &ReceiveOptions,
) -> Result<OfferDecision, AppError> {
    decide_offer_with_sas(terminal, view, options).map(|result| result.0)
}

pub fn decide_offer_with_sas(
    terminal: &dyn ReceiveTerminal,
    view: &OfferView,
    options: &ReceiveOptions,
) -> Result<(OfferDecision, bool), AppError> {
    terminal.offer(view);
    let choice = if options.interactive {
        terminal.choose_offer(view)?
    } else if options.assume_yes {
        // Non-interactive approval is intentionally one-shot and can never create trust.
        OfferChoice::AcceptOnce
    } else {
        OfferChoice::Reject
    };
    Ok(match choice {
        OfferChoice::AcceptOnce => (OfferDecision::AcceptOnce, false),
        OfferChoice::AcceptAndTrust { sas_verified: true } => (OfferDecision::AcceptAndTrust, true),
        OfferChoice::AcceptAndTrust {
            sas_verified: false,
        } => {
            return Err(AppError::Identity(
                "accept-and-trust requires explicit SAS comparison".to_owned(),
            ));
        }
        OfferChoice::Reject => (
            OfferDecision::Reject {
                reason: RejectionReason::UserRejected,
            },
            false,
        ),
    })
}

/// First interrupt requests a durable graceful stop; the second requests immediate termination.
#[derive(Debug, Default)]
pub struct ShutdownController {
    count: AtomicU8,
    cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownAction {
    GracefulSnapshot,
    Force,
}

impl ShutdownController {
    #[must_use]
    pub fn request(&self) -> ShutdownAction {
        let previous = self.count.fetch_add(1, Ordering::SeqCst);
        if previous == 0 {
            self.cancellation.cancel();
            ShutdownAction::GracefulSnapshot
        } else {
            ShutdownAction::Force
        }
    }

    #[must_use]
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

pub fn deliver_received_text(
    payload: &TextPayload,
    clipboard: &mut dyn Clipboard,
    output: Option<&Path>,
    stdout: &mut dyn Write,
) -> Result<TextDelivery, AppError> {
    deliver_text(payload, clipboard, output, stdout).map_err(map_text_error)
}
