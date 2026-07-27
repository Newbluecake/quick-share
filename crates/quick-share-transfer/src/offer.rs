//! Bounded offer confirmation, trust transition, rate limiting, and authorization grants.

use crate::{
    auth::{PeerAuthContext, PreAuthorizationPermission},
    noise::SasCode,
};
use quick_share_core::{
    config::TrustedPolicy,
    identity::{TrustStatus, TrustStoreError, TrustedDeviceStore},
    paths::RelativePath,
};
use quick_share_protocol::{
    ContentKind, DeviceId, ManifestEntryKind, OfferDecision, TransferId, TransferOffer,
    TransferStatus,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::oneshot;
use zeroize::Zeroize;

const DEFAULT_MAX_PENDING: usize = 16;
const DEFAULT_MAX_OFFERS_PER_WINDOW: usize = 5;
const DEFAULT_REPLAY_CACHE: usize = 4096;

/// Receiver policy for bounded unknown/trusted offers.
#[derive(Debug, Clone)]
pub struct OfferPolicy {
    pub trusted_policy: TrustedPolicy,
    pub confirmation_timeout: Duration,
    pub grant_ttl: Duration,
    pub rate_window: Duration,
    pub max_offers_per_window: usize,
    pub max_pending: usize,
    pub replay_ttl: Duration,
    pub terminal_retention: Duration,
    pub max_replay_entries: usize,
    pub trusted_auto_max_bytes: u64,
    pub trusted_auto_max_entries: usize,
}

impl Default for OfferPolicy {
    fn default() -> Self {
        Self {
            trusted_policy: TrustedPolicy::Auto,
            confirmation_timeout: Duration::from_secs(120),
            grant_ttl: Duration::from_secs(60 * 60),
            rate_window: Duration::from_secs(60),
            max_offers_per_window: DEFAULT_MAX_OFFERS_PER_WINDOW,
            max_pending: DEFAULT_MAX_PENDING,
            replay_ttl: Duration::from_secs(10 * 60),
            terminal_retention: Duration::from_secs(10 * 60),
            max_replay_entries: DEFAULT_REPLAY_CACHE,
            trusted_auto_max_bytes: 1024 * 1024 * 1024,
            trusted_auto_max_entries: 1_000,
        }
    }
}

/// Terminal-safe view model containing no local paths, secrets, or executable content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferView {
    pub transfer_id: TransferId,
    pub sender_device_id: DeviceId,
    pub sender_name: String,
    pub peer_address: Option<SocketAddr>,
    pub sas: SasCode,
    pub content_kind: ContentKind,
    pub entry_count: usize,
    pub total_bytes: u64,
    pub entries: Vec<OfferEntryView>,
    pub identity_changed: bool,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferEntryView {
    pub relative_path: String,
    pub kind: &'static str,
    pub size: u64,
}

/// Result channel returned to the authenticated offer owner.
pub struct OfferSubmission {
    pub view: OfferView,
    pub resolution: oneshot::Receiver<OfferResolution>,
}

/// Secret bearer token. Debug and Display never expose bytes.
pub struct AuthorizationToken([u8; 32]);

impl AuthorizationToken {
    /// Reconstructs a bearer received inside an authenticated Noise frame.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn with_bytes<R>(&self, callback: impl FnOnce(&[u8; 32]) -> R) -> R {
        callback(&self.0)
    }
}

impl Drop for AuthorizationToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for AuthorizationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationToken([REDACTED])")
    }
}

/// Resolution sent exactly once through the confirmation channel.
#[derive(Debug)]
pub struct OfferResolution {
    pub status: TransferStatus,
    pub authorization: Option<AuthorizationToken>,
}

/// Post-confirmation operations granted to one transfer owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorizationPermission {
    OfferStatus,
    TransferStatus,
    ChunkUpload,
    Complete,
    Cancel,
}

/// Manifest snapshot returned only after token, peer, transfer, permission, and expiry match.
#[derive(Debug, Clone)]
pub struct AuthorizedOffer {
    pub offer: TransferOffer,
    pub manifest_digest: [u8; 32],
}

/// Thread-safe in-memory offer manager. Persisted transfer data remains in `TransferStore`.
pub struct OfferManager {
    trust_store: TrustedDeviceStore,
    policy: OfferPolicy,
    state: Mutex<ManagerState>,
}

impl fmt::Debug for OfferManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OfferManager")
            .field("policy", &self.policy)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

impl OfferManager {
    #[must_use]
    pub fn new(trust_store: TrustedDeviceStore, policy: OfferPolicy) -> Self {
        Self {
            trust_store,
            policy,
            state: Mutex::new(ManagerState::default()),
        }
    }

    /// Creates a replay-protected offer and a single-use confirmation channel.
    pub fn create_offer(
        &self,
        peer: &PeerAuthContext,
        peer_address: Option<SocketAddr>,
        offer: TransferOffer,
        now: Instant,
    ) -> Result<OfferSubmission, OfferError> {
        if !peer.allows_pre_authorization(PreAuthorizationPermission::OfferCreate) {
            return Err(OfferError::Unauthorized);
        }
        offer
            .validate()
            .map_err(|error| OfferError::InvalidOffer(error.to_string()))?;
        for entry in &offer.entries {
            let path = RelativePath::parse(&entry.relative_path)
                .map_err(|error| OfferError::InvalidOffer(error.to_string()))?;
            if path.as_str() != entry.relative_path {
                return Err(OfferError::InvalidOffer(
                    "manifest paths must use normalized forward slashes".to_owned(),
                ));
            }
        }
        if &offer.sender.device_id != peer.device_id() || offer.sender.name != peer.claimed_name() {
            return Err(OfferError::IdentityMismatch);
        }
        let manifest_digest = digest_offer(&offer)?;
        let mut state = self.lock_state()?;
        purge(&mut state, &self.policy, now);
        if state.seen.contains_key(&offer.transfer_id)
            || state.offers.contains_key(&offer.transfer_id)
        {
            return Err(OfferError::Replay);
        }
        enforce_rate(&mut state, &self.policy, peer.device_id(), now)?;
        let pending = state
            .offers
            .values()
            .filter(|stored| stored.status == TransferStatus::Offered)
            .count();
        if pending >= self.policy.max_pending {
            return Err(OfferError::QueueFull);
        }
        if state.seen.len() >= self.policy.max_replay_entries {
            return Err(OfferError::ReplayCacheFull);
        }

        let currently_trusted = peer.is_trusted()
            && matches!(
                self.trust_store
                    .check(peer.device_id(), &peer.public_key())?,
                TrustStatus::Trusted(_)
            );
        let view = offer_view(peer, peer_address, &offer, currently_trusted);
        let (sender, resolution) = oneshot::channel();
        let auto_accept = currently_trusted
            && self.policy.trusted_policy == TrustedPolicy::Auto
            && offer.total_bytes <= self.policy.trusted_auto_max_bytes
            && offer.entries.len() <= self.policy.trusted_auto_max_entries;
        let mut stored = StoredOffer {
            owner: peer.device_id().clone(),
            owner_name: peer.name().to_owned(),
            owner_public_key: peer.public_key(),
            offer,
            manifest_digest,
            status: TransferStatus::Offered,
            deadline: now
                .checked_add(self.policy.confirmation_timeout)
                .unwrap_or(now),
            updated_at: now,
            resolution: Some(sender),
        };
        if auto_accept {
            let token = issue_grant(&mut state, &stored, &self.policy, now)?;
            stored.status = TransferStatus::Accepted;
            stored.updated_at = now;
            send_resolution(&mut stored, TransferStatus::Accepted, Some(token));
        }
        state.seen.insert(stored.offer.transfer_id, now);
        state.offers.insert(stored.offer.transfer_id, stored);
        Ok(OfferSubmission { view, resolution })
    }

    /// Applies a local terminal decision. Trust requires explicit SAS verification.
    pub fn decide(
        &self,
        transfer_id: TransferId,
        decision: OfferDecision,
        sas_verified: bool,
        now: Instant,
    ) -> Result<(), OfferError> {
        let mut state = self.lock_state()?;
        purge(&mut state, &self.policy, now);
        let mut stored = state
            .offers
            .remove(&transfer_id)
            .ok_or(OfferError::NotFound)?;
        if stored.status != TransferStatus::Offered {
            let error = if stored.status == TransferStatus::Expired {
                OfferError::Expired
            } else {
                OfferError::AlreadyResolved
            };
            state.offers.insert(transfer_id, stored);
            return Err(error);
        }
        if now >= stored.deadline {
            stored.status = TransferStatus::Expired;
            stored.updated_at = now;
            send_resolution(&mut stored, TransferStatus::Expired, None);
            state.offers.insert(transfer_id, stored);
            return Err(OfferError::Expired);
        }

        if matches!(decision, OfferDecision::AcceptAndTrust) && !sas_verified {
            state.offers.insert(transfer_id, stored);
            return Err(OfferError::SasVerificationRequired);
        }
        let decision_result = match decision {
            OfferDecision::Reject { .. } => Ok((TransferStatus::Rejected, None)),
            OfferDecision::AcceptOnce => issue_grant(&mut state, &stored, &self.policy, now)
                .map(|token| (TransferStatus::Accepted, Some(token))),
            OfferDecision::AcceptAndTrust => {
                let token = issue_grant(&mut state, &stored, &self.policy, now);
                match token {
                    Ok(token) => {
                        if let Err(error) = self.trust_store.trust_peer(
                            stored.owner.clone(),
                            &stored.owner_name,
                            stored.owner_public_key,
                        ) {
                            state.grants.remove(&transfer_id);
                            Err(OfferError::TrustStore(error))
                        } else {
                            Ok((TransferStatus::Accepted, Some(token)))
                        }
                    }
                    Err(error) => Err(error),
                }
            }
        };
        let (status, token) = match decision_result {
            Ok(result) => result,
            Err(error) => {
                state.offers.insert(transfer_id, stored);
                return Err(error);
            }
        };
        stored.status = status;
        stored.updated_at = now;
        send_resolution(&mut stored, status, token);
        state.offers.insert(transfer_id, stored);
        Ok(())
    }

    /// Returns the single policy deadline used by both manager state and network prompts.
    #[must_use]
    pub fn confirmation_timeout(&self) -> Duration {
        self.policy.confirmation_timeout
    }

    /// Expires pending confirmations and wakes their owner channels.
    pub fn expire(&self, now: Instant) -> Result<usize, OfferError> {
        let mut state = self.lock_state()?;
        let mut count = 0;
        for stored in state.offers.values_mut() {
            if stored.status == TransferStatus::Offered && now >= stored.deadline {
                stored.status = TransferStatus::Expired;
                stored.updated_at = now;
                send_resolution(stored, TransferStatus::Expired, None);
                count += 1;
            }
        }
        purge(&mut state, &self.policy, now);
        Ok(count)
    }

    /// Owner-only status query; does not disclose whether other transfer IDs exist.
    pub fn status(
        &self,
        peer_device_id: &DeviceId,
        transfer_id: TransferId,
        now: Instant,
    ) -> Result<TransferStatus, OfferError> {
        let mut state = self.lock_state()?;
        purge(&mut state, &self.policy, now);
        state
            .offers
            .get(&transfer_id)
            .filter(|stored| &stored.owner == peer_device_id)
            .map(|stored| stored.status)
            .ok_or(OfferError::Unauthorized)
    }

    /// Validates a secret grant against all authorization dimensions.
    pub fn authorize(
        &self,
        token: &AuthorizationToken,
        peer_device_id: &DeviceId,
        transfer_id: TransferId,
        permission: AuthorizationPermission,
        now: Instant,
    ) -> Result<AuthorizedOffer, OfferError> {
        let mut state = self.lock_state()?;
        purge(&mut state, &self.policy, now);
        let grant = state
            .grants
            .get(&transfer_id)
            .ok_or(OfferError::Unauthorized)?;
        let presented_hash = token_hash(&token.0);
        if !bool::from(presented_hash.ct_eq(&grant.token_hash))
            || &grant.owner != peer_device_id
            || grant.transfer_id != transfer_id
            || now >= grant.expires_at
            || !grant.permissions.contains(&permission)
        {
            return Err(OfferError::Unauthorized);
        }
        let stored = state
            .offers
            .get(&transfer_id)
            .ok_or(OfferError::Unauthorized)?;
        if stored.owner != grant.owner
            || stored.manifest_digest != grant.manifest_digest
            || stored.status != TransferStatus::Accepted
        {
            return Err(OfferError::Unauthorized);
        }
        Ok(AuthorizedOffer {
            offer: stored.offer.clone(),
            manifest_digest: stored.manifest_digest,
        })
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, ManagerState>, OfferError> {
        self.state.lock().map_err(|_| OfferError::Internal)
    }
}

#[derive(Default)]
struct ManagerState {
    offers: BTreeMap<TransferId, StoredOffer>,
    grants: BTreeMap<TransferId, StoredGrant>,
    seen: BTreeMap<TransferId, Instant>,
    rates: BTreeMap<DeviceId, VecDeque<Instant>>,
}

struct StoredOffer {
    owner: DeviceId,
    owner_name: String,
    owner_public_key: [u8; 32],
    offer: TransferOffer,
    manifest_digest: [u8; 32],
    status: TransferStatus,
    deadline: Instant,
    updated_at: Instant,
    resolution: Option<oneshot::Sender<OfferResolution>>,
}

struct StoredGrant {
    token_hash: [u8; 32],
    owner: DeviceId,
    transfer_id: TransferId,
    manifest_digest: [u8; 32],
    permissions: BTreeSet<AuthorizationPermission>,
    expires_at: Instant,
}

fn issue_grant(
    state: &mut ManagerState,
    offer: &StoredOffer,
    policy: &OfferPolicy,
    now: Instant,
) -> Result<AuthorizationToken, OfferError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| OfferError::EntropyUnavailable)?;
    let token = AuthorizationToken(bytes);
    state.grants.insert(
        offer.offer.transfer_id,
        StoredGrant {
            token_hash: token_hash(&token.0),
            owner: offer.owner.clone(),
            transfer_id: offer.offer.transfer_id,
            manifest_digest: offer.manifest_digest,
            permissions: BTreeSet::from([
                AuthorizationPermission::OfferStatus,
                AuthorizationPermission::TransferStatus,
                AuthorizationPermission::ChunkUpload,
                AuthorizationPermission::Complete,
                AuthorizationPermission::Cancel,
            ]),
            expires_at: now.checked_add(policy.grant_ttl).unwrap_or(now),
        },
    );
    Ok(token)
}

fn token_hash(token: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("quick-share/qsp1/authorization-token", token)
}

fn digest_offer(offer: &TransferOffer) -> Result<[u8; 32], OfferError> {
    let bytes =
        serde_json::to_vec(offer).map_err(|error| OfferError::InvalidOffer(error.to_string()))?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

fn enforce_rate(
    state: &mut ManagerState,
    policy: &OfferPolicy,
    device_id: &DeviceId,
    now: Instant,
) -> Result<(), OfferError> {
    let window_start = now.checked_sub(policy.rate_window).unwrap_or(now);
    let entries = state.rates.entry(device_id.clone()).or_default();
    while entries.front().is_some_and(|seen| *seen <= window_start) {
        entries.pop_front();
    }
    if entries.len() >= policy.max_offers_per_window {
        return Err(OfferError::RateLimited);
    }
    entries.push_back(now);
    Ok(())
}

fn purge(state: &mut ManagerState, policy: &OfferPolicy, now: Instant) {
    for offer in state.offers.values_mut() {
        if offer.status == TransferStatus::Offered && now >= offer.deadline {
            offer.status = TransferStatus::Expired;
            offer.updated_at = now;
            send_resolution(offer, TransferStatus::Expired, None);
        }
    }
    let replay_cutoff = now.checked_sub(policy.replay_ttl).unwrap_or(now);
    state.seen.retain(|_, seen| *seen > replay_cutoff);
    let rate_cutoff = now.checked_sub(policy.rate_window).unwrap_or(now);
    state.rates.retain(|_, entries| {
        while entries.front().is_some_and(|seen| *seen <= rate_cutoff) {
            entries.pop_front();
        }
        !entries.is_empty()
    });
    state.grants.retain(|_, grant| now < grant.expires_at);
    let terminal_cutoff = now.checked_sub(policy.terminal_retention).unwrap_or(now);
    let grants = &state.grants;
    state.offers.retain(|transfer_id, offer| {
        offer.status == TransferStatus::Offered
            || grants.contains_key(transfer_id)
            || offer.updated_at > terminal_cutoff
    });
}

fn send_resolution(
    offer: &mut StoredOffer,
    status: TransferStatus,
    authorization: Option<AuthorizationToken>,
) {
    if let Some(sender) = offer.resolution.take() {
        let _ = sender.send(OfferResolution {
            status,
            authorization,
        });
    }
}

fn offer_view(
    peer: &PeerAuthContext,
    peer_address: Option<SocketAddr>,
    offer: &TransferOffer,
    currently_trusted: bool,
) -> OfferView {
    OfferView {
        transfer_id: offer.transfer_id,
        sender_device_id: peer.device_id().clone(),
        sender_name: peer.name().to_owned(),
        peer_address,
        sas: peer.sas(),
        content_kind: offer.content_kind,
        entry_count: offer.entries.len(),
        total_bytes: offer.total_bytes,
        entries: offer
            .entries
            .iter()
            .map(|entry| OfferEntryView {
                relative_path: entry.relative_path.clone(),
                kind: match entry.kind {
                    ManifestEntryKind::File => "file",
                    ManifestEntryKind::Directory => "directory",
                    ManifestEntryKind::Symlink { .. } => "symlink",
                    ManifestEntryKind::Text { .. } => "text",
                },
                size: entry.size,
            })
            .collect(),
        identity_changed: peer.change_reason().is_some(),
        trusted: currently_trusted,
    }
}

#[derive(Debug, Error)]
pub enum OfferError {
    #[error("offer is invalid: {0}")]
    InvalidOffer(String),
    #[error("offer sender does not match the authenticated Noise identity")]
    IdentityMismatch,
    #[error("offer request is unauthorized")]
    Unauthorized,
    #[error("offer transfer ID was replayed")]
    Replay,
    #[error("offer replay cache is full")]
    ReplayCacheFull,
    #[error("too many pending offers")]
    QueueFull,
    #[error("offer rate limit exceeded")]
    RateLimited,
    #[error("offer was not found")]
    NotFound,
    #[error("offer was already resolved")]
    AlreadyResolved,
    #[error("offer confirmation expired")]
    Expired,
    #[error("SAS verification is required before trusting this device")]
    SasVerificationRequired,
    #[error("secure random authorization token generation failed")]
    EntropyUnavailable,
    #[error(transparent)]
    TrustStore(#[from] TrustStoreError),
    #[error("internal offer state is unavailable")]
    Internal,
}
