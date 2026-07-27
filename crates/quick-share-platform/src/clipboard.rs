//! Native text clipboard abstraction with structured headless degradation.

use thiserror::Error;

/// Minimal injectable text clipboard boundary.
pub trait Clipboard: Send {
    fn read_text(&mut self) -> Result<String, ClipboardError>;
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// Native arboard adapter without image support.
pub struct NativeClipboard {
    inner: arboard::Clipboard,
}

impl std::fmt::Debug for NativeClipboard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeClipboard")
            .finish_non_exhaustive()
    }
}

impl NativeClipboard {
    /// Connects to the desktop clipboard. Linux headless sessions fail before backend startup.
    pub fn connect() -> Result<Self, ClipboardError> {
        ensure_desktop_session()?;
        arboard::Clipboard::new()
            .map(|inner| Self { inner })
            .map_err(classify_error)
    }
}

impl Clipboard for NativeClipboard {
    fn read_text(&mut self) -> Result<String, ClipboardError> {
        self.inner.get_text().map_err(classify_error)
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.inner.set_text(text.to_owned()).map_err(classify_error)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn ensure_desktop_session() -> Result<(), ClipboardError> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(ClipboardError::Unavailable(
            "no DISPLAY or WAYLAND_DISPLAY; use stdout or --output".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn ensure_desktop_session() -> Result<(), ClipboardError> {
    Ok(())
}

fn classify_error(error: arboard::Error) -> ClipboardError {
    match error {
        arboard::Error::ContentNotAvailable => ClipboardError::ContentUnavailable,
        arboard::Error::ClipboardNotSupported => ClipboardError::Unsupported,
        arboard::Error::ClipboardOccupied => ClipboardError::Busy,
        arboard::Error::ConversionFailure => ClipboardError::Unsupported,
        other => ClipboardError::Unavailable(sanitize_error(other)),
    }
}

fn sanitize_error(error: impl std::fmt::Display) -> String {
    error
        .to_string()
        .chars()
        .filter(|character| !character.is_control())
        .take(300)
        .collect()
}

/// Recoverable native clipboard failure.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ClipboardError {
    #[error("clipboard is unavailable: {0}")]
    Unavailable(String),
    #[error("clipboard does not currently contain text")]
    ContentUnavailable,
    #[error("clipboard is temporarily busy")]
    Busy,
    #[error("text clipboard is unsupported in this environment")]
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::sanitize_error;

    #[test]
    fn backend_error_text_cannot_inject_terminal_controls() {
        assert_eq!(sanitize_error("failure\n\u{1b}[31mred"), "failure[31mred");
        assert_eq!(sanitize_error("x".repeat(500)).len(), 300);
    }
}
