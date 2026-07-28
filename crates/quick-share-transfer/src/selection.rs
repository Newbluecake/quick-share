//! Bounded, idempotent remote source-selection state machine.

use crate::auth::PeerAuthContext;
use async_trait::async_trait;
use quick_share_core::identity::TrustedDeviceStore;
use quick_share_protocol::{
    DeviceId, RequestId, SourceSelectionRequest, SourceSelectionResponse, SourceSelectionStatus,
    TransferId,
};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const MAX_SELECTION_DURATION: Duration = Duration::from_secs(60 * 60);
const MAX_SELECTION_PENDING: usize = 64;
const MAX_SELECTION_TERMINAL: usize = 4_096;
const MAX_SELECTION_REQUESTS_PER_DEVICE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPolicy {
    pub maximum_pending: usize,
    pub maximum_terminal: usize,
    pub maximum_requests_per_device: usize,
    pub rate_window: Duration,
    pub request_timeout: Duration,
    pub terminal_ttl: Duration,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self {
            maximum_pending: 16,
            maximum_terminal: 256,
            maximum_requests_per_device: 4,
            rate_window: Duration::from_secs(60),
            request_timeout: Duration::from_secs(10 * 60),
            terminal_ttl: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAuthorization {
    AcceptOnce,
    AcceptAndTrust { sas_verified: bool },
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SelectionHandlerError {
    #[error("selection was cancelled")]
    Cancelled,
    #[error("desktop UI is busy")]
    Busy,
    #[error("desktop UI is unavailable")]
    UiUnavailable,
    #[error("selection preparation failed")]
    Failed,
}

#[async_trait]
pub trait SelectionHandler: Send + Sync {
    async fn authorize(
        &self,
        peer: &PeerAuthContext,
        cancellation: CancellationToken,
    ) -> Result<SelectionAuthorization, SelectionHandlerError>;

    async fn select_source(
        &self,
        peer: &PeerAuthContext,
        cancellation: CancellationToken,
    ) -> Result<TransferId, SelectionHandlerError>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct CallbackTarget {
    pub endpoint: SocketAddr,
    pub requester_device_id: DeviceId,
    pub requester_public_key: [u8; 32],
}

impl fmt::Debug for CallbackTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallbackTarget")
            .field("endpoint", &self.endpoint)
            .field("requester_device_id", &self.requester_device_id)
            .field("requester_public_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResult {
    pub response: SourceSelectionResponse,
    pub callback: Option<CallbackTarget>,
}

impl SelectionResult {
    fn terminal(status: SourceSelectionStatus) -> Self {
        Self {
            response: SourceSelectionResponse {
                status,
                transfer_id: None,
            },
            callback: None,
        }
    }
}

pub struct SelectionManager<H> {
    handler: Arc<H>,
    trust_store: TrustedDeviceStore,
    policy: SelectionPolicy,
    state: Mutex<SelectionState>,
    ui_active: AtomicBool,
}

impl<H> fmt::Debug for SelectionManager<H> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectionManager")
            .field("trust_store", &"[REDACTED]")
            .field("policy", &self.policy)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

impl<H> SelectionManager<H>
where
    H: SelectionHandler + 'static,
{
    pub fn new(
        handler: Arc<H>,
        trust_store: TrustedDeviceStore,
        policy: SelectionPolicy,
    ) -> Result<Self, SelectionError> {
        if policy.maximum_pending == 0
            || policy.maximum_pending > MAX_SELECTION_PENDING
            || policy.maximum_terminal == 0
            || policy.maximum_terminal > MAX_SELECTION_TERMINAL
            || policy.maximum_requests_per_device == 0
            || policy.maximum_requests_per_device > MAX_SELECTION_REQUESTS_PER_DEVICE
            || policy.rate_window.is_zero()
            || policy.rate_window > MAX_SELECTION_DURATION
            || policy.request_timeout.is_zero()
            || policy.request_timeout > MAX_SELECTION_DURATION
            || policy.terminal_ttl.is_zero()
            || policy.terminal_ttl > MAX_SELECTION_DURATION
        {
            return Err(SelectionError::InvalidPolicy);
        }
        Ok(Self {
            handler,
            trust_store,
            policy,
            state: Mutex::new(SelectionState::default()),
            ui_active: AtomicBool::new(false),
        })
    }

    pub async fn handle(
        &self,
        request_id: RequestId,
        peer: PeerAuthContext,
        observed_source_ip: IpAddr,
        request: SourceSelectionRequest,
        now: Instant,
    ) -> Result<SelectionResult, SelectionError> {
        request
            .validate()
            .map_err(|_| SelectionError::InvalidRequest)?;
        if request.requester.device_id != *peer.device_id()
            || request.requester.name != peer.claimed_name()
        {
            return Err(SelectionError::IdentityMismatch);
        }
        let fingerprint = RequestFingerprint {
            device_id: peer.device_id().clone(),
            public_key: peer.public_key(),
            source_ip: observed_source_ip,
            callback_port: request.callback_port,
        };

        let registration = {
            let mut state = self.lock_state()?;
            self.prune(&mut state, now);
            match state.requests.get(&request_id) {
                Some(SelectionRecord::Terminal {
                    fingerprint: existing,
                    result,
                    ..
                }) => {
                    if existing != &fingerprint {
                        return Err(SelectionError::Replay);
                    }
                    Registration::Immediate(result.clone())
                }
                Some(SelectionRecord::Pending {
                    fingerprint: existing,
                    sender,
                    ..
                }) => {
                    if existing != &fingerprint {
                        return Err(SelectionError::Replay);
                    }
                    Registration::Duplicate(sender.subscribe())
                }
                None => {
                    if state.pending_count >= self.policy.maximum_pending
                        || self.rate_limited(&mut state, peer.device_id(), now)
                    {
                        let result = SelectionResult::terminal(SourceSelectionStatus::Busy);
                        state.requests.insert(
                            request_id,
                            SelectionRecord::Terminal {
                                fingerprint: fingerprint.clone(),
                                result: result.clone(),
                                expires_at: now + self.policy.terminal_ttl,
                            },
                        );
                        self.trim_terminals(&mut state);
                        Registration::Immediate(result)
                    } else {
                        let (sender, _) = watch::channel(None);
                        state.requests.insert(
                            request_id,
                            SelectionRecord::Pending {
                                fingerprint: fingerprint.clone(),
                                sender: sender.clone(),
                                expires_at: now + self.policy.request_timeout,
                            },
                        );
                        state.pending_count += 1;
                        Registration::New(sender)
                    }
                }
            }
        };
        let sender = match registration {
            Registration::New(sender) => sender,
            Registration::Duplicate(receiver) => {
                return self.wait_for_duplicate(receiver).await;
            }
            Registration::Immediate(result) => return Ok(result),
        };

        let needs_ui = !matches!(peer, PeerAuthContext::Changed { .. });
        let ui_lease = if needs_ui {
            let Some(lease) = self.acquire_ui() else {
                return self.finish(
                    request_id,
                    fingerprint,
                    sender,
                    SelectionResult::terminal(SourceSelectionStatus::Busy),
                );
            };
            Some(lease)
        } else {
            None
        };

        let cancellation = CancellationToken::new();
        let work = self.process(
            &peer,
            observed_source_ip,
            request.callback_port,
            cancellation.clone(),
        );
        let result = match tokio::time::timeout(self.policy.request_timeout, work).await {
            Ok(result) => result,
            Err(_) => {
                cancellation.cancel();
                SelectionResult::terminal(SourceSelectionStatus::Expired)
            }
        };
        drop(ui_lease);
        self.finish(request_id, fingerprint, sender, result)
    }

    async fn process(
        &self,
        peer: &PeerAuthContext,
        source_ip: IpAddr,
        callback_port: u16,
        cancellation: CancellationToken,
    ) -> SelectionResult {
        if matches!(peer, PeerAuthContext::Changed { .. }) {
            return SelectionResult::terminal(SourceSelectionStatus::Rejected);
        }
        if matches!(peer, PeerAuthContext::Unknown { .. }) {
            let authorization = match self
                .handler
                .authorize(peer, cancellation.child_token())
                .await
            {
                Ok(choice) => choice,
                Err(error) => return SelectionResult::terminal(map_handler_error(error)),
            };
            match authorization {
                SelectionAuthorization::Reject => {
                    return SelectionResult::terminal(SourceSelectionStatus::Rejected);
                }
                SelectionAuthorization::AcceptOnce => {}
                SelectionAuthorization::AcceptAndTrust {
                    sas_verified: false,
                } => return SelectionResult::terminal(SourceSelectionStatus::Rejected),
                SelectionAuthorization::AcceptAndTrust { sas_verified: true } => {
                    if self
                        .trust_store
                        .trust_peer(
                            peer.device_id().clone(),
                            peer.claimed_name(),
                            peer.public_key(),
                        )
                        .is_err()
                    {
                        return SelectionResult::terminal(SourceSelectionStatus::Failed);
                    }
                }
            }
        }
        let transfer_id = match self
            .handler
            .select_source(peer, cancellation.child_token())
            .await
        {
            Ok(transfer_id) if !cancellation.is_cancelled() => transfer_id,
            Ok(_) => return SelectionResult::terminal(SourceSelectionStatus::Expired),
            Err(error) => return SelectionResult::terminal(map_handler_error(error)),
        };
        SelectionResult {
            response: SourceSelectionResponse {
                status: SourceSelectionStatus::Ready,
                transfer_id: Some(transfer_id),
            },
            callback: Some(CallbackTarget {
                endpoint: SocketAddr::new(source_ip, callback_port),
                requester_device_id: peer.device_id().clone(),
                requester_public_key: peer.public_key(),
            }),
        }
    }

    async fn wait_for_duplicate(
        &self,
        mut receiver: watch::Receiver<Option<SelectionResult>>,
    ) -> Result<SelectionResult, SelectionError> {
        let wait = async {
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return Ok(result);
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| SelectionError::Internal)?;
            }
        };
        tokio::time::timeout(self.policy.request_timeout, wait)
            .await
            .map_err(|_| SelectionError::Timeout)?
    }

    fn finish(
        &self,
        request_id: RequestId,
        fingerprint: RequestFingerprint,
        sender: watch::Sender<Option<SelectionResult>>,
        result: SelectionResult,
    ) -> Result<SelectionResult, SelectionError> {
        result
            .response
            .validate()
            .map_err(|_| SelectionError::Internal)?;
        let mut state = self.lock_state()?;
        match state.requests.get(&request_id) {
            Some(SelectionRecord::Pending {
                fingerprint: existing,
                ..
            }) if existing == &fingerprint => {}
            Some(SelectionRecord::Terminal {
                fingerprint: existing,
                result: existing_result,
                ..
            }) if existing == &fingerprint => return Ok(existing_result.clone()),
            Some(_) => return Err(SelectionError::Replay),
            None => return Err(SelectionError::Internal),
        }
        state.pending_count = state.pending_count.saturating_sub(1);
        state.requests.insert(
            request_id,
            SelectionRecord::Terminal {
                fingerprint,
                result: result.clone(),
                expires_at: Instant::now() + self.policy.terminal_ttl,
            },
        );
        self.trim_terminals(&mut state);
        let _ = sender.send(Some(result.clone()));
        Ok(result)
    }

    fn acquire_ui(&self) -> Option<UiLease<'_>> {
        self.ui_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| UiLease(&self.ui_active))
    }

    fn rate_limited(&self, state: &mut SelectionState, device_id: &DeviceId, now: Instant) -> bool {
        let requests = state.rates.entry(device_id.clone()).or_default();
        while requests.front().is_some_and(|request| {
            now.saturating_duration_since(*request) >= self.policy.rate_window
        }) {
            requests.pop_front();
        }
        if requests.len() >= self.policy.maximum_requests_per_device {
            true
        } else {
            requests.push_back(now);
            false
        }
    }

    fn prune(&self, state: &mut SelectionState, now: Instant) {
        state.requests.retain(|_, record| {
            !matches!(
                record,
                SelectionRecord::Terminal { expires_at, .. } if *expires_at <= now
            )
        });
        let expired_pending = state
            .requests
            .iter()
            .filter_map(|(request_id, record)| match record {
                SelectionRecord::Pending { expires_at, .. } if *expires_at <= now => {
                    Some(*request_id)
                }
                SelectionRecord::Pending { .. } | SelectionRecord::Terminal { .. } => None,
            })
            .collect::<Vec<_>>();
        for request_id in expired_pending {
            if let Some(SelectionRecord::Pending {
                fingerprint,
                sender,
                ..
            }) = state.requests.remove(&request_id)
            {
                let result = SelectionResult::terminal(SourceSelectionStatus::Expired);
                let _ = sender.send(Some(result.clone()));
                state.requests.insert(
                    request_id,
                    SelectionRecord::Terminal {
                        fingerprint,
                        result,
                        expires_at: now + self.policy.terminal_ttl,
                    },
                );
                state.pending_count = state.pending_count.saturating_sub(1);
            }
        }
        state.rates.retain(|_, requests| {
            while requests.front().is_some_and(|request| {
                now.saturating_duration_since(*request) >= self.policy.rate_window
            }) {
                requests.pop_front();
            }
            !requests.is_empty()
        });
        self.trim_terminals(state);
    }

    fn trim_terminals(&self, state: &mut SelectionState) {
        if state
            .requests
            .values()
            .filter(|record| matches!(record, SelectionRecord::Terminal { .. }))
            .count()
            > self.policy.maximum_terminal
        {
            let mut terminals = state
                .requests
                .iter()
                .filter_map(|(request_id, record)| match record {
                    SelectionRecord::Terminal { expires_at, .. } => {
                        Some((*request_id, *expires_at))
                    }
                    SelectionRecord::Pending { .. } => None,
                })
                .collect::<Vec<_>>();
            terminals.sort_by_key(|(_, expiry)| *expiry);
            let remove = terminals.len().saturating_sub(self.policy.maximum_terminal);
            for (request_id, _) in terminals.into_iter().take(remove) {
                state.requests.remove(&request_id);
            }
        }
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, SelectionState>, SelectionError> {
        self.state.lock().map_err(|_| SelectionError::Internal)
    }
}

fn map_handler_error(error: SelectionHandlerError) -> SourceSelectionStatus {
    match error {
        SelectionHandlerError::Cancelled => SourceSelectionStatus::Cancelled,
        SelectionHandlerError::Busy => SourceSelectionStatus::Busy,
        SelectionHandlerError::UiUnavailable => SourceSelectionStatus::UiUnavailable,
        SelectionHandlerError::Failed => SourceSelectionStatus::Failed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestFingerprint {
    device_id: DeviceId,
    public_key: [u8; 32],
    source_ip: IpAddr,
    callback_port: u16,
}

enum Registration {
    New(watch::Sender<Option<SelectionResult>>),
    Duplicate(watch::Receiver<Option<SelectionResult>>),
    Immediate(SelectionResult),
}

enum SelectionRecord {
    Pending {
        fingerprint: RequestFingerprint,
        sender: watch::Sender<Option<SelectionResult>>,
        expires_at: Instant,
    },
    Terminal {
        fingerprint: RequestFingerprint,
        result: SelectionResult,
        expires_at: Instant,
    },
}

#[derive(Default)]
struct SelectionState {
    requests: BTreeMap<RequestId, SelectionRecord>,
    rates: BTreeMap<DeviceId, VecDeque<Instant>>,
    pending_count: usize,
}

struct UiLease<'a>(&'a AtomicBool);

impl Drop for UiLease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SelectionError {
    #[error("selection policy is invalid")]
    InvalidPolicy,
    #[error("selection request is invalid")]
    InvalidRequest,
    #[error("selection requester does not match the authenticated identity")]
    IdentityMismatch,
    #[error("selection request ID was reused with different authenticated data")]
    Replay,
    #[error("selection duplicate wait timed out")]
    Timeout,
    #[error("selection state is unavailable")]
    Internal,
}
