//! One-shot exact authorization for a remote-selection callback offer.

use quick_share_core::identity::device_id_from_public_key;
use quick_share_protocol::{DeviceId, RequestId, TransferId, TransferOffer};
use std::{
    collections::BTreeMap,
    fmt,
    net::IpAddr,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};
use thiserror::Error;

const MAX_EXPECTED_REPLAY_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_EXPECTED_PENDING: usize = 64;
const MAX_EXPECTED_REPLAY_ENTRIES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedOfferPolicy {
    pub maximum_pending: usize,
    pub maximum_replay_entries: usize,
    pub replay_ttl: Duration,
}

impl Default for ExpectedOfferPolicy {
    fn default() -> Self {
        Self {
            maximum_pending: 32,
            maximum_replay_entries: 256,
            replay_ttl: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExpectedCallback {
    pub request_id: RequestId,
    pub transfer_id: TransferId,
    pub sender_device_id: DeviceId,
    pub sender_public_key: [u8; 32],
    pub source_ip: IpAddr,
    pub expires_at: Instant,
}

impl fmt::Debug for ExpectedCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpectedCallback")
            .field("request_id", &self.request_id)
            .field("transfer_id", &self.transfer_id)
            .field("sender_device_id", &self.sender_device_id)
            .field("sender_public_key", &"[REDACTED]")
            .field("source_ip", &self.source_ip)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Proof that a callback matched and consumed one exact expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedOfferGrant {
    pub request_id: RequestId,
    pub transfer_id: TransferId,
}

pub struct ExpectedOfferRegistry {
    policy: ExpectedOfferPolicy,
    state: Mutex<ExpectedState>,
}

impl fmt::Debug for ExpectedOfferRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpectedOfferRegistry")
            .field("policy", &self.policy)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

impl ExpectedOfferRegistry {
    pub fn new(policy: ExpectedOfferPolicy) -> Result<Self, ExpectedOfferError> {
        if policy.maximum_pending == 0
            || policy.maximum_pending > MAX_EXPECTED_PENDING
            || policy.maximum_replay_entries == 0
            || policy.maximum_replay_entries > MAX_EXPECTED_REPLAY_ENTRIES
            || policy.replay_ttl.is_zero()
            || policy.replay_ttl > MAX_EXPECTED_REPLAY_TTL
        {
            return Err(ExpectedOfferError::InvalidPolicy);
        }
        Ok(Self {
            policy,
            state: Mutex::new(ExpectedState::default()),
        })
    }

    pub fn register(
        &self,
        expected: ExpectedCallback,
        now: Instant,
    ) -> Result<(), ExpectedOfferError> {
        if expected.expires_at <= now {
            return Err(ExpectedOfferError::Expired);
        }
        if device_id_from_public_key(&expected.sender_public_key)
            .map_err(|_| ExpectedOfferError::IdentityMismatch)?
            != expected.sender_device_id
        {
            return Err(ExpectedOfferError::IdentityMismatch);
        }
        let mut state = self.lock_state()?;
        self.prune(&mut state, now);
        if state.consumed.contains_key(&expected.request_id)
            || state.expired.contains_key(&expected.request_id)
        {
            return Err(ExpectedOfferError::Replay);
        }
        if let Some(existing) = state.pending.get(&expected.request_id) {
            return if existing == &expected {
                Ok(())
            } else {
                Err(ExpectedOfferError::Replay)
            };
        }
        if state.pending.len() >= self.policy.maximum_pending
            || state
                .pending
                .values()
                .any(|item| item.transfer_id == expected.transfer_id)
        {
            return Err(ExpectedOfferError::Capacity);
        }
        state.pending.insert(expected.request_id, expected);
        Ok(())
    }

    pub fn consume(
        &self,
        authenticated_device_id: &DeviceId,
        authenticated_public_key: &[u8; 32],
        source_ip: IpAddr,
        offer: &TransferOffer,
        now: Instant,
    ) -> Result<ExpectedOfferGrant, ExpectedOfferError> {
        let request_id = offer.initiated_by.ok_or(ExpectedOfferError::NotExpected)?;
        let mut state = self.lock_state()?;
        self.prune(&mut state, now);
        if state.consumed.contains_key(&request_id) {
            return Err(ExpectedOfferError::Replay);
        }
        if state.expired.contains_key(&request_id) {
            return Err(ExpectedOfferError::Expired);
        }
        let expected = state
            .pending
            .get(&request_id)
            .ok_or(ExpectedOfferError::NotExpected)?;
        if expected.expires_at <= now {
            state.pending.remove(&request_id);
            self.remember_consumed(&mut state, request_id, now);
            return Err(ExpectedOfferError::Expired);
        }
        if &expected.sender_device_id != authenticated_device_id
            || &expected.sender_public_key != authenticated_public_key
            || offer.sender.device_id != *authenticated_device_id
            || expected.transfer_id != offer.transfer_id
            || expected.source_ip != source_ip
        {
            return Err(ExpectedOfferError::Mismatch);
        }
        state.pending.remove(&request_id);
        self.remember_consumed(&mut state, request_id, now);
        Ok(ExpectedOfferGrant {
            request_id,
            transfer_id: offer.transfer_id,
        })
    }

    fn remember_consumed(&self, state: &mut ExpectedState, request_id: RequestId, now: Instant) {
        state
            .consumed
            .insert(request_id, now + self.policy.replay_ttl);
        self.trim_replay(state);
    }

    fn trim_replay(&self, state: &mut ExpectedState) {
        while state.consumed.len() + state.expired.len() > self.policy.maximum_replay_entries {
            let consumed = state
                .consumed
                .iter()
                .min_by_key(|(_, expiry)| **expiry)
                .map(|(request_id, expiry)| (*request_id, *expiry));
            let expired = state
                .expired
                .iter()
                .min_by_key(|(_, expiry)| **expiry)
                .map(|(request_id, expiry)| (*request_id, *expiry));
            match (consumed, expired) {
                (Some((request_id, consumed_expiry)), Some((_, expired_expiry)))
                    if consumed_expiry <= expired_expiry =>
                {
                    state.consumed.remove(&request_id);
                }
                (_, Some((request_id, _))) => {
                    state.expired.remove(&request_id);
                }
                (Some((request_id, _)), None) => {
                    state.consumed.remove(&request_id);
                }
                (None, None) => break,
            }
        }
    }

    fn prune(&self, state: &mut ExpectedState, now: Instant) {
        state.consumed.retain(|_, expiry| *expiry > now);
        state.expired.retain(|_, expiry| *expiry > now);
        let expired = state
            .pending
            .iter()
            .filter_map(|(request_id, expected)| {
                (expected.expires_at <= now).then_some(*request_id)
            })
            .collect::<Vec<_>>();
        for request_id in expired {
            state.pending.remove(&request_id);
            state
                .expired
                .insert(request_id, now + self.policy.replay_ttl);
        }
        self.trim_replay(state);
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, ExpectedState>, ExpectedOfferError> {
        self.state.lock().map_err(|_| ExpectedOfferError::Internal)
    }
}

#[derive(Default)]
struct ExpectedState {
    pending: BTreeMap<RequestId, ExpectedCallback>,
    consumed: BTreeMap<RequestId, Instant>,
    expired: BTreeMap<RequestId, Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExpectedOfferError {
    #[error("expected-offer policy is invalid")]
    InvalidPolicy,
    #[error("expected callback identity is invalid")]
    IdentityMismatch,
    #[error("expected callback capacity is exhausted")]
    Capacity,
    #[error("callback offer was not expected")]
    NotExpected,
    #[error("callback offer does not match its exact expectation")]
    Mismatch,
    #[error("callback expectation expired")]
    Expired,
    #[error("callback request was already consumed")]
    Replay,
    #[error("expected callback state is unavailable")]
    Internal,
}
