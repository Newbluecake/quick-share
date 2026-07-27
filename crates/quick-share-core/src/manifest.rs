//! Safe source traversal and transfer-manifest construction.

use crate::paths::{PathError, RelativePath, ensure_no_symlink_ancestors};
use quick_share_protocol::{EntryId, MAX_MANIFEST_ENTRIES};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Source state captured while constructing an offer for later TOCTOU checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    /// File length at scan time.
    pub size: u64,
    /// Last-modified nanoseconds since Unix epoch, when available.
    pub modified_ns: Option<u128>,
    /// Whether the source was read-only.
    pub readonly: bool,
}

impl SourceSnapshot {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        Self {
            size: metadata.len(),
            modified_ns,
            readonly: metadata.permissions().readonly(),
        }
    }
}

/// Portable manifest entry kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestEntryKind {
    /// Regular file, including empty and files larger than 4 GiB.
    File,
    /// Directory, including empty directories.
    Directory,
    /// Link metadata. The target is never followed unless explicitly requested.
    Symlink { target: String },
}

/// Internal source entry. The absolute source path is not serializable to the wire.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    /// Non-zero ID scoped to this manifest.
    pub id: EntryId,
    /// Portable destination-relative path.
    pub relative_path: RelativePath,
    /// Source kind.
    pub kind: ManifestEntryKind,
    /// Payload bytes for regular files.
    pub size: u64,
    /// Source state for pre-read verification.
    pub source_snapshot: SourceSnapshot,
    /// Local source used by the transfer layer only.
    pub source_path: PathBuf,
}

impl ManifestEntry {
    /// Re-reads metadata immediately before transfer to detect common source changes.
    pub fn source_is_unchanged(&self) -> Result<bool, io::Error> {
        let metadata = if matches!(self.kind, ManifestEntryKind::Symlink { .. }) {
            fs::symlink_metadata(&self.source_path)?
        } else {
            fs::metadata(&self.source_path)?
        };
        if SourceSnapshot::from_metadata(&metadata) != self.source_snapshot {
            return Ok(false);
        }
        if let ManifestEntryKind::Symlink { target } = &self.kind {
            return Ok(fs::read_link(&self.source_path)?.to_str() == Some(target.as_str()));
        }
        Ok(true)
    }
}

/// Constructed source manifest.
#[derive(Debug, Clone)]
pub struct TransferManifest {
    /// Deterministically traversed entries.
    pub entries: Vec<ManifestEntry>,
    /// Sum of regular-file bytes.
    pub total_bytes: u64,
}

/// Bounded manifest builder.
#[derive(Debug, Clone)]
pub struct ManifestBuilder {
    follow_links: bool,
    max_entries: usize,
}

impl Default for ManifestBuilder {
    fn default() -> Self {
        Self {
            follow_links: false,
            max_entries: MAX_MANIFEST_ENTRIES,
        }
    }
}

impl ManifestBuilder {
    /// Creates a default bounded builder that preserves links as links.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Explicitly follows source links and enables cycle checks.
    #[must_use]
    pub const fn follow_links(mut self, follow: bool) -> Self {
        self.follow_links = follow;
        self
    }

    /// Reduces the entry bound. Production callers should retain the protocol maximum.
    #[must_use]
    pub fn max_entries(mut self, limit: usize) -> Self {
        self.max_entries = limit.min(MAX_MANIFEST_ENTRIES);
        self
    }

    /// Scans files and directories into one manifest with unique top-level names.
    pub fn build(self, sources: &[PathBuf]) -> Result<TransferManifest, ManifestError> {
        if sources.is_empty() {
            return Err(ManifestError::NoSources);
        }
        let mut state = BuildState {
            builder: &self,
            entries: Vec::new(),
            total_bytes: 0,
            top_names: BTreeSet::new(),
        };
        for source in sources {
            let canonical_name;
            let name = match source.file_name().and_then(|name| name.to_str()) {
                Some(name) if !name.is_empty() => name,
                _ => {
                    canonical_name = fs::canonicalize(source)?;
                    canonical_name
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| ManifestError::InvalidSourceName(source.clone()))?
                }
            };
            let top = state.unique_top_name(name)?;
            let relative = RelativePath::parse(top)?;
            state.add_path(source, relative, &mut BTreeSet::new())?;
        }
        Ok(TransferManifest {
            entries: state.entries,
            total_bytes: state.total_bytes,
        })
    }
}

struct BuildState<'a> {
    builder: &'a ManifestBuilder,
    entries: Vec<ManifestEntry>,
    total_bytes: u64,
    top_names: BTreeSet<String>,
}

impl BuildState<'_> {
    fn unique_top_name(&mut self, requested: &str) -> Result<String, ManifestError> {
        RelativePath::parse(requested)?;
        if self.top_names.insert(collision_key(requested)) {
            return Ok(requested.to_owned());
        }
        let (stem, extension) = split_extension(requested);
        for index in 1..=10_000_u32 {
            let candidate = extension.map_or_else(
                || format!("{stem} ({index})"),
                |extension| format!("{stem} ({index}).{extension}"),
            );
            RelativePath::parse(&candidate)?;
            if self.top_names.insert(collision_key(&candidate)) {
                return Ok(candidate);
            }
        }
        Err(ManifestError::DuplicateNameExhausted(requested.to_owned()))
    }

    fn add_path(
        &mut self,
        source: &Path,
        relative: RelativePath,
        active_directories: &mut BTreeSet<PathBuf>,
    ) -> Result<(), ManifestError> {
        let link_metadata = fs::symlink_metadata(source)?;
        if link_metadata.file_type().is_symlink() && !self.builder.follow_links {
            let target = fs::read_link(source)?;
            let target = target
                .to_str()
                .ok_or_else(|| ManifestError::NonUnicodeLink(source.to_path_buf()))?;
            return self.push(
                source,
                relative,
                ManifestEntryKind::Symlink {
                    target: target.to_owned(),
                },
                0,
                &link_metadata,
            );
        }

        let metadata = if link_metadata.file_type().is_symlink() {
            fs::metadata(source).map_err(|error| ManifestError::FollowLink {
                path: source.to_path_buf(),
                source: error,
            })?
        } else {
            link_metadata
        };
        if metadata.is_file() {
            return self.push(
                source,
                relative,
                ManifestEntryKind::File,
                metadata.len(),
                &metadata,
            );
        }
        if !metadata.is_dir() {
            return Err(ManifestError::UnsupportedType(source.to_path_buf()));
        }

        let canonical = fs::canonicalize(source)?;
        if !active_directories.insert(canonical.clone()) {
            return Err(ManifestError::SymlinkCycle(source.to_path_buf()));
        }
        self.push(
            source,
            relative.clone(),
            ManifestEntryKind::Directory,
            0,
            &metadata,
        )?;
        let mut children: Vec<_> = fs::read_dir(source)?.collect::<Result<_, _>>()?;
        children.sort_by_key(fs::DirEntry::file_name);
        let mut destination_names = BTreeSet::new();
        for child in children {
            let name = child
                .file_name()
                .into_string()
                .map_err(|_| ManifestError::InvalidSourceName(child.path()))?;
            let portable_name = unique_portable_name(&name, &mut destination_names)?;
            let child_relative = RelativePath::parse(format!("{relative}/{portable_name}"))?;
            self.add_path(&child.path(), child_relative, active_directories)?;
        }
        active_directories.remove(&canonical);
        Ok(())
    }

    fn push(
        &mut self,
        source: &Path,
        relative_path: RelativePath,
        kind: ManifestEntryKind,
        size: u64,
        metadata: &fs::Metadata,
    ) -> Result<(), ManifestError> {
        if self.entries.len() >= self.builder.max_entries {
            return Err(ManifestError::TooManyEntries {
                limit: self.builder.max_entries,
            });
        }
        let raw_id =
            u32::try_from(self.entries.len() + 1).map_err(|_| ManifestError::TooManyEntries {
                limit: self.builder.max_entries,
            })?;
        let id = EntryId::new(raw_id).ok_or(ManifestError::TooManyEntries {
            limit: self.builder.max_entries,
        })?;
        if matches!(kind, ManifestEntryKind::File) {
            self.total_bytes = self
                .total_bytes
                .checked_add(size)
                .ok_or(ManifestError::TotalSizeOverflow)?;
        }
        self.entries.push(ManifestEntry {
            id,
            relative_path,
            kind,
            size,
            source_snapshot: SourceSnapshot::from_metadata(metadata),
            source_path: source.to_path_buf(),
        });
        Ok(())
    }
}

fn unique_portable_name(
    requested: &str,
    names: &mut BTreeSet<String>,
) -> Result<String, ManifestError> {
    RelativePath::parse(requested)?;
    if names.insert(collision_key(requested)) {
        return Ok(requested.to_owned());
    }
    let (stem, extension) = split_extension(requested);
    for index in 1..=10_000_u32 {
        let candidate = extension.map_or_else(
            || format!("{stem} ({index})"),
            |extension| format!("{stem} ({index}).{extension}"),
        );
        RelativePath::parse(&candidate)?;
        if names.insert(collision_key(&candidate)) {
            return Ok(candidate);
        }
    }
    Err(ManifestError::DuplicateNameExhausted(requested.to_owned()))
}

fn collision_key(name: &str) -> String {
    name.nfc().flat_map(char::to_lowercase).collect()
}

fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    }
}

/// Receiver policy for a link after validating its target relative to the link parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymlinkDisposition {
    /// Safe relative target and platform support are present.
    Create,
    /// Absolute or escaping target requires explicit user confirmation.
    NeedsConfirmation { reason: &'static str },
    /// Platform cannot create links; persist a human-readable notice instead.
    SaveAsNotice { reason: &'static str },
}

impl SymlinkDisposition {
    /// Classifies without touching the destination file system.
    #[must_use]
    pub fn classify(link: &RelativePath, target: &str, platform_supports_links: bool) -> Self {
        if !platform_supports_links {
            return Self::SaveAsNotice {
                reason: "symbolic links are unavailable on this platform",
            };
        }
        if target.is_empty()
            || target.contains('\0')
            || target.starts_with(['/', '\\'])
            || is_drive_prefixed(target)
        {
            return Self::NeedsConfirmation {
                reason: "symbolic-link target is absolute or malformed",
            };
        }
        let mut depth = link.components().count().saturating_sub(1);
        for component in target.split(['/', '\\']) {
            match component {
                "" => {
                    return Self::NeedsConfirmation {
                        reason: "symbolic-link target has an empty component",
                    };
                }
                "." => {}
                ".." if depth == 0 => {
                    return Self::NeedsConfirmation {
                        reason: "symbolic-link target escapes the receive root",
                    };
                }
                ".." => depth -= 1,
                component if RelativePath::parse(component).is_ok() => depth += 1,
                _ => {
                    return Self::NeedsConfirmation {
                        reason: "symbolic-link target is not portable",
                    };
                }
            }
        }
        Self::Create
    }
}

/// Result of committing received link metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymlinkCommitOutcome {
    /// Native symbolic link was created.
    Created(PathBuf),
    /// A non-executable explanatory file was created instead.
    NoticeSaved(PathBuf),
}

/// Creates a validated link, or safely degrades when confirmation or platform privilege is absent.
pub fn commit_symlink(
    root: &Path,
    link: &RelativePath,
    target: &str,
    target_is_directory: bool,
    allow_unsafe: bool,
) -> Result<SymlinkCommitOutcome, ManifestError> {
    ensure_no_symlink_ancestors(root, link, true)?;
    let destination = link.resolve_under(root);
    if matches!(
        SymlinkDisposition::classify(link, target, true),
        SymlinkDisposition::NeedsConfirmation { .. }
    ) && !allow_unsafe
    {
        return save_link_notice(
            &destination,
            target,
            "target requires explicit confirmation",
        );
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    match create_native_symlink(target, &destination, target_is_directory) {
        Ok(()) => Ok(SymlinkCommitOutcome::Created(destination)),
        Err(error) if symlink_is_unavailable(&error) => save_link_notice(
            &destination,
            target,
            "native symbolic-link creation is unavailable",
        ),
        Err(error) => Err(ManifestError::Io(error)),
    }
}

fn save_link_notice(
    destination: &Path,
    target: &str,
    reason: &str,
) -> Result<SymlinkCommitOutcome, ManifestError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ManifestError::InvalidSourceName(destination.to_path_buf()))?;
    let notice = destination.with_file_name(format!("{file_name}.quick-share-symlink.txt"));
    if let Some(parent) = notice.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&notice)?;
    writeln!(file, "Quick Share did not create a symbolic link.")?;
    writeln!(file, "Reason: {reason}")?;
    writeln!(file, "Original target: {target}")?;
    file.sync_all()?;
    Ok(SymlinkCommitOutcome::NoticeSaved(notice))
}

#[cfg(unix)]
fn create_native_symlink(target: &str, destination: &Path, _is_directory: bool) -> io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
fn create_native_symlink(target: &str, destination: &Path, is_directory: bool) -> io::Result<()> {
    if is_directory {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}

#[cfg(not(any(unix, windows)))]
fn create_native_symlink(
    _target: &str,
    _destination: &Path,
    _is_directory: bool,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links are unsupported",
    ))
}

fn symlink_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
    ) || error.raw_os_error() == Some(1314)
}

fn is_drive_prefixed(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Manifest construction failure.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// No source operands were supplied.
    #[error("at least one source is required")]
    NoSources,
    /// Source basename is absent or non-Unicode.
    #[error("source has no portable Unicode name: {0}")]
    InvalidSourceName(PathBuf),
    /// Path failed portable validation.
    #[error(transparent)]
    Path(#[from] PathError),
    /// File-system scan failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Explicit link following found a broken or inaccessible target.
    #[error("cannot follow symbolic link {path}: {source}")]
    FollowLink {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// Link target cannot be represented in QSP/1.
    #[error("symbolic-link target is not Unicode: {0}")]
    NonUnicodeLink(PathBuf),
    /// Explicit link following encountered a directory cycle.
    #[error("symbolic-link cycle detected at {0}")]
    SymlinkCycle(PathBuf),
    /// Socket, device, FIFO, or other unsupported source.
    #[error("unsupported source file type: {0}")]
    UnsupportedType(PathBuf),
    /// Entry count exceeded its hard bound.
    #[error("manifest exceeds the {limit}-entry limit")]
    TooManyEntries { limit: usize },
    /// Multiple top names exhausted bounded renaming.
    #[error("could not create a unique top-level name for {0:?}")]
    DuplicateNameExhausted(String),
    /// Sum of source sizes exceeded `u64`.
    #[error("manifest total size overflowed")]
    TotalSizeOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_manifest_enforces_the_twenty_thousand_entry_bound() {
        let root = tempfile::tempdir().expect("temporary root");
        let source = root.path().join("source");
        fs::write(&source, b"").expect("source file");
        let metadata = fs::metadata(&source).expect("metadata");
        let builder = ManifestBuilder::new();
        let mut state = BuildState {
            builder: &builder,
            entries: Vec::new(),
            total_bytes: 0,
            top_names: BTreeSet::new(),
        };
        for index in 0..MAX_MANIFEST_ENTRIES {
            state
                .push(
                    &source,
                    RelativePath::parse(format!("{index}.txt")).expect("relative path"),
                    ManifestEntryKind::File,
                    0,
                    &metadata,
                )
                .expect("entry within limit");
        }
        assert_eq!(state.entries.len(), MAX_MANIFEST_ENTRIES);
        assert!(matches!(
            state.push(
                &source,
                RelativePath::parse("overflow.txt").expect("overflow path"),
                ManifestEntryKind::File,
                0,
                &metadata,
            ),
            Err(ManifestError::TooManyEntries {
                limit: MAX_MANIFEST_ENTRIES
            })
        ));
    }
}
