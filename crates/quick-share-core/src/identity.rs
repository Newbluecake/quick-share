//! Persistent Noise static identity and trusted-device storage.

use quick_share_platform::{FileSensitivity, StorageError, atomic_write, atomic_write_new};
use quick_share_protocol::DeviceId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, fs, io, path::PathBuf};
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

const IDENTITY_VERSION: u8 = 1;
const TRUST_VERSION: u8 = 1;

/// Long-term device identity. Debug output never exposes its private key.
pub struct DeviceIdentity {
    private_key: StaticSecret,
    public_key: [u8; 32],
    device_id: DeviceId,
}

impl DeviceIdentity {
    /// X25519 public key used for Noise pinning.
    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Stable ID derived from the complete public key.
    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        self.device_id.clone()
    }

    /// Restricts private-key access to a caller-supplied closure.
    pub fn with_private_key<R>(&self, callback: impl FnOnce(&StaticSecret) -> R) -> R {
        callback(&self.private_key)
    }
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceIdentity")
            .field("private_key", &"[REDACTED]")
            .field("public_key", &"[REDACTED]")
            .field("device_id", &self.device_id)
            .finish()
    }
}

/// Loads or creates a stable identity file.
#[derive(Debug, Clone)]
pub struct IdentityStore {
    path: PathBuf,
}

impl IdentityStore {
    /// Creates an identity store at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Loads an existing identity or atomically creates a new one.
    pub fn load_or_create(&self) -> Result<DeviceIdentity, IdentityError> {
        if self.path.exists() {
            return self.load();
        }
        let private_key = StaticSecret::random();
        let public_key = PublicKey::from(&private_key).to_bytes();
        let persisted = IdentityFile {
            version: IDENTITY_VERSION,
            private_key: hex::encode(private_key.to_bytes()),
            public_key: hex::encode(public_key),
        };
        let bytes = serde_json::to_vec_pretty(&persisted)
            .map_err(|error| IdentityError::InvalidFile(error.to_string()))?;
        match atomic_write_new(&self.path, &bytes, FileSensitivity::Private) {
            Ok(()) => build_identity(private_key, public_key),
            Err(StorageError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => {
                self.load()
            }
            Err(error) => Err(IdentityError::Storage(error)),
        }
    }

    fn load(&self) -> Result<DeviceIdentity, IdentityError> {
        let persisted: IdentityFile = serde_json::from_slice(&fs::read(&self.path)?)
            .map_err(|error| IdentityError::InvalidFile(error.to_string()))?;
        if persisted.version != IDENTITY_VERSION {
            return Err(IdentityError::InvalidFile(format!(
                "unsupported identity version {}",
                persisted.version
            )));
        }
        let private_key = StaticSecret::from(decode_key("privateKey", &persisted.private_key)?);
        let public_key = decode_key("publicKey", &persisted.public_key)?;
        if PublicKey::from(&private_key).to_bytes() != public_key {
            return Err(IdentityError::InvalidFile(
                "public key does not match private key".to_owned(),
            ));
        }
        build_identity(private_key, public_key)
    }
}

fn build_identity(
    private_key: StaticSecret,
    public_key: [u8; 32],
) -> Result<DeviceIdentity, IdentityError> {
    let device_id = derive_device_id(&public_key).map_err(IdentityError::InvalidFile)?;
    Ok(DeviceIdentity {
        private_key,
        public_key,
        device_id,
    })
}

/// Derives the stable QSP device ID from a complete X25519 static public key.
pub fn device_id_from_public_key(public_key: &[u8; 32]) -> Result<DeviceId, IdentityError> {
    derive_device_id(public_key).map_err(IdentityError::InvalidFile)
}

fn derive_device_id(public_key: &[u8; 32]) -> Result<DeviceId, String> {
    let mut id = [0_u8; 16];
    id.copy_from_slice(&blake3::hash(public_key).as_bytes()[..16]);
    DeviceId::parse(format!("qs_{}", hex::encode(id))).map_err(|error| error.to_string())
}

fn decode_key(field: &str, value: &str) -> Result<[u8; 32], IdentityError> {
    let decoded = hex::decode(value)
        .map_err(|error| IdentityError::InvalidFile(format!("invalid {field}: {error}")))?;
    decoded
        .try_into()
        .map_err(|_| IdentityError::InvalidFile(format!("{field} must contain 32 bytes")))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityFile {
    version: u8,
    private_key: String,
    public_key: String,
}

/// A pinned peer identity.
#[derive(Clone, PartialEq, Eq)]
pub struct TrustedDevice {
    /// Stable public-key-derived identifier.
    pub device_id: DeviceId,
    /// Local user-assigned name.
    pub name: String,
    /// Complete pinned static public key.
    pub public_key: [u8; 32],
}

impl fmt::Debug for TrustedDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrustedDevice")
            .field("device_id", &self.device_id)
            .field("name", &self.name)
            .field("public_key", &"[REDACTED]")
            .finish()
    }
}

/// Result of checking an advertised ID and complete static key against local pinning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStatus {
    /// No pinned record exists for the advertised ID.
    Unknown,
    /// ID and complete public key match the pin.
    Trusted(TrustedDevice),
    /// Advertised ID exists but the presented static key differs.
    KeyMismatch,
}

/// Atomic trusted-device registry.
#[derive(Debug, Clone)]
pub struct TrustedDeviceStore {
    path: PathBuf,
}

impl TrustedDeviceStore {
    /// Creates a registry at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns all trusted devices.
    pub fn list(&self) -> Result<Vec<TrustedDevice>, TrustStoreError> {
        self.load()?
            .devices
            .into_iter()
            .map(TrustedDevice::try_from)
            .collect()
    }

    /// Finds a pinned device by case-insensitive local name for impersonation warnings.
    pub fn find_by_name(&self, name: &str) -> Result<Option<TrustedDevice>, TrustStoreError> {
        let key = name.to_lowercase();
        Ok(self
            .list()?
            .into_iter()
            .find(|device| device.name.to_lowercase() == key))
    }

    /// Checks both advertised ID and complete static public key.
    pub fn check(
        &self,
        id: &DeviceId,
        presented_key: &[u8; 32],
    ) -> Result<TrustStatus, TrustStoreError> {
        let device = self
            .list()?
            .into_iter()
            .find(|device| &device.device_id == id);
        Ok(match device {
            Some(device) if &device.public_key == presented_key => TrustStatus::Trusted(device),
            Some(_) => TrustStatus::KeyMismatch,
            None => TrustStatus::Unknown,
        })
    }

    /// Adds or updates a locally represented identity.
    pub fn trust(&self, identity: &DeviceIdentity, name: &str) -> Result<(), TrustStoreError> {
        self.trust_peer(identity.device_id(), name, identity.public_key())
    }

    /// Atomically pins an authenticated remote static public key.
    pub fn trust_peer(
        &self,
        device_id: DeviceId,
        name: &str,
        public_key: [u8; 32],
    ) -> Result<(), TrustStoreError> {
        validate_name(name)?;
        let derived = derive_device_id(&public_key).map_err(TrustStoreError::InvalidFile)?;
        if derived != device_id {
            return Err(TrustStoreError::InvalidFile(
                "deviceId does not match authenticated publicKey".to_owned(),
            ));
        }
        let mut file = self.load()?;
        let device = TrustedDeviceFile {
            device_id,
            name: name.to_owned(),
            public_key: hex::encode(public_key),
        };
        if let Some(existing) = file
            .devices
            .iter_mut()
            .find(|item| item.device_id == device.device_id)
        {
            *existing = device;
        } else {
            file.devices.push(device);
        }
        self.save(&file)
    }

    /// Changes a local trusted-device name.
    pub fn rename(&self, id: &DeviceId, name: &str) -> Result<(), TrustStoreError> {
        validate_name(name)?;
        let mut file = self.load()?;
        let device = file
            .devices
            .iter_mut()
            .find(|item| &item.device_id == id)
            .ok_or_else(|| TrustStoreError::NotFound(id.to_string()))?;
        device.name = name.to_owned();
        self.save(&file)
    }

    /// Removes an ID and reports whether it existed.
    pub fn remove(&self, id: &DeviceId) -> Result<bool, TrustStoreError> {
        let mut file = self.load()?;
        let old_len = file.devices.len();
        file.devices.retain(|item| &item.device_id != id);
        let removed = file.devices.len() != old_len;
        if removed {
            self.save(&file)?;
        }
        Ok(removed)
    }

    fn load(&self) -> Result<TrustedDevicesFile, TrustStoreError> {
        if !self.path.exists() {
            return Ok(TrustedDevicesFile {
                version: TRUST_VERSION,
                devices: Vec::new(),
            });
        }
        let source = fs::read_to_string(&self.path)?;
        let file: TrustedDevicesFile = toml::from_str(&source)
            .map_err(|error| TrustStoreError::InvalidFile(error.to_string()))?;
        if file.version != TRUST_VERSION {
            return Err(TrustStoreError::InvalidFile(format!(
                "unsupported trust store version {}",
                file.version
            )));
        }
        let mut ids = BTreeSet::new();
        for device in &file.devices {
            validate_name(&device.name)?;
            let public_key = decode_trusted_key(&device.public_key)?;
            let derived = derive_device_id(&public_key).map_err(TrustStoreError::InvalidFile)?;
            if derived != device.device_id {
                return Err(TrustStoreError::InvalidFile(
                    "deviceId does not match publicKey".to_owned(),
                ));
            }
            if !ids.insert(device.device_id.clone()) {
                return Err(TrustStoreError::InvalidFile(
                    "trusted-device file contains duplicate deviceId values".to_owned(),
                ));
            }
        }
        Ok(file)
    }

    fn save(&self, file: &TrustedDevicesFile) -> Result<(), TrustStoreError> {
        let source = toml::to_string_pretty(file)
            .map_err(|error| TrustStoreError::InvalidFile(error.to_string()))?;
        atomic_write(&self.path, source.as_bytes(), FileSensitivity::Normal)?;
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), TrustStoreError> {
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(TrustStoreError::InvalidName);
    }
    Ok(())
}

impl TryFrom<TrustedDeviceFile> for TrustedDevice {
    type Error = TrustStoreError;

    fn try_from(value: TrustedDeviceFile) -> Result<Self, Self::Error> {
        let public_key = decode_trusted_key(&value.public_key)?;
        Ok(Self {
            device_id: value.device_id,
            name: value.name,
            public_key,
        })
    }
}

fn decode_trusted_key(value: &str) -> Result<[u8; 32], TrustStoreError> {
    let decoded =
        hex::decode(value).map_err(|error| TrustStoreError::InvalidFile(error.to_string()))?;
    decoded
        .try_into()
        .map_err(|_| TrustStoreError::InvalidFile("publicKey must contain 32 bytes".to_owned()))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedDevicesFile {
    version: u8,
    devices: Vec<TrustedDeviceFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedDeviceFile {
    device_id: DeviceId,
    name: String,
    public_key: String,
}

/// Identity persistence failure.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// Existing file is corrupt or cryptographically inconsistent.
    #[error("invalid identity file: {0}")]
    InvalidFile(String),
    /// File read failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Atomic persistence or permission enforcement failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Trusted-device persistence failure.
#[derive(Debug, Error)]
pub enum TrustStoreError {
    /// File structure, key or version is invalid.
    #[error("invalid trusted-device file: {0}")]
    InvalidFile(String),
    /// Local name is outside the accepted bounds.
    #[error("trusted-device name must contain 1 to 64 characters")]
    InvalidName,
    /// Requested ID does not exist.
    #[error("trusted device {0} was not found")]
    NotFound(String),
    /// File read failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Atomic persistence failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
}
