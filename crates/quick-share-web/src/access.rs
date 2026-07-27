use std::{
    fmt,
    sync::Mutex,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

const TOKEN_BYTES: usize = 16;

pub struct AccessToken(String);

impl AccessToken {
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccessToken([REDACTED])")
    }
}

impl Drop for AccessToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub struct WebAccess {
    token_hash: [u8; 32],
    expires_at: Instant,
    max_downloads: Option<u32>,
    state: Mutex<AccessState>,
    quota_shutdown: CancellationToken,
}

#[derive(Debug, Default)]
struct AccessState {
    download_sessions: u32,
}

impl WebAccess {
    pub fn generate(
        ttl: Duration,
        max_downloads: Option<u32>,
    ) -> Result<(Self, AccessToken), AccessError> {
        if matches!(max_downloads, Some(0)) {
            return Err(AccessError::InvalidLimit);
        }
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| AccessError::EntropyUnavailable)?;
        let token = AccessToken(hex::encode(bytes));
        bytes.zeroize();
        let access = Self {
            token_hash: token_hash(token.expose()),
            expires_at: Instant::now().checked_add(ttl).unwrap_or_else(Instant::now),
            max_downloads,
            state: Mutex::new(AccessState::default()),
            quota_shutdown: CancellationToken::new(),
        };
        Ok((access, token))
    }

    pub(crate) fn authorize(&self, presented: &str) -> Result<(), AccessError> {
        if Instant::now() >= self.expires_at {
            return Err(AccessError::Unauthorized);
        }
        let presented_hash = token_hash(presented);
        if !bool::from(presented_hash.ct_eq(&self.token_hash)) {
            return Err(AccessError::Unauthorized);
        }
        Ok(())
    }

    pub(crate) fn begin_download(&self, presented: &str) -> Result<(), AccessError> {
        self.authorize(presented)?;
        let mut state = self.state.lock().map_err(|_| AccessError::Internal)?;
        if self
            .max_downloads
            .is_some_and(|limit| state.download_sessions >= limit)
        {
            return Err(AccessError::QuotaExceeded);
        }
        state.download_sessions = state
            .download_sessions
            .checked_add(1)
            .ok_or(AccessError::QuotaExceeded)?;
        if self
            .max_downloads
            .is_some_and(|limit| state.download_sessions >= limit)
        {
            self.quota_shutdown.cancel();
        }
        Ok(())
    }

    #[must_use]
    pub fn remaining_ttl(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }

    #[must_use]
    pub const fn max_downloads(&self) -> Option<u32> {
        self.max_downloads
    }

    #[must_use]
    pub fn quota_shutdown(&self) -> CancellationToken {
        self.quota_shutdown.clone()
    }
}

impl fmt::Debug for WebAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebAccess")
            .field("token_hash", &"[REDACTED]")
            .field("remaining_ttl", &self.remaining_ttl())
            .field("max_downloads", &self.max_downloads)
            .finish_non_exhaustive()
    }
}

fn token_hash(token: &str) -> [u8; 32] {
    blake3::derive_key("quick-share/web/access-token/v1", token.as_bytes())
}

#[derive(Debug, Error)]
pub enum AccessError {
    #[error("download limit must be greater than zero")]
    InvalidLimit,
    #[error("secure Web token generation failed")]
    EntropyUnavailable,
    #[error("Web access token is missing, invalid, or expired")]
    Unauthorized,
    #[error("Web download session limit reached")]
    QuotaExceeded,
    #[error("Web access state is unavailable")]
    Internal,
}
