//! Cross-platform relative-path validation and destination conflict policy.

use crate::config::ConflictPolicy;
use quick_share_protocol::MAX_RELATIVE_PATH_BYTES;
use std::{
    fmt,
    path::{Path, PathBuf},
};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// A normalized portable path that can never be absolute or contain `..`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativePath(String);

impl RelativePath {
    /// Parses a slash-separated path under strict Windows/macOS/Unix-compatible rules.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, PathError> {
        let value = value.as_ref();
        if value.is_empty() || value.len() > MAX_RELATIVE_PATH_BYTES {
            return Err(PathError::Invalid(value.to_owned()));
        }
        if value.starts_with(['/', '\\']) || has_windows_prefix(value) {
            return Err(PathError::Absolute(value.to_owned()));
        }
        let mut normalized = Vec::new();
        for component in value.split(['/', '\\']) {
            validate_component(component)?;
            normalized.push(component);
        }
        Ok(Self(normalized.join("/")))
    }

    /// Stable slash-separated representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Resolves this already-validated path below `root` without canonicalization.
    #[must_use]
    pub fn resolve_under(&self, root: &Path) -> PathBuf {
        let mut destination = root.to_path_buf();
        for component in self.0.split('/') {
            destination.push(component);
        }
        destination
    }

    /// NFC and case-fold-like key used to detect cross-platform destination collisions.
    #[must_use]
    pub fn collision_key(&self) -> String {
        self.0.nfc().flat_map(char::to_lowercase).collect()
    }

    pub(crate) fn components(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Debug for RelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RelativePath")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn has_windows_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validate_component(component: &str) -> Result<(), PathError> {
    if component.is_empty() || matches!(component, "." | "..") {
        return Err(PathError::Invalid(component.to_owned()));
    }
    if component.len() > 255
        || component.ends_with(['.', ' '])
        || component.chars().any(|character| {
            character == '\0'
                || character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
        || is_windows_reserved(component)
    {
        return Err(PathError::Invalid(component.to_owned()));
    }
    Ok(())
}

fn is_windows_reserved(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(
            stem.strip_prefix("COM"),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
        )
        || matches!(
            stem.strip_prefix("LPT"),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
        )
}

/// Applies destination conflict policy and keeps the path under `root`.
pub fn resolve_destination(
    root: &Path,
    relative: &RelativePath,
    policy: ConflictPolicy,
) -> Result<Option<PathBuf>, PathError> {
    ensure_no_symlink_ancestors(root, relative, true)?;
    let destination = relative.resolve_under(root);
    if !destination.exists() {
        return Ok(Some(destination));
    }
    match policy {
        ConflictPolicy::Rename => find_renamed_destination(&destination).map(Some),
        ConflictPolicy::Skip => Ok(None),
        ConflictPolicy::Overwrite => Ok(Some(destination)),
        ConflictPolicy::Ask => Err(PathError::InteractionRequired(destination)),
        ConflictPolicy::Error => Err(PathError::DestinationExists(destination)),
    }
}

pub(crate) fn ensure_no_symlink_ancestors(
    root: &Path,
    relative: &RelativePath,
    include_leaf: bool,
) -> Result<(), PathError> {
    let mut current = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        current.push(component);
        let is_leaf = index + 1 == component_count;
        if is_leaf && !include_leaf {
            break;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathError::SymlinkAncestor(current));
            }
            Ok(metadata) if !is_leaf && !metadata.is_dir() => {
                return Err(PathError::NonDirectoryAncestor(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(PathError::InspectAncestor {
                    path: current,
                    source: error,
                });
            }
        }
    }
    Ok(())
}

fn find_renamed_destination(path: &Path) -> Result<PathBuf, PathError> {
    let parent = path
        .parent()
        .ok_or_else(|| PathError::Invalid(path.display().to_string()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| PathError::Invalid(path.display().to_string()))?;
    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (file_name, None),
    };
    for index in 1..=10_000_u32 {
        let candidate_name = extension.map_or_else(
            || format!("{stem} ({index})"),
            |extension| format!("{stem} ({index}).{extension}"),
        );
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(PathError::RenameExhausted(path.to_path_buf()))
}

/// Path validation or destination resolution failure.
#[derive(Debug, Error)]
pub enum PathError {
    /// Empty, parent, malformed, reserved, or non-portable component.
    #[error("invalid portable relative path: {0:?}")]
    Invalid(String),
    /// Absolute, drive-prefixed, or UNC path.
    #[error("absolute paths are not allowed: {0:?}")]
    Absolute(String),
    /// Destination exists under `error` policy.
    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),
    /// `ask` cannot be resolved without an interactive decision.
    #[error("destination requires interactive conflict resolution: {0}")]
    InteractionRequired(PathBuf),
    /// Existing path component is a symbolic link and could redirect writes.
    #[error("destination path contains a symbolic-link component: {0}")]
    SymlinkAncestor(PathBuf),
    /// Existing ancestor is not a directory.
    #[error("destination ancestor is not a directory: {0}")]
    NonDirectoryAncestor(PathBuf),
    /// Existing ancestor could not be inspected.
    #[error("cannot inspect destination ancestor {path}: {source}")]
    InspectAncestor {
        /// Path being inspected.
        path: PathBuf,
        /// File-system failure.
        #[source]
        source: std::io::Error,
    },
    /// No bounded rename candidate was available.
    #[error("could not find a free destination name for {0}")]
    RenameExhausted(PathBuf),
}
