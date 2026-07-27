//! Bounded sender work queue, monotonic progress, cancellation, and finite retry policy.

use crate::{StoreError, hash_file, offer::AuthorizationToken};
use async_trait::async_trait;
use quick_share_core::manifest::{
    ManifestEntry as SourceEntry, ManifestEntryKind as SourceEntryKind, TransferManifest,
};
use quick_share_protocol::{
    AuthorizationProof, CancelReason, ChunkAck, ChunkData, ChunkDescriptor, EntryId,
    MAX_CHUNK_FRAME_BYTES, MAX_CHUNK_SIZE, MAX_TRANSFER_CHUNKS, MIN_CHUNK_SIZE, ManifestEntryKind,
    RequestId, TransferCancel, TransferComplete, TransferCompleteAck, TransferId, TransferOffer,
    TransferStatus, TransferStatusRequest, TransferStatusResponse,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;

const MAX_SENDER_WORKERS: usize = 64;
const MAX_SENDER_QUEUE: usize = 1024;

/// Source file and immutable expected full digest from the accepted offer.
#[derive(Clone)]
pub struct SendFile {
    pub source: SourceEntry,
    pub final_digest: [u8; 32],
}

impl std::fmt::Debug for SendFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SendFile")
            .field("entry_id", &self.source.id)
            .field("relative_path", &self.source.relative_path)
            .field("size", &self.source.size)
            .field("source_path", &"[REDACTED]")
            .field("final_digest", &"[REDACTED]")
            .finish()
    }
}

impl SendFile {
    pub fn new(source: SourceEntry, final_digest: [u8; 32]) -> Result<Self, SenderError> {
        if !matches!(source.kind, SourceEntryKind::File) {
            return Err(SenderError::InvalidPlan(
                "only regular payload files enter the sender queue".to_owned(),
            ));
        }
        Ok(Self {
            source,
            final_digest,
        })
    }

    /// Hashes with a bounded buffer and checks metadata before and after preparation.
    pub fn prepare(source: SourceEntry) -> Result<Self, SenderError> {
        if !source.source_is_unchanged()? {
            return Err(SenderError::SourceChanged(source.id));
        }
        let digest = hash_file(&source.source_path)?;
        if !source.source_is_unchanged()? {
            return Err(SenderError::SourceChanged(source.id));
        }
        Self::new(source, digest)
    }
}

#[derive(Debug, Clone)]
pub struct TransferPlan {
    pub transfer_id: TransferId,
    pub manifest_digest: [u8; 32],
    pub chunk_size: u32,
    pub total_bytes: u64,
    pub files: Vec<SendFile>,
}

impl TransferPlan {
    /// Binds and hashes local sources against the wire offer.
    /// Call this synchronous preparation function from a bounded blocking worker.
    pub fn from_offer(
        offer: &TransferOffer,
        source_manifest: TransferManifest,
    ) -> Result<Self, SenderError> {
        offer
            .validate()
            .map_err(|error| SenderError::InvalidPlan(error.to_string()))?;
        if offer.entries.len() != source_manifest.entries.len() {
            return Err(SenderError::InvalidPlan(
                "wire and local manifest entry counts differ".to_owned(),
            ));
        }
        let local: BTreeMap<_, _> = source_manifest
            .entries
            .into_iter()
            .map(|entry| (entry.id, entry))
            .collect();
        let mut files = Vec::new();
        for wire in &offer.entries {
            let source = local.get(&wire.id).ok_or_else(|| {
                SenderError::InvalidPlan("wire entry has no local source".to_owned())
            })?;
            if source.relative_path.as_str() != wire.relative_path || source.size != wire.size {
                return Err(SenderError::InvalidPlan(
                    "wire entry does not match its local source snapshot".to_owned(),
                ));
            }
            match (&wire.kind, &source.kind) {
                (ManifestEntryKind::File, SourceEntryKind::File) => {
                    let expected = wire.digest.ok_or_else(|| {
                        SenderError::InvalidPlan("wire file has no digest".to_owned())
                    })?;
                    let prepared = SendFile::prepare(source.clone())?;
                    if prepared.final_digest != expected {
                        return Err(SenderError::InvalidPlan(
                            "wire digest does not match the local source snapshot".to_owned(),
                        ));
                    }
                    files.push(prepared);
                }
                (ManifestEntryKind::Directory, SourceEntryKind::Directory) => {}
                (
                    ManifestEntryKind::Symlink {
                        target: wire_target,
                    },
                    SourceEntryKind::Symlink {
                        target: source_target,
                    },
                ) if wire_target == source_target => {}
                _ => {
                    return Err(SenderError::InvalidPlan(
                        "wire and local entry kinds differ".to_owned(),
                    ));
                }
            }
        }
        let manifest_digest = *blake3::hash(
            &serde_json::to_vec(offer)
                .map_err(|error| SenderError::InvalidPlan(error.to_string()))?,
        )
        .as_bytes();
        let plan = Self {
            transfer_id: offer.transfer_id,
            manifest_digest,
            chunk_size: offer.chunk_size,
            total_bytes: offer.total_bytes,
            files,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), SenderError> {
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&self.chunk_size) {
            return Err(SenderError::InvalidPlan(
                "chunk size is outside QSP limits".to_owned(),
            ));
        }
        let mut ids = BTreeMap::new();
        let mut total_bytes = 0_u64;
        let mut total_chunks = 0_u64;
        for file in &self.files {
            if ids.insert(file.source.id, ()).is_some() {
                return Err(SenderError::InvalidPlan(
                    "duplicate source entry ID".to_owned(),
                ));
            }
            total_bytes = total_bytes.checked_add(file.source.size).ok_or_else(|| {
                SenderError::InvalidPlan("total source size overflowed".to_owned())
            })?;
            total_chunks = total_chunks
                .checked_add(file.source.size.div_ceil(u64::from(self.chunk_size)))
                .ok_or_else(|| {
                    SenderError::InvalidPlan("total chunk count overflowed".to_owned())
                })?;
        }
        if total_bytes != self.total_bytes || total_chunks > MAX_TRANSFER_CHUNKS {
            return Err(SenderError::InvalidPlan(
                "source totals do not match the bounded transfer plan".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SenderPolicy {
    pub concurrent_files: usize,
    pub queue_capacity: usize,
    pub retry: RetryPolicy,
}

impl Default for SenderPolicy {
    fn default() -> Self {
        Self {
            concurrent_files: 4,
            queue_capacity: 16,
            retry: RetryPolicy::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total attempts, including the initial request.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    /// Maximum positive jitter as a fraction of the exponential delay.
    pub jitter_fraction: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
            jitter_fraction: 0.25,
        }
    }
}

impl RetryPolicy {
    pub fn validate(&self) -> Result<(), SenderError> {
        if self.max_attempts == 0
            || self.max_attempts > 3
            || !self.jitter_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.jitter_fraction)
        {
            return Err(SenderError::InvalidPlan("invalid retry policy".to_owned()));
        }
        Ok(())
    }

    #[must_use]
    pub fn delay_for(&self, failed_attempt: u32, entropy: u64) -> Duration {
        let exponent = failed_attempt.saturating_sub(1).min(31);
        let base = self
            .base_delay
            .saturating_mul(1_u32.checked_shl(exponent).unwrap_or(u32::MAX))
            .min(self.max_delay);
        let jitter_max = base.mul_f64(self.jitter_fraction);
        let fraction = entropy as f64 / u64::MAX as f64;
        base.saturating_add(jitter_max.mul_f64(fraction))
            .min(self.max_delay)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressEvent {
    pub current_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: f64,
    pub eta: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct TextSendPlan {
    pub transfer_id: TransferId,
    pub manifest_digest: [u8; 32],
    pub entry_id: EntryId,
    pub bytes: Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferSummary {
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_chunks: u64,
    pub status: TransferStatus,
}

#[derive(Debug, Error, Clone)]
pub enum TransportError {
    #[error("retryable transport interruption")]
    Retryable,
    #[error("non-retryable remote or protocol failure")]
    Fatal,
    #[error("remote cancelled the transfer")]
    Cancelled,
    #[error("remote reported an integrity failure")]
    Integrity,
    #[error("remote rejected authorization")]
    Unauthorized,
    #[error(
        "remote resource limit was reached; check receiver disk space, output permissions, and concurrency limits"
    )]
    ResourceLimit,
}

impl TransportError {
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable)
    }
}

#[async_trait]
pub trait TransferTransport: Send + Sync {
    async fn status(
        &self,
        request: TransferStatusRequest,
    ) -> Result<TransferStatusResponse, TransportError>;

    async fn send_fragment(
        &self,
        request_id: RequestId,
        frame: ChunkData,
    ) -> Result<Option<ChunkAck>, TransportError>;

    async fn complete(
        &self,
        request: TransferComplete,
    ) -> Result<TransferCompleteAck, TransportError>;

    async fn cancel(&self, request: TransferCancel) -> Result<(), TransportError>;

    /// Re-establishes authenticated Noise transport and clears partial request-local streams.
    async fn reconnect(&self) -> Result<(), TransportError>;
}

pub struct TransferSender<T> {
    transport: Arc<T>,
    policy: SenderPolicy,
}

impl<T> TransferSender<T>
where
    T: TransferTransport + 'static,
{
    pub fn new(transport: Arc<T>, policy: SenderPolicy) -> Result<Self, SenderError> {
        if policy.concurrent_files == 0
            || policy.concurrent_files > MAX_SENDER_WORKERS
            || policy.queue_capacity == 0
            || policy.queue_capacity > MAX_SENDER_QUEUE
        {
            return Err(SenderError::InvalidPlan(
                "sender concurrency limits must be non-zero".to_owned(),
            ));
        }
        policy.retry.validate()?;
        Ok(Self { transport, policy })
    }

    pub async fn send(
        &self,
        plan: TransferPlan,
        authorization: &AuthorizationToken,
        cancellation: CancellationToken,
        progress: Option<mpsc::Sender<ProgressEvent>>,
    ) -> Result<TransferSummary, SenderError> {
        plan.validate()?;
        for file in &plan.files {
            if !file.source.source_is_unchanged()? {
                return Err(SenderError::SourceChanged(file.source.id));
            }
        }
        let proof =
            Arc::new(authorization.with_bytes(|bytes| AuthorizationProof::from_bytes(*bytes)));
        let status = match self
            .retry_status(plan.transfer_id, proof.as_ref(), &cancellation)
            .await
        {
            Ok(status) => status,
            Err(SenderError::Cancelled) => {
                let _ = self
                    .transport
                    .cancel(TransferCancel {
                        transfer_id: plan.transfer_id,
                        authorization: proof.as_ref().clone(),
                        reason: CancelReason::User,
                    })
                    .await;
                return Err(SenderError::Cancelled);
            }
            Err(error) => return Err(error),
        };
        validate_resume_status(&plan, &status)?;
        if status.status == TransferStatus::Completed {
            return Ok(TransferSummary {
                transferred_bytes: plan.total_bytes,
                total_bytes: plan.total_bytes,
                uploaded_chunks: 0,
                status: TransferStatus::Completed,
            });
        }
        if matches!(
            status.status,
            TransferStatus::Cancelled | TransferStatus::Failed
        ) {
            return Err(SenderError::RemoteTerminal(status.status));
        }

        let current = resumed_bytes(&plan, &status)?;
        let tracker = Arc::new(ProgressTracker::new(plan.total_bytes, current, progress));
        tracker.emit();
        let missing = missing_map(&status);
        let (work_tx, work_rx) = mpsc::channel::<WorkItem>(self.policy.queue_capacity);
        let receiver = Arc::new(tokio::sync::Mutex::new(work_rx));
        let mut workers = JoinSet::new();
        for _ in 0..self.policy.concurrent_files {
            workers.spawn(worker(
                Arc::clone(&self.transport),
                Arc::clone(&receiver),
                Arc::clone(&tracker),
                plan.chunk_size,
                Arc::clone(&proof),
                self.policy.retry.clone(),
                cancellation.clone(),
            ));
        }

        let files = plan.files.clone();
        let transfer_id = plan.transfer_id;
        let chunk_size = plan.chunk_size;
        let producer_cancel = cancellation.clone();
        workers.spawn(async move {
            for file in files {
                let chunk_count = u32::try_from(file.source.size.div_ceil(u64::from(chunk_size)))
                    .map_err(|_| SenderError::InvalidPlan("chunk count exceeds QSP/1".to_owned()))?;
                for index in 0..chunk_count {
                    if !missing
                        .get(&file.source.id)
                        .is_some_and(|bitmap| bitmap.is_missing(index))
                    {
                        continue;
                    }
                    tokio::select! {
                        _ = producer_cancel.cancelled() => return Err(SenderError::Cancelled),
                        result = work_tx.send(WorkItem { transfer_id, file: file.clone(), index }) => {
                            result.map_err(|_| SenderError::WorkerFailed)?;
                        }
                    }
                }
            }
            Ok::<(), SenderError>(())
        });

        let mut worker_error = None;
        while let Some(result) = workers.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) if worker_error.is_none() => {
                    cancellation.cancel();
                    worker_error = Some(error);
                }
                Err(_) if worker_error.is_none() => {
                    cancellation.cancel();
                    worker_error = Some(SenderError::WorkerFailed);
                }
                _ => {}
            }
        }
        if let Some(error) = worker_error {
            let reason = if matches!(error, SenderError::SourceChanged(_)) {
                CancelReason::SourceChanged
            } else {
                CancelReason::User
            };
            let _ = self
                .transport
                .cancel(TransferCancel {
                    transfer_id: plan.transfer_id,
                    authorization: proof.as_ref().clone(),
                    reason,
                })
                .await;
            return Err(error);
        }
        for file in &plan.files {
            if !file.source.source_is_unchanged()? {
                let _ = self
                    .transport
                    .cancel(TransferCancel {
                        transfer_id: plan.transfer_id,
                        authorization: proof.as_ref().clone(),
                        reason: CancelReason::SourceChanged,
                    })
                    .await;
                return Err(SenderError::SourceChanged(file.source.id));
            }
        }
        let completion = match self
            .retry_complete(
                plan.transfer_id,
                plan.manifest_digest,
                proof.as_ref(),
                &cancellation,
            )
            .await
        {
            Ok(completion) => completion,
            Err(SenderError::Cancelled) => {
                let _ = self
                    .transport
                    .cancel(TransferCancel {
                        transfer_id: plan.transfer_id,
                        authorization: proof.as_ref().clone(),
                        reason: CancelReason::User,
                    })
                    .await;
                return Err(SenderError::Cancelled);
            }
            Err(error) => return Err(error),
        };
        if completion.transfer_id != plan.transfer_id
            || completion.status != TransferStatus::Completed
        {
            return Err(SenderError::InvalidResponse);
        }
        let snapshot = tracker.snapshot();
        Ok(TransferSummary {
            transferred_bytes: snapshot.current_bytes,
            total_bytes: plan.total_bytes,
            uploaded_chunks: tracker.uploaded_chunks(),
            status: TransferStatus::Completed,
        })
    }

    /// Sends one bounded in-memory text entry without writing plaintext to a temporary file.
    pub async fn send_text(
        &self,
        plan: TextSendPlan,
        authorization: &AuthorizationToken,
        cancellation: CancellationToken,
        progress: Option<mpsc::Sender<ProgressEvent>>,
    ) -> Result<TransferSummary, SenderError> {
        if plan.bytes.len() > crate::text::MAX_TEXT_BYTES {
            return Err(SenderError::InvalidPlan(
                "text payload exceeds the 1 MiB bound".to_owned(),
            ));
        }
        let proof =
            Arc::new(authorization.with_bytes(|bytes| AuthorizationProof::from_bytes(*bytes)));
        let status = match self
            .retry_status(plan.transfer_id, proof.as_ref(), &cancellation)
            .await
        {
            Ok(status) => status,
            Err(SenderError::Cancelled) => {
                let _ = self
                    .transport
                    .cancel(TransferCancel {
                        transfer_id: plan.transfer_id,
                        authorization: proof.as_ref().clone(),
                        reason: CancelReason::User,
                    })
                    .await;
                return Err(SenderError::Cancelled);
            }
            Err(error) => return Err(error),
        };
        status
            .validate()
            .map_err(|error| SenderError::InvalidPlan(error.to_string()))?;
        let chunk_count = u32::from(!plan.bytes.is_empty());
        if status.transfer_id != plan.transfer_id
            || status.manifest_digest != plan.manifest_digest
            || status.entries.len() != 1
            || status.entries[0].entry_id != plan.entry_id
            || status.entries[0].chunks.chunk_count != chunk_count
        {
            return Err(SenderError::ResumeMismatch);
        }
        if status.status == TransferStatus::Completed {
            return Ok(TransferSummary {
                transferred_bytes: plan.bytes.len() as u64,
                total_bytes: plan.bytes.len() as u64,
                uploaded_chunks: 0,
                status: TransferStatus::Completed,
            });
        }
        if matches!(
            status.status,
            TransferStatus::Cancelled | TransferStatus::Failed
        ) {
            return Err(SenderError::RemoteTerminal(status.status));
        }
        let missing = chunk_count == 1 && status.entries[0].chunks.is_missing(0);
        let resumed = if missing { 0 } else { plan.bytes.len() as u64 };
        let tracker = Arc::new(ProgressTracker::new(
            plan.bytes.len() as u64,
            resumed,
            progress,
        ));
        tracker.emit();
        if missing {
            let chunk = ReadChunk {
                bytes: plan.bytes.to_vec(),
                digest: *blake3::hash(&plan.bytes).as_bytes(),
            };
            let descriptor = ChunkDescriptor {
                transfer_id: plan.transfer_id,
                entry_id: plan.entry_id,
                index: 0,
                offset: 0,
                length: u32::try_from(chunk.bytes.len()).map_err(|_| {
                    SenderError::InvalidPlan("text chunk length exceeds QSP/1".to_owned())
                })?,
                digest: chunk.digest,
            };
            if let Err(error) = send_chunk_with_retry(
                self.transport.as_ref(),
                descriptor,
                &chunk,
                proof.as_ref(),
                &self.policy.retry,
                &cancellation,
            )
            .await
            {
                if matches!(error, SenderError::Cancelled) {
                    let _ = self
                        .transport
                        .cancel(TransferCancel {
                            transfer_id: plan.transfer_id,
                            authorization: proof.as_ref().clone(),
                            reason: CancelReason::User,
                        })
                        .await;
                }
                return Err(error);
            }
            tracker.record(plan.bytes.len() as u64);
        }
        let completion = match self
            .retry_complete(
                plan.transfer_id,
                plan.manifest_digest,
                proof.as_ref(),
                &cancellation,
            )
            .await
        {
            Ok(completion) => completion,
            Err(SenderError::Cancelled) => {
                let _ = self
                    .transport
                    .cancel(TransferCancel {
                        transfer_id: plan.transfer_id,
                        authorization: proof.as_ref().clone(),
                        reason: CancelReason::User,
                    })
                    .await;
                return Err(SenderError::Cancelled);
            }
            Err(error) => return Err(error),
        };
        if completion.transfer_id != plan.transfer_id
            || completion.status != TransferStatus::Completed
        {
            return Err(SenderError::InvalidResponse);
        }
        let snapshot = tracker.snapshot();
        Ok(TransferSummary {
            transferred_bytes: snapshot.current_bytes,
            total_bytes: plan.bytes.len() as u64,
            uploaded_chunks: tracker.uploaded_chunks(),
            status: TransferStatus::Completed,
        })
    }

    async fn retry_complete(
        &self,
        transfer_id: TransferId,
        manifest_digest: [u8; 32],
        proof: &AuthorizationProof,
        cancellation: &CancellationToken,
    ) -> Result<TransferCompleteAck, SenderError> {
        let mut attempt = 1;
        loop {
            let result = tokio::select! {
                _ = cancellation.cancelled() => return Err(SenderError::Cancelled),
                result = self.transport.complete(TransferComplete {
                    transfer_id,
                    authorization: proof.clone(),
                    manifest_digest,
                }) => result,
            };
            match result {
                Ok(ack) => return Ok(ack),
                Err(error) if error.is_retryable() && attempt < self.policy.retry.max_attempts => {
                    self.transport
                        .reconnect()
                        .await
                        .map_err(SenderError::Transport)?;
                    sleep_retry(&self.policy.retry, attempt, cancellation).await?;
                    attempt += 1;
                }
                Err(error) => return Err(SenderError::Transport(error)),
            }
        }
    }

    async fn retry_status(
        &self,
        transfer_id: TransferId,
        proof: &AuthorizationProof,
        cancellation: &CancellationToken,
    ) -> Result<TransferStatusResponse, SenderError> {
        let mut attempt = 1;
        loop {
            if cancellation.is_cancelled() {
                return Err(SenderError::Cancelled);
            }
            let result = tokio::select! {
                _ = cancellation.cancelled() => return Err(SenderError::Cancelled),
                result = self.transport.status(TransferStatusRequest {
                    transfer_id,
                    authorization: proof.clone(),
                }) => result,
            };
            match result {
                Ok(status) => return Ok(status),
                Err(error) if error.is_retryable() && attempt < self.policy.retry.max_attempts => {
                    self.transport
                        .reconnect()
                        .await
                        .map_err(SenderError::Transport)?;
                    sleep_retry(&self.policy.retry, attempt, cancellation).await?;
                    attempt += 1;
                }
                Err(error) => return Err(SenderError::Transport(error)),
            }
        }
    }
}

#[derive(Clone)]
struct WorkItem {
    transfer_id: TransferId,
    file: SendFile,
    index: u32,
}

async fn worker<T: TransferTransport + 'static>(
    transport: Arc<T>,
    receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<WorkItem>>>,
    tracker: Arc<ProgressTracker>,
    chunk_size: u32,
    proof: Arc<AuthorizationProof>,
    retry: RetryPolicy,
    cancellation: CancellationToken,
) -> Result<(), SenderError> {
    loop {
        let work = tokio::select! {
            _ = cancellation.cancelled() => return Err(SenderError::Cancelled),
            work = async {
                let mut receiver = receiver.lock().await;
                receiver.recv().await
            } => work,
        };
        let Some(work) = work else {
            return Ok(());
        };
        let chunk = read_chunk(work.file.clone(), work.index, chunk_size).await?;
        let length = chunk.bytes.len() as u64;
        let descriptor = ChunkDescriptor {
            transfer_id: work.transfer_id,
            entry_id: work.file.source.id,
            index: work.index,
            offset: u64::from(work.index) * u64::from(chunk_size),
            length: u32::try_from(chunk.bytes.len())
                .map_err(|_| SenderError::InvalidPlan("chunk length exceeds QSP/1".to_owned()))?,
            digest: chunk.digest,
        };
        send_chunk_with_retry(
            transport.as_ref(),
            descriptor,
            &chunk,
            proof.as_ref(),
            &retry,
            &cancellation,
        )
        .await?;
        tracker.record(length);
    }
}

struct ReadChunk {
    bytes: Vec<u8>,
    digest: [u8; 32],
}

async fn read_chunk(file: SendFile, index: u32, chunk_size: u32) -> Result<ReadChunk, SenderError> {
    tokio::task::spawn_blocking(move || {
        if !file.source.source_is_unchanged()? {
            return Err(SenderError::SourceChanged(file.source.id));
        }
        let offset = u64::from(index) * u64::from(chunk_size);
        let remaining = file.source.size.saturating_sub(offset);
        let length = remaining.min(u64::from(chunk_size));
        let length = usize::try_from(length)
            .map_err(|_| SenderError::InvalidPlan("chunk length exceeds platform".to_owned()))?;
        let mut bytes = vec![0_u8; length];
        let mut source = File::open(&file.source.source_path)?;
        source.seek(SeekFrom::Start(offset))?;
        source.read_exact(&mut bytes)?;
        if !file.source.source_is_unchanged()? {
            return Err(SenderError::SourceChanged(file.source.id));
        }
        let digest = *blake3::hash(&bytes).as_bytes();
        Ok(ReadChunk { bytes, digest })
    })
    .await
    .map_err(|_| SenderError::WorkerFailed)?
}

async fn send_chunk_with_retry<T: TransferTransport>(
    transport: &T,
    descriptor: ChunkDescriptor,
    chunk: &ReadChunk,
    proof: &AuthorizationProof,
    retry: &RetryPolicy,
    cancellation: &CancellationToken,
) -> Result<(), SenderError> {
    // Preserve one request identity across retries so a lost ACK is idempotent end-to-end.
    let request_id = random_request_id()?;
    let mut attempt = 1;
    loop {
        if cancellation.is_cancelled() {
            return Err(SenderError::Cancelled);
        }
        match send_fragments(
            transport,
            request_id,
            &descriptor,
            &chunk.bytes,
            proof,
            cancellation,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(_) if cancellation.is_cancelled() => return Err(SenderError::Cancelled),
            Err(error) if error.is_retryable() && attempt < retry.max_attempts => {
                transport
                    .reconnect()
                    .await
                    .map_err(SenderError::Transport)?;
                sleep_retry(retry, attempt, cancellation).await?;
                attempt += 1;
            }
            Err(error) => return Err(SenderError::Transport(error)),
        }
    }
}

fn random_request_id() -> Result<RequestId, SenderError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| SenderError::InvalidPlan("secure request ID generation failed".to_owned()))?;
    Ok(RequestId::from_bytes(bytes))
}

async fn send_fragments<T: TransferTransport>(
    transport: &T,
    request_id: RequestId,
    descriptor: &ChunkDescriptor,
    bytes: &[u8],
    proof: &AuthorizationProof,
    cancellation: &CancellationToken,
) -> Result<(), TransportError> {
    let mut offset = 0_usize;
    let mut ack = None;
    while offset < bytes.len() {
        let end = (offset + MAX_CHUNK_FRAME_BYTES).min(bytes.len());
        ack = tokio::select! {
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            result = transport.send_fragment(
                request_id,
                ChunkData {
                    authorization: proof.clone(),
                    descriptor: descriptor.clone(),
                    fragment_offset: offset as u32,
                    final_fragment: end == bytes.len(),
                    payload: bytes[offset..end].to_vec(),
                },
            ) => result?,
        };
        if ack.is_some() && end != bytes.len() {
            break;
        }
        offset = end;
    }
    let ack = ack.ok_or(TransportError::Retryable)?;
    if ack.transfer_id != descriptor.transfer_id
        || ack.entry_id != descriptor.entry_id
        || ack.index != descriptor.index
        || ack.accepted_length != descriptor.length
    {
        return Err(TransportError::Fatal);
    }
    Ok(())
}

async fn sleep_retry(
    retry: &RetryPolicy,
    failed_attempt: u32,
    cancellation: &CancellationToken,
) -> Result<(), SenderError> {
    let mut entropy = [0_u8; 8];
    let _ = getrandom::fill(&mut entropy);
    let delay = retry.delay_for(failed_attempt, u64::from_le_bytes(entropy));
    tokio::select! {
        _ = cancellation.cancelled() => Err(SenderError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

fn validate_resume_status(
    plan: &TransferPlan,
    status: &TransferStatusResponse,
) -> Result<(), SenderError> {
    status
        .validate()
        .map_err(|error| SenderError::InvalidPlan(error.to_string()))?;
    if status.transfer_id != plan.transfer_id || status.manifest_digest != plan.manifest_digest {
        return Err(SenderError::ResumeMismatch);
    }
    let expected: BTreeMap<_, _> = plan
        .files
        .iter()
        .map(|file| {
            (
                file.source.id,
                u32::try_from(file.source.size.div_ceil(u64::from(plan.chunk_size)))
                    .unwrap_or(u32::MAX),
            )
        })
        .collect();
    if status.entries.len() != expected.len()
        || status
            .entries
            .iter()
            .any(|entry| expected.get(&entry.entry_id).copied() != Some(entry.chunks.chunk_count))
    {
        return Err(SenderError::ResumeMismatch);
    }
    Ok(())
}

fn missing_map(
    status: &TransferStatusResponse,
) -> BTreeMap<EntryId, quick_share_protocol::MissingChunkBitmap> {
    status
        .entries
        .iter()
        .map(|entry| (entry.entry_id, entry.chunks.clone()))
        .collect()
}

fn resumed_bytes(plan: &TransferPlan, status: &TransferStatusResponse) -> Result<u64, SenderError> {
    let missing = missing_map(status);
    let mut received = 0_u64;
    for file in &plan.files {
        let chunks = missing
            .get(&file.source.id)
            .ok_or(SenderError::ResumeMismatch)?;
        for index in 0..chunks.chunk_count {
            if !chunks.is_missing(index) {
                let offset = u64::from(index) * u64::from(plan.chunk_size);
                received = received.saturating_add(
                    file.source
                        .size
                        .saturating_sub(offset)
                        .min(u64::from(plan.chunk_size)),
                );
            }
        }
    }
    Ok(received)
}

struct ProgressTracker {
    total: u64,
    initial: u64,
    started: Instant,
    state: Mutex<ProgressState>,
    sender: Option<mpsc::Sender<ProgressEvent>>,
}

struct ProgressState {
    current: u64,
    uploaded_chunks: u64,
    last_emitted_bytes: u64,
}

impl ProgressTracker {
    fn new(total: u64, current: u64, sender: Option<mpsc::Sender<ProgressEvent>>) -> Self {
        Self {
            total,
            initial: current,
            started: Instant::now(),
            state: Mutex::new(ProgressState {
                current,
                uploaded_chunks: 0,
                last_emitted_bytes: current,
            }),
            sender,
        }
    }

    fn record(&self, bytes: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.current = state.current.saturating_add(bytes).min(self.total);
            state.uploaded_chunks += 1;
            let step = (self.total / 100).max(1);
            if state.current == self.total
                || state.current.saturating_sub(state.last_emitted_bytes) >= step
            {
                state.last_emitted_bytes = state.current;
                self.try_emit(&state);
            }
        }
    }

    fn emit(&self) {
        if let Ok(state) = self.state.lock() {
            self.try_emit(&state);
        }
    }

    fn try_emit(&self, state: &ProgressState) {
        let Some(sender) = &self.sender else {
            return;
        };
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        let speed = state.current.saturating_sub(self.initial) as f64 / elapsed;
        let remaining = self.total.saturating_sub(state.current);
        let eta = (speed > 0.0).then(|| Duration::from_secs_f64(remaining as f64 / speed));
        let _ = sender.try_send(ProgressEvent {
            current_bytes: state.current,
            total_bytes: self.total,
            bytes_per_second: speed,
            eta,
        });
    }

    fn snapshot(&self) -> ProgressEvent {
        let state = self.state.lock().expect("progress mutex poisoned");
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        ProgressEvent {
            current_bytes: state.current,
            total_bytes: self.total,
            bytes_per_second: state.current.saturating_sub(self.initial) as f64 / elapsed,
            eta: None,
        }
    }

    fn uploaded_chunks(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.uploaded_chunks)
    }
}

#[derive(Debug, Error)]
pub enum SenderError {
    #[error("invalid sender plan: {0}")]
    InvalidPlan(String),
    #[error("source entry {0:?} changed during transfer")]
    SourceChanged(EntryId),
    #[error("receiver resume state does not match this source snapshot")]
    ResumeMismatch,
    #[error("remote transfer is terminal: {0:?}")]
    RemoteTerminal(TransferStatus),
    #[error("transfer was cancelled")]
    Cancelled,
    #[error("transport failed: {0}")]
    Transport(#[from] TransportError),
    #[error("receiver returned an invalid acknowledgement")]
    InvalidResponse,
    #[error("sender worker failed")]
    WorkerFailed,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}
