//! Platform-neutral desktop interaction contract. Native implementations remain target-gated.

use std::{
    fmt,
    path::{Path, PathBuf},
};
use thiserror::Error;

mod broker;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod icon;
#[cfg(any(windows, test))]
mod mapping;
#[cfg(windows)]
mod source_picker;
#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub(crate) use broker::PendingSourceRequest;
pub use broker::{DesktopBroker, DesktopBrokerReceiver, DesktopWake, desktop_broker};
#[cfg(windows)]
pub(crate) use mapping::{DialogMapper, MessageRequest, MessageResult, NativeDialogs, SourceKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationDialog {
    /// Untrusted remote display name; native UI must render it as plain text.
    pub device_name: String,
    /// Authenticated stable identity shown for disambiguation.
    pub device_id: String,
    /// Authenticated session verification code.
    pub sas: String,
    pub identity_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationChoice {
    AcceptOnce,
    AcceptAndTrust,
    Reject,
    Cancelled,
}

impl AuthorizationChoice {
    #[must_use]
    pub const fn into_decision(self) -> Option<Self> {
        match self {
            Self::Cancelled => None,
            decision => Some(decision),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDialog {
    /// Untrusted remote display name; native UI must render it as plain text.
    pub requester_name: String,
    /// Local initial directory chosen from the last successful source selection.
    pub initial_directory: PathBuf,
}

#[derive(Clone, PartialEq, Eq)]
pub enum SourceChoice {
    /// Local paths returned by the native multi-file picker.
    Files(Vec<PathBuf>),
    /// Local root returned by the native directory picker.
    Folder(PathBuf),
    Cancelled,
}

impl SourceChoice {
    /// Returns selected roots while preserving cancellation as a normal result.
    pub fn selected_paths(&self) -> Result<Option<Vec<&Path>>, DesktopError> {
        match self {
            Self::Files(paths) if paths.is_empty() => Err(DesktopError::InvalidSelection),
            Self::Files(paths) => Ok(Some(paths.iter().map(PathBuf::as_path).collect())),
            Self::Folder(path) if path.as_os_str().is_empty() => {
                Err(DesktopError::InvalidSelection)
            }
            Self::Folder(path) => Ok(Some(vec![path.as_path()])),
            Self::Cancelled => Ok(None),
        }
    }
}

impl fmt::Debug for SourceChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Files(paths) => formatter
                .debug_tuple("Files")
                .field(&format_args!("[REDACTED; {} path(s)]", paths.len()))
                .finish(),
            Self::Folder(_) => formatter
                .debug_tuple("Folder")
                .field(&"[REDACTED]")
                .finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReceiveDirectoryDialog {
    /// Untrusted remote display name; native UI must render it as plain text.
    pub sender_name: String,
    /// Local default selected by trusted-device preference policy.
    pub current_directory: PathBuf,
}

impl fmt::Debug for ReceiveDirectoryDialog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiveDirectoryDialog")
            .field("sender_name", &self.sender_name)
            .field("current_directory", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum DirectoryChoice {
    /// Local destination confirmed by the user.
    Confirm(PathBuf),
    Cancelled,
}

impl DirectoryChoice {
    #[must_use]
    pub fn into_directory(self) -> Option<PathBuf> {
        match self {
            Self::Confirm(path) => Some(path),
            Self::Cancelled => None,
        }
    }
}

impl fmt::Debug for DirectoryChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirm(_) => formatter
                .debug_tuple("Confirm")
                .field(&"[REDACTED]")
                .finish(),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConflictDialog {
    /// Untrusted relative manifest path; native UI must render it as plain text.
    pub relative_path: String,
    pub directory: bool,
}

impl fmt::Debug for ConflictDialog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConflictDialog")
            .field("relative_path", &"[REDACTED]")
            .field("directory", &self.directory)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictScope {
    ThisEntry,
    AllRemaining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Decision {
        action: ConflictAction,
        scope: ConflictScope,
    },
    Cancelled,
}

impl ConflictChoice {
    #[must_use]
    pub const fn into_decision(self) -> Option<Self> {
        match self {
            Self::Cancelled => None,
            decision => Some(decision),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotification {
    pub title: String,
    pub message: String,
}

/// Synchronous UI boundary. Callers must marshal invocations to the native UI thread.
pub trait DesktopInteraction: Send + Sync {
    fn authorize_peer(
        &self,
        request: &AuthorizationDialog,
    ) -> Result<AuthorizationChoice, DesktopError>;

    fn choose_send_source(&self, request: &SourceDialog) -> Result<SourceChoice, DesktopError>;

    fn confirm_receive_directory(
        &self,
        request: &ReceiveDirectoryDialog,
    ) -> Result<DirectoryChoice, DesktopError>;

    fn resolve_conflict(&self, request: &ConflictDialog) -> Result<ConflictChoice, DesktopError>;

    fn notify(&self, notification: &DesktopNotification) -> Result<(), DesktopError>;
}

/// Safe stub for platforms without a production desktop adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableDesktop;

impl DesktopInteraction for UnavailableDesktop {
    fn authorize_peer(
        &self,
        _request: &AuthorizationDialog,
    ) -> Result<AuthorizationChoice, DesktopError> {
        Err(DesktopError::Unsupported)
    }

    fn choose_send_source(&self, _request: &SourceDialog) -> Result<SourceChoice, DesktopError> {
        Err(DesktopError::Unsupported)
    }

    fn confirm_receive_directory(
        &self,
        _request: &ReceiveDirectoryDialog,
    ) -> Result<DirectoryChoice, DesktopError> {
        Err(DesktopError::Unsupported)
    }

    fn resolve_conflict(&self, _request: &ConflictDialog) -> Result<ConflictChoice, DesktopError> {
        Err(DesktopError::Unsupported)
    }

    fn notify(&self, _notification: &DesktopNotification) -> Result<(), DesktopError> {
        Err(DesktopError::Unsupported)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DesktopError {
    #[error("desktop interaction is unsupported on this platform")]
    Unsupported,
    #[error("desktop interaction is unavailable in the current session")]
    Unavailable,
    #[error("desktop interaction is busy")]
    Busy,
    #[error("desktop interaction deadline expired")]
    TimedOut,
    #[error("desktop event loop exited")]
    EventLoopExited,
    #[error("desktop selection is empty or invalid")]
    InvalidSelection,
    #[error("receive directory is invalid")]
    InvalidDirectory,
    #[error("receive directory is not writable")]
    DirectoryNotWritable,
    #[error("desktop backend failed")]
    Backend,
}
