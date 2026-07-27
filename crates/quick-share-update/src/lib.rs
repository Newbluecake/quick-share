#![forbid(unsafe_code)]
//! Signed, bounded, fixed-origin self-update orchestration.

mod checksum;
mod github;
mod replace;
mod signature;

pub use checksum::{ChecksumError, expected_sha256};
pub use github::{GitHubReleaseProvider, is_allowed_redirect, validate_asset_url};
pub use replace::SafeSelfReplacer;
pub use signature::Ed25519ReleaseVerifier;

use async_trait::async_trait;
use semver::Version;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub const GITHUB_OWNER: &str = "Newbluecake";
pub const GITHUB_REPOSITORY: &str = "quick-share";
pub const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS";
pub const CHECKSUM_SIGNATURE_ASSET: &str = "SHA256SUMS.sig";
pub const MAX_RELEASE_METADATA_BYTES: usize = 1024 * 1024;
pub const MAX_CHECKSUMS_BYTES: u64 = 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: u64 = 4096;
pub const MAX_UPDATE_ASSET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
}

impl fmt::Debug for ReleaseAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReleaseAsset")
            .field("name", &self.name)
            .field("download_url", &"[FIXED GITHUB RELEASE ORIGIN]")
            .field("size", &self.size)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub tag: String,
    pub binary: ReleaseAsset,
    pub checksums: ReleaseAsset,
    pub signature: ReleaseAsset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate { current: Version },
    Available(Box<UpdatePlan>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub current: Version,
    pub release: ReleaseInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReceipt {
    pub previous: Version,
    pub installed: Version,
}

#[async_trait]
pub trait ReleaseProvider: Send + Sync {
    async fn release(&self, requested: Option<&Version>) -> Result<ReleaseInfo, UpdateError>;

    async fn download(
        &self,
        asset: &ReleaseAsset,
        destination: &Path,
        limit: u64,
        cancellation: &CancellationToken,
    ) -> Result<(), UpdateError>;
}

pub trait ReleaseSignatureVerifier: Send + Sync {
    fn verify(&self, checksums: &[u8], signature: &[u8]) -> Result<(), UpdateError>;
}

#[async_trait]
pub trait ExecutableReplacer: Send + Sync {
    async fn replace(
        &self,
        current_executable: &Path,
        candidate: &Path,
        expected_version: &Version,
    ) -> Result<(), UpdateError>;
}

pub struct UpdateService<P, V, R> {
    provider: Arc<P>,
    verifier: Arc<V>,
    replacer: Arc<R>,
    current_version: Version,
    current_executable: PathBuf,
}

impl<P, V, R> UpdateService<P, V, R>
where
    P: ReleaseProvider,
    V: ReleaseSignatureVerifier,
    R: ExecutableReplacer,
{
    #[must_use]
    pub fn new(
        provider: Arc<P>,
        verifier: Arc<V>,
        replacer: Arc<R>,
        current_version: Version,
        current_executable: PathBuf,
    ) -> Self {
        Self {
            provider,
            verifier,
            replacer,
            current_version,
            current_executable,
        }
    }

    pub async fn check(&self, requested: Option<&Version>) -> Result<UpdateCheck, UpdateError> {
        let release = self.provider.release(requested).await?;
        if let Some(requested) = requested
            && &release.version != requested
        {
            return Err(UpdateError::UnexpectedVersion {
                expected: requested.clone(),
                actual: release.version,
            });
        }
        if release.version < self.current_version {
            if requested.is_some() {
                return Err(UpdateError::DowngradeRefused {
                    current: self.current_version.clone(),
                    requested: release.version,
                });
            }
            return Ok(UpdateCheck::UpToDate {
                current: self.current_version.clone(),
            });
        }
        if release.version == self.current_version {
            return Ok(UpdateCheck::UpToDate {
                current: self.current_version.clone(),
            });
        }
        Ok(UpdateCheck::Available(Box::new(UpdatePlan {
            current: self.current_version.clone(),
            release,
        })))
    }

    pub async fn install(
        &self,
        plan: &UpdatePlan,
        cancellation: &CancellationToken,
    ) -> Result<UpdateReceipt, UpdateError> {
        if plan.current != self.current_version {
            return Err(UpdateError::StalePlan);
        }
        if plan.release.version < self.current_version {
            return Err(UpdateError::DowngradeRefused {
                current: self.current_version.clone(),
                requested: plan.release.version.clone(),
            });
        }
        if plan.release.version == self.current_version {
            return Err(UpdateError::StalePlan);
        }
        let parent = self
            .current_executable
            .parent()
            .ok_or(UpdateError::ExecutableHasNoParent)?;
        let checksums = CandidateFile::new(parent, "checksums")?;
        self.provider
            .download(
                &plan.release.checksums,
                checksums.path(),
                MAX_CHECKSUMS_BYTES,
                cancellation,
            )
            .await?;
        let checksum_bytes = fs::read(checksums.path())?;
        let signature = CandidateFile::new(parent, "signature")?;
        self.provider
            .download(
                &plan.release.signature,
                signature.path(),
                MAX_SIGNATURE_BYTES,
                cancellation,
            )
            .await?;
        let signature_bytes = fs::read(signature.path())?;
        self.verifier.verify(&checksum_bytes, &signature_bytes)?;
        let expected = expected_sha256(&checksum_bytes, &plan.release.binary.name)?;

        let candidate = CandidateFile::new(parent, executable_suffix())?;
        self.provider
            .download(
                &plan.release.binary,
                candidate.path(),
                MAX_UPDATE_ASSET_BYTES,
                cancellation,
            )
            .await?;
        let actual = sha256_file(candidate.path())?;
        if actual != expected {
            return Err(UpdateError::ChecksumMismatch);
        }
        make_executable(candidate.path(), &self.current_executable)?;
        self.replacer
            .replace(
                &self.current_executable,
                candidate.path(),
                &plan.release.version,
            )
            .await?;
        Ok(UpdateReceipt {
            previous: self.current_version.clone(),
            installed: plan.release.version.clone(),
        })
    }
}

fn sha256_file(path: &Path) -> Result<[u8; 32], UpdateError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest.finalize().into())
}

fn executable_suffix() -> &'static str {
    if cfg!(windows) {
        "candidate.exe"
    } else {
        "candidate"
    }
}

fn make_executable(candidate: &Path, current: &Path) -> Result<(), UpdateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let current_mode = fs::metadata(current)?.permissions().mode() & 0o777;
        fs::set_permissions(candidate, fs::Permissions::from_mode(current_mode | 0o500))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (candidate, current);
    }
    Ok(())
}

struct CandidateFile {
    path: PathBuf,
}

impl CandidateFile {
    fn new(parent: &Path, suffix: &str) -> Result<Self, UpdateError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| UpdateError::Randomness)?;
        let path = parent.join(format!(
            ".quick-share-update-{}-{suffix}",
            hex::encode(random)
        ));
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CandidateFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[must_use]
pub fn current_target() -> Option<&'static str> {
    target_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
}

#[must_use]
pub fn target_for(os: &str, arch: &str, musl: bool) -> Option<&'static str> {
    match (os, arch, musl) {
        ("linux", "x86_64", true) => Some("x86_64-unknown-linux-musl"),
        ("linux", "x86_64", false) => Some("x86_64-unknown-linux-gnu"),
        ("macos", "x86_64", _) => Some("x86_64-apple-darwin"),
        ("macos", "aarch64", _) => Some("aarch64-apple-darwin"),
        ("windows", "x86_64", _) => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

#[must_use]
pub fn asset_name_for_target(target: &str) -> String {
    if target.ends_with("windows-msvc") {
        format!("quick-share-{target}.exe")
    } else {
        format!("quick-share-{target}")
    }
}

#[derive(Error)]
pub enum UpdateError {
    #[error("the current platform or architecture has no published update asset")]
    UnsupportedTarget,
    #[error("release metadata exceeded the hard size limit")]
    MetadataTooLarge,
    #[error("release metadata is invalid: {0}")]
    InvalidMetadata(String),
    #[error("release asset URL is outside the fixed Quick Share GitHub repository")]
    UntrustedAssetUrl,
    #[error("release asset {0} is missing")]
    MissingAsset(String),
    #[error("release asset exceeded the {0}-byte limit")]
    AssetTooLarge(u64),
    #[error("release download was interrupted")]
    DownloadInterrupted,
    #[error("release download returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("release checksum does not match the downloaded executable")]
    ChecksumMismatch,
    #[error("release Ed25519 signature verification failed")]
    SignatureInvalid,
    #[error("requested release {expected} returned unexpected release {actual}")]
    UnexpectedVersion { expected: Version, actual: Version },
    #[error("refusing to downgrade from {current} to {requested}")]
    DowngradeRefused {
        current: Version,
        requested: Version,
    },
    #[error("the update plan is stale and must be checked again")]
    StalePlan,
    #[error("the current executable has no parent directory")]
    ExecutableHasNoParent,
    #[error("the downloaded executable did not start as version {0}")]
    CandidateDidNotStart(Version),
    #[error("executable replacement failed; the previous executable was preserved: {0}")]
    ReplacementFailed(String),
    #[error("failed to restore the previous executable after replacement failure: {0}")]
    RollbackFailed(String),
    #[error("secure randomness is unavailable")]
    Randomness,
    #[error("update was cancelled")]
    Cancelled,
    #[error(transparent)]
    Checksum(#[from] ChecksumError),
    #[error("update I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("update HTTP request failed")]
    Http(#[from] reqwest::Error),
    #[error("release version is invalid: {0}")]
    Version(#[from] semver::Error),
}

impl fmt::Debug for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
