//! Typed configuration loading and legacy-safe migration.

use quick_share_platform::{AppDirs, FileSensitivity, StorageError, atomic_write};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf};
use thiserror::Error;

/// Fully merged application configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Device display settings.
    pub device: DeviceConfig,
    /// Receive defaults.
    pub receive: ReceiveConfig,
    /// Link-local discovery behavior.
    pub discovery: DiscoveryConfig,
    /// Listener behavior.
    pub network: NetworkConfig,
    /// Direct transfer limits.
    pub transfer: TransferConfig,
    /// Traditional Web mode.
    pub web: WebConfig,
}

/// Device display configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceConfig {
    /// Untrusted display name announced to peers.
    pub name: String,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            name: "quick-share".to_owned(),
        }
    }
}

/// Receiver behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReceiveConfig {
    /// Destination directory.
    pub output: PathBuf,
    /// Whether trusted devices auto-accept.
    pub trusted_policy: TrustedPolicy,
    /// Default destination conflict behavior.
    pub conflict: ConflictPolicy,
}

impl Default for ReceiveConfig {
    fn default() -> Self {
        Self {
            output: PathBuf::new(),
            trusted_policy: TrustedPolicy::Auto,
            conflict: ConflictPolicy::Rename,
        }
    }
}

/// Confirmation behavior for trusted devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustedPolicy {
    /// Automatically accept within configured limits.
    Auto,
    /// Require confirmation every time.
    Confirm,
}

/// Existing destination behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Generate a non-conflicting name.
    Rename,
    /// Ask on an interactive terminal.
    Ask,
    /// Skip existing entries.
    Skip,
    /// Explicitly overwrite.
    Overwrite,
    /// Fail the transfer.
    Error,
}

/// mDNS scanning and interface filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscoveryConfig {
    /// Scan duration in milliseconds.
    pub timeout_ms: u64,
    /// Whether virtual interfaces may be announced.
    pub include_virtual: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 1_800,
            include_virtual: false,
        }
    }
}

/// Listener defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Zero requests automatic port selection.
    pub port: u16,
    /// `lan` selects filtered LAN interfaces.
    pub bind: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            port: 0,
            bind: "lan".to_owned(),
        }
    }
}

/// Direct transfer resource limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransferConfig {
    /// Chunk bytes.
    pub chunk_size: u32,
    /// Concurrent file streams.
    pub concurrent_files: usize,
    /// Concurrent receive tasks.
    pub max_receive_tasks: usize,
    /// Retry attempts for retryable failures.
    pub retry_count: u8,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024,
            concurrent_files: 4,
            max_receive_tasks: 1,
            retry_count: 3,
        }
    }
}

/// Traditional Web mode defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    /// Service lifetime such as `5m`, `30s`, or `1h`.
    pub timeout: String,
    /// Maximum counted download sessions.
    pub max_downloads: u32,
    /// Whether browser uploads are enabled.
    pub upload: bool,
    /// Explicit plaintext opt-in. Default is false.
    pub allow_http: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            timeout: "5m".to_owned(),
            max_downloads: 10,
            upload: false,
            allow_http: false,
        }
    }
}

/// Highest-priority values supplied by the CLI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigOverrides {
    /// Override receive output.
    pub receive_output: Option<PathBuf>,
    /// Override discovery timeout.
    pub discovery_timeout_ms: Option<u64>,
    /// Override Web download limit.
    pub web_max_downloads: Option<u32>,
    /// Explicitly opt into plaintext HTTP.
    pub web_allow_http: Option<bool>,
}

/// Merges defaults, TOML, environment and CLI values.
#[derive(Debug, Clone)]
pub struct ConfigLoader {
    dirs: AppDirs,
}

impl ConfigLoader {
    /// Creates a loader using the supplied platform directories.
    #[must_use]
    pub const fn new(dirs: AppDirs) -> Self {
        Self { dirs }
    }

    /// Loads the standard `config.toml`, or defaults when it does not exist.
    pub fn load(
        &self,
        environment: &BTreeMap<String, String>,
        overrides: &ConfigOverrides,
    ) -> Result<AppConfig, ConfigError> {
        let path = self.dirs.config_dir().join("config.toml");
        match fs::read_to_string(path) {
            Ok(source) => self.load_from_str(Some(&source), environment, overrides),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.load_from_str(None, environment, overrides)
            }
            Err(error) => Err(ConfigError::Io(error)),
        }
    }

    /// Atomically saves a validated `config.toml`.
    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        validate(config)?;
        let source = toml::to_string_pretty(config).map_err(ConfigError::TomlSerialize)?;
        atomic_write(
            self.dirs.config_dir().join("config.toml"),
            source.as_bytes(),
            FileSensitivity::Normal,
        )?;
        Ok(())
    }

    /// Loads configuration from optional TOML text and deterministic environment input.
    pub fn load_from_str(
        &self,
        source: Option<&str>,
        environment: &BTreeMap<String, String>,
        overrides: &ConfigOverrides,
    ) -> Result<AppConfig, ConfigError> {
        let mut config = match source {
            Some(source) => toml::from_str(source).map_err(ConfigError::Toml)?,
            None => AppConfig::default(),
        };
        if config.receive.output.as_os_str().is_empty() {
            config.receive.output = self.dirs.download_dir().to_path_buf();
        }
        self.apply_environment(&mut config, environment)?;
        self.apply_overrides(&mut config, overrides);
        validate(&config)?;
        Ok(config)
    }

    /// Migrates only the old safe `last_dir` preference.
    pub fn migrate_legacy_json(&self, source: &str) -> Result<AppConfig, ConfigError> {
        #[derive(Deserialize)]
        struct LegacyConfig {
            #[serde(default)]
            last_dir: Option<PathBuf>,
        }
        let legacy: LegacyConfig = serde_json::from_str(source).map_err(ConfigError::LegacyJson)?;
        let mut config = AppConfig::default();
        config.receive.output = legacy
            .last_dir
            .unwrap_or_else(|| self.dirs.download_dir().to_path_buf());
        Ok(config)
    }

    fn apply_environment(
        &self,
        config: &mut AppConfig,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), ConfigError> {
        for (key, value) in environment {
            match key.as_str() {
                "QUICK_SHARE__RECEIVE__OUTPUT" => {
                    config.receive.output = PathBuf::from(value);
                }
                "QUICK_SHARE__DISCOVERY__TIMEOUT_MS" => {
                    config.discovery.timeout_ms = parse_environment(key, value)?;
                }
                "QUICK_SHARE__WEB__MAX_DOWNLOADS" => {
                    config.web.max_downloads = parse_environment(key, value)?;
                }
                "QUICK_SHARE__WEB__ALLOW_HTTP" => {
                    config.web.allow_http = parse_environment(key, value)?;
                }
                _ if key.starts_with("QUICK_SHARE__") => {
                    return Err(ConfigError::UnknownEnvironment(key.clone()));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn apply_overrides(&self, config: &mut AppConfig, overrides: &ConfigOverrides) {
        if let Some(value) = &overrides.receive_output {
            config.receive.output.clone_from(value);
        }
        if let Some(value) = overrides.discovery_timeout_ms {
            config.discovery.timeout_ms = value;
        }
        if let Some(value) = overrides.web_max_downloads {
            config.web.max_downloads = value;
        }
        if let Some(value) = overrides.web_allow_http {
            config.web.allow_http = value;
        }
    }
}

fn parse_environment<T>(key: &str, value: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error: T::Err| ConfigError::Environment {
            key: key.to_owned(),
            message: error.to_string(),
        })
}

fn validate(config: &AppConfig) -> Result<(), ConfigError> {
    if config.device.name.is_empty() || config.device.name.chars().count() > 64 {
        return Err(ConfigError::Validation(
            "device.name must contain 1 to 64 characters".to_owned(),
        ));
    }
    if config.discovery.timeout_ms == 0 {
        return Err(ConfigError::Validation(
            "discovery.timeout_ms must be positive".to_owned(),
        ));
    }
    if config.web.max_downloads == 0 {
        return Err(ConfigError::Validation(
            "web.max_downloads must be positive".to_owned(),
        ));
    }
    if !valid_duration(&config.web.timeout) {
        return Err(ConfigError::Validation(
            "web.timeout must be a positive duration ending in s, m, or h".to_owned(),
        ));
    }
    if config.transfer.concurrent_files == 0 || config.transfer.max_receive_tasks == 0 {
        return Err(ConfigError::Validation(
            "transfer concurrency must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn valid_duration(value: &str) -> bool {
    let Some((suffix_index, suffix)) = value.char_indices().next_back() else {
        return false;
    };
    if !matches!(suffix, 's' | 'm' | 'h') {
        return false;
    }
    value[..suffix_index]
        .parse::<u64>()
        .is_ok_and(|amount| amount > 0)
}

/// Typed configuration failures.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// TOML structure or value failed to decode.
    #[error("invalid TOML configuration: {0}")]
    Toml(#[source] toml::de::Error),
    /// TOML serialization failed.
    #[error("failed to serialize TOML configuration: {0}")]
    TomlSerialize(#[source] toml::ser::Error),
    /// Legacy JSON failed to decode.
    #[error("invalid legacy JSON configuration: {0}")]
    LegacyJson(#[source] serde_json::Error),
    /// A known environment value failed to parse.
    #[error("invalid environment variable {key}: {message}")]
    Environment {
        /// Environment variable name.
        key: String,
        /// Parser context without secret contents.
        message: String,
    },
    /// Unknown namespaced variables fail instead of being silently ignored.
    #[error("unknown Quick Share environment variable {0}")]
    UnknownEnvironment(String),
    /// Cross-field semantic validation failed.
    #[error("invalid configuration: {0}")]
    Validation(String),
    /// Configuration read failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Atomic configuration persistence failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
}
