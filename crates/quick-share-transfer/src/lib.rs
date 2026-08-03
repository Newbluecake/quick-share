#![forbid(unsafe_code)]
//! Crash-consistent local staging and verified atomic file commit.

pub mod auth;
pub mod direct;
pub mod expected_offer;
pub mod network;
pub mod noise;
pub mod offer;
pub mod receiver;
pub mod receiver_router;
pub mod selection;
pub mod sender;
pub mod text;

use cap_std::{ambient_authority, fs::Dir};
use quick_share_core::{
    config::ConflictPolicy,
    destination::{DestinationDisposition, DestinationPlan},
    paths::{PathError, RelativePath, resolve_destination},
};
use quick_share_platform::{FileSensitivity, StorageError, atomic_write};
use quick_share_protocol::{
    ChunkDescriptor, DeviceId, EntryId, MAX_CHUNK_SIZE, MAX_MANIFEST_ENTRIES, MIN_CHUNK_SIZE,
    TransferId, TransferStatus,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

const STORE_VERSION: u8 = 1;
const STAGING_DIRECTORY: &str = ".quick-share-staging";
const MAX_TOTAL_CHUNKS: u64 = 1_000_000;
const MAX_STATE_BYTES: u64 = 128 * 1024 * 1024;
const CHUNK_LOG_NAME: &str = "chunks.log";
const CHUNK_RECORD_BYTES: u64 = 40;
const MAX_CHUNK_LOG_BYTES: u64 = MAX_TOTAL_CHUNKS * CHUNK_RECORD_BYTES;
const SMALL_CHUNK_DURABILITY_BYTES: u32 = 64 * 1024;
const SMALL_CHUNK_SYNC_BATCH: u32 = 256;

/// One regular file accepted into receiver staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFile {
    pub entry_id: EntryId,
    pub relative_path: RelativePath,
    pub size: u64,
    pub chunk_size: u32,
    pub final_digest: [u8; 32],
}

impl StagedFile {
    pub fn new(
        entry_id: EntryId,
        relative_path: RelativePath,
        size: u64,
        chunk_size: u32,
        final_digest: [u8; 32],
    ) -> Result<Self, StoreError> {
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
            return Err(StoreError::InvalidManifest(
                "chunk size is outside QSP limits".to_owned(),
            ));
        }
        let chunk_count = size.div_ceil(u64::from(chunk_size));
        if chunk_count > MAX_TOTAL_CHUNKS {
            return Err(StoreError::InvalidManifest(
                "file requires too many chunks".to_owned(),
            ));
        }
        usize::try_from(chunk_count).map_err(|_| {
            StoreError::InvalidManifest("chunk count exceeds this platform".to_owned())
        })?;
        Ok(Self {
            entry_id,
            relative_path,
            size,
            chunk_size,
            final_digest,
        })
    }

    fn chunk_count(&self) -> Result<usize, StoreError> {
        usize::try_from(self.size.div_ceil(u64::from(self.chunk_size))).map_err(|_| {
            StoreError::InvalidManifest("chunk count exceeds this platform".to_owned())
        })
    }
}

/// Deterministic crash injection used by storage tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    None,
    StorageFullBeforeWrite,
    AfterDataSync,
    BeforeJournalReplace,
    AfterJournalReplace,
    AfterCommitIntent,
    AfterFileRename,
}

/// Result of a verified conflict-aware commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed(PathBuf),
    Skipped,
}

/// Durable metadata-entry commit state used for directory/link idempotency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataState {
    Pending,
    Committing { destination: RelativePath },
    Committed { destination: RelativePath },
    Skipped,
}

/// Persistent staging engine scoped to one transfer.
pub struct TransferStore {
    output_root: PathBuf,
    output_dir: Dir,
    staging_path: PathBuf,
    manifest: StoreManifest,
    journal: ResumeJournal,
    pending_small_chunk_records: u32,
}

/// Entry-scoped bounded offset writer backed by a `TransferStore` journal.
pub struct ChunkWriter<'a> {
    store: &'a mut TransferStore,
    entry_id: EntryId,
}

/// Result of beginning one replay-safe streaming chunk upload.
pub enum ChunkBegin {
    Upload(Box<ChunkUpload>),
    Duplicate,
}

/// Bounded sequential plaintext chunk sink. Journal state changes only in `finish`.
pub struct ChunkUpload {
    descriptor: ChunkDescriptor,
    file: File,
    written: u32,
    hasher: blake3::Hasher,
}

impl fmt::Debug for ChunkUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChunkUpload")
            .field("transfer_id", &self.descriptor.transfer_id)
            .field("entry_id", &self.descriptor.entry_id)
            .field("index", &self.descriptor.index)
            .field("written", &self.written)
            .field("hasher", &"[REDACTED]")
            .finish()
    }
}

impl ChunkUpload {
    pub fn write_fragment(&mut self, fragment_offset: u32, bytes: &[u8]) -> Result<(), StoreError> {
        let length = u32::try_from(bytes.len())
            .map_err(|_| StoreError::InvalidChunk("fragment length overflowed".to_owned()))?;
        let end = self
            .written
            .checked_add(length)
            .ok_or_else(|| StoreError::InvalidChunk("fragment range overflowed".to_owned()))?;
        if bytes.is_empty() || fragment_offset != self.written || end > self.descriptor.length {
            return Err(StoreError::InvalidChunk(
                "chunk fragments must be non-empty, ordered, and bounded".to_owned(),
            ));
        }
        write_all_at(
            &self.file,
            self.descriptor.offset + u64::from(self.written),
            bytes,
        )?;
        self.hasher.update(bytes);
        self.written = end;
        Ok(())
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ChunkDescriptor {
        &self.descriptor
    }
}

impl ChunkWriter<'_> {
    pub fn write(
        &mut self,
        descriptor: &ChunkDescriptor,
        bytes: &[u8],
        fault: FaultPoint,
    ) -> Result<(), StoreError> {
        self.store
            .write_chunk(self.entry_id, descriptor, bytes, fault)
    }

    pub fn missing_chunks(&self) -> Result<Vec<u32>, StoreError> {
        self.store.missing_chunks(self.entry_id)
    }
}

impl TransferStore {
    /// Creates staging under the final output file system.
    pub fn create(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
        sender_device_id: DeviceId,
        files: Vec<StagedFile>,
    ) -> Result<Self, StoreError> {
        Self::create_inner(output_root, transfer_id, sender_device_id, files, None)
    }

    /// Creates staging cryptographically bound to the accepted offer snapshot.
    pub fn create_bound(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
        sender_device_id: DeviceId,
        files: Vec<StagedFile>,
        manifest_digest: [u8; 32],
    ) -> Result<Self, StoreError> {
        Self::create_inner(
            output_root,
            transfer_id,
            sender_device_id,
            files,
            Some(manifest_digest),
        )
    }

    fn create_inner(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
        sender_device_id: DeviceId,
        files: Vec<StagedFile>,
        manifest_digest: Option<[u8; 32]>,
    ) -> Result<Self, StoreError> {
        validate_files(&files)?;
        let output_root = output_root.as_ref().to_path_buf();
        fs::create_dir_all(&output_root)?;
        let staging_root = prepare_staging_root(&output_root)?;
        let staging_path = staging_root.join(transfer_id.as_uuid().to_string());
        for attempt in 0..2 {
            match fs::create_dir(&staging_path) {
                Ok(()) => break,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(StoreError::AlreadyExists(transfer_id));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if attempt == 1 {
                        return Err(StoreError::Io(error));
                    }
                    // An empty staging root may have been removed by a concurrent
                    // cleanup after we prepared it; recreate it and retry once.
                    prepare_staging_root(&output_root)?;
                }
                Err(error) => return Err(StoreError::Io(error)),
            }
        }
        let files_path = staging_path.join("files");
        fs::create_dir(&files_path)?;

        let manifest =
            StoreManifest::from_files(transfer_id, sender_device_id, &files, manifest_digest);
        let journal = ResumeJournal::from_files(transfer_id, &files)?;
        save_json(staging_path.join("manifest.json"), &manifest)?;
        for file in &files {
            let part_path = files_path.join(format!("{}.part", file.entry_id.get()));
            let part = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(part_path)?;
            part.set_len(file.size)?;
        }
        sync_directory(&files_path)?;
        save_json(staging_path.join("state.json"), &journal)?;
        let output_dir = Dir::open_ambient_dir(&output_root, ambient_authority())?;
        Ok(Self {
            output_root,
            output_dir,
            staging_path,
            manifest,
            journal,
            pending_small_chunk_records: 0,
        })
    }

    /// Reopens and reconciles an interrupted transfer.
    pub fn reopen(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
        expected_sender: &DeviceId,
    ) -> Result<Self, StoreError> {
        Self::reopen_inner(output_root, transfer_id, expected_sender, None)
    }

    /// Reopens only when sender and immutable offer digest both match persisted state.
    pub fn reopen_bound(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
        expected_sender: &DeviceId,
        expected_manifest_digest: [u8; 32],
    ) -> Result<Self, StoreError> {
        Self::reopen_inner(
            output_root,
            transfer_id,
            expected_sender,
            Some(expected_manifest_digest),
        )
    }

    fn reopen_inner(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
        expected_sender: &DeviceId,
        expected_manifest_digest: Option<[u8; 32]>,
    ) -> Result<Self, StoreError> {
        let output_root = output_root.as_ref().to_path_buf();
        let staging_path =
            staging_root_checked(&output_root)?.join(transfer_id.as_uuid().to_string());
        let manifest: StoreManifest = read_json(staging_path.join("manifest.json"))?;
        let journal: ResumeJournal = read_json(staging_path.join("state.json"))?;
        if manifest.version != STORE_VERSION
            || journal.version != STORE_VERSION
            || manifest.transfer_id != transfer_id
            || journal.transfer_id != transfer_id
            || &manifest.sender_device_id != expected_sender
            || expected_manifest_digest.is_some()
                && manifest.decoded_offer_digest()? != expected_manifest_digest
        {
            return Err(StoreError::InvalidJournal(
                "version, transfer ID, or sender identity mismatch".to_owned(),
            ));
        }
        let files = manifest.to_files()?;
        validate_files(&files)?;
        validate_journal(&files, &journal)?;
        let output_dir = Dir::open_ambient_dir(&output_root, ambient_authority())?;
        let mut store = Self {
            output_root,
            output_dir,
            staging_path,
            manifest,
            journal,
            pending_small_chunk_records: 0,
        };
        store.replay_chunk_log()?;
        store.reconcile_commit_intents()?;
        Ok(store)
    }

    /// Borrows an entry-scoped chunk writer.
    pub fn chunk_writer(&mut self, entry_id: EntryId) -> Result<ChunkWriter<'_>, StoreError> {
        self.state_index(entry_id)?;
        Ok(ChunkWriter {
            store: self,
            entry_id,
        })
    }

    /// Begins a chunk without buffering its complete plaintext in memory.
    pub fn begin_chunk(&mut self, descriptor: &ChunkDescriptor) -> Result<ChunkBegin, StoreError> {
        if descriptor.transfer_id != self.manifest.transfer_id {
            return Err(StoreError::InvalidChunk(
                "descriptor transfer ID mismatch".to_owned(),
            ));
        }
        let file = self.file(descriptor.entry_id)?.clone();
        descriptor
            .validate(file.chunk_size, file.size)
            .map_err(|error| StoreError::InvalidChunk(error.to_string()))?;
        let state_index = self.state_index(descriptor.entry_id)?;
        let chunk_index = descriptor.index as usize;
        if let Some(recorded) = &self.journal.entries[state_index].completed[chunk_index] {
            return if recorded == &hex::encode(descriptor.digest) {
                Ok(ChunkBegin::Duplicate)
            } else {
                Err(StoreError::ConflictingChunk {
                    entry_id: descriptor.entry_id,
                    index: descriptor.index,
                })
            };
        }
        let file = OpenOptions::new()
            .write(true)
            .open(self.part_path(descriptor.entry_id))?;
        Ok(ChunkBegin::Upload(Box::new(ChunkUpload {
            descriptor: descriptor.clone(),
            file,
            written: 0,
            hasher: blake3::Hasher::new(),
        })))
    }

    /// Syncs and records one completed streaming upload atomically in the resume journal.
    pub fn finish_chunk(&mut self, upload: ChunkUpload) -> Result<(), StoreError> {
        if upload.written != upload.descriptor.length {
            return Err(StoreError::InvalidChunk(
                "chunk upload ended before its declared length".to_owned(),
            ));
        }
        let actual = *upload.hasher.finalize().as_bytes();
        if actual != upload.descriptor.digest {
            return Err(StoreError::ChunkDigestMismatch {
                entry_id: upload.descriptor.entry_id,
                index: upload.descriptor.index,
            });
        }
        let durable_now = upload.descriptor.length > SMALL_CHUNK_DURABILITY_BYTES;
        if durable_now {
            upload.file.sync_data()?;
        }
        let state_index = self.state_index(upload.descriptor.entry_id)?;
        let chunk_index = upload.descriptor.index as usize;
        if let Some(recorded) = &self.journal.entries[state_index].completed[chunk_index] {
            return if recorded == &hex::encode(actual) {
                Ok(())
            } else {
                Err(StoreError::ConflictingChunk {
                    entry_id: upload.descriptor.entry_id,
                    index: upload.descriptor.index,
                })
            };
        }
        self.append_chunk_record(
            upload.descriptor.entry_id,
            upload.descriptor.index,
            actual,
            durable_now,
        )?;
        self.journal.entries[state_index].completed[chunk_index] = Some(hex::encode(actual));
        Ok(())
    }

    /// Writes and syncs one validated block before atomically recording completion.
    pub fn write_chunk(
        &mut self,
        entry_id: EntryId,
        descriptor: &ChunkDescriptor,
        bytes: &[u8],
        fault: FaultPoint,
    ) -> Result<(), StoreError> {
        if descriptor.transfer_id != self.manifest.transfer_id || descriptor.entry_id != entry_id {
            return Err(StoreError::InvalidChunk(
                "descriptor transfer or entry ID mismatch".to_owned(),
            ));
        }
        let file = self.file(entry_id)?.clone();
        if fault == FaultPoint::StorageFullBeforeWrite {
            return Err(StoreError::Io(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected storage-full condition",
            )));
        }
        descriptor
            .validate(file.chunk_size, file.size)
            .map_err(|error| StoreError::InvalidChunk(error.to_string()))?;
        if bytes.len() != descriptor.length as usize {
            return Err(StoreError::InvalidChunk(
                "payload length does not match descriptor".to_owned(),
            ));
        }
        let actual = *blake3::hash(bytes).as_bytes();
        if actual != descriptor.digest {
            return Err(StoreError::ChunkDigestMismatch {
                entry_id,
                index: descriptor.index,
            });
        }
        let state_index = self.state_index(entry_id)?;
        let chunk_index = descriptor.index as usize;
        if let Some(recorded) = &self.journal.entries[state_index].completed[chunk_index] {
            return if recorded == &hex::encode(actual) {
                Ok(())
            } else {
                Err(StoreError::ConflictingChunk {
                    entry_id,
                    index: descriptor.index,
                })
            };
        }

        let part = OpenOptions::new()
            .write(true)
            .open(self.part_path(entry_id))?;
        write_all_at(&part, descriptor.offset, bytes)?;
        let durable_now = descriptor.length > SMALL_CHUNK_DURABILITY_BYTES;
        if durable_now {
            part.sync_data()?;
        }
        if fault == FaultPoint::AfterDataSync {
            return Err(StoreError::InjectedFault(fault));
        }

        if fault == FaultPoint::BeforeJournalReplace {
            return Err(StoreError::InjectedFault(fault));
        }
        self.append_chunk_record(entry_id, descriptor.index, actual, durable_now)?;
        self.journal.entries[state_index].completed[chunk_index] = Some(hex::encode(actual));
        if fault == FaultPoint::AfterJournalReplace {
            return Err(StoreError::InjectedFault(fault));
        }
        Ok(())
    }

    pub fn chunk_count(&self, entry_id: EntryId) -> Result<u32, StoreError> {
        let file = self.file(entry_id)?;
        u32::try_from(file.size.div_ceil(u64::from(file.chunk_size)))
            .map_err(|_| StoreError::InvalidManifest("chunk count exceeds QSP/1".to_owned()))
    }

    pub fn payload_entries(&self) -> Vec<EntryId> {
        self.manifest
            .files
            .iter()
            .map(|file| file.entry_id)
            .collect()
    }

    /// Registers bounded non-payload entries before any metadata commit begins.
    pub fn register_metadata(&mut self, entries: &[EntryId]) -> Result<(), StoreError> {
        if entries.len() > MAX_MANIFEST_ENTRIES {
            return Err(StoreError::InvalidManifest(
                "metadata entry count exceeds QSP limits".to_owned(),
            ));
        }
        let requested: BTreeSet<_> = entries.iter().copied().collect();
        if requested.len() != entries.len()
            || self
                .manifest
                .files
                .iter()
                .any(|file| requested.contains(&file.entry_id))
        {
            return Err(StoreError::InvalidManifest(
                "duplicate or payload-overlapping metadata entry ID".to_owned(),
            ));
        }
        let existing: BTreeSet<_> = self
            .journal
            .metadata
            .iter()
            .map(|entry| entry.entry_id)
            .collect();
        if existing.is_empty() {
            self.journal.metadata = entries
                .iter()
                .map(|entry_id| MetadataJournal {
                    entry_id: *entry_id,
                    status: MetadataStatus::Pending,
                })
                .collect();
            self.save_journal()?;
        } else if existing != requested {
            return Err(StoreError::InvalidJournal(
                "metadata entries do not match the accepted offer".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn metadata_state(&self, entry_id: EntryId) -> Result<MetadataState, StoreError> {
        let status = &self
            .journal
            .metadata
            .iter()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(StoreError::UnknownEntry(entry_id))?
            .status;
        Ok(match status {
            MetadataStatus::Pending => MetadataState::Pending,
            MetadataStatus::Committing { destination } => MetadataState::Committing {
                destination: RelativePath::parse(destination)?,
            },
            MetadataStatus::Committed { destination } => MetadataState::Committed {
                destination: RelativePath::parse(destination)?,
            },
            MetadataStatus::Skipped => MetadataState::Skipped,
        })
    }

    pub fn begin_metadata_commit(
        &mut self,
        entry_id: EntryId,
        destination: &RelativePath,
    ) -> Result<(), StoreError> {
        let entry = self
            .journal
            .metadata
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(StoreError::UnknownEntry(entry_id))?;
        if !matches!(entry.status, MetadataStatus::Pending) {
            return Err(StoreError::InvalidJournal(
                "metadata entry is already resolved".to_owned(),
            ));
        }
        entry.status = MetadataStatus::Committing {
            destination: destination.as_str().to_owned(),
        };
        self.save_journal()
    }

    /// Persists multiple metadata intents in one bounded atomic checkpoint.
    pub fn begin_metadata_batch(
        &mut self,
        entries: &[(EntryId, RelativePath)],
    ) -> Result<(), StoreError> {
        if entries.len() > MAX_MANIFEST_ENTRIES
            || entries
                .iter()
                .map(|(id, _)| *id)
                .collect::<BTreeSet<_>>()
                .len()
                != entries.len()
        {
            return Err(StoreError::InvalidManifest(
                "metadata batch contains too many or duplicate entries".to_owned(),
            ));
        }
        for (entry_id, _) in entries {
            let entry = self
                .journal
                .metadata
                .iter()
                .find(|entry| entry.entry_id == *entry_id)
                .ok_or(StoreError::UnknownEntry(*entry_id))?;
            if !matches!(entry.status, MetadataStatus::Pending) {
                return Err(StoreError::InvalidJournal(
                    "metadata batch entry is already resolved".to_owned(),
                ));
            }
        }
        for (entry_id, destination) in entries {
            let entry = self
                .journal
                .metadata
                .iter_mut()
                .find(|entry| entry.entry_id == *entry_id)
                .ok_or(StoreError::UnknownEntry(*entry_id))?;
            entry.status = MetadataStatus::Committing {
                destination: destination.as_str().to_owned(),
            };
        }
        self.save_journal()
    }

    /// Completes multiple previously persisted metadata intents in one checkpoint.
    pub fn finish_metadata_batch(&mut self, entries: &[EntryId]) -> Result<(), StoreError> {
        if entries.len() > MAX_MANIFEST_ENTRIES
            || entries.iter().copied().collect::<BTreeSet<_>>().len() != entries.len()
        {
            return Err(StoreError::InvalidManifest(
                "metadata batch contains too many or duplicate entries".to_owned(),
            ));
        }
        for entry_id in entries {
            let entry = self
                .journal
                .metadata
                .iter()
                .find(|entry| entry.entry_id == *entry_id)
                .ok_or(StoreError::UnknownEntry(*entry_id))?;
            if !matches!(entry.status, MetadataStatus::Committing { .. }) {
                return Err(StoreError::InvalidJournal(
                    "metadata batch entry has no persisted intent".to_owned(),
                ));
            }
        }
        for entry_id in entries {
            let entry = self
                .journal
                .metadata
                .iter_mut()
                .find(|entry| entry.entry_id == *entry_id)
                .ok_or(StoreError::UnknownEntry(*entry_id))?;
            let destination = match &entry.status {
                MetadataStatus::Committing { destination } => destination.clone(),
                _ => {
                    return Err(StoreError::InvalidJournal(
                        "metadata batch entry changed after validation".to_owned(),
                    ));
                }
            };
            entry.status = MetadataStatus::Committed { destination };
        }
        self.save_journal()
    }

    pub fn finish_metadata_commit(
        &mut self,
        entry_id: EntryId,
        skipped: bool,
    ) -> Result<(), StoreError> {
        let entry = self
            .journal
            .metadata
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(StoreError::UnknownEntry(entry_id))?;
        entry.status = match (&entry.status, skipped) {
            (_, true) => MetadataStatus::Skipped,
            (MetadataStatus::Committing { destination }, false) => MetadataStatus::Committed {
                destination: destination.clone(),
            },
            _ => {
                return Err(StoreError::InvalidJournal(
                    "metadata commit has no persisted intent".to_owned(),
                ));
            }
        };
        self.save_journal()
    }

    #[must_use]
    pub const fn transfer_status(&self) -> TransferStatus {
        self.journal.status
    }

    pub fn set_transfer_status(&mut self, status: TransferStatus) -> Result<(), StoreError> {
        self.journal.status = status;
        self.save_journal()
    }

    pub fn missing_chunks(&self, entry_id: EntryId) -> Result<Vec<u32>, StoreError> {
        let state = &self.journal.entries[self.state_index(entry_id)?];
        Ok(state
            .completed
            .iter()
            .enumerate()
            .filter_map(|(index, digest)| {
                digest
                    .is_none()
                    .then(|| u32::try_from(index).ok())
                    .flatten()
            })
            .collect())
    }

    /// Verifies one complete staged entry and reads at most `maximum` bytes without exposing it.
    pub fn read_verified_bytes(
        &mut self,
        entry_id: EntryId,
        maximum: usize,
    ) -> Result<Vec<u8>, StoreError> {
        let file = self.file(entry_id)?.clone();
        if file.size > maximum as u64 {
            return Err(StoreError::PayloadTooLarge);
        }
        if !self.missing_chunks(entry_id)?.is_empty() {
            return Err(StoreError::Incomplete(entry_id));
        }
        let actual = hash_file(self.part_path(entry_id))?;
        if actual != file.decoded_digest()? {
            return Err(StoreError::FinalDigestMismatch { entry_id });
        }
        let bytes = fs::read(self.part_path(entry_id))?;
        if bytes.len() > maximum {
            return Err(StoreError::PayloadTooLarge);
        }
        let state_index = self.state_index(entry_id)?;
        self.journal.entries[state_index].status = EntryStatus::Verified;
        self.save_journal()?;
        Ok(bytes)
    }

    /// Verifies the full file, persists commit intent, then atomically moves it into place.
    pub fn commit_file(
        &mut self,
        entry_id: EntryId,
        conflict: ConflictPolicy,
        fault: FaultPoint,
    ) -> Result<CommitOutcome, StoreError> {
        let file = self.file(entry_id)?.clone();
        let state_index = self.state_index(entry_id)?;
        match self.journal.entries[state_index].status.clone() {
            EntryStatus::Committed { destination } => {
                return Ok(CommitOutcome::Committed(
                    RelativePath::parse(destination)?.resolve_under(&self.output_root),
                ));
            }
            EntryStatus::Skipped => return Ok(CommitOutcome::Skipped),
            EntryStatus::Committing { .. } if self.part_path(entry_id).exists() => {
                // Re-evaluate the conflict policy after an interrupted pre-rename intent.
                // This prevents a raced rename destination from becoming permanently stuck.
                self.journal.entries[state_index].status = EntryStatus::Verified;
                self.save_journal()?;
            }
            EntryStatus::Committing { .. } => {
                self.reconcile_commit_intents()?;
                return self.commit_file(entry_id, conflict, fault);
            }
            EntryStatus::Receiving | EntryStatus::Verified => {}
        }
        if !self.missing_chunks(entry_id)?.is_empty() {
            return Err(StoreError::Incomplete(entry_id));
        }
        let actual = hash_file(self.part_path(entry_id))?;
        if actual != file.decoded_digest()? {
            return Err(StoreError::FinalDigestMismatch { entry_id });
        }
        self.journal.entries[state_index].status = EntryStatus::Verified;
        self.save_journal()?;

        let destination =
            match resolve_destination(&self.output_root, &file.parsed_path()?, conflict)? {
                Some(destination) => destination,
                None => {
                    self.journal.entries[state_index].status = EntryStatus::Skipped;
                    self.save_journal()?;
                    return Ok(CommitOutcome::Skipped);
                }
            };
        let relative_destination = path_relative_to_root(&self.output_root, &destination)?;
        self.journal.entries[state_index].status = EntryStatus::Committing {
            destination: relative_destination.as_str().to_owned(),
        };
        self.save_journal()?;
        if fault == FaultPoint::AfterCommitIntent {
            return Err(StoreError::InjectedFault(fault));
        }
        self.finish_commit(
            entry_id,
            relative_destination.as_str().to_owned(),
            conflict,
            fault,
        )
    }

    /// Verifies and atomically commits a bounded set with one durable intent
    /// checkpoint and one final checkpoint. This avoids O(n²) journal rewrites for
    /// large small-file manifests while preserving restart reconciliation.
    pub fn commit_files_batch(
        &mut self,
        entry_ids: &[EntryId],
        conflict: ConflictPolicy,
    ) -> Result<Vec<CommitOutcome>, StoreError> {
        if entry_ids.len() > MAX_MANIFEST_ENTRIES
            || entry_ids.iter().copied().collect::<BTreeSet<_>>().len() != entry_ids.len()
        {
            return Err(StoreError::InvalidManifest(
                "batch commit contains too many or duplicate entries".to_owned(),
            ));
        }
        let mut plans = Vec::with_capacity(entry_ids.len());
        let mut outcomes = Vec::with_capacity(entry_ids.len());
        for &entry_id in entry_ids {
            let file = self.file(entry_id)?.clone();
            let state_index = self.state_index(entry_id)?;
            match self.journal.entries[state_index].status.clone() {
                EntryStatus::Committed { destination } => {
                    outcomes.push(Some(CommitOutcome::Committed(
                        RelativePath::parse(destination)?.resolve_under(&self.output_root),
                    )));
                    plans.push(None);
                    continue;
                }
                EntryStatus::Skipped => {
                    outcomes.push(Some(CommitOutcome::Skipped));
                    plans.push(None);
                    continue;
                }
                EntryStatus::Committing { .. } if self.part_path(entry_id).exists() => {
                    self.journal.entries[state_index].status = EntryStatus::Verified;
                }
                EntryStatus::Committing { .. } => {
                    self.reconcile_commit_intents()?;
                    return self.commit_files_batch(entry_ids, conflict);
                }
                EntryStatus::Receiving | EntryStatus::Verified => {}
            }
            if !self.missing_chunks(entry_id)?.is_empty() {
                return Err(StoreError::Incomplete(entry_id));
            }
            if hash_file(self.part_path(entry_id))? != file.decoded_digest()? {
                return Err(StoreError::FinalDigestMismatch { entry_id });
            }
            let destination =
                resolve_destination(&self.output_root, &file.parsed_path()?, conflict)?;
            match destination {
                Some(destination) => {
                    let relative = path_relative_to_root(&self.output_root, &destination)?;
                    self.journal.entries[state_index].status = EntryStatus::Committing {
                        destination: relative.as_str().to_owned(),
                    };
                    plans.push(Some((entry_id, relative)));
                    outcomes.push(None);
                }
                None => {
                    self.journal.entries[state_index].status = EntryStatus::Skipped;
                    plans.push(None);
                    outcomes.push(Some(CommitOutcome::Skipped));
                }
            }
        }
        self.save_journal()?;

        let mut parents = BTreeSet::new();
        for (index, plan) in plans.into_iter().enumerate() {
            let Some((entry_id, relative)) = plan else {
                continue;
            };
            let final_path = relative.resolve_under(&self.output_root);
            let destination_relative = PathBuf::from(relative.as_str());
            if let Some(parent) = destination_relative.parent() {
                self.output_dir.create_dir_all(parent)?;
                parents.insert(parent.to_path_buf());
            } else {
                parents.insert(PathBuf::new());
            }
            let source_relative = self.part_relative_path(entry_id);
            let replace = conflict == ConflictPolicy::Overwrite && final_path.exists();
            if replace {
                self.output_dir.rename(
                    &source_relative,
                    &self.output_dir,
                    &destination_relative,
                )?;
            } else {
                self.output_dir
                    .hard_link(&source_relative, &self.output_dir, &destination_relative)
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::AlreadyExists {
                            StoreError::Path(PathError::DestinationExists(final_path.clone()))
                        } else {
                            StoreError::Io(error)
                        }
                    })?;
                self.output_dir.remove_file(&source_relative)?;
            }
            let state_index = self.state_index(entry_id)?;
            self.journal.entries[state_index].status = EntryStatus::Committed {
                destination: relative.as_str().to_owned(),
            };
            outcomes[index] = Some(CommitOutcome::Committed(final_path));
        }
        for parent in parents {
            sync_parent_path(
                &self.output_root,
                (!parent.as_os_str().is_empty()).then_some(parent.as_path()),
            )?;
        }
        self.save_journal()?;
        outcomes
            .into_iter()
            .map(|outcome| {
                outcome.ok_or_else(|| {
                    StoreError::InvalidJournal("batch commit produced no outcome".to_owned())
                })
            })
            .collect()
    }

    /// Commits payload files to the exact validated destinations in a persisted plan.
    /// No-clobber entries report `ConflictPending` if a destination appears after planning.
    pub fn commit_files_planned(
        &mut self,
        entry_ids: &[EntryId],
        plan: &DestinationPlan,
        fault: FaultPoint,
    ) -> Result<Vec<CommitOutcome>, StoreError> {
        plan.validate()
            .map_err(|error| StoreError::InvalidManifest(error.to_string()))?;
        if entry_ids.len() > MAX_MANIFEST_ENTRIES
            || entry_ids.iter().copied().collect::<BTreeSet<_>>().len() != entry_ids.len()
        {
            return Err(StoreError::InvalidManifest(
                "planned batch contains too many or duplicate entries".to_owned(),
            ));
        }
        let entry_ids = entry_ids.to_vec();
        let mut commits = Vec::with_capacity(entry_ids.len());
        let mut outcomes = Vec::with_capacity(entry_ids.len());

        for &entry_id in &entry_ids {
            let disposition = plan.entry(entry_id).ok_or_else(|| {
                StoreError::InvalidManifest(
                    "destination plan is missing a payload entry".to_owned(),
                )
            })?;
            let file = self.file(entry_id)?.clone();
            let state_index = self.state_index(entry_id)?;
            match self.journal.entries[state_index].status.clone() {
                EntryStatus::Committed { destination } => {
                    if !matches!(
                        disposition,
                        DestinationDisposition::Commit { relative_path, .. }
                            if relative_path.as_str() == destination
                    ) {
                        return Err(StoreError::InvalidJournal(
                            "destination plan changed an already committed entry".to_owned(),
                        ));
                    }
                    outcomes.push(Some(CommitOutcome::Committed(
                        RelativePath::parse(destination)?.resolve_under(&self.output_root),
                    )));
                    commits.push(None);
                    continue;
                }
                EntryStatus::Skipped => {
                    if !matches!(disposition, DestinationDisposition::Skip) {
                        return Err(StoreError::InvalidJournal(
                            "destination plan changed an already skipped entry".to_owned(),
                        ));
                    }
                    outcomes.push(Some(CommitOutcome::Skipped));
                    commits.push(None);
                    continue;
                }
                EntryStatus::Committing { .. } if self.part_path(entry_id).exists() => {
                    self.journal.entries[state_index].status = EntryStatus::Verified;
                }
                EntryStatus::Committing { .. } => {
                    self.reconcile_commit_intents()?;
                    return self.commit_files_planned(&entry_ids, plan, fault);
                }
                EntryStatus::Receiving | EntryStatus::Verified => {}
            }
            if !self.missing_chunks(entry_id)?.is_empty() {
                return Err(StoreError::Incomplete(entry_id));
            }
            if hash_file(self.part_path(entry_id))? != file.decoded_digest()? {
                return Err(StoreError::FinalDigestMismatch { entry_id });
            }

            match disposition {
                DestinationDisposition::Skip => {
                    self.journal.entries[state_index].status = EntryStatus::Skipped;
                    commits.push(None);
                    outcomes.push(Some(CommitOutcome::Skipped));
                }
                DestinationDisposition::Commit {
                    relative_path,
                    replace_existing,
                } => {
                    match resolve_destination(
                        &self.output_root,
                        relative_path,
                        ConflictPolicy::Error,
                    ) {
                        Ok(_) => {}
                        Err(PathError::DestinationExists(_)) if *replace_existing => {}
                        Err(PathError::DestinationExists(_)) => {
                            return Err(StoreError::ConflictPending { entry_id });
                        }
                        Err(error) => return Err(error.into()),
                    }
                    self.journal.entries[state_index].status = EntryStatus::Committing {
                        destination: relative_path.as_str().to_owned(),
                    };
                    commits.push(Some((entry_id, relative_path.clone(), *replace_existing)));
                    outcomes.push(None);
                }
            }
        }
        self.save_journal()?;
        if fault == FaultPoint::AfterCommitIntent {
            return Err(StoreError::InjectedFault(fault));
        }

        let mut parents = BTreeSet::new();
        let mut renamed = false;
        for (index, commit) in commits.into_iter().enumerate() {
            let Some((entry_id, relative, replace_existing)) = commit else {
                continue;
            };
            let final_path = relative.resolve_under(&self.output_root);
            let destination_relative = PathBuf::from(relative.as_str());
            if let Some(parent) = destination_relative.parent() {
                self.output_dir.create_dir_all(parent)?;
                parents.insert(parent.to_path_buf());
            } else {
                parents.insert(PathBuf::new());
            }
            let source_relative = self.part_relative_path(entry_id);
            if replace_existing {
                self.output_dir.rename(
                    &source_relative,
                    &self.output_dir,
                    &destination_relative,
                )?;
            } else {
                self.output_dir
                    .hard_link(&source_relative, &self.output_dir, &destination_relative)
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::AlreadyExists {
                            StoreError::ConflictPending { entry_id }
                        } else {
                            StoreError::Io(error)
                        }
                    })?;
                self.output_dir.remove_file(&source_relative)?;
            }
            renamed = true;
            let state_index = self.state_index(entry_id)?;
            self.journal.entries[state_index].status = EntryStatus::Committed {
                destination: relative.as_str().to_owned(),
            };
            outcomes[index] = Some(CommitOutcome::Committed(final_path));
            if fault == FaultPoint::AfterFileRename {
                return Err(StoreError::InjectedFault(fault));
            }
        }
        for parent in parents {
            sync_parent_path(
                &self.output_root,
                (!parent.as_os_str().is_empty()).then_some(parent.as_path()),
            )?;
        }
        if fault == FaultPoint::BeforeJournalReplace && renamed {
            return Err(StoreError::InjectedFault(fault));
        }
        self.save_journal()?;
        if fault == FaultPoint::AfterJournalReplace && renamed {
            return Err(StoreError::InjectedFault(fault));
        }
        outcomes
            .into_iter()
            .map(|outcome| {
                outcome.ok_or_else(|| {
                    StoreError::InvalidJournal("planned commit produced no outcome".to_owned())
                })
            })
            .collect()
    }

    fn finish_commit(
        &mut self,
        entry_id: EntryId,
        destination: String,
        conflict: ConflictPolicy,
        fault: FaultPoint,
    ) -> Result<CommitOutcome, StoreError> {
        let relative = RelativePath::parse(&destination)?;
        let final_path = relative.resolve_under(&self.output_root);
        let destination_relative = PathBuf::from(relative.as_str());
        if let Some(parent) = destination_relative.parent() {
            self.output_dir.create_dir_all(parent)?;
        }
        let source_relative = self.part_relative_path(entry_id);
        let replace = conflict == ConflictPolicy::Overwrite && final_path.exists();
        if replace {
            self.output_dir
                .rename(&source_relative, &self.output_dir, &destination_relative)?;
        } else {
            self.output_dir
                .hard_link(&source_relative, &self.output_dir, &destination_relative)
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        StoreError::Path(PathError::DestinationExists(final_path.clone()))
                    } else {
                        StoreError::Io(error)
                    }
                })?;
            self.output_dir.remove_file(&source_relative)?;
        }
        sync_parent_path(&self.output_root, destination_relative.parent())?;
        if fault == FaultPoint::AfterFileRename {
            return Err(StoreError::InjectedFault(fault));
        }
        let state_index = self.state_index(entry_id)?;
        self.journal.entries[state_index].status = EntryStatus::Committed { destination };
        self.save_journal()?;
        Ok(CommitOutcome::Committed(final_path))
    }

    fn reconcile_commit_intents(&mut self) -> Result<(), StoreError> {
        let mut changed = false;
        for index in 0..self.journal.entries.len() {
            let entry_id = self.journal.entries[index].entry_id;
            match self.journal.entries[index].status.clone() {
                EntryStatus::Committing { destination } => {
                    let final_path =
                        RelativePath::parse(&destination)?.resolve_under(&self.output_root);
                    let part_exists = self.part_path(entry_id).exists();
                    if final_path.is_file() {
                        if hash_file(&final_path)? != self.file(entry_id)?.decoded_digest()? {
                            return Err(StoreError::InconsistentCommit(entry_id));
                        }
                        if part_exists {
                            fs::remove_file(self.part_path(entry_id))?;
                        }
                        self.journal.entries[index].status = EntryStatus::Committed { destination };
                        changed = true;
                    } else if !part_exists {
                        return Err(StoreError::InconsistentCommit(entry_id));
                    }
                }
                EntryStatus::Committed { destination } => {
                    let final_path =
                        RelativePath::parse(destination)?.resolve_under(&self.output_root);
                    if !final_path.is_file()
                        || hash_file(final_path)? != self.file(entry_id)?.decoded_digest()?
                    {
                        return Err(StoreError::InconsistentCommit(entry_id));
                    }
                }
                EntryStatus::Receiving | EntryStatus::Verified | EntryStatus::Skipped => {}
            }
        }
        if changed {
            self.save_journal()?;
        }
        Ok(())
    }

    pub fn is_committed(&self, entry_id: EntryId) -> Result<bool, StoreError> {
        Ok(matches!(
            self.journal.entries[self.state_index(entry_id)?].status,
            EntryStatus::Committed { .. }
        ))
    }

    /// Staging path, always nested under the configured output root.
    #[must_use]
    pub fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    /// Offer digest bound into persisted state for restart-safe source matching.
    pub fn offer_digest(&self) -> Result<Option<[u8; 32]>, StoreError> {
        self.manifest.decoded_offer_digest()
    }

    /// Sender identity bound into the persisted resume manifest.
    #[must_use]
    pub fn sender_device_id(&self) -> &DeviceId {
        &self.manifest.sender_device_id
    }

    pub fn part_metadata(&self, entry_id: EntryId) -> Result<fs::Metadata, StoreError> {
        Ok(fs::metadata(self.part_path(entry_id))?)
    }

    /// Lists recoverable transfer IDs without creating or deleting any state.
    pub fn list_staging(output_root: impl AsRef<Path>) -> Result<Vec<TransferId>, StoreError> {
        let mut transfers = Vec::new();
        let root = match staging_root_checked(output_root.as_ref()) {
            Ok(root) => root,
            Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(transfers);
            }
            Err(error) => return Err(error),
        };
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(value) = entry.file_name().to_str()
                && let Ok(uuid) = value.parse()
            {
                transfers.push(TransferId::new(uuid));
            }
        }
        transfers.sort_by_key(|id| id.as_uuid());
        Ok(transfers)
    }

    /// Explicitly discards resumable staging after local/authorized cancellation.
    /// The now-empty staging root is removed as well.
    pub fn discard(self) -> Result<(), StoreError> {
        fs::remove_dir_all(self.staging_path)?;
        remove_empty_staging_root(&self.output_root);
        Ok(())
    }

    /// Explicit local cleanup for a listed inactive staging transfer. Removes the
    /// staging root too once no transfer subdirectory remains.
    pub fn discard_staging(
        output_root: impl AsRef<Path>,
        transfer_id: TransferId,
    ) -> Result<(), StoreError> {
        let staging =
            staging_root_checked(output_root.as_ref())?.join(transfer_id.as_uuid().to_string());
        let metadata = fs::symlink_metadata(&staging)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::UnsafeStagingRoot(staging));
        }
        fs::remove_dir_all(staging)?;
        remove_empty_staging_root(output_root.as_ref());
        Ok(())
    }

    /// Removes staging only after every entry is committed or explicitly skipped,
    /// including the staging root itself once no transfer subdirectory remains.
    pub fn cleanup_if_complete(self) -> Result<bool, StoreError> {
        let complete = self.journal.entries.iter().all(|entry| {
            matches!(
                entry.status,
                EntryStatus::Committed { .. } | EntryStatus::Skipped
            )
        }) && self.journal.metadata.iter().all(|entry| {
            matches!(
                entry.status,
                MetadataStatus::Committed { .. } | MetadataStatus::Skipped
            )
        });
        if complete {
            fs::remove_dir_all(self.staging_path)?;
            remove_empty_staging_root(&self.output_root);
        }
        Ok(complete)
    }

    fn file(&self, entry_id: EntryId) -> Result<&StagedFileRecord, StoreError> {
        self.manifest
            .files
            .iter()
            .find(|file| file.entry_id == entry_id)
            .ok_or(StoreError::UnknownEntry(entry_id))
    }

    fn state_index(&self, entry_id: EntryId) -> Result<usize, StoreError> {
        self.journal
            .entries
            .iter()
            .position(|entry| entry.entry_id == entry_id)
            .ok_or(StoreError::UnknownEntry(entry_id))
    }

    fn part_path(&self, entry_id: EntryId) -> PathBuf {
        self.staging_path
            .join("files")
            .join(format!("{}.part", entry_id.get()))
    }

    fn part_relative_path(&self, entry_id: EntryId) -> PathBuf {
        PathBuf::from(STAGING_DIRECTORY)
            .join(self.manifest.transfer_id.as_uuid().to_string())
            .join("files")
            .join(format!("{}.part", entry_id.get()))
    }

    fn append_chunk_record(
        &mut self,
        entry_id: EntryId,
        index: u32,
        digest: [u8; 32],
        durable_now: bool,
    ) -> Result<(), StoreError> {
        let path = self.staging_path.join(CHUNK_LOG_NAME);
        let length = fs::metadata(&path).map_or(0, |metadata| metadata.len());
        if !length.is_multiple_of(CHUNK_RECORD_BYTES)
            || length.saturating_add(CHUNK_RECORD_BYTES) > MAX_CHUNK_LOG_BYTES
        {
            return Err(StoreError::InvalidJournal(
                "chunk completion log exceeds its fixed record bound".to_owned(),
            ));
        }
        let mut record = [0_u8; CHUNK_RECORD_BYTES as usize];
        record[..4].copy_from_slice(&entry_id.get().to_be_bytes());
        record[4..8].copy_from_slice(&index.to_be_bytes());
        record[8..].copy_from_slice(&digest);
        let mut log = OpenOptions::new().create(true).append(true).open(path)?;
        log.write_all(&record)?;
        if durable_now {
            log.sync_data()?;
            self.pending_small_chunk_records = 0;
        } else {
            self.pending_small_chunk_records = self.pending_small_chunk_records.saturating_add(1);
            if self.pending_small_chunk_records >= SMALL_CHUNK_SYNC_BATCH {
                log.sync_data()?;
                self.pending_small_chunk_records = 0;
            }
        }
        Ok(())
    }

    fn replay_chunk_log(&mut self) -> Result<(), StoreError> {
        let path = self.staging_path.join(CHUNK_LOG_NAME);
        let mut bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(StoreError::Io(error)),
        };
        if bytes.len() as u64 > MAX_CHUNK_LOG_BYTES {
            return Err(StoreError::InvalidJournal(
                "chunk completion log exceeds its byte bound".to_owned(),
            ));
        }
        let complete_length =
            bytes.len() / CHUNK_RECORD_BYTES as usize * CHUNK_RECORD_BYTES as usize;
        if complete_length != bytes.len() {
            bytes.truncate(complete_length);
            let log = OpenOptions::new().write(true).open(&path)?;
            log.set_len(complete_length as u64)?;
            log.sync_data()?;
        }
        for record in bytes.chunks_exact(CHUNK_RECORD_BYTES as usize) {
            let entry_raw = u32::from_be_bytes(record[..4].try_into().expect("fixed record"));
            let entry_id = EntryId::new(entry_raw).ok_or_else(|| {
                StoreError::InvalidJournal("chunk log contains a zero entry ID".to_owned())
            })?;
            let index = u32::from_be_bytes(record[4..8].try_into().expect("fixed record"));
            let digest: [u8; 32] = record[8..].try_into().expect("fixed record");
            let state_index = self.state_index(entry_id)?;
            let slot = self.journal.entries[state_index]
                .completed
                .get_mut(index as usize)
                .ok_or_else(|| {
                    StoreError::InvalidJournal(
                        "chunk log index exceeds the accepted manifest".to_owned(),
                    )
                })?;
            let encoded = hex::encode(digest);
            match slot {
                Some(existing) if existing != &encoded => {
                    return Err(StoreError::ConflictingChunk { entry_id, index });
                }
                Some(_) => {}
                None => *slot = Some(encoded),
            }
        }
        Ok(())
    }

    fn save_journal(&self) -> Result<(), StoreError> {
        save_json(self.staging_path.join("state.json"), &self.journal)
    }
}

fn validate_files(files: &[StagedFile]) -> Result<(), StoreError> {
    if files.len() > MAX_MANIFEST_ENTRIES {
        return Err(StoreError::InvalidManifest(
            "file count exceeds QSP limits".to_owned(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut total_chunks = 0_u64;
    for file in files {
        total_chunks = total_chunks
            .checked_add(file.size.div_ceil(u64::from(file.chunk_size)))
            .ok_or_else(|| {
                StoreError::InvalidManifest("total chunk count overflowed".to_owned())
            })?;
        if total_chunks > MAX_TOTAL_CHUNKS {
            return Err(StoreError::InvalidManifest(
                "transfer requires too many chunks".to_owned(),
            ));
        }
        file.chunk_count()?;
        if !ids.insert(file.entry_id) || !paths.insert(file.relative_path.collision_key()) {
            return Err(StoreError::InvalidManifest(
                "duplicate entry ID or destination path".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_journal(files: &[StagedFile], journal: &ResumeJournal) -> Result<(), StoreError> {
    if files.len() != journal.entries.len() {
        return Err(StoreError::InvalidJournal(
            "entry count mismatch".to_owned(),
        ));
    }
    if journal.metadata.len() > MAX_MANIFEST_ENTRIES {
        return Err(StoreError::InvalidJournal(
            "metadata entry count exceeds QSP limits".to_owned(),
        ));
    }
    let mut metadata_ids = BTreeSet::new();
    for metadata in &journal.metadata {
        if !metadata_ids.insert(metadata.entry_id) {
            return Err(StoreError::InvalidJournal(
                "duplicate metadata entry state".to_owned(),
            ));
        }
        match &metadata.status {
            MetadataStatus::Committing { destination }
            | MetadataStatus::Committed { destination } => {
                RelativePath::parse(destination)?;
            }
            MetadataStatus::Pending | MetadataStatus::Skipped => {}
        }
    }
    for file in files {
        let state = journal
            .entries
            .iter()
            .find(|entry| entry.entry_id == file.entry_id)
            .ok_or_else(|| StoreError::InvalidJournal("missing entry state".to_owned()))?;
        if state.completed.len() != file.chunk_count()? {
            return Err(StoreError::InvalidJournal(
                "chunk count mismatch".to_owned(),
            ));
        }
        if state.completed.iter().flatten().any(|digest| {
            digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(StoreError::InvalidJournal(
                "invalid recorded chunk digest".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StoreError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent_path(output_root: &Path, parent: Option<&Path>) -> Result<(), StoreError> {
    let path = parent.map_or_else(
        || output_root.to_path_buf(),
        |parent| output_root.join(parent),
    );
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_path(_output_root: &Path, _parent: Option<&Path>) -> Result<(), StoreError> {
    Ok(())
}

fn prepare_staging_root(output_root: &Path) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(output_root)?;
    let root = output_root.join(STAGING_DIRECTORY);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(StoreError::UnsafeStagingRoot(root));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&root)?,
        Err(error) => return Err(StoreError::Io(error)),
    }
    Ok(root)
}

/// Resolves and validates the staging root without creating it.
fn staging_root_checked(output_root: &Path) -> Result<PathBuf, StoreError> {
    let root = output_root.join(STAGING_DIRECTORY);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(StoreError::UnsafeStagingRoot(root))
        }
        Ok(_) => Ok(root),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(StoreError::Io(error)),
        Err(error) => Err(StoreError::Io(error)),
    }
}

/// Best-effort removal of the staging root once no transfer subdirectory remains.
/// Only ever removes an empty directory; the root is lazily recreated by
/// `prepare_staging_root` on the next accepted transfer, so this is always safe.
fn remove_empty_staging_root(output_root: &Path) {
    let _ = fs::remove_dir(output_root.join(STAGING_DIRECTORY));
}

fn path_relative_to_root(root: &Path, destination: &Path) -> Result<RelativePath, StoreError> {
    let relative = destination
        .strip_prefix(root)
        .map_err(|_| StoreError::InvalidManifest("destination escaped output root".to_owned()))?;
    RelativePath::parse(relative.to_string_lossy().replace('\\', "/")).map_err(StoreError::Path)
}

fn save_json(path: PathBuf, value: &impl Serialize) -> Result<(), StoreError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| StoreError::InvalidJournal(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(StoreError::InvalidJournal(
            "serialized state exceeds the byte limit".to_owned(),
        ));
    }
    atomic_write(path, &bytes, FileSensitivity::Normal)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: PathBuf) -> Result<T, StoreError> {
    if fs::metadata(&path)?.len() > MAX_STATE_BYTES {
        return Err(StoreError::InvalidJournal(
            "persisted state exceeds the byte limit".to_owned(),
        ));
    }
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| StoreError::InvalidJournal(error.to_string()))
}

/// Streams a file through a fixed-size buffer and returns BLAKE3.
pub fn hash_file(path: impl AsRef<Path>) -> Result<[u8; 32], StoreError> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(*hasher.finalize().as_bytes())
}

#[cfg(unix)]
fn write_all_at(file: &File, mut offset: u64, mut bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    while !bytes.is_empty() {
        let count = file.write_at(bytes, offset)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "offset write returned zero",
            ));
        }
        offset += count as u64;
        bytes = &bytes[count..];
    }
    Ok(())
}

#[cfg(windows)]
fn write_all_at(file: &File, mut offset: u64, mut bytes: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !bytes.is_empty() {
        let count = file.seek_write(bytes, offset)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "offset write returned zero",
            ));
        }
        offset += count as u64;
        bytes = &bytes[count..];
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn write_all_at(file: &File, offset: u64, bytes: &[u8]) -> io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = file;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoreManifest {
    version: u8,
    transfer_id: TransferId,
    sender_device_id: DeviceId,
    #[serde(default)]
    offer_digest: Option<String>,
    files: Vec<StagedFileRecord>,
}

impl StoreManifest {
    fn from_files(
        transfer_id: TransferId,
        sender_device_id: DeviceId,
        files: &[StagedFile],
        offer_digest: Option<[u8; 32]>,
    ) -> Self {
        Self {
            version: STORE_VERSION,
            transfer_id,
            sender_device_id,
            offer_digest: offer_digest.map(hex::encode),
            files: files
                .iter()
                .map(|file| StagedFileRecord {
                    entry_id: file.entry_id,
                    relative_path: file.relative_path.as_str().to_owned(),
                    size: file.size,
                    chunk_size: file.chunk_size,
                    final_digest: hex::encode(file.final_digest),
                })
                .collect(),
        }
    }

    fn decoded_offer_digest(&self) -> Result<Option<[u8; 32]>, StoreError> {
        self.offer_digest
            .as_deref()
            .map(|digest| {
                hex::decode(digest)
                    .map_err(|error| StoreError::InvalidManifest(error.to_string()))?
                    .try_into()
                    .map_err(|_| {
                        StoreError::InvalidManifest("offer digest must contain 32 bytes".to_owned())
                    })
            })
            .transpose()
    }

    fn to_files(&self) -> Result<Vec<StagedFile>, StoreError> {
        self.files
            .iter()
            .map(|file| {
                let digest: [u8; 32] = hex::decode(&file.final_digest)
                    .map_err(|error| StoreError::InvalidManifest(error.to_string()))?
                    .try_into()
                    .map_err(|_| {
                        StoreError::InvalidManifest("final digest must contain 32 bytes".to_owned())
                    })?;
                StagedFile::new(
                    file.entry_id,
                    RelativePath::parse(&file.relative_path)?,
                    file.size,
                    file.chunk_size,
                    digest,
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedFileRecord {
    entry_id: EntryId,
    relative_path: String,
    size: u64,
    chunk_size: u32,
    final_digest: String,
}

impl StagedFileRecord {
    fn parsed_path(&self) -> Result<RelativePath, StoreError> {
        RelativePath::parse(&self.relative_path).map_err(StoreError::Path)
    }

    fn decoded_digest(&self) -> Result<[u8; 32], StoreError> {
        hex::decode(&self.final_digest)
            .map_err(|error| StoreError::InvalidManifest(error.to_string()))?
            .try_into()
            .map_err(|_| {
                StoreError::InvalidManifest("final digest must contain 32 bytes".to_owned())
            })
    }
}

const fn default_transfer_status() -> TransferStatus {
    TransferStatus::Accepted
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResumeJournal {
    version: u8,
    transfer_id: TransferId,
    #[serde(default = "default_transfer_status")]
    status: TransferStatus,
    #[serde(default)]
    metadata: Vec<MetadataJournal>,
    entries: Vec<EntryJournal>,
}

impl ResumeJournal {
    fn from_files(transfer_id: TransferId, files: &[StagedFile]) -> Result<Self, StoreError> {
        Ok(Self {
            version: STORE_VERSION,
            transfer_id,
            status: TransferStatus::Accepted,
            metadata: Vec::new(),
            entries: files
                .iter()
                .map(|file| {
                    Ok(EntryJournal {
                        entry_id: file.entry_id,
                        completed: vec![None; file.chunk_count()?],
                        status: EntryStatus::Receiving,
                    })
                })
                .collect::<Result<_, StoreError>>()?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataJournal {
    entry_id: EntryId,
    status: MetadataStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum MetadataStatus {
    Pending,
    Committing { destination: String },
    Committed { destination: String },
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EntryJournal {
    entry_id: EntryId,
    completed: Vec<Option<String>>,
    status: EntryStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum EntryStatus {
    Receiving,
    Verified,
    Committing { destination: String },
    Committed { destination: String },
    Skipped,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("invalid transfer manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid resume journal: {0}")]
    InvalidJournal(String),
    #[error("staging already exists for transfer {0:?}")]
    AlreadyExists(TransferId),
    #[error("unsafe staging root: {0}")]
    UnsafeStagingRoot(PathBuf),
    #[error("unknown manifest entry {0:?}")]
    UnknownEntry(EntryId),
    #[error("invalid chunk: {0}")]
    InvalidChunk(String),
    #[error("chunk {index} digest mismatch for entry {entry_id:?}")]
    ChunkDigestMismatch { entry_id: EntryId, index: u32 },
    #[error("chunk {index} conflicts with recorded digest for entry {entry_id:?}")]
    ConflictingChunk { entry_id: EntryId, index: u32 },
    #[error("entry {0:?} is incomplete")]
    Incomplete(EntryId),
    #[error("destination changed after planning for entry {entry_id:?}")]
    ConflictPending { entry_id: EntryId },
    #[error("verified payload exceeds the in-memory delivery bound")]
    PayloadTooLarge,
    #[error("final digest mismatch for entry {entry_id:?}")]
    FinalDigestMismatch { entry_id: EntryId },
    #[error("commit state is inconsistent for entry {0:?}")]
    InconsistentCommit(EntryId),
    #[error("injected storage fault at {0:?}")]
    InjectedFault(FaultPoint),
}
