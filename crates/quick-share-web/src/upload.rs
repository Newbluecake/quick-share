use quick_share_core::{
    config::ConflictPolicy,
    paths::{PathError, RelativePath, resolve_destination},
};
use quick_share_platform::{StorageError, atomic_move};
use std::{
    collections::VecDeque,
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};
use uuid::Uuid;

pub const MAX_UPLOAD_FILES: usize = 1_000;
const STAGING_DIRECTORY: &str = ".quick-share-web-upload";

#[derive(Clone)]
pub struct UploadConfig {
    pub output_root: PathBuf,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_body_bytes: usize,
    pub conflict: ConflictPolicy,
    pub max_uploads_per_window: usize,
    pub rate_window: Duration,
    pub max_concurrent_uploads: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadProgress {
    pub file_name: String,
    pub file_bytes: u64,
    pub request_bytes: u64,
}

pub struct UploadService {
    config: UploadConfig,
    canonical_root: PathBuf,
    staging_root: PathBuf,
    password_hash: Option<[u8; 32]>,
    recent_uploads: Mutex<VecDeque<Instant>>,
    concurrency: Arc<Semaphore>,
    progress: Option<mpsc::Sender<UploadProgress>>,
}

impl UploadService {
    pub fn new(config: UploadConfig, password: Option<&str>) -> Result<Self, UploadError> {
        Self::new_with_progress(config, password, None)
    }

    pub fn new_with_progress(
        config: UploadConfig,
        password: Option<&str>,
        progress: Option<mpsc::Sender<UploadProgress>>,
    ) -> Result<Self, UploadError> {
        validate_config(&config)?;
        fs::create_dir_all(&config.output_root)?;
        let metadata = fs::symlink_metadata(&config.output_root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(UploadError::UnsafeOutputRoot);
        }
        let canonical_root = fs::canonicalize(&config.output_root)?;
        let staging_root = config.output_root.join(STAGING_DIRECTORY);
        fs::create_dir_all(&staging_root)?;
        let staging_metadata = fs::symlink_metadata(&staging_root)?;
        let canonical_staging = fs::canonicalize(&staging_root)?;
        if staging_metadata.file_type().is_symlink()
            || !staging_metadata.is_dir()
            || !canonical_staging.starts_with(&canonical_root)
        {
            return Err(UploadError::UnsafeOutputRoot);
        }
        let concurrency = Arc::new(Semaphore::new(config.max_concurrent_uploads));
        Ok(Self {
            config,
            canonical_root,
            staging_root: canonical_staging,
            password_hash: password.map(password_hash),
            recent_uploads: Mutex::new(VecDeque::new()),
            concurrency,
            progress,
        })
    }

    #[must_use]
    pub const fn max_body_bytes(&self) -> usize {
        self.config.max_body_bytes
    }

    pub(crate) const fn requires_password(&self) -> bool {
        self.password_hash.is_some()
    }

    pub(crate) fn authorize_password(&self, presented: Option<&str>) -> Result<(), UploadError> {
        let Some(expected) = self.password_hash else {
            return Ok(());
        };
        let Some(presented) = presented else {
            return Err(UploadError::Unauthorized);
        };
        if !bool::from(password_hash(presented).ct_eq(&expected)) {
            return Err(UploadError::Unauthorized);
        }
        Ok(())
    }

    pub(crate) async fn begin_request(&self) -> Result<OwnedSemaphorePermit, UploadError> {
        self.enforce_rate()?;
        Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| UploadError::Busy)
    }

    pub(crate) async fn stage_field(
        &self,
        file_name: &str,
        field: &mut axum::extract::multipart::Field<'_>,
        request_bytes: &mut u64,
    ) -> Result<PreparedUpload, UploadError> {
        let safe_name = validate_file_name(file_name)?;
        if fs::canonicalize(&self.config.output_root)? != self.canonical_root
            || fs::canonicalize(&self.staging_root)? != self.staging_root
        {
            return Err(UploadError::UnsafeOutputRoot);
        }
        let staging = self
            .staging_root
            .join(format!("{}.part", Uuid::now_v7().simple()));
        let mut guard = StagingGuard::new(staging.clone());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .await?;
        let mut file_bytes = 0_u64;
        let mut last_reported = 0_u64;
        while let Some(chunk) = field.chunk().await.map_err(UploadError::Multipart)? {
            let chunk_len = u64::try_from(chunk.len()).map_err(|_| UploadError::FileTooLarge)?;
            file_bytes = file_bytes
                .checked_add(chunk_len)
                .ok_or(UploadError::FileTooLarge)?;
            *request_bytes = request_bytes
                .checked_add(chunk_len)
                .ok_or(UploadError::TotalTooLarge)?;
            if file_bytes > self.config.max_file_bytes {
                return Err(UploadError::FileTooLarge);
            }
            if *request_bytes > self.config.max_total_bytes {
                return Err(UploadError::TotalTooLarge);
            }
            file.write_all(&chunk).await?;
            if file_bytes.saturating_sub(last_reported) >= 1024 * 1024 {
                last_reported = file_bytes;
                if let Some(progress) = &self.progress {
                    let _ = progress.try_send(UploadProgress {
                        file_name: safe_name.clone(),
                        file_bytes,
                        request_bytes: *request_bytes,
                    });
                }
            }
        }
        if let Some(progress) = &self.progress {
            let _ = progress.try_send(UploadProgress {
                file_name: safe_name.clone(),
                file_bytes,
                request_bytes: *request_bytes,
            });
        }
        file.sync_all().await?;
        drop(file);
        guard.keep_for_commit();
        Ok(PreparedUpload {
            file_name: safe_name,
            bytes: file_bytes,
            staging,
            cleanup: guard,
        })
    }

    pub(crate) async fn commit(
        &self,
        mut upload: PreparedUpload,
    ) -> Result<CompletedUpload, UploadError> {
        let root = self.config.output_root.clone();
        let canonical_root = self.canonical_root.clone();
        let staging = upload.staging.clone();
        let name = upload.file_name.clone();
        let conflict = self.config.conflict;
        let destination = tokio::task::spawn_blocking(move || {
            if fs::canonicalize(&root)? != canonical_root {
                return Err(UploadError::UnsafeOutputRoot);
            }
            let relative = RelativePath::parse(&name)?;
            let destination = resolve_destination(&root, &relative, conflict)?;
            let Some(destination) = destination else {
                fs::remove_file(&staging)?;
                return Ok(None);
            };
            atomic_move(
                &staging,
                &destination,
                conflict == ConflictPolicy::Overwrite,
            )?;
            Ok::<_, UploadError>(Some(destination))
        })
        .await
        .map_err(|_| UploadError::Internal)??;
        upload.cleanup.disarm();
        Ok(CompletedUpload {
            file_name: destination
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_owned),
            bytes: upload.bytes,
            skipped: destination.is_none(),
        })
    }

    fn enforce_rate(&self) -> Result<(), UploadError> {
        let now = Instant::now();
        let cutoff = now.checked_sub(self.config.rate_window).unwrap_or(now);
        let mut recent = self
            .recent_uploads
            .lock()
            .map_err(|_| UploadError::Internal)?;
        while recent.front().is_some_and(|seen| *seen <= cutoff) {
            recent.pop_front();
        }
        if recent.len() >= self.config.max_uploads_per_window {
            return Err(UploadError::RateLimited);
        }
        recent.push_back(now);
        Ok(())
    }
}

impl fmt::Debug for UploadConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadConfig")
            .field("output_root", &"[REDACTED]")
            .field("max_file_bytes", &self.max_file_bytes)
            .field("max_total_bytes", &self.max_total_bytes)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("conflict", &self.conflict)
            .field("max_uploads_per_window", &self.max_uploads_per_window)
            .field("rate_window", &self.rate_window)
            .field("max_concurrent_uploads", &self.max_concurrent_uploads)
            .finish()
    }
}

impl fmt::Debug for UploadService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadService")
            .field("config", &self.config)
            .field("canonical_root", &"[REDACTED]")
            .field("staging_root", &"[REDACTED]")
            .field("password_hash", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

pub(crate) struct PreparedUpload {
    file_name: String,
    bytes: u64,
    staging: PathBuf,
    cleanup: StagingGuard,
}

pub(crate) struct CompletedUpload {
    pub file_name: Option<String>,
    pub bytes: u64,
    pub skipped: bool,
}

struct StagingGuard {
    path: PathBuf,
    active: bool,
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, active: true }
    }

    fn keep_for_commit(&mut self) {
        // The guard remains active; this method documents ownership transfer to PreparedUpload.
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn validate_config(config: &UploadConfig) -> Result<(), UploadError> {
    if config.max_file_bytes == 0
        || config.max_total_bytes == 0
        || config.max_body_bytes == 0
        || config.max_file_bytes > config.max_total_bytes
        || config.max_uploads_per_window == 0
        || config.rate_window.is_zero()
        || config.max_concurrent_uploads == 0
        || matches!(config.conflict, ConflictPolicy::Ask)
    {
        return Err(UploadError::InvalidConfig);
    }
    Ok(())
}

fn validate_file_name(source: &str) -> Result<String, UploadError> {
    if source.is_empty()
        || source.contains('/')
        || source.contains('\\')
        || Path::new(source).file_name().and_then(|name| name.to_str()) != Some(source)
    {
        return Err(UploadError::InvalidFileName);
    }
    RelativePath::parse(source).map_err(|_| UploadError::InvalidFileName)?;
    Ok(source.to_owned())
}

fn password_hash(password: &str) -> [u8; 32] {
    blake3::derive_key("quick-share/web/upload-password/v1", password.as_bytes())
}

#[derive(Debug, Error)]
pub enum UploadError {
    #[error("upload configuration is invalid")]
    InvalidConfig,
    #[error("upload output root is unsafe or changed")]
    UnsafeOutputRoot,
    #[error("upload access is unauthorized")]
    Unauthorized,
    #[error("upload rate limit reached")]
    RateLimited,
    #[error("all upload workers are busy")]
    Busy,
    #[error("multipart upload is invalid")]
    Multipart(#[source] axum::extract::multipart::MultipartError),
    #[error("upload filename is invalid")]
    InvalidFileName,
    #[error("uploaded file exceeds its byte limit")]
    FileTooLarge,
    #[error("upload request exceeds its total byte limit")]
    TotalTooLarge,
    #[error("upload service internal state is unavailable")]
    Internal,
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("upload filesystem operation failed")]
    Io(#[from] io::Error),
}
