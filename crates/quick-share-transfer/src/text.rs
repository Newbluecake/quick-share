//! Bounded text payloads and safe clipboard/stdout/file delivery.

use quick_share_platform::{
    FileSensitivity, StorageError, atomic_write_new,
    clipboard::{Clipboard, ClipboardError},
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{self, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Maximum UTF-8 text bytes accepted in one payload.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// User-selected text source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    Literal,
    Clipboard,
}

/// Received or outgoing plain text. Debug output never includes content.
#[derive(Clone, PartialEq, Eq)]
pub struct TextPayload {
    source: TextSource,
    text: String,
    digest: [u8; 32],
}

impl TextPayload {
    pub fn new(source: TextSource, text: impl Into<String>) -> Result<Self, TextError> {
        let text = text.into();
        if text.len() > MAX_TEXT_BYTES {
            return Err(TextError::TooLarge {
                actual: text.len(),
                limit: MAX_TEXT_BYTES,
            });
        }
        let digest = *blake3::hash(text.as_bytes()).as_bytes();
        Ok(Self {
            source,
            text,
            digest,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn source(&self) -> TextSource {
        self.source
    }

    #[must_use]
    pub fn summary(&self) -> TextSummary {
        TextSummary {
            source: self.source,
            bytes: self.text.len(),
            characters: self.text.chars().count(),
            digest_prefix: hex::encode(&self.digest[..6]),
        }
    }
}

impl fmt::Debug for TextPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextPayload")
            .field("source", &self.source)
            .field("text", &"[REDACTED]")
            .field("summary", &self.summary())
            .finish()
    }
}

/// Safe content-free display metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSummary {
    pub source: TextSource,
    pub bytes: usize,
    pub characters: usize,
    pub digest_prefix: String,
}

/// Reads a bounded outgoing payload from an injected clipboard.
pub fn capture_clipboard(clipboard: &mut dyn Clipboard) -> Result<TextPayload, TextError> {
    TextPayload::new(TextSource::Clipboard, clipboard.read_text()?)
}

/// Delivery destination after attempting the native clipboard first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextDeliveryTarget {
    Clipboard,
    File(PathBuf),
    Stdout,
}

/// Delivery result retains a structured clipboard warning for user-facing diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDelivery {
    pub target: TextDeliveryTarget,
    pub clipboard_warning: Option<ClipboardError>,
}

/// Writes text to the clipboard, then safely degrades to explicit file or stdout.
pub fn deliver_text(
    payload: &TextPayload,
    clipboard: &mut dyn Clipboard,
    output: Option<&Path>,
    stdout: &mut dyn Write,
) -> Result<TextDelivery, TextError> {
    if let Some(path) = output {
        atomic_write_new(path, payload.as_str().as_bytes(), FileSensitivity::Normal)?;
        return Ok(TextDelivery {
            target: TextDeliveryTarget::File(path.to_path_buf()),
            clipboard_warning: None,
        });
    }
    match clipboard.write_text(payload.as_str()) {
        Ok(()) => Ok(TextDelivery {
            target: TextDeliveryTarget::Clipboard,
            clipboard_warning: None,
        }),
        Err(error) => {
            stdout.write_all(payload.as_str().as_bytes())?;
            stdout.flush()?;
            Ok(TextDelivery {
                target: TextDeliveryTarget::Stdout,
                clipboard_warning: Some(error),
            })
        }
    }
}

/// Text capture or delivery failure.
#[derive(Debug, Error)]
pub enum TextError {
    #[error("text payload has {actual} bytes, exceeding the {limit}-byte limit")]
    TooLarge { actual: usize, limit: usize },
    #[error(transparent)]
    Clipboard(#[from] ClipboardError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("text output failed: {0}")]
    Output(#[from] io::Error),
}
