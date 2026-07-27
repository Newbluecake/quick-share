#![forbid(unsafe_code)]
//! Cross-platform adapters for directories, permissions, clipboard, and atomic files.

pub mod clipboard;
pub mod encoding;
pub mod network;

use directories::{ProjectDirs, UserDirs};
#[cfg(unix)]
use std::fs::File;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, TempPath};
use thiserror::Error;

/// Platform-standard application and user directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    downloads: PathBuf,
}

impl AppDirs {
    /// Resolves standard directories for the current user.
    pub fn discover() -> Result<Self, StorageError> {
        let project = ProjectDirs::from("io", "quick-share", "quick-share")
            .ok_or(StorageError::DirectoriesUnavailable)?;
        let user = UserDirs::new().ok_or(StorageError::DirectoriesUnavailable)?;
        let downloads = user
            .download_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| user.home_dir().join("Downloads"));
        Ok(Self {
            config: project.config_dir().to_path_buf(),
            data: project.data_dir().to_path_buf(),
            cache: project.cache_dir().to_path_buf(),
            downloads,
        })
    }

    /// Creates deterministic isolated directories for tests and embedded use.
    #[must_use]
    pub fn for_test(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
            downloads: root.join("Downloads"),
        }
    }

    /// User configuration directory.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    /// Persistent application data directory.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    /// Disposable cache directory.
    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    /// Default receive directory.
    #[must_use]
    pub fn download_dir(&self) -> &Path {
        &self.downloads
    }
}

/// Controls post-write permissions for an atomically stored file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSensitivity {
    /// Normal user configuration.
    Normal,
    /// Identity private key or equivalent secret material.
    Private,
}

/// Writes a complete sibling temporary file, syncs it, and atomically replaces `path`.
pub fn atomic_write(
    path: impl AsRef<Path>,
    bytes: &[u8],
    sensitivity: FileSensitivity,
) -> Result<(), StorageError> {
    let path = path.as_ref();
    let parent = path.parent().ok_or(StorageError::MissingParent)?;
    let temporary = prepare_temporary(parent, bytes, sensitivity)?;
    temporary
        .persist(path)
        .map_err(|error| StorageError::Io(error.error))?;
    sync_directory(parent)?;
    Ok(())
}

/// Atomically creates `path` and fails with `AlreadyExists` instead of replacing it.
pub fn atomic_write_new(
    path: impl AsRef<Path>,
    bytes: &[u8],
    sensitivity: FileSensitivity,
) -> Result<(), StorageError> {
    let path = path.as_ref();
    let parent = path.parent().ok_or(StorageError::MissingParent)?;
    let temporary = prepare_temporary(parent, bytes, sensitivity)?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| StorageError::Io(error.error))?;
    sync_directory(parent)?;
    Ok(())
}

/// Atomically moves a staged file to `destination`, optionally replacing an existing file.
/// Both paths must reside on the same file system.
pub fn atomic_move(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    replace: bool,
) -> Result<(), StorageError> {
    let destination = destination.as_ref();
    let parent = destination.parent().ok_or(StorageError::MissingParent)?;
    fs::create_dir_all(parent)?;
    let temporary = TempPath::try_from_path(source.as_ref())?;
    let result = if replace {
        temporary.persist(destination)
    } else {
        temporary.persist_noclobber(destination)
    };
    if let Err(error) = result {
        let io_error = error.error;
        error
            .path
            .keep()
            .map_err(|keep_error| StorageError::Io(keep_error.error))?;
        return Err(StorageError::Io(io_error));
    }
    sync_directory(parent)?;
    Ok(())
}

fn prepare_temporary(
    parent: &Path,
    bytes: &[u8],
    sensitivity: FileSensitivity,
) -> Result<NamedTempFile, StorageError> {
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    if sensitivity == FileSensitivity::Private {
        set_private_permissions(temporary.path())?;
    }
    Ok(temporary)
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(windows)]
fn set_private_permissions(path: &Path) -> Result<(), StorageError> {
    use windows_permissions::{
        LocalBox, SecurityDescriptor,
        constants::{SeObjectType, SecurityInformation},
        utilities::current_process_sid,
        wrappers::{ConvertSidToStringSid, SetNamedSecurityInfo},
    };

    let current_sid = current_process_sid()?;
    let sid = ConvertSidToStringSid(&current_sid)?;
    let sddl = format!(
        "D:P(A;;FA;;;{})(A;;FA;;;SY)(A;;FA;;;BA)",
        sid.to_string_lossy()
    );
    let descriptor: LocalBox<SecurityDescriptor> = sddl.parse()?;
    let dacl = descriptor.dacl().ok_or_else(|| {
        StorageError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "private security descriptor has no DACL",
        ))
    })?;
    SetNamedSecurityInfo(
        path,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl | SecurityInformation::ProtectedDacl,
        None,
        None,
        Some(dacl),
        None,
    )?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn set_private_permissions(_path: &Path) -> Result<(), StorageError> {
    Err(StorageError::PrivatePermissionsUnsupported)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

/// Platform storage failures.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Platform standard directories are unavailable.
    #[error("platform user directories are unavailable")]
    DirectoriesUnavailable,
    /// Target path has no parent directory.
    #[error("target path has no parent directory")]
    MissingParent,
    /// Platform cannot enforce private file permissions.
    #[error("private file permissions are unsupported on this platform")]
    PrivatePermissionsUnsupported,
    /// File-system or permissions operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}
