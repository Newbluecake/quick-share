//! Peer identity classification after a completed Noise XX handshake.

use crate::noise::{HandshakeEvidence, SasCode};
use quick_share_core::identity::{TrustStatus, TrustStoreError, TrustedDeviceStore};
use quick_share_protocol::DeviceId;
use std::fmt;
use thiserror::Error;

/// Untrusted identity hint from discovery/INFO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerClaim {
    pub device_id: DeviceId,
    pub name: String,
}

/// Minimal operations available before an offer is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreAuthorizationPermission {
    Info,
    OfferCreate,
}

/// Why a peer must be shown as changed rather than silently trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityChange {
    PinnedKeyChanged,
    ClaimedIdDoesNotMatchKey,
    TrustedNameUsesDifferentKey,
}

#[derive(Clone)]
pub struct PeerIdentity {
    device_id: DeviceId,
    claimed_name: String,
    display_name: String,
    public_key: [u8; 32],
    sas: SasCode,
}

impl fmt::Debug for PeerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerIdentity")
            .field("device_id", &self.device_id)
            .field("claimed_name", &self.claimed_name)
            .field("display_name", &self.display_name)
            .field("public_key", &"[REDACTED]")
            .field("sas", &self.sas)
            .finish()
    }
}

/// Trust decision based on the full authenticated static key, never IP or display name alone.
#[derive(Clone)]
pub enum PeerAuthContext {
    Unknown {
        identity: PeerIdentity,
    },
    Trusted {
        identity: PeerIdentity,
    },
    Changed {
        identity: PeerIdentity,
        reason: IdentityChange,
    },
}

impl PeerAuthContext {
    #[must_use]
    pub fn device_id(&self) -> &DeviceId {
        &self.identity().device_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.identity().display_name
    }

    /// Authenticated peer-supplied name used only to validate its wire offer claim.
    #[must_use]
    pub fn claimed_name(&self) -> &str {
        &self.identity().claimed_name
    }

    #[must_use]
    pub const fn sas(&self) -> SasCode {
        self.identity().sas
    }

    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.identity().public_key
    }

    #[must_use]
    pub const fn is_trusted(&self) -> bool {
        matches!(self, Self::Trusted { .. })
    }

    #[must_use]
    pub const fn change_reason(&self) -> Option<IdentityChange> {
        match self {
            Self::Changed { reason, .. } => Some(*reason),
            Self::Unknown { .. } | Self::Trusted { .. } => None,
        }
    }

    /// Unknown and changed peers can only negotiate INFO and submit a bounded offer.
    #[must_use]
    pub const fn allows_pre_authorization(&self, permission: PreAuthorizationPermission) -> bool {
        matches!(
            permission,
            PreAuthorizationPermission::Info | PreAuthorizationPermission::OfferCreate
        )
    }

    const fn identity(&self) -> &PeerIdentity {
        match self {
            Self::Unknown { identity }
            | Self::Trusted { identity }
            | Self::Changed { identity, .. } => identity,
        }
    }
}

impl fmt::Debug for PeerAuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { identity } => formatter.debug_tuple("Unknown").field(identity).finish(),
            Self::Trusted { identity } => formatter.debug_tuple("Trusted").field(identity).finish(),
            Self::Changed { identity, reason } => formatter
                .debug_struct("Changed")
                .field("identity", identity)
                .field("reason", reason)
                .finish(),
        }
    }
}

/// Classifies a post-handshake identity against complete locally pinned keys.
pub fn classify_peer(
    claim: PeerClaim,
    evidence: &HandshakeEvidence,
    trust_store: &TrustedDeviceStore,
) -> Result<PeerAuthContext, AuthError> {
    validate_claim_name(&claim.name)?;
    let public_key = evidence.remote_static();
    let actual_device_id = evidence.remote_device_id.clone();
    let mut identity = PeerIdentity {
        device_id: actual_device_id.clone(),
        claimed_name: claim.name.clone(),
        display_name: claim.name.clone(),
        public_key,
        sas: evidence.sas,
    };

    match trust_store.check(&claim.device_id, &public_key)? {
        TrustStatus::Trusted(device) => {
            identity.display_name = device.name;
            return Ok(PeerAuthContext::Trusted { identity });
        }
        TrustStatus::KeyMismatch => {
            return Ok(PeerAuthContext::Changed {
                identity,
                reason: IdentityChange::PinnedKeyChanged,
            });
        }
        TrustStatus::Unknown => {}
    }
    if claim.device_id != actual_device_id {
        return Ok(PeerAuthContext::Changed {
            identity,
            reason: IdentityChange::ClaimedIdDoesNotMatchKey,
        });
    }
    if let Some(existing) = trust_store.find_by_name(&claim.name)?
        && existing.public_key != public_key
    {
        return Ok(PeerAuthContext::Changed {
            identity,
            reason: IdentityChange::TrustedNameUsesDifferentKey,
        });
    }
    Ok(PeerAuthContext::Unknown { identity })
}

fn validate_claim_name(name: &str) -> Result<(), AuthError> {
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(AuthError::InvalidClaim);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("peer identity claim has an invalid display name")]
    InvalidClaim,
    #[error(transparent)]
    TrustStore(#[from] TrustStoreError),
}
