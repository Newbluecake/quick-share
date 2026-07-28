#![forbid(unsafe_code)]
//! Stable command-line contract and application exit-status mapping.

#[cfg(windows)]
mod agent;
pub mod app;
mod configured_discovery;
#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
mod desktop_prompt;
pub mod devices;
pub mod orchestration;
mod remote_receive;
pub mod terminal;

use clap::{ArgAction, ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;

/// Parses argv, including `sc`/`rc` argv[0] shortcuts, into an execution-independent intent.
pub fn parse_intent_from<I, T>(arguments: I) -> Result<CommandIntent, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let arguments: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    Cli::try_parse_from(normalize_shortcut(arguments)).map(CommandIntent::from)
}

fn normalize_shortcut(mut arguments: Vec<OsString>) -> Vec<OsString> {
    if arguments.is_empty() {
        return arguments;
    }
    let stem = Path::new(&arguments[0])
        .file_stem()
        .and_then(|stem| stem.to_str());
    match stem.map(str::to_ascii_lowercase).as_deref() {
        Some("sc") => arguments.insert(1, OsString::from("send")),
        Some("rc") => arguments.insert(1, OsString::from("receive")),
        _ => {}
    }
    arguments
}

#[derive(Debug, Parser)]
#[command(
    name = "quick-share",
    version,
    about = "Quick Share - secure local file sharing",
    arg_required_else_help = true,
    subcommand_required = true
)]
struct Cli {
    /// Increase diagnostics (`-vv` for more detail). Secrets are always redacted.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,
    /// Use an explicit TOML configuration file.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Discover a receiver and securely send files, text, or clipboard content.
    #[command(
        long_about = "Discover a compatible receiver and send selected content securely.\n\nAutomatic mode falls back to Web sharing only when discovery succeeds with zero compatible receivers. After a receiver is selected, rejection, timeout, or transfer failure returns an error and does not fall back to Web mode."
    )]
    Send(SendArgs),
    /// Advertise this device and receive direct transfers.
    Receive(ReceiveArgs),
    /// Run the Windows desktop agent for bidirectional interactive transfers.
    Agent(AgentArgs),
    /// Share selected content through the browser-oriented HTTPS service.
    Serve(ServeArgs),
    /// List and manage pinned trusted devices.
    Devices(DevicesArgs),
    /// Inspect and update non-secret preferences.
    Config(ConfigArgs),
    /// Check for or install a signed Quick Share update.
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
#[command(
    group(
    ArgGroup::new("content")
        .required(true)
        .multiple(false)
        .args(["paths", "text", "clipboard"])
    )
)]
struct SendArgs {
    /// Files or directories to send.
    #[arg(value_name = "PATH", num_args = 1..)]
    paths: Vec<PathBuf>,
    /// Send literal text instead of paths.
    #[arg(long, value_name = "TEXT")]
    text: Option<String>,
    /// Read and send the current clipboard text.
    #[arg(long)]
    clipboard: bool,
    /// Skip discovery and connect only to this device ID or host:port.
    #[arg(long, value_name = "DEVICE_OR_ADDRESS", conflicts_with = "web")]
    peer: Option<String>,
    /// Skip discovery and use traditional Web sharing.
    #[arg(long, conflicts_with = "peer")]
    web: bool,
    /// Resume an interrupted transfer with the exact same content and transfer UUID.
    #[arg(long, value_name = "TRANSFER_ID", conflicts_with = "web")]
    resume: Option<String>,
    /// Follow source symbolic links instead of preserving link metadata.
    #[arg(long)]
    follow_links: bool,
    /// Explicitly permit plaintext HTTP if Web mode is selected or reached by zero-device fallback.
    #[arg(long, conflicts_with = "peer")]
    allow_http: bool,
    /// Approve non-interactive confirmations where policy permits.
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct ReceiveArgs {
    /// Ask a remote desktop agent to choose source files and call this receiver back.
    #[arg(long)]
    request: bool,
    /// Target desktop agent. Without this option, an interactive request scans for agents.
    #[arg(long, value_name = "DEVICE_OR_ADDRESS", requires = "request")]
    peer: Option<String>,
    /// Existing directory (or extensionless new path) for files; a new path with an extension for one-shot text output.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
    /// Listener port; zero requests automatic selection.
    #[arg(long)]
    port: Option<u16>,
    /// Listener interface policy or explicit address.
    #[arg(long, value_name = "LAN_OR_ADDRESS")]
    bind: Option<String>,
    /// Stop after one completed, rejected, or expired offer.
    #[arg(long)]
    once: bool,
    /// Override trusted-device confirmation policy.
    #[arg(long, value_enum)]
    trusted_policy: Option<TrustedPolicyArg>,
    /// Approve non-interactive confirmations where policy permits.
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct AgentArgs {
    /// Listener port; zero requests automatic selection.
    #[arg(long)]
    port: Option<u16>,
    /// Listener interface policy or explicit address.
    #[arg(long, value_name = "LAN_OR_ADDRESS")]
    bind: Option<String>,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("served_content")
        .required(true)
        .multiple(true)
        .args(["paths", "upload"])
))]
struct ServeArgs {
    /// Explicit files or directories exposed by the service.
    #[arg(value_name = "PATH", num_args = 1..)]
    paths: Vec<PathBuf>,
    /// Enable browser uploads.
    #[arg(long)]
    upload: bool,
    /// Upload destination directory.
    #[arg(long, value_name = "PATH", requires = "upload")]
    output: Option<PathBuf>,
    /// Require this additional password for browser uploads.
    ///
    /// Prefer the QUICK_SHARE_UPLOAD_PASSWORD environment variable to avoid shell history.
    #[arg(
        long,
        value_name = "PASSWORD",
        env = "QUICK_SHARE_UPLOAD_PASSWORD",
        hide_env_values = true,
        requires = "upload"
    )]
    upload_password: Option<UploadPassword>,
    /// Maximum counted download sessions.
    #[arg(long)]
    max_downloads: Option<u32>,
    /// Service timeout, such as `5m` or `30s`.
    #[arg(long, value_name = "DURATION")]
    timeout: Option<String>,
    /// Explicitly opt into plaintext HTTP and display a runtime warning.
    #[arg(long, conflicts_with_all = ["cert", "key"])]
    allow_http: bool,
    /// PEM certificate chain. Must be paired with `--key`.
    #[arg(
        long,
        value_name = "PATH",
        requires = "key",
        conflicts_with = "allow_http"
    )]
    cert: Option<PathBuf>,
    /// PEM private key. Must be paired with `--cert`.
    #[arg(
        long,
        value_name = "PATH",
        requires = "cert",
        conflicts_with = "allow_http"
    )]
    key: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct DevicesArgs {
    #[command(subcommand)]
    command: DevicesCommand,
}

#[derive(Debug, Subcommand)]
enum DevicesCommand {
    /// List pinned devices without secret material.
    List {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Change a device's local display name.
    Rename { device: String, name: String },
    /// Remove a pinned device.
    Remove {
        device: String,
        /// Required when stdin is not interactive.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Show effective non-secret configuration.
    Show {
        /// Emit TOML instead of the human-readable view.
        #[arg(long)]
        toml: bool,
    },
    /// Print the platform-standard configuration path.
    Path,
    /// Set one supported non-secret preference.
    Set { key: ConfigKey, value: String },
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Check availability without replacing the executable.
    #[arg(long)]
    check: bool,
    /// Install this exact release instead of the newest compatible release.
    #[arg(long, value_name = "VERSION")]
    version: Option<String>,
    /// Approve replacement without an interactive prompt.
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TrustedPolicyArg {
    Auto,
    Confirm,
}

/// Parsed command plus global settings passed to later orchestration without clap dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandIntent {
    /// Global diagnostics and configuration selection.
    pub global: GlobalOptions,
    /// Selected operation.
    pub command: IntentCommand,
}

/// Selected top-level operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentCommand {
    /// Direct/automatic/Web send request.
    Send(SendIntent),
    /// Receiver request.
    Receive(ReceiveIntent),
    /// Windows desktop agent request.
    Agent(AgentIntent),
    /// Traditional Web request.
    Serve(ServeIntent),
    /// Trusted-device operation.
    Devices(DevicesIntent),
    /// Configuration operation.
    Config(ConfigIntent),
    /// Updater operation.
    Update(UpdateIntent),
}

/// Global options shared by every operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalOptions {
    /// Diagnostics verbosity count.
    pub verbosity: u8,
    /// Explicit configuration file, when supplied.
    pub config_path: Option<PathBuf>,
}

/// One mutually exclusive source payload and send routing policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendIntent {
    pub paths: Vec<PathBuf>,
    pub text: Option<String>,
    pub clipboard: bool,
    pub peer: Option<String>,
    pub web: bool,
    pub resume: Option<String>,
    pub follow_links: bool,
    pub allow_http: bool,
    pub assume_yes: bool,
}

/// Receiver command data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveIntent {
    pub request_remote: bool,
    pub peer: Option<String>,
    pub output: Option<PathBuf>,
    pub port: Option<u16>,
    pub bind: Option<String>,
    pub once: bool,
    pub confirm_trusted: Option<bool>,
    pub assume_yes: bool,
}

impl ReceiveIntent {
    /// Non-interactive remote requests must identify exactly which agent to contact.
    pub fn validate_interaction(&self, interactive: bool) -> Result<(), AppError> {
        if self.request_remote && !interactive && self.peer.is_none() {
            return Err(AppError::Usage(
                "non-interactive remote receive requires --peer".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Windows desktop agent command data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIntent {
    pub port: Option<u16>,
    pub bind: Option<String>,
}

/// Web command data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeIntent {
    pub paths: Vec<PathBuf>,
    pub upload: bool,
    pub output: Option<PathBuf>,
    pub upload_password: Option<UploadPassword>,
    pub max_downloads: Option<u32>,
    pub timeout: Option<String>,
    pub allow_http: bool,
    pub certificate: Option<PathBuf>,
    pub private_key: Option<PathBuf>,
}

/// A CLI-supplied upload password whose diagnostics are always redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadPassword(String);

impl UploadPassword {
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl FromStr for UploadPassword {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(
                "upload password must be 1-1024 bytes and contain no control characters".to_owned(),
            );
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Debug for UploadPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UploadPassword([REDACTED])")
    }
}

/// Trusted-device command data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicesIntent {
    List { json: bool },
    Rename { device: String, name: String },
    Remove { device: String, assume_yes: bool },
}

/// Whitelisted non-secret preference keys accepted by `config set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfigKey {
    #[value(name = "device.name")]
    DeviceName,
    #[value(name = "receive.output")]
    ReceiveOutput,
    #[value(name = "receive.trusted-policy")]
    ReceiveTrustedPolicy,
    #[value(name = "receive.conflict")]
    ReceiveConflict,
    #[value(name = "discovery.timeout-ms")]
    DiscoveryTimeoutMs,
    #[value(name = "discovery.include-virtual")]
    DiscoveryIncludeVirtual,
    #[value(name = "discovery.peers")]
    DiscoveryPeers,
    #[value(name = "network.port")]
    NetworkPort,
    #[value(name = "network.bind")]
    NetworkBind,
    #[value(name = "transfer.chunk-size")]
    TransferChunkSize,
    #[value(name = "transfer.concurrent-files")]
    TransferConcurrentFiles,
    #[value(name = "transfer.max-receive-tasks")]
    TransferMaxReceiveTasks,
    #[value(name = "transfer.retry-count")]
    TransferRetryCount,
    #[value(name = "web.timeout")]
    WebTimeout,
    #[value(name = "web.max-downloads")]
    WebMaxDownloads,
    #[value(name = "web.upload")]
    WebUpload,
    #[value(name = "web.allow-http")]
    WebAllowHttp,
}

/// Non-secret configuration command data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigIntent {
    /// Show effective non-secret configuration.
    Show { toml: bool },
    /// Print the platform-standard configuration path.
    Path,
    /// Persist one whitelisted non-secret preference.
    Set { key: ConfigKey, value: String },
}

/// Update command data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateIntent {
    pub check_only: bool,
    pub version: Option<String>,
    pub assume_yes: bool,
}

impl From<Cli> for CommandIntent {
    fn from(cli: Cli) -> Self {
        let global = GlobalOptions {
            verbosity: cli.verbose,
            config_path: cli.config,
        };
        let command = match cli.command {
            Commands::Send(args) => IntentCommand::Send(SendIntent {
                paths: args.paths,
                text: args.text,
                clipboard: args.clipboard,
                peer: args.peer,
                web: args.web,
                resume: args.resume,
                follow_links: args.follow_links,
                allow_http: args.allow_http,
                assume_yes: args.yes,
            }),
            Commands::Receive(args) => IntentCommand::Receive(ReceiveIntent {
                request_remote: args.request,
                peer: args.peer,
                output: args.output,
                port: args.port,
                bind: args.bind,
                once: args.once || args.request,
                confirm_trusted: args
                    .trusted_policy
                    .map(|value| value == TrustedPolicyArg::Confirm),
                assume_yes: args.yes,
            }),
            Commands::Agent(args) => IntentCommand::Agent(AgentIntent {
                port: args.port,
                bind: args.bind,
            }),
            Commands::Serve(args) => IntentCommand::Serve(ServeIntent {
                paths: args.paths,
                upload: args.upload,
                output: args.output,
                upload_password: args.upload_password,
                max_downloads: args.max_downloads,
                timeout: args.timeout,
                allow_http: args.allow_http,
                certificate: args.cert,
                private_key: args.key,
            }),
            Commands::Devices(args) => IntentCommand::Devices(match args.command {
                DevicesCommand::List { json } => DevicesIntent::List { json },
                DevicesCommand::Rename { device, name } => DevicesIntent::Rename { device, name },
                DevicesCommand::Remove { device, yes } => DevicesIntent::Remove {
                    device,
                    assume_yes: yes,
                },
            }),
            Commands::Config(args) => IntentCommand::Config(match args.command {
                ConfigCommand::Show { toml } => ConfigIntent::Show { toml },
                ConfigCommand::Path => ConfigIntent::Path,
                ConfigCommand::Set { key, value } => ConfigIntent::Set { key, value },
            }),
            Commands::Update(args) => IntentCommand::Update(UpdateIntent {
                check_only: args.check,
                version: args.version,
                assume_yes: args.yes,
            }),
        };
        Self { global, command }
    }
}

/// Stable process statuses documented by the QSP CLI contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitStatus {
    Success = 0,
    Usage = 2,
    PeerUnavailable = 3,
    Rejected = 4,
    Identity = 5,
    Filesystem = 6,
    Network = 7,
    Integrity = 8,
    Update = 9,
}

/// Application-level failures mapped independently from clap parser failures.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid command: {0}")]
    Usage(String),
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("confirmation required in non-interactive mode: {0}; pass --yes to approve")]
    ConfirmationRequired(String),
    #[error("peer unavailable: {0}")]
    PeerUnavailable(String),
    #[error("offer rejected or expired: {0}")]
    Rejected(String),
    #[error("identity or authorization failure: {0}")]
    Identity(String),
    #[error("local file-system failure: {0}")]
    Filesystem(String),
    #[error("network or discovery failure: {0}")]
    Network(String),
    #[error("integrity verification failure: {0}")]
    Integrity(String),
    #[error("update failure: {0}")]
    Update(String),
    #[error("cancelled")]
    Cancelled,
}

/// Converts an explicit desktop cancellation into the CLI's successful cancellation outcome.
pub fn require_desktop_choice<T>(choice: Option<T>) -> Result<T, AppError> {
    choice.ok_or(AppError::Cancelled)
}

impl From<quick_share_platform::desktop::DesktopError> for AppError {
    fn from(error: quick_share_platform::desktop::DesktopError) -> Self {
        use quick_share_platform::desktop::DesktopError;
        match error {
            DesktopError::Unsupported => {
                Self::Usage("desktop interaction is unsupported on this platform".to_owned())
            }
            DesktopError::Unavailable => {
                Self::Usage("desktop interaction is unavailable in this session".to_owned())
            }
            DesktopError::Busy => {
                Self::Usage("another desktop interaction is already active".to_owned())
            }
            DesktopError::TimedOut => {
                Self::Usage("desktop interaction deadline expired".to_owned())
            }
            DesktopError::EventLoopExited => Self::Usage("desktop event loop exited".to_owned()),
            DesktopError::InvalidSelection => {
                Self::Filesystem("desktop selection is empty or invalid".to_owned())
            }
            DesktopError::InvalidDirectory => {
                Self::Filesystem("receive directory is invalid".to_owned())
            }
            DesktopError::DirectoryNotWritable => {
                Self::Filesystem("receive directory is not writable".to_owned())
            }
            DesktopError::Backend => Self::Filesystem("desktop backend failed".to_owned()),
        }
    }
}

impl AppError {
    /// Stable semantic status.
    #[must_use]
    pub const fn status(&self) -> ExitStatus {
        match self {
            Self::Usage(_) | Self::Config(_) | Self::ConfirmationRequired(_) => ExitStatus::Usage,
            Self::PeerUnavailable(_) => ExitStatus::PeerUnavailable,
            Self::Rejected(_) => ExitStatus::Rejected,
            Self::Identity(_) => ExitStatus::Identity,
            Self::Filesystem(_) => ExitStatus::Filesystem,
            Self::Network(_) => ExitStatus::Network,
            Self::Integrity(_) => ExitStatus::Integrity,
            Self::Update(_) => ExitStatus::Update,
            Self::Cancelled => ExitStatus::Success,
        }
    }

    /// Numeric process exit code.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.status() as u8
    }
}

/// Terminal state plus explicit approval used before any destructive confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionPolicy {
    interactive: bool,
    assume_yes: bool,
}

impl InteractionPolicy {
    #[must_use]
    pub const fn new(interactive: bool, assume_yes: bool) -> Self {
        Self {
            interactive,
            assume_yes,
        }
    }

    /// Allows callers to prompt only on a terminal; otherwise requires `--yes`.
    pub fn require_confirmation(self, operation: &str) -> Result<(), AppError> {
        if self.interactive || self.assume_yes {
            Ok(())
        } else {
            Err(AppError::ConfirmationRequired(operation.to_owned()))
        }
    }
}
