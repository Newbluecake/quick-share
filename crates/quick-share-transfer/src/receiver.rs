//! Authorized, resumable receiver state machine over already-authenticated Noise frames.

use crate::{
    ChunkBegin, ChunkUpload, CommitOutcome, MetadataState, StagedFile, StoreError, TransferStore,
    offer::{
        AuthorizationPermission, AuthorizationToken, AuthorizedOffer, OfferError, OfferManager,
    },
    text::{MAX_TEXT_BYTES, TextPayload, TextSource},
};
use cap_std::{ambient_authority, fs::Dir};
use quick_share_core::{
    config::ConflictPolicy,
    destination::{DestinationDisposition, DestinationPlan, DestinationPlanError},
    manifest::SymlinkDisposition,
    paths::{PathError, RelativePath, resolve_destination},
};
use quick_share_protocol::{
    AuthorizationProof, ChunkAck, ChunkData, DeviceId, EntryTransferStatus, ManifestEntryKind,
    MissingChunkBitmap, RequestId, TransferCancel, TransferComplete, TransferCompleteAck,
    TransferId, TransferStatus, TransferStatusRequest, TransferStatusResponse,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MAX_RECEIVER_TASKS: usize = 16;
const MAX_RECEIVER_FILE_STREAMS: usize = 64;

#[derive(Debug, Clone)]
pub struct ReceiverPolicy {
    pub conflict: ConflictPolicy,
    pub max_receive_tasks: usize,
    pub max_file_streams: usize,
}

/// Bounded receiver-side progress signal; no payload bytes or local absolute paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverProgressEvent {
    pub transfer_id: TransferId,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub status: TransferStatus,
}

impl ReceiverPolicy {
    pub(crate) fn validate(&self) -> Result<(), ReceiverError> {
        if self.max_receive_tasks == 0
            || self.max_receive_tasks > MAX_RECEIVER_TASKS
            || self.max_file_streams == 0
            || self.max_file_streams > MAX_RECEIVER_FILE_STREAMS
        {
            return Err(ReceiverError::InvalidLimits);
        }
        Ok(())
    }
}

impl Default for ReceiverPolicy {
    fn default() -> Self {
        Self {
            conflict: ConflictPolicy::Rename,
            max_receive_tasks: 1,
            max_file_streams: 4,
        }
    }
}

/// Direct-server receiver boundary implemented by fixed-root and routed receivers.
pub trait ReceiverEndpoint: Send + Sync {
    fn status(
        &self,
        peer: &DeviceId,
        request: TransferStatusRequest,
        now: Instant,
    ) -> Result<TransferStatusResponse, ReceiverError>;

    fn receive_chunk(
        &self,
        peer: &DeviceId,
        request_id: RequestId,
        frame: ChunkData,
        now: Instant,
    ) -> Result<Option<ChunkAck>, ReceiverError>;

    fn complete(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
    ) -> Result<TransferCompleteAck, ReceiverError>;

    fn cancel(
        &self,
        peer: &DeviceId,
        request: TransferCancel,
        now: Instant,
    ) -> Result<(), ReceiverError>;

    fn disconnect(&self, peer: &DeviceId) -> Result<usize, ReceiverError>;
}

/// Synchronous disk endpoint intended to be called by a bounded network/blocking worker.
pub struct ReceiverService {
    offers: Arc<OfferManager>,
    output_root: PathBuf,
    output_dir: Dir,
    policy: ReceiverPolicy,
    progress: Option<mpsc::Sender<ReceiverProgressEvent>>,
    state: Mutex<ReceiverState>,
}

impl std::fmt::Debug for ReceiverService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReceiverService")
            .field("output_root", &"[REDACTED]")
            .field("policy", &self.policy)
            .field("state", &"[REDACTED]")
            .finish()
    }
}

impl ReceiverService {
    pub fn new(
        offers: Arc<OfferManager>,
        output_root: impl AsRef<Path>,
        policy: ReceiverPolicy,
    ) -> Result<Self, ReceiverError> {
        Self::new_with_progress(offers, output_root, policy, None)
    }

    pub fn new_with_progress(
        offers: Arc<OfferManager>,
        output_root: impl AsRef<Path>,
        policy: ReceiverPolicy,
        progress: Option<mpsc::Sender<ReceiverProgressEvent>>,
    ) -> Result<Self, ReceiverError> {
        policy.validate()?;
        fs::create_dir_all(output_root.as_ref())?;
        let output_root = output_root.as_ref().to_path_buf();
        let output_dir = Dir::open_ambient_dir(&output_root, ambient_authority())?;
        Ok(Self {
            offers,
            output_root,
            output_dir,
            policy,
            progress,
            state: Mutex::new(ReceiverState::default()),
        })
    }

    /// Returns compact missing-chunk state only to the authorized transfer owner.
    pub fn status(
        &self,
        peer: &DeviceId,
        request: TransferStatusRequest,
        now: Instant,
    ) -> Result<TransferStatusResponse, ReceiverError> {
        let authorized = self.authorize(
            &request.authorization,
            peer,
            request.transfer_id,
            AuthorizationPermission::TransferStatus,
            now,
        )?;
        let mut state = self.lock_state()?;
        let transfer = self.ensure_transfer(&mut state, peer, authorized)?;
        let entries = transfer
            .store
            .payload_entries()
            .into_iter()
            .map(|entry_id| {
                let chunk_count = transfer.store.chunk_count(entry_id)?;
                let missing = transfer.store.missing_chunks(entry_id)?;
                Ok(EntryTransferStatus {
                    entry_id,
                    chunks: MissingChunkBitmap::from_missing(chunk_count, missing)
                        .map_err(|error| ReceiverError::InvalidFrame(error.to_string()))?,
                })
            })
            .collect::<Result<Vec<_>, ReceiverError>>()?;
        self.emit_progress(transfer);
        let response = TransferStatusResponse {
            transfer_id: request.transfer_id,
            status: transfer.store.transfer_status(),
            manifest_digest: transfer.manifest_digest,
            entries,
        };
        response
            .validate()
            .map_err(|error| ReceiverError::InvalidFrame(error.to_string()))?;
        Ok(response)
    }

    /// Streams one bounded `CHUNK_DATA` fragment into staging and ACKs only after journal sync.
    pub fn receive_chunk(
        &self,
        peer: &DeviceId,
        request_id: RequestId,
        frame: ChunkData,
        now: Instant,
    ) -> Result<Option<ChunkAck>, ReceiverError> {
        frame
            .validate()
            .map_err(|error| ReceiverError::InvalidFrame(error.to_string()))?;
        let transfer_id = frame.descriptor.transfer_id;
        let authorized = self.authorize(
            &frame.authorization,
            peer,
            transfer_id,
            AuthorizationPermission::ChunkUpload,
            now,
        )?;
        let mut state = self.lock_state()?;
        let current_status = self
            .ensure_transfer(&mut state, peer, authorized)?
            .store
            .transfer_status();
        if state
            .transfers
            .get(&transfer_id)
            .is_some_and(|transfer| transfer.cancellation.is_cancelled())
            || matches!(
                current_status,
                TransferStatus::Cancelled | TransferStatus::Completed | TransferStatus::Failed
            )
        {
            return Err(ReceiverError::Terminal(current_status));
        }

        let key = (frame.descriptor.entry_id, frame.descriptor.index);
        let needs_begin = !state
            .transfers
            .get(&transfer_id)
            .ok_or(ReceiverError::NotFound)?
            .uploads
            .contains_key(&request_id);
        if needs_begin {
            if frame.fragment_offset != 0 {
                return Err(ReceiverError::FragmentOrder);
            }
            let active_streams = state
                .transfers
                .values()
                .map(|transfer| transfer.uploads.len())
                .sum::<usize>();
            if active_streams >= self.policy.max_file_streams {
                return Err(ReceiverError::FileStreamLimit);
            }
            let transfer = state
                .transfers
                .get_mut(&transfer_id)
                .ok_or(ReceiverError::NotFound)?;
            if transfer.upload_keys.contains(&key) {
                return Err(ReceiverError::ChunkAlreadyActive);
            }
            match transfer.store.begin_chunk(&frame.descriptor)? {
                ChunkBegin::Duplicate => {
                    return Ok(Some(ChunkAck {
                        transfer_id,
                        entry_id: frame.descriptor.entry_id,
                        index: frame.descriptor.index,
                        accepted_length: frame.descriptor.length,
                        duplicate: true,
                    }));
                }
                ChunkBegin::Upload(upload) => {
                    if transfer.store.transfer_status() != TransferStatus::Transferring {
                        transfer
                            .store
                            .set_transfer_status(TransferStatus::Transferring)?;
                    }
                    transfer.upload_keys.insert(key);
                    transfer.uploads.insert(request_id, *upload);
                }
            }
        }

        let transfer = state
            .transfers
            .get_mut(&transfer_id)
            .ok_or(ReceiverError::NotFound)?;
        let upload = transfer
            .uploads
            .get_mut(&request_id)
            .ok_or(ReceiverError::FragmentOrder)?;
        if upload.descriptor() != &frame.descriptor {
            transfer.uploads.remove(&request_id);
            transfer.upload_keys.remove(&key);
            return Err(ReceiverError::FragmentDescriptorChanged);
        }
        if let Err(error) = upload.write_fragment(frame.fragment_offset, &frame.payload) {
            transfer.uploads.remove(&request_id);
            transfer.upload_keys.remove(&key);
            return Err(error.into());
        }
        if !frame.final_fragment {
            return Ok(None);
        }
        let upload = transfer
            .uploads
            .remove(&request_id)
            .ok_or(ReceiverError::FragmentOrder)?;
        transfer.upload_keys.remove(&key);
        let accepted_length = upload.descriptor().length;
        transfer.store.finish_chunk(upload)?;
        transfer.received_bytes = transfer
            .received_bytes
            .saturating_add(u64::from(accepted_length))
            .min(transfer.offer.total_bytes);
        self.emit_progress(transfer);
        Ok(Some(ChunkAck {
            transfer_id,
            entry_id: frame.descriptor.entry_id,
            index: frame.descriptor.index,
            accepted_length: frame.descriptor.length,
            duplicate: false,
        }))
    }

    /// Verifies every payload digest and commits files plus metadata entries under a root capability.
    pub fn complete(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
    ) -> Result<TransferCompleteAck, ReceiverError> {
        self.complete_inner(peer, request, now, None)
    }

    /// Completes against an immutable receiver-local destination plan.
    pub fn complete_with_plan(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
        plan: &DestinationPlan,
    ) -> Result<TransferCompleteAck, ReceiverError> {
        plan.validate()?;
        self.complete_inner(peer, request, now, Some(plan))
    }

    fn complete_inner(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
        plan: Option<&DestinationPlan>,
    ) -> Result<TransferCompleteAck, ReceiverError> {
        let authorized = self.authorize(
            &request.authorization,
            peer,
            request.transfer_id,
            AuthorizationPermission::Complete,
            now,
        )?;
        if authorized.manifest_digest != request.manifest_digest {
            return Err(ReceiverError::ManifestChanged);
        }
        let mut state = self.lock_state()?;
        let transfer = self.ensure_transfer(&mut state, peer, authorized)?;
        if let Some(plan) = plan {
            plan.validate_for_offer(&transfer.offer)?;
        }
        if transfer.store.transfer_status() == TransferStatus::Completed {
            if transfer.completed_text.is_none() {
                transfer.completed_text = load_text_payload(&mut transfer.store, &transfer.offer)?;
            }
            return Ok(TransferCompleteAck {
                transfer_id: request.transfer_id,
                status: TransferStatus::Completed,
            });
        }
        if matches!(
            transfer.store.transfer_status(),
            TransferStatus::Cancelled | TransferStatus::Failed
        ) {
            return Err(ReceiverError::Terminal(transfer.store.transfer_status()));
        }
        if !transfer.uploads.is_empty() {
            return Err(ReceiverError::UploadsActive);
        }
        for entry_id in transfer.store.payload_entries() {
            if !transfer.store.missing_chunks(entry_id)?.is_empty() {
                transfer.store.set_transfer_status(TransferStatus::Paused)?;
                return Err(ReceiverError::Incomplete);
            }
        }
        transfer
            .store
            .set_transfer_status(TransferStatus::Verifying)?;

        let result = (|| -> Result<Option<TextPayload>, ReceiverError> {
            let mut directories = transfer
                .offer
                .entries
                .iter()
                .filter(|entry| matches!(entry.kind, ManifestEntryKind::Directory))
                .map(|entry| Ok((entry.id, RelativePath::parse(&entry.relative_path)?)))
                .collect::<Result<Vec<_>, ReceiverError>>()?;
            directories.sort_by_key(|(_, path)| path.as_str().matches('/').count());
            if let Some(plan) = plan {
                self.commit_directories_planned(&mut transfer.store, &directories, plan)?;
            } else {
                self.commit_directories_batch(&mut transfer.store, &directories)?;
            }
            let mut completed_text = None;
            let mut file_entries = Vec::new();
            for entry in &transfer.offer.entries {
                match entry.kind {
                    ManifestEntryKind::File => file_entries.push(entry.id),
                    ManifestEntryKind::Text { .. } => {
                        completed_text = load_text_payload(&mut transfer.store, &transfer.offer)?;
                    }
                    ManifestEntryKind::Directory | ManifestEntryKind::Symlink { .. } => {}
                }
            }
            let _outcomes: Vec<CommitOutcome> = if let Some(plan) = plan {
                transfer
                    .store
                    .commit_files_planned(&file_entries, plan, crate::FaultPoint::None)
                    .map_err(map_planned_store_error)?
            } else {
                transfer
                    .store
                    .commit_files_batch(&file_entries, self.policy.conflict)?
            };
            for entry in &transfer.offer.entries {
                if let ManifestEntryKind::Symlink { target } = &entry.kind {
                    let requested = RelativePath::parse(&entry.relative_path)?;
                    if let Some(plan) = plan {
                        self.commit_symlink_planned(
                            &mut transfer.store,
                            entry.id,
                            plan,
                            &requested,
                            target,
                        )?;
                    } else {
                        self.commit_symlink(&mut transfer.store, entry.id, &requested, target)?;
                    }
                }
            }
            Ok(completed_text)
        })();
        let completed_text = match result {
            Ok(payload) => payload,
            Err(error) => {
                let status = if matches!(
                    error,
                    ReceiverError::Store(StoreError::FinalDigestMismatch { .. })
                ) {
                    TransferStatus::Failed
                } else {
                    TransferStatus::Paused
                };
                transfer.store.set_transfer_status(status)?;
                return Err(error);
            }
        };
        transfer.completed_text = completed_text;
        transfer
            .store
            .set_transfer_status(TransferStatus::Completed)?;
        self.emit_progress(transfer);
        Ok(TransferCompleteAck {
            transfer_id: request.transfer_id,
            status: TransferStatus::Completed,
        })
    }

    /// Cancels without exposing a final partial file. Staging remains explicitly cleanable.
    pub fn cancel(
        &self,
        peer: &DeviceId,
        request: TransferCancel,
        now: Instant,
    ) -> Result<(), ReceiverError> {
        let authorized = self.authorize(
            &request.authorization,
            peer,
            request.transfer_id,
            AuthorizationPermission::Cancel,
            now,
        )?;
        let mut state = self.lock_state()?;
        let transfer = self.ensure_transfer(&mut state, peer, authorized)?;
        if transfer.store.transfer_status() == TransferStatus::Cancelled {
            return Ok(());
        }
        if matches!(
            transfer.store.transfer_status(),
            TransferStatus::Completed | TransferStatus::Failed
        ) {
            return Err(ReceiverError::Terminal(transfer.store.transfer_status()));
        }
        transfer.cancellation.cancel();
        transfer.uploads.clear();
        transfer.upload_keys.clear();
        transfer
            .store
            .set_transfer_status(TransferStatus::Cancelled)?;
        self.emit_progress(transfer);
        Ok(())
    }

    /// Marks interrupted streams resumable and releases all per-connection file slots.
    pub fn disconnect(&self, peer: &DeviceId) -> Result<usize, ReceiverError> {
        let mut state = self.lock_state()?;
        let mut count = 0;
        for transfer in state.transfers.values_mut() {
            if &transfer.owner == peer
                && transfer.store.transfer_status() == TransferStatus::Transferring
            {
                count += transfer.uploads.len();
                transfer.uploads.clear();
                transfer.upload_keys.clear();
                transfer.store.set_transfer_status(TransferStatus::Paused)?;
                self.emit_progress(transfer);
            }
        }
        Ok(count)
    }

    pub fn completed_text(
        &self,
        peer: &DeviceId,
        transfer_id: TransferId,
    ) -> Result<Option<TextPayload>, ReceiverError> {
        let mut state = self.lock_state()?;
        let transfer = state
            .transfers
            .get_mut(&transfer_id)
            .ok_or(ReceiverError::NotFound)?;
        if &transfer.owner != peer || transfer.store.transfer_status() != TransferStatus::Completed
        {
            return Err(ReceiverError::NotFound);
        }
        if transfer.completed_text.is_none() {
            transfer.completed_text = load_text_payload(&mut transfer.store, &transfer.offer)?;
        }
        Ok(transfer.completed_text.clone())
    }

    /// Persists every active transfer as paused during graceful process shutdown.
    pub fn pause_all(&self) -> Result<usize, ReceiverError> {
        let mut state = self.lock_state()?;
        let mut paused = 0;
        for transfer in state.transfers.values_mut() {
            if matches!(
                transfer.store.transfer_status(),
                TransferStatus::Accepted | TransferStatus::Transferring | TransferStatus::Verifying
            ) {
                transfer.uploads.clear();
                transfer.upload_keys.clear();
                transfer.store.set_transfer_status(TransferStatus::Paused)?;
                self.emit_progress(transfer);
                paused += 1;
            }
        }
        Ok(paused)
    }

    pub fn list_resumable(&self) -> Result<Vec<TransferId>, ReceiverError> {
        Ok(TransferStore::list_staging(&self.output_root)?)
    }

    /// Reports whether a transfer reserves a receiver task; an uninitialized service
    /// is considered active so concurrent router creation cannot bypass the limit.
    pub fn is_active(&self, transfer_id: TransferId) -> bool {
        self.state.lock().map_or(true, |state| {
            state.transfers.get(&transfer_id).is_none_or(|transfer| {
                matches!(
                    transfer.store.transfer_status(),
                    TransferStatus::Accepted
                        | TransferStatus::Transferring
                        | TransferStatus::Paused
                        | TransferStatus::Verifying
                )
            })
        })
    }

    /// Local administrative cleanup; never called automatically after a network error.
    pub fn cleanup(&self, transfer_id: TransferId) -> Result<(), ReceiverError> {
        let mut state = self.lock_state()?;
        if let Some(transfer) = state.transfers.remove(&transfer_id) {
            transfer.store.discard()?;
        } else {
            TransferStore::discard_staging(&self.output_root, transfer_id)?;
        }
        Ok(())
    }

    fn ensure_transfer<'a>(
        &self,
        state: &'a mut ReceiverState,
        peer: &DeviceId,
        authorized: AuthorizedOffer,
    ) -> Result<&'a mut ActiveTransfer, ReceiverError> {
        let transfer_id = authorized.offer.transfer_id;
        if let Some(existing) = state.transfers.get(&transfer_id) {
            if &existing.owner != peer || existing.manifest_digest != authorized.manifest_digest {
                return Err(ReceiverError::ManifestChanged);
            }
        } else {
            let active_tasks = state
                .transfers
                .values()
                .filter(|transfer| {
                    matches!(
                        transfer.store.transfer_status(),
                        TransferStatus::Accepted
                            | TransferStatus::Transferring
                            | TransferStatus::Paused
                            | TransferStatus::Verifying
                    )
                })
                .count();
            if active_tasks >= self.policy.max_receive_tasks {
                return Err(ReceiverError::ReceiveTaskLimit);
            }
            let files = staged_files(&authorized)?;
            let mut store = match TransferStore::create_bound(
                &self.output_root,
                transfer_id,
                peer.clone(),
                files,
                authorized.manifest_digest,
            ) {
                Ok(store) => store,
                Err(StoreError::AlreadyExists(_)) => TransferStore::reopen_bound(
                    &self.output_root,
                    transfer_id,
                    peer,
                    authorized.manifest_digest,
                )?,
                Err(error) => return Err(error.into()),
            };
            if matches!(
                store.transfer_status(),
                TransferStatus::Transferring | TransferStatus::Verifying
            ) {
                store.set_transfer_status(TransferStatus::Paused)?;
            }
            let metadata_entries = authorized
                .offer
                .entries
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.kind,
                        ManifestEntryKind::Directory | ManifestEntryKind::Symlink { .. }
                    )
                })
                .map(|entry| entry.id)
                .collect::<Vec<_>>();
            store.register_metadata(&metadata_entries)?;
            let received_bytes = received_bytes(&store, &authorized.offer)?;
            let cancellation = CancellationToken::new();
            let last_reported_status = store.transfer_status();
            if last_reported_status == TransferStatus::Cancelled {
                cancellation.cancel();
            }
            state.transfers.insert(
                transfer_id,
                ActiveTransfer {
                    owner: peer.clone(),
                    offer: authorized.offer,
                    manifest_digest: authorized.manifest_digest,
                    store,
                    uploads: BTreeMap::new(),
                    upload_keys: BTreeSet::new(),
                    received_bytes,
                    last_reported_bytes: received_bytes,
                    last_reported_status,
                    cancellation,
                    completed_text: None,
                },
            );
        }
        state
            .transfers
            .get_mut(&transfer_id)
            .ok_or(ReceiverError::NotFound)
    }

    fn authorize(
        &self,
        proof: &AuthorizationProof,
        peer: &DeviceId,
        transfer_id: TransferId,
        permission: AuthorizationPermission,
        now: Instant,
    ) -> Result<AuthorizedOffer, ReceiverError> {
        let token = proof.with_bytes(|bytes| AuthorizationToken::from_bytes(*bytes));
        Ok(self
            .offers
            .authorize(&token, peer, transfer_id, permission, now)?)
    }

    fn commit_directories_planned(
        &self,
        store: &mut TransferStore,
        directories: &[(quick_share_protocol::EntryId, RelativePath)],
        plan: &DestinationPlan,
    ) -> Result<(), ReceiverError> {
        let mut intents = Vec::new();
        let mut commits = Vec::new();
        for (entry_id, _) in directories {
            let disposition = plan
                .entry(*entry_id)
                .ok_or(ReceiverError::ManifestChanged)?;
            match store.metadata_state(*entry_id)? {
                MetadataState::Committed { destination } => {
                    if !matches!(
                        disposition,
                        DestinationDisposition::Commit { relative_path, .. }
                            if relative_path == &destination
                    ) {
                        return Err(ReceiverError::ManifestChanged);
                    }
                    continue;
                }
                MetadataState::Skipped => {
                    if !matches!(disposition, DestinationDisposition::Skip) {
                        return Err(ReceiverError::ManifestChanged);
                    }
                    continue;
                }
                MetadataState::Committing { destination } => {
                    let DestinationDisposition::Commit {
                        relative_path,
                        replace_existing,
                    } = disposition
                    else {
                        return Err(ReceiverError::ManifestChanged);
                    };
                    if &destination != relative_path {
                        return Err(ReceiverError::ManifestChanged);
                    }
                    commits.push((*entry_id, destination, *replace_existing));
                }
                MetadataState::Pending => match disposition {
                    DestinationDisposition::Skip => {
                        store.finish_metadata_commit(*entry_id, true)?;
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
                            Err(PathError::DestinationExists(path)) => {
                                let metadata = fs::symlink_metadata(&path)?;
                                if metadata.file_type().is_symlink() {
                                    return Err(PathError::SymlinkAncestor(path).into());
                                }
                                if !replace_existing {
                                    return Err(ReceiverError::ConflictPending {
                                        entry_id: *entry_id,
                                    });
                                }
                            }
                            Err(error) => return Err(error.into()),
                        }
                        intents.push((*entry_id, relative_path.clone()));
                        commits.push((*entry_id, relative_path.clone(), *replace_existing));
                    }
                },
            }
        }
        if !intents.is_empty() {
            store.begin_metadata_batch(&intents)?;
        }

        let mut completed = Vec::with_capacity(commits.len());
        for (entry_id, relative, replace_existing) in commits {
            let absolute = relative.resolve_under(&self.output_root);
            match fs::symlink_metadata(&absolute) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PathError::SymlinkAncestor(absolute).into());
                }
                Ok(metadata) if metadata.is_dir() && replace_existing => {}
                Ok(metadata) if !metadata.is_dir() && replace_existing => {
                    self.output_dir.remove_file(relative.as_str())?;
                    self.output_dir.create_dir_all(relative.as_str())?;
                }
                Ok(_) => return Err(ReceiverError::ConflictPending { entry_id }),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let create = if replace_existing {
                        self.output_dir.create_dir_all(relative.as_str())
                    } else {
                        let destination = PathBuf::from(relative.as_str());
                        if let Some(parent) = destination.parent() {
                            self.output_dir.create_dir_all(parent)?;
                        }
                        self.output_dir.create_dir(&destination)
                    };
                    match create {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            return Err(ReceiverError::ConflictPending { entry_id });
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
            completed.push(entry_id);
        }
        if !completed.is_empty() {
            store.finish_metadata_batch(&completed)?;
        }
        Ok(())
    }

    fn commit_directories_batch(
        &self,
        store: &mut TransferStore,
        directories: &[(quick_share_protocol::EntryId, RelativePath)],
    ) -> Result<(), ReceiverError> {
        let mut intents = Vec::new();
        let mut plans = Vec::new();
        for (entry_id, requested) in directories {
            let relative = match store.metadata_state(*entry_id)? {
                MetadataState::Committed { .. } | MetadataState::Skipped => continue,
                MetadataState::Committing { destination } => destination,
                MetadataState::Pending => {
                    let destination = requested.resolve_under(&self.output_root);
                    match resolve_destination(&self.output_root, requested, ConflictPolicy::Error) {
                        Ok(_) => {}
                        Err(PathError::DestinationExists(_)) if destination.is_dir() => {
                            if fs::symlink_metadata(&destination)?.file_type().is_symlink() {
                                return Err(PathError::SymlinkAncestor(destination).into());
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                    intents.push((*entry_id, requested.clone()));
                    requested.clone()
                }
            };
            plans.push((*entry_id, relative));
        }
        if !intents.is_empty() {
            store.begin_metadata_batch(&intents)?;
        }
        let mut completed = Vec::with_capacity(plans.len());
        for (entry_id, relative) in plans {
            let destination = relative.resolve_under(&self.output_root);
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PathError::SymlinkAncestor(destination).into());
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(PathError::NonDirectoryAncestor(destination).into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    self.output_dir.create_dir_all(relative.as_str())?;
                }
                Err(error) => return Err(error.into()),
            }
            completed.push(entry_id);
        }
        if !completed.is_empty() {
            store.finish_metadata_batch(&completed)?;
        }
        Ok(())
    }

    fn commit_symlink_planned(
        &self,
        store: &mut TransferStore,
        entry_id: quick_share_protocol::EntryId,
        plan: &DestinationPlan,
        requested: &RelativePath,
        target: &str,
    ) -> Result<(), ReceiverError> {
        let disposition = plan.entry(entry_id).ok_or(ReceiverError::ManifestChanged)?;
        let metadata_state = store.metadata_state(entry_id)?;
        if matches!(disposition, DestinationDisposition::Skip) {
            return match metadata_state {
                MetadataState::Pending => {
                    store.finish_metadata_commit(entry_id, true)?;
                    Ok(())
                }
                MetadataState::Skipped => Ok(()),
                MetadataState::Committing { .. } | MetadataState::Committed { .. } => {
                    Err(ReceiverError::ManifestChanged)
                }
            };
        }
        let DestinationDisposition::Commit {
            relative_path,
            replace_existing,
        } = disposition
        else {
            return Err(ReceiverError::ManifestChanged);
        };
        let relative = match metadata_state {
            MetadataState::Committed { destination } => {
                if relative_path != &destination {
                    return Err(ReceiverError::ManifestChanged);
                }
                return Ok(());
            }
            MetadataState::Skipped => return Err(ReceiverError::ManifestChanged),
            MetadataState::Committing { destination } => {
                if &destination != relative_path {
                    return Err(ReceiverError::ManifestChanged);
                }
                destination
            }
            MetadataState::Pending => {
                match resolve_destination(&self.output_root, relative_path, ConflictPolicy::Error) {
                    Ok(_) => {}
                    Err(PathError::DestinationExists(_)) if *replace_existing => {}
                    Err(PathError::DestinationExists(_)) => {
                        return Err(ReceiverError::ConflictPending { entry_id });
                    }
                    Err(error) => return Err(error.into()),
                }
                store.begin_metadata_commit(entry_id, relative_path)?;
                relative_path.clone()
            }
        };
        let destination = PathBuf::from(relative.as_str());
        let absolute = relative.resolve_under(&self.output_root);
        if let Ok(metadata) = fs::symlink_metadata(&absolute) {
            if metadata.file_type().is_symlink()
                && link_target_matches(&fs::read_link(&absolute)?, target)
            {
                store.finish_metadata_commit(entry_id, false)?;
                return Ok(());
            }
            if !replace_existing || metadata.is_dir() {
                return Err(ReceiverError::ConflictPending { entry_id });
            }
            self.output_dir.remove_file(&destination)?;
        }
        if let Some(parent) = destination.parent() {
            self.output_dir.create_dir_all(parent)?;
        }
        match SymlinkDisposition::classify(requested, target, true) {
            SymlinkDisposition::Create => {
                match create_cap_symlink(&self.output_dir, target, &destination) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(ReceiverError::ConflictPending { entry_id });
                    }
                    Err(error) if symlink_unavailable(&error) => self.save_symlink_notice(
                        &destination,
                        target,
                        "native symbolic-link creation is unavailable",
                    )?,
                    Err(error) => return Err(error.into()),
                }
            }
            SymlinkDisposition::NeedsConfirmation { .. } => self.save_symlink_notice(
                &destination,
                target,
                "target requires explicit confirmation",
            )?,
            SymlinkDisposition::SaveAsNotice { reason } => {
                self.save_symlink_notice(&destination, target, reason)?;
            }
        }
        store.finish_metadata_commit(entry_id, false)?;
        Ok(())
    }

    fn commit_symlink(
        &self,
        store: &mut TransferStore,
        entry_id: quick_share_protocol::EntryId,
        requested: &RelativePath,
        target: &str,
    ) -> Result<(), ReceiverError> {
        let relative = match store.metadata_state(entry_id)? {
            MetadataState::Committed { .. } | MetadataState::Skipped => return Ok(()),
            MetadataState::Committing { destination } => destination,
            MetadataState::Pending => {
                let Some(destination) =
                    resolve_destination(&self.output_root, requested, self.policy.conflict)?
                else {
                    store.finish_metadata_commit(entry_id, true)?;
                    return Ok(());
                };
                let relative = RelativePath::parse(
                    destination
                        .strip_prefix(&self.output_root)
                        .map_err(|_| ReceiverError::ManifestChanged)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                )?;
                store.begin_metadata_commit(entry_id, &relative)?;
                relative
            }
        };
        let destination = PathBuf::from(relative.as_str());
        let absolute = relative.resolve_under(&self.output_root);
        let notice = notice_path(&absolute)?;
        if let Ok(metadata) = fs::symlink_metadata(&absolute) {
            if metadata.file_type().is_symlink()
                && link_target_matches(&fs::read_link(&absolute)?, target)
            {
                store.finish_metadata_commit(entry_id, false)?;
                return Ok(());
            }
            return Err(PathError::DestinationExists(absolute).into());
        }
        if notice.exists() {
            store.finish_metadata_commit(entry_id, false)?;
            return Ok(());
        }
        if let Some(parent) = destination.parent() {
            self.output_dir.create_dir_all(parent)?;
        }
        match SymlinkDisposition::classify(requested, target, true) {
            SymlinkDisposition::Create => {
                match create_cap_symlink(&self.output_dir, target, &destination) {
                    Ok(()) => {}
                    Err(error) if symlink_unavailable(&error) => self.save_symlink_notice(
                        &destination,
                        target,
                        "native symbolic-link creation is unavailable",
                    )?,
                    Err(error) => return Err(error.into()),
                }
            }
            SymlinkDisposition::NeedsConfirmation { .. } => self.save_symlink_notice(
                &destination,
                target,
                "target requires explicit confirmation",
            )?,
            SymlinkDisposition::SaveAsNotice { reason } => {
                self.save_symlink_notice(&destination, target, reason)?;
            }
        }
        store.finish_metadata_commit(entry_id, false)?;
        Ok(())
    }

    fn save_symlink_notice(
        &self,
        destination: &Path,
        target: &str,
        reason: &str,
    ) -> Result<(), ReceiverError> {
        let notice = notice_path(destination)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = self.output_dir.open_with(&notice, &options)?;
        writeln!(file, "Quick Share did not create a symbolic link.")?;
        writeln!(file, "Reason: {reason}")?;
        writeln!(file, "Original target: {target}")?;
        file.sync_all()?;
        Ok(())
    }

    /// Child token used by a future socket/blocking worker to stop disk work promptly.
    pub fn cancellation_token(
        &self,
        transfer_id: TransferId,
    ) -> Result<CancellationToken, ReceiverError> {
        let state = self.lock_state()?;
        state
            .transfers
            .get(&transfer_id)
            .map(|transfer| transfer.cancellation.clone())
            .ok_or(ReceiverError::NotFound)
    }

    fn emit_progress(&self, transfer: &mut ActiveTransfer) {
        let status = transfer.store.transfer_status();
        let step = (transfer.offer.total_bytes / 100).max(1);
        if status == transfer.last_reported_status
            && transfer.received_bytes != transfer.offer.total_bytes
            && transfer
                .received_bytes
                .saturating_sub(transfer.last_reported_bytes)
                < step
        {
            return;
        }
        transfer.last_reported_bytes = transfer.received_bytes;
        transfer.last_reported_status = status;
        if let Some(progress) = &self.progress {
            let _ = progress.try_send(ReceiverProgressEvent {
                transfer_id: transfer.offer.transfer_id,
                received_bytes: transfer.received_bytes,
                total_bytes: transfer.offer.total_bytes,
                status,
            });
        }
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, ReceiverState>, ReceiverError> {
        self.state.lock().map_err(|_| ReceiverError::Internal)
    }
}

impl ReceiverEndpoint for ReceiverService {
    fn status(
        &self,
        peer: &DeviceId,
        request: TransferStatusRequest,
        now: Instant,
    ) -> Result<TransferStatusResponse, ReceiverError> {
        ReceiverService::status(self, peer, request, now)
    }

    fn receive_chunk(
        &self,
        peer: &DeviceId,
        request_id: RequestId,
        frame: ChunkData,
        now: Instant,
    ) -> Result<Option<ChunkAck>, ReceiverError> {
        ReceiverService::receive_chunk(self, peer, request_id, frame, now)
    }

    fn complete(
        &self,
        peer: &DeviceId,
        request: TransferComplete,
        now: Instant,
    ) -> Result<TransferCompleteAck, ReceiverError> {
        ReceiverService::complete(self, peer, request, now)
    }

    fn cancel(
        &self,
        peer: &DeviceId,
        request: TransferCancel,
        now: Instant,
    ) -> Result<(), ReceiverError> {
        ReceiverService::cancel(self, peer, request, now)
    }

    fn disconnect(&self, peer: &DeviceId) -> Result<usize, ReceiverError> {
        ReceiverService::disconnect(self, peer)
    }
}

#[derive(Default)]
struct ReceiverState {
    transfers: BTreeMap<TransferId, ActiveTransfer>,
}

struct ActiveTransfer {
    owner: DeviceId,
    offer: quick_share_protocol::TransferOffer,
    manifest_digest: [u8; 32],
    store: TransferStore,
    uploads: BTreeMap<RequestId, ChunkUpload>,
    upload_keys: BTreeSet<(quick_share_protocol::EntryId, u32)>,
    received_bytes: u64,
    last_reported_bytes: u64,
    last_reported_status: TransferStatus,
    cancellation: CancellationToken,
    completed_text: Option<TextPayload>,
}

fn map_planned_store_error(error: StoreError) -> ReceiverError {
    match error {
        StoreError::ConflictPending { entry_id } => ReceiverError::ConflictPending { entry_id },
        error => ReceiverError::Store(error),
    }
}

fn load_text_payload(
    store: &mut TransferStore,
    offer: &quick_share_protocol::TransferOffer,
) -> Result<Option<TextPayload>, ReceiverError> {
    let Some(entry) = offer
        .entries
        .iter()
        .find(|entry| matches!(entry.kind, ManifestEntryKind::Text { .. }))
    else {
        return Ok(None);
    };
    let media_type = match &entry.kind {
        ManifestEntryKind::Text { media_type } => media_type,
        _ => return Ok(None),
    };
    let bytes = store.read_verified_bytes(entry.id, MAX_TEXT_BYTES)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| ReceiverError::InvalidText("text payload is not valid UTF-8".to_owned()))?;
    let source = if media_type
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("source=clipboard"))
    {
        TextSource::Clipboard
    } else {
        TextSource::Literal
    };
    TextPayload::new(source, text)
        .map(Some)
        .map_err(|error| ReceiverError::InvalidText(error.to_string()))
}

fn received_bytes(
    store: &TransferStore,
    offer: &quick_share_protocol::TransferOffer,
) -> Result<u64, ReceiverError> {
    let mut received = 0_u64;
    for entry in &offer.entries {
        if !matches!(
            entry.kind,
            ManifestEntryKind::File | ManifestEntryKind::Text { .. }
        ) {
            continue;
        }
        let missing: BTreeSet<_> = store.missing_chunks(entry.id)?.into_iter().collect();
        for index in 0..store.chunk_count(entry.id)? {
            if !missing.contains(&index) {
                let offset = u64::from(index) * u64::from(offer.chunk_size);
                received = received.saturating_add(
                    entry
                        .size
                        .saturating_sub(offset)
                        .min(u64::from(offer.chunk_size)),
                );
            }
        }
    }
    Ok(received.min(offer.total_bytes))
}

fn staged_files(authorized: &AuthorizedOffer) -> Result<Vec<StagedFile>, ReceiverError> {
    authorized
        .offer
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                ManifestEntryKind::File | ManifestEntryKind::Text { .. }
            )
        })
        .map(|entry| {
            StagedFile::new(
                entry.id,
                RelativePath::parse(&entry.relative_path)?,
                entry.size,
                authorized.offer.chunk_size,
                entry.digest.ok_or(ReceiverError::ManifestChanged)?,
            )
            .map_err(ReceiverError::Store)
        })
        .collect()
}

#[cfg(unix)]
fn create_cap_symlink(dir: &Dir, target: &str, destination: &Path) -> io::Result<()> {
    dir.symlink(target, destination)
}

#[cfg(windows)]
fn create_cap_symlink(dir: &Dir, target: &str, destination: &Path) -> io::Result<()> {
    // Windows does not reliably dereference forward slashes in relative symlink targets.
    // This is a separator-only semantic conversion after portable target validation.
    dir.symlink_file(target.replace('/', "\\"), destination)
}

#[cfg(not(any(unix, windows)))]
fn create_cap_symlink(_dir: &Dir, _target: &str, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links are unsupported",
    ))
}

#[cfg(windows)]
fn link_target_matches(stored: &Path, expected: &str) -> bool {
    stored.to_string_lossy().replace('\\', "/") == expected.replace('\\', "/")
}

#[cfg(not(windows))]
fn link_target_matches(stored: &Path, expected: &str) -> bool {
    stored.to_str() == Some(expected)
}

fn notice_path(destination: &Path) -> Result<PathBuf, ReceiverError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ReceiverError::ManifestChanged)?;
    Ok(destination.with_file_name(format!("{file_name}.quick-share-symlink.txt")))
}

fn symlink_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
    ) || error.raw_os_error() == Some(1314)
}

#[derive(Debug, Error)]
pub enum ReceiverError {
    #[error("invalid receiver resource limits")]
    InvalidLimits,
    #[error("invalid authenticated transfer frame: {0}")]
    InvalidFrame(String),
    #[error("transfer authorization failed")]
    Unauthorized,
    #[error("transfer manifest changed across authorization or resume")]
    ManifestChanged,
    #[error("received text payload is invalid: {0}")]
    InvalidText(String),
    #[error("chunk fragments arrived out of order")]
    FragmentOrder,
    #[error("chunk descriptor changed during a fragmented upload")]
    FragmentDescriptorChanged,
    #[error("the chunk is already active on another request")]
    ChunkAlreadyActive,
    #[error("receiver file-stream limit reached")]
    FileStreamLimit,
    #[error("receiver transfer-task limit reached")]
    ReceiveTaskLimit,
    #[error("transfer has active uploads")]
    UploadsActive,
    #[error("transfer is incomplete")]
    Incomplete,
    #[error("destination changed after planning for entry {entry_id:?}")]
    ConflictPending {
        entry_id: quick_share_protocol::EntryId,
    },
    #[error("transfer is in terminal state {0:?}")]
    Terminal(TransferStatus),
    #[error("transfer was not found")]
    NotFound,
    #[error("metadata entry was skipped by conflict policy")]
    SkippedMetadata,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Offer(#[from] OfferError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    DestinationPlan(#[from] DestinationPlanError),
    #[error("receiver I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("receiver state is unavailable")]
    Internal,
}
