//! Receiver-local destination planning with explicit conflict decisions.

use crate::paths::{PathError, RelativePath, resolve_destination};
use quick_share_protocol::{EntryId, ManifestEntryKind, TransferOffer};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Explicit user decision for an existing destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictDecision {
    Overwrite,
    Skip,
    Rename,
}

/// Conflict class used to scope an "apply to all" choice safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    File,
    Directory,
}

/// Per-entry decisions plus optional same-kind fallbacks for subsequent conflicts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictSelections {
    per_entry: BTreeMap<EntryId, ConflictDecision>,
    apply_to_all_files: Option<ConflictDecision>,
    apply_to_all_directories: Option<ConflictDecision>,
}

impl ConflictSelections {
    #[must_use]
    pub fn with_entry(mut self, entry_id: EntryId, decision: ConflictDecision) -> Self {
        self.per_entry.insert(entry_id, decision);
        self
    }

    #[must_use]
    pub const fn apply_to_all(mut self, kind: ConflictKind, decision: ConflictDecision) -> Self {
        match kind {
            ConflictKind::File => self.apply_to_all_files = Some(decision),
            ConflictKind::Directory => self.apply_to_all_directories = Some(decision),
        }
        self
    }

    fn decision(&self, entry_id: EntryId, kind: ConflictKind) -> Option<ConflictDecision> {
        self.per_entry.get(&entry_id).copied().or(match kind {
            ConflictKind::File => self.apply_to_all_files,
            ConflictKind::Directory => self.apply_to_all_directories,
        })
    }
}

/// Final receiver-local action for one manifest entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DestinationDisposition {
    Commit {
        relative_path: RelativePath,
        replace_existing: bool,
    },
    Skip,
}

/// Complete, validated, transfer-scoped destination mapping.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationPlan {
    entries: BTreeMap<EntryId, DestinationDisposition>,
}

impl fmt::Debug for DestinationPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DestinationPlan")
            .field(
                "entries",
                &format_args!("[REDACTED; {} entry(s)]", self.entries.len()),
            )
            .finish()
    }
}

impl DestinationPlan {
    /// Builds a plan without silently resolving any existing destination.
    pub fn build(
        output_root: &Path,
        offer: &TransferOffer,
        selections: &ConflictSelections,
    ) -> Result<Self, DestinationPlanError> {
        offer
            .validate()
            .map_err(|error| DestinationPlanError::InvalidOffer(error.to_string()))?;
        let metadata = fs::metadata(output_root).map_err(DestinationPlanError::OutputRoot)?;
        if !metadata.is_dir() {
            return Err(DestinationPlanError::OutputRootNotDirectory(
                output_root.to_path_buf(),
            ));
        }

        let mut ordered = offer.entries.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|entry| entry.relative_path.matches('/').count());
        let reserved_paths = offer
            .entries
            .iter()
            .map(|entry| RelativePath::parse(&entry.relative_path).map(|path| path.collision_key()))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut entries = BTreeMap::new();
        let mut skipped_prefixes = Vec::<RelativePath>::new();
        let mut renamed_prefixes = Vec::<(RelativePath, RelativePath)>::new();

        for entry in ordered {
            let original = RelativePath::parse(&entry.relative_path)?;
            if skipped_prefixes
                .iter()
                .any(|prefix| has_path_prefix(&original, prefix))
            {
                entries.insert(entry.id, DestinationDisposition::Skip);
                continue;
            }
            let requested = rewrite_path(&original, &renamed_prefixes)?;
            let exists = match resolve_destination(
                output_root,
                &requested,
                crate::config::ConflictPolicy::Error,
            ) {
                Ok(Some(_)) => false,
                Ok(None) => false,
                Err(PathError::DestinationExists(_)) => true,
                Err(error) => return Err(error.into()),
            };
            if !exists {
                entries.insert(
                    entry.id,
                    DestinationDisposition::Commit {
                        relative_path: requested,
                        replace_existing: false,
                    },
                );
                continue;
            }

            let is_directory = matches!(entry.kind, ManifestEntryKind::Directory);
            let conflict_kind = if is_directory {
                ConflictKind::Directory
            } else {
                ConflictKind::File
            };
            let decision = selections
                .decision(entry.id, conflict_kind)
                .ok_or_else(|| DestinationPlanError::UnresolvedConflict {
                    entry_id: entry.id,
                    path: requested.clone(),
                })?;
            match decision {
                ConflictDecision::Skip => {
                    if is_directory {
                        skipped_prefixes.push(original);
                    }
                    entries.insert(entry.id, DestinationDisposition::Skip);
                }
                ConflictDecision::Overwrite => {
                    entries.insert(
                        entry.id,
                        DestinationDisposition::Commit {
                            relative_path: requested,
                            replace_existing: true,
                        },
                    );
                }
                ConflictDecision::Rename => {
                    let renamed = find_renamed_relative(
                        output_root,
                        &requested,
                        is_directory,
                        &reserved_paths,
                        &entries,
                    )?;
                    if is_directory {
                        renamed_prefixes.push((original, renamed.clone()));
                    }
                    entries.insert(
                        entry.id,
                        DestinationDisposition::Commit {
                            relative_path: renamed,
                            replace_existing: false,
                        },
                    );
                }
            }
        }

        let plan = Self { entries };
        plan.validate_for_offer(offer)?;
        Ok(plan)
    }

    #[must_use]
    pub fn entry(&self, entry_id: EntryId) -> Option<&DestinationDisposition> {
        self.entries.get(&entry_id)
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = (EntryId, &DestinationDisposition)> {
        self.entries
            .iter()
            .map(|(entry_id, value)| (*entry_id, value))
    }

    pub fn validate(&self) -> Result<(), DestinationPlanError> {
        let mut destinations = BTreeSet::new();
        for disposition in self.entries.values() {
            if let DestinationDisposition::Commit { relative_path, .. } = disposition
                && !destinations.insert(relative_path.collision_key())
            {
                return Err(DestinationPlanError::DuplicateDestination(
                    relative_path.clone(),
                ));
            }
        }
        Ok(())
    }

    pub fn validate_for_offer(&self, offer: &TransferOffer) -> Result<(), DestinationPlanError> {
        if self.entries.len() != offer.entries.len()
            || offer
                .entries
                .iter()
                .any(|entry| !self.entries.contains_key(&entry.id))
        {
            return Err(DestinationPlanError::EntrySetMismatch);
        }
        self.validate()
    }
}

fn has_path_prefix(path: &RelativePath, prefix: &RelativePath) -> bool {
    path == prefix
        || path
            .as_str()
            .strip_prefix(prefix.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn rewrite_path(
    original: &RelativePath,
    renamed: &[(RelativePath, RelativePath)],
) -> Result<RelativePath, PathError> {
    let mapping = renamed
        .iter()
        .filter(|(source, _)| has_path_prefix(original, source))
        .max_by_key(|(source, _)| source.as_str().len());
    let Some((source, destination)) = mapping else {
        return Ok(original.clone());
    };
    let suffix = original
        .as_str()
        .strip_prefix(source.as_str())
        .unwrap_or_default();
    RelativePath::parse(format!("{}{suffix}", destination.as_str()))
}

fn find_renamed_relative(
    output_root: &Path,
    requested: &RelativePath,
    directory: bool,
    reserved_paths: &BTreeSet<String>,
    planned: &BTreeMap<EntryId, DestinationDisposition>,
) -> Result<RelativePath, DestinationPlanError> {
    let requested_path = PathBuf::from(requested.as_str());
    let parent = requested_path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = requested_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DestinationPlanError::InvalidDestination(requested.clone()))?;
    let (stem, extension) = if directory {
        (file_name, None)
    } else {
        match file_name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
            _ => (file_name, None),
        }
    };
    for index in 1..=10_000_u32 {
        let candidate_name = extension.map_or_else(
            || format!("{stem} ({index})"),
            |extension| format!("{stem} ({index}).{extension}"),
        );
        let candidate = if parent.as_os_str().is_empty() {
            PathBuf::from(candidate_name)
        } else {
            parent.join(candidate_name)
        };
        let candidate = RelativePath::parse(candidate.to_string_lossy().replace('\\', "/"))?;
        let candidate_key = candidate.collision_key();
        let collides_with_manifest = reserved_paths.contains(&candidate_key)
            || (directory
                && reserved_paths
                    .iter()
                    .any(|path| path.starts_with(&format!("{candidate_key}/"))));
        let collides_with_plan = planned.values().any(|disposition| {
            matches!(
                disposition,
                DestinationDisposition::Commit { relative_path, .. }
                    if relative_path.collision_key() == candidate_key
            )
        });
        if collides_with_manifest || collides_with_plan {
            continue;
        }
        match resolve_destination(
            output_root,
            &candidate,
            crate::config::ConflictPolicy::Error,
        ) {
            Ok(Some(_)) => return Ok(candidate),
            Ok(None) | Err(PathError::DestinationExists(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(DestinationPlanError::RenameExhausted(requested.clone()))
}

#[derive(Debug, Error)]
pub enum DestinationPlanError {
    #[error("transfer offer is invalid: {0}")]
    InvalidOffer(String),
    #[error("cannot inspect output root: {0}")]
    OutputRoot(#[source] std::io::Error),
    #[error("output root is not a directory: {0}")]
    OutputRootNotDirectory(PathBuf),
    #[error("destination conflict for entry {entry_id:?}: {path}")]
    UnresolvedConflict {
        entry_id: EntryId,
        path: RelativePath,
    },
    #[error("destination plan does not contain exactly the offer entry IDs")]
    EntrySetMismatch,
    #[error("destination plan contains a portable path collision: {0}")]
    DuplicateDestination(RelativePath),
    #[error("destination cannot be renamed safely: {0}")]
    InvalidDestination(RelativePath),
    #[error("no bounded rename candidate is available for {0}")]
    RenameExhausted(RelativePath),
    #[error(transparent)]
    Path(#[from] PathError),
}
