//! Receiver-local per-device destination preferences and active transfer bindings.

use crate::{
    destination::{DestinationPlan, DestinationPlanError},
    identity::{TrustStatus, TrustedDevice, device_id_from_public_key},
};
use quick_share_platform::{AppDirs, FileSensitivity, StorageError, atomic_write};
use quick_share_protocol::{DeviceId, TransferId};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
use thiserror::Error;

const DESTINATION_STORE_VERSION: u8 = 1;
const BINDING_STORE_VERSION: u8 = 1;

/// Atomic mapping from complete trusted identities to their last confirmed directory.
#[derive(Clone)]
pub struct ReceiveDestinationStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl ReceiveDestinationStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Returns a path only when both the stable ID and complete pinned key match.
    pub fn get(&self, device: &TrustedDevice) -> Result<Option<PathBuf>, ReceiveStateError> {
        validate_trusted_device(device)?;
        let _guard = self.lock()?;
        let file = self.load()?;
        Ok(file
            .destinations
            .into_iter()
            .find(|entry| {
                entry.device_id == device.device_id
                    && decode_key(&entry.public_key).is_ok_and(|key| key == device.public_key)
            })
            .map(|entry| entry.path))
    }

    /// Uses this trusted device's remembered path or the platform Downloads directory.
    pub fn preferred_directory(
        &self,
        device: &TrustedDevice,
        dirs: &AppDirs,
    ) -> Result<PathBuf, ReceiveStateError> {
        Ok(self
            .get(device)?
            .unwrap_or_else(|| dirs.download_dir().to_path_buf()))
    }

    /// Persists an existing absolute directory for an already trusted identity.
    pub fn remember(
        &self,
        trust_status: &TrustStatus,
        path: impl AsRef<Path>,
    ) -> Result<(), ReceiveStateError> {
        let TrustStatus::Trusted(device) = trust_status else {
            return Err(ReceiveStateError::UntrustedDevice);
        };
        validate_trusted_device(device)?;
        let path = path.as_ref();
        if !path.is_absolute() || !path.is_dir() {
            return Err(ReceiveStateError::InvalidDestination(path.to_path_buf()));
        }
        let _guard = self.lock()?;
        let mut file = self.load()?;
        if let Some(existing) = file
            .destinations
            .iter_mut()
            .find(|entry| entry.device_id == device.device_id)
        {
            if decode_key(&existing.public_key)? != device.public_key {
                return Err(ReceiveStateError::IdentityKeyMismatch(
                    device.device_id.clone(),
                ));
            }
            existing.path = path.to_path_buf();
        } else {
            file.destinations.push(DeviceDestinationFile {
                device_id: device.device_id.clone(),
                public_key: hex::encode(device.public_key),
                path: path.to_path_buf(),
            });
        }
        self.save(&file)
    }

    pub fn forget(&self, device_id: &DeviceId) -> Result<bool, ReceiveStateError> {
        let _guard = self.lock()?;
        let mut file = self.load()?;
        let old_len = file.destinations.len();
        file.destinations
            .retain(|entry| &entry.device_id != device_id);
        let removed = file.destinations.len() != old_len;
        if removed {
            self.save(&file)?;
        }
        Ok(removed)
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>, ReceiveStateError> {
        self.lock
            .lock()
            .map_err(|_| ReceiveStateError::LockPoisoned)
    }

    fn load(&self) -> Result<ReceiveDestinationsFile, ReceiveStateError> {
        let source = match fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ReceiveDestinationsFile {
                    version: DESTINATION_STORE_VERSION,
                    destinations: Vec::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let file: ReceiveDestinationsFile = toml::from_str(&source)
            .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?;
        if file.version != DESTINATION_STORE_VERSION {
            return Err(ReceiveStateError::InvalidFile(format!(
                "unsupported receive destination version {}",
                file.version
            )));
        }
        let mut ids = BTreeSet::new();
        for entry in &file.destinations {
            let key = decode_key(&entry.public_key)?;
            if device_id_from_public_key(&key)
                .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?
                != entry.device_id
                || !ids.insert(entry.device_id.clone())
            {
                return Err(ReceiveStateError::InvalidFile(
                    "receive destinations contain an invalid or duplicate identity".to_owned(),
                ));
            }
        }
        Ok(file)
    }

    fn save(&self, file: &ReceiveDestinationsFile) -> Result<(), ReceiveStateError> {
        let source = toml::to_string_pretty(file)
            .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?;
        atomic_write(&self.path, source.as_bytes(), FileSensitivity::Normal)?;
        Ok(())
    }
}

impl fmt::Debug for ReceiveDestinationStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiveDestinationStore")
            .field("path", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiveDestinationsFile {
    version: u8,
    #[serde(default)]
    destinations: Vec<DeviceDestinationFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceDestinationFile {
    device_id: DeviceId,
    public_key: String,
    path: PathBuf,
}

/// Durable root and conflict plan for one accepted resumable transfer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiveBinding {
    pub transfer_id: TransferId,
    pub sender_device_id: DeviceId,
    pub manifest_digest: [u8; 32],
    pub output_root: PathBuf,
    pub destination_plan: DestinationPlan,
}

impl fmt::Debug for ReceiveBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiveBinding")
            .field("transfer_id", &self.transfer_id)
            .field("sender_device_id", &self.sender_device_id)
            .field("manifest_digest", &"[REDACTED]")
            .field("output_root", &"[REDACTED]")
            .field("destination_plan", &self.destination_plan)
            .finish()
    }
}

impl ReceiveBinding {
    fn validate(&self) -> Result<(), ReceiveStateError> {
        if !self.output_root.is_absolute() {
            return Err(ReceiveStateError::InvalidDestination(
                self.output_root.clone(),
            ));
        }
        self.destination_plan.validate()?;
        Ok(())
    }
}

/// Atomic active-transfer registry. Existing IDs are immutable and idempotent.
#[derive(Clone)]
pub struct ReceiveBindingStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl ReceiveBindingStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn bind(&self, binding: ReceiveBinding) -> Result<(), ReceiveStateError> {
        binding.validate()?;
        if !binding.output_root.is_dir() {
            return Err(ReceiveStateError::InvalidDestination(
                binding.output_root.clone(),
            ));
        }
        let _guard = self.lock()?;
        let mut file = self.load()?;
        if let Some(existing) = file
            .bindings
            .iter()
            .find(|entry| entry.transfer_id == binding.transfer_id)
        {
            return if existing == &binding {
                Ok(())
            } else {
                Err(ReceiveStateError::BindingConflict(binding.transfer_id))
            };
        }
        file.bindings.push(binding);
        self.save(&file)
    }

    pub fn get(
        &self,
        transfer_id: TransferId,
    ) -> Result<Option<ReceiveBinding>, ReceiveStateError> {
        let _guard = self.lock()?;
        Ok(self
            .load()?
            .bindings
            .into_iter()
            .find(|binding| binding.transfer_id == transfer_id))
    }

    pub fn list(&self) -> Result<Vec<ReceiveBinding>, ReceiveStateError> {
        let _guard = self.lock()?;
        Ok(self.load()?.bindings)
    }

    /// Atomically replaces only the conflict plan while preserving immutable routing fields.
    pub fn update_plan(
        &self,
        transfer_id: TransferId,
        destination_plan: DestinationPlan,
    ) -> Result<ReceiveBinding, ReceiveStateError> {
        destination_plan.validate()?;
        let _guard = self.lock()?;
        let mut file = self.load()?;
        let binding = file
            .bindings
            .iter_mut()
            .find(|binding| binding.transfer_id == transfer_id)
            .ok_or(ReceiveStateError::BindingNotFound(transfer_id))?;
        binding.destination_plan = destination_plan;
        let updated = binding.clone();
        self.save(&file)?;
        Ok(updated)
    }

    pub fn remove(&self, transfer_id: TransferId) -> Result<bool, ReceiveStateError> {
        let _guard = self.lock()?;
        let mut file = self.load()?;
        let old_len = file.bindings.len();
        file.bindings
            .retain(|binding| binding.transfer_id != transfer_id);
        let removed = file.bindings.len() != old_len;
        if removed {
            self.save(&file)?;
        }
        Ok(removed)
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>, ReceiveStateError> {
        self.lock
            .lock()
            .map_err(|_| ReceiveStateError::LockPoisoned)
    }

    fn load(&self) -> Result<ReceiveBindingsFile, ReceiveStateError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ReceiveBindingsFile {
                    version: BINDING_STORE_VERSION,
                    bindings: Vec::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let file: ReceiveBindingsFile = serde_json::from_slice(&bytes)
            .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?;
        if file.version != BINDING_STORE_VERSION {
            return Err(ReceiveStateError::InvalidFile(format!(
                "unsupported receive binding version {}",
                file.version
            )));
        }
        let mut ids = BTreeSet::new();
        for binding in &file.bindings {
            binding.validate()?;
            if !ids.insert(binding.transfer_id) {
                return Err(ReceiveStateError::InvalidFile(
                    "receive bindings contain duplicate transfer IDs".to_owned(),
                ));
            }
        }
        Ok(file)
    }

    fn save(&self, file: &ReceiveBindingsFile) -> Result<(), ReceiveStateError> {
        let bytes = serde_json::to_vec_pretty(file)
            .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?;
        atomic_write(&self.path, &bytes, FileSensitivity::Normal)?;
        Ok(())
    }
}

impl fmt::Debug for ReceiveBindingStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiveBindingStore")
            .field("path", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiveBindingsFile {
    version: u8,
    #[serde(default)]
    bindings: Vec<ReceiveBinding>,
}

fn validate_trusted_device(device: &TrustedDevice) -> Result<(), ReceiveStateError> {
    let derived = device_id_from_public_key(&device.public_key)
        .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?;
    if derived != device.device_id {
        return Err(ReceiveStateError::IdentityKeyMismatch(
            device.device_id.clone(),
        ));
    }
    Ok(())
}

fn decode_key(value: &str) -> Result<[u8; 32], ReceiveStateError> {
    hex::decode(value)
        .map_err(|error| ReceiveStateError::InvalidFile(error.to_string()))?
        .try_into()
        .map_err(|_| ReceiveStateError::InvalidFile("public key must contain 32 bytes".to_owned()))
}

#[derive(Debug, Error)]
pub enum ReceiveStateError {
    #[error("receive state file is invalid: {0}")]
    InvalidFile(String),
    #[error("receive destination must be an existing absolute directory: {0}")]
    InvalidDestination(PathBuf),
    #[error("receive destination may be saved only for a currently trusted device")]
    UntrustedDevice,
    #[error("receive destination identity key changed for {0}")]
    IdentityKeyMismatch(DeviceId),
    #[error("receive binding already exists with different immutable data: {0:?}")]
    BindingConflict(TransferId),
    #[error("receive binding does not exist: {0:?}")]
    BindingNotFound(TransferId),
    #[error(transparent)]
    DestinationPlan(#[from] DestinationPlanError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("receive state lock is unavailable")]
    LockPoisoned,
    #[error(transparent)]
    Storage(#[from] StorageError),
}
