#![forbid(unsafe_code)]
//! QSP wire types, validation limits, and protocol version negotiation.

use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::SerializeStruct,
};
use std::{collections::BTreeSet, fmt, num::NonZeroU32, str::FromStr};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroize;

/// Current QSP major version.
pub const PROTOCOL_MAJOR: u16 = 1;
/// Maximum accepted Unicode scalar count in a display device name.
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
/// Maximum entries accepted in one transfer offer.
/// Allows 10,000 files plus bounded directory/symlink metadata overhead.
pub const MAX_MANIFEST_ENTRIES: usize = 20_000;
/// Maximum UTF-8 bytes accepted in one relative path.
pub const MAX_RELATIVE_PATH_BYTES: usize = 4_096;
/// Maximum serialized offer size.
pub const MAX_OFFER_BYTES: usize = 8 * 1024 * 1024;
/// Minimum negotiable data chunk size.
pub const MIN_CHUNK_SIZE: u32 = 256 * 1024;
/// Maximum negotiable data chunk size.
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;
/// Maximum plaintext bytes in one `CHUNK_DATA` fragment frame.
pub const MAX_CHUNK_FRAME_BYTES: usize = 1024 * 1024;
/// Maximum chunks represented by one transfer resume status.
pub const MAX_TRANSFER_CHUNKS: u64 = 1_000_000;

/// QSP protocol version. Major changes are incompatible; minor versions downgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    /// Incompatible protocol generation.
    pub major: u16,
    /// Backward-compatible feature generation.
    pub minor: u16,
}

impl ProtocolVersion {
    /// QSP 1.0.
    pub const V1_0: Self = Self::new(1, 0);
    /// QSP 1.1 with authenticated remote source-selection control messages.
    pub const V1_1: Self = Self::new(1, 1);

    /// Creates a protocol version.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

/// Bounded capability set. Unknown enum values fail closed during decoding.
pub type Capabilities = BTreeSet<Capability>;

/// Capabilities that can be intersected during negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// Regular file payloads.
    Files,
    /// Directory manifests.
    Directories,
    /// Text payloads.
    Text,
    /// Resumable chunk transfer.
    Resume,
    /// Symbolic-link metadata.
    Symlinks,
    /// Authenticated request for the peer to select local source content.
    RemoteSelection,
}

/// Result of version and capability negotiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedProtocol {
    /// Agreed major/minor version.
    pub version: ProtocolVersion,
    /// Capabilities supported by both peers.
    pub capabilities: Capabilities,
}

/// Negotiates a protocol version and capability intersection.
pub fn negotiate(
    local_version: ProtocolVersion,
    local_capabilities: &BTreeSet<Capability>,
    remote_version: ProtocolVersion,
    remote_capabilities: &BTreeSet<Capability>,
) -> Result<NegotiatedProtocol, ProtocolError> {
    if local_version.major != remote_version.major {
        return Err(ProtocolError::new(
            ErrorCode::IncompatibleProtocol,
            format!(
                "incompatible protocol major versions: {} and {}",
                local_version.major, remote_version.major
            ),
        ));
    }

    let version = ProtocolVersion::new(
        local_version.major,
        local_version.minor.min(remote_version.minor),
    );
    let mut capabilities = local_capabilities
        .intersection(remote_capabilities)
        .copied()
        .collect::<BTreeSet<_>>();
    if version < ProtocolVersion::V1_1 {
        capabilities.remove(&Capability::RemoteSelection);
    }

    Ok(NegotiatedProtocol {
        version,
        capabilities,
    })
}

/// Stable device identity derived from a full public-key fingerprint.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    /// Parses and validates a `qs_`-prefixed, 128-bit hexadecimal identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let suffix = value
            .strip_prefix("qs_")
            .ok_or(ValidationError::InvalidDeviceId)?;
        if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ValidationError::InvalidDeviceId);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Returns the stable wire representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("DeviceId").field(&self.0).finish()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for DeviceId {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Stable QSP application message kinds carried inside Noise records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    InfoRequest = 1,
    InfoResponse = 2,
    OfferCreate = 3,
    OfferStatus = 4,
    TransferStatus = 5,
    ChunkData = 6,
    ChunkAck = 7,
    TransferComplete = 8,
    TransferCancel = 9,
    ProtocolError = 10,
    SourceSelectionRequest = 11,
    SourceSelectionResponse = 12,
}

impl TryFrom<u8> for MessageType {
    type Error = ValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::InfoRequest),
            2 => Ok(Self::InfoResponse),
            3 => Ok(Self::OfferCreate),
            4 => Ok(Self::OfferStatus),
            5 => Ok(Self::TransferStatus),
            6 => Ok(Self::ChunkData),
            7 => Ok(Self::ChunkAck),
            8 => Ok(Self::TransferComplete),
            9 => Ok(Self::TransferCancel),
            10 => Ok(Self::ProtocolError),
            11 => Ok(Self::SourceSelectionRequest),
            12 => Ok(Self::SourceSelectionResponse),
            _ => Err(ValidationError::UnknownMessageType),
        }
    }
}

/// Unique request identifier used for replay and response correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(Uuid);

impl RequestId {
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0.into_bytes()
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }
}

/// Unique transfer identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransferId(Uuid);

impl TransferId {
    /// Wraps a UUID generated by the caller.
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0.into_bytes()
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }
}

/// Non-zero entry identifier scoped to a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(NonZeroU32);

impl EntryId {
    /// Creates a non-zero entry identifier.
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        match NonZeroU32::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the numeric identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Public peer information used during offers and capability negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceInfo {
    /// Stable device identity.
    pub device_id: DeviceId,
    /// Terminal display name. It is untrusted UI text.
    pub name: String,
    /// Advertised protocol capabilities.
    pub capabilities: Capabilities,
}

impl DeviceInfo {
    /// Rejects names that could exhaust or inject terminal output downstream.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() || self.name.chars().any(char::is_control) {
            return Err(ValidationError::InvalidDeviceName);
        }
        if self.name.chars().count() > MAX_DEVICE_NAME_CHARS {
            return Err(ValidationError::DeviceNameTooLong);
        }
        Ok(())
    }
}

/// Top-level payload category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    /// One or more file-system entries.
    Files,
    /// A text or clipboard payload.
    Text,
}

/// Receiver response to an authenticated transfer offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfferDecision {
    /// Authorize only this transfer.
    AcceptOnce,
    /// Authorize this transfer and pin the sender's complete static key.
    AcceptAndTrust,
    /// Reject without disclosing sensitive receiver context.
    Reject { reason: RejectionReason },
}

/// Stable rejection categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    /// Explicit local user rejection.
    UserRejected,
    /// Local confirmation expired.
    ConfirmationTimeout,
    /// Offer exceeded local receiver policy.
    Policy,
}

/// Stable transfer lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    /// Waiting for a receiver decision.
    Offered,
    /// Authorized but no payload received yet.
    Accepted,
    /// Payload is moving.
    Transferring,
    /// Authenticated transfer is resumable after interruption.
    Paused,
    /// All payload is present and being verified.
    Verifying,
    /// Verified payload was atomically committed.
    Completed,
    /// Receiver declined the offer.
    Rejected,
    /// Confirmation timed out.
    Expired,
    /// Local or remote user cancelled.
    Cancelled,
    /// Unrecoverable failure.
    Failed,
}

/// Authenticated fixed-block payload descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkDescriptor {
    /// Transfer containing the entry.
    pub transfer_id: TransferId,
    /// Entry receiving this block.
    pub entry_id: EntryId,
    /// Zero-based fixed-block number.
    pub index: u32,
    /// Byte offset in the entry.
    pub offset: u64,
    /// Payload bytes following this descriptor.
    pub length: u32,
    /// BLAKE3 digest of the plaintext block.
    pub digest: [u8; 32],
}

impl ChunkDescriptor {
    /// Validates fixed-block arithmetic before allocating or writing payload data.
    pub fn validate(&self, chunk_size: u32, entry_size: u64) -> Result<(), ValidationError> {
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size)
            || self.length == 0
            || self.length > chunk_size
        {
            return Err(ValidationError::InvalidChunkSize);
        }
        let expected_offset = u64::from(self.index)
            .checked_mul(u64::from(chunk_size))
            .ok_or(ValidationError::InvalidChunkRange)?;
        let end = self
            .offset
            .checked_add(u64::from(self.length))
            .ok_or(ValidationError::InvalidChunkRange)?;
        if self.offset != expected_offset || end > entry_size {
            return Err(ValidationError::InvalidChunkRange);
        }
        Ok(())
    }
}

/// Type-specific manifest metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ManifestEntryKind {
    /// Regular file.
    File,
    /// Directory, including an empty directory.
    Directory,
    /// Symbolic-link target text. It is not followed implicitly.
    Symlink {
        /// Link target exactly as read from the source file system.
        target: String,
    },
    /// Text payload with a declared media type.
    Text {
        /// Usually `text/plain; charset=utf-8`.
        media_type: String,
    },
}

/// One manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Transfer-scoped identifier.
    pub id: EntryId,
    /// Portable relative path. Full containment validation belongs to core.
    pub relative_path: String,
    /// File-system or text entry kind.
    pub kind: ManifestEntryKind,
    /// File/text bytes; directories and links use zero.
    pub size: u64,
    /// Full BLAKE3 of file/text content; absent for directories and links.
    pub digest: Option<[u8; 32]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestEntryWire {
    id: EntryId,
    relative_path: String,
    kind: ManifestEntryTag,
    size: u64,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    digest: Option<[u8; 32]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum ManifestEntryTag {
    File,
    Directory,
    Symlink,
    Text,
}

impl Serialize for ManifestEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let extra_fields = usize::from(matches!(
            &self.kind,
            ManifestEntryKind::Symlink { .. } | ManifestEntryKind::Text { .. }
        )) + usize::from(self.digest.is_some());
        let mut state = serializer.serialize_struct("ManifestEntry", 4 + extra_fields)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("relativePath", &self.relative_path)?;
        match &self.kind {
            ManifestEntryKind::File => state.serialize_field("kind", "file")?,
            ManifestEntryKind::Directory => state.serialize_field("kind", "directory")?,
            ManifestEntryKind::Symlink { target } => {
                state.serialize_field("kind", "symlink")?;
                state.serialize_field("target", target)?;
            }
            ManifestEntryKind::Text { media_type } => {
                state.serialize_field("kind", "text")?;
                state.serialize_field("mediaType", media_type)?;
            }
        }
        state.serialize_field("size", &self.size)?;
        if let Some(digest) = self.digest {
            state.serialize_field("digest", &digest)?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for ManifestEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ManifestEntryWire::deserialize(deserializer)?;
        let kind = match wire.kind {
            ManifestEntryTag::File => {
                if wire.target.is_some() || wire.media_type.is_some() || wire.digest.is_none() {
                    return Err(D::Error::custom(
                        "file entry has invalid type-specific fields or no digest",
                    ));
                }
                ManifestEntryKind::File
            }
            ManifestEntryTag::Directory => {
                if wire.target.is_some() || wire.media_type.is_some() || wire.digest.is_some() {
                    return Err(D::Error::custom("directory entry has type-specific fields"));
                }
                ManifestEntryKind::Directory
            }
            ManifestEntryTag::Symlink => {
                if wire.media_type.is_some() || wire.digest.is_some() {
                    return Err(D::Error::custom("symlink entry has text or digest fields"));
                }
                ManifestEntryKind::Symlink {
                    target: wire
                        .target
                        .ok_or_else(|| D::Error::custom("symlink entry is missing target"))?,
                }
            }
            ManifestEntryTag::Text => {
                if wire.target.is_some() || wire.digest.is_none() {
                    return Err(D::Error::custom(
                        "text entry has symlink fields or no digest",
                    ));
                }
                ManifestEntryKind::Text {
                    media_type: wire
                        .media_type
                        .ok_or_else(|| D::Error::custom("text entry is missing mediaType"))?,
                }
            }
        };
        Ok(Self {
            id: wire.id,
            relative_path: wire.relative_path,
            kind,
            size: wire.size,
            digest: wire.digest,
        })
    }
}

/// Capability negotiation request allowed immediately after authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfoRequest {
    pub protocol_version: ProtocolVersion,
    pub capabilities: Capabilities,
}

/// Authenticated peer information response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfoResponse {
    pub protocol_version: ProtocolVersion,
    pub device: DeviceInfo,
}

impl InfoResponse {
    /// Applies semantic bounds after strict deserialization.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.device.validate()
    }
}

/// Builds a version-compatible INFO response without exposing QSP/1.1-only
/// capabilities to a strict QSP/1.0 decoder.
pub fn compatible_info_response(
    local_version: ProtocolVersion,
    mut local_device: DeviceInfo,
    request: &InfoRequest,
) -> Result<InfoResponse, ProtocolError> {
    local_device.validate().map_err(|error| {
        ProtocolError::new(
            ErrorCode::InvalidMessage,
            format!("local device information is invalid: {error}"),
        )
    })?;
    let negotiated = negotiate(
        local_version,
        &local_device.capabilities,
        request.protocol_version,
        &request.capabilities,
    )?;
    if negotiated.version < ProtocolVersion::V1_1 {
        local_device
            .capabilities
            .remove(&Capability::RemoteSelection);
    }
    Ok(InfoResponse {
        protocol_version: negotiated.version,
        device: local_device,
    })
}

/// Authenticated request for the peer to choose source content and call back
/// to the requester's observed network address on a bounded non-zero port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSelectionRequest {
    pub requester: DeviceInfo,
    pub callback_port: u16,
}

impl SourceSelectionRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.requester.validate()?;
        if self.callback_port == 0 {
            return Err(ValidationError::InvalidCallbackPort);
        }
        Ok(())
    }
}

/// Stable terminal result of one source-selection request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSelectionStatus {
    Ready,
    Cancelled,
    Rejected,
    Busy,
    UiUnavailable,
    Expired,
    Failed,
}

/// Correlated response. Only `ready` may expose the transfer ID that must
/// appear in the authenticated callback offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSelectionResponse {
    pub status: SourceSelectionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<TransferId>,
}

impl SourceSelectionResponse {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if (self.status == SourceSelectionStatus::Ready) != self.transfer_id.is_some() {
            return Err(ValidationError::InvalidSelectionState);
        }
        Ok(())
    }
}

/// A transfer offer sent before any payload is authorized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferOffer {
    /// Sender protocol version.
    pub protocol_version: ProtocolVersion,
    /// Random transfer identifier.
    pub transfer_id: TransferId,
    /// Remote-selection request that authorized this callback offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiated_by: Option<RequestId>,
    /// Sender display and capability information.
    pub sender: DeviceInfo,
    /// Top-level payload category.
    pub content_kind: ContentKind,
    /// Proposed chunk size.
    pub chunk_size: u32,
    /// Sum of byte-carrying entries.
    pub total_bytes: u64,
    /// Bounded manifest entries.
    pub entries: Vec<ManifestEntry>,
}

impl TransferOffer {
    /// Enforces allocation and semantic limits after deserialization.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.sender.validate()?;
        if self.entries.len() > MAX_MANIFEST_ENTRIES {
            return Err(ValidationError::TooManyEntries);
        }
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&self.chunk_size) {
            return Err(ValidationError::InvalidChunkSize);
        }
        if self.entries.is_empty() {
            return Err(ValidationError::EmptyManifest);
        }
        match self.content_kind {
            ContentKind::Files
                if self
                    .entries
                    .iter()
                    .any(|entry| matches!(entry.kind, ManifestEntryKind::Text { .. })) =>
            {
                return Err(ValidationError::InvalidContentKind);
            }
            ContentKind::Text
                if self.entries.len() != 1
                    || !matches!(self.entries[0].kind, ManifestEntryKind::Text { .. }) =>
            {
                return Err(ValidationError::InvalidContentKind);
            }
            ContentKind::Files | ContentKind::Text => {}
        }

        let mut ids = BTreeSet::new();
        let mut total = 0_u64;
        for entry in &self.entries {
            if entry.relative_path.is_empty()
                || entry.relative_path.len() > MAX_RELATIVE_PATH_BYTES
                || entry.relative_path.contains('\0')
            {
                return Err(ValidationError::InvalidRelativePath);
            }
            if !ids.insert(entry.id) {
                return Err(ValidationError::DuplicateEntryId);
            }
            match &entry.kind {
                ManifestEntryKind::Directory | ManifestEntryKind::Symlink { .. }
                    if entry.size != 0 || entry.digest.is_some() =>
                {
                    return Err(ValidationError::InvalidEntrySize);
                }
                ManifestEntryKind::File | ManifestEntryKind::Text { .. }
                    if entry.digest.is_none() =>
                {
                    return Err(ValidationError::MissingContentDigest);
                }
                ManifestEntryKind::Symlink { target }
                    if target.len() > MAX_RELATIVE_PATH_BYTES || target.contains('\0') =>
                {
                    return Err(ValidationError::InvalidRelativePath);
                }
                ManifestEntryKind::Text { media_type }
                    if media_type.is_empty() || media_type.len() > 128 =>
                {
                    return Err(ValidationError::InvalidMediaType);
                }
                _ => {}
            }
            total = total
                .checked_add(entry.size)
                .ok_or(ValidationError::TotalSizeOverflow)?;
        }
        if total != self.total_bytes {
            return Err(ValidationError::TotalSizeMismatch);
        }
        let encoded_size = serde_json::to_vec(self)
            .map_err(|_| ValidationError::SerializationFailed)?
            .len();
        if encoded_size > MAX_OFFER_BYTES {
            return Err(ValidationError::OfferTooLarge);
        }
        Ok(())
    }
}

/// Strict create-offer control payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfferCreate {
    pub offer: TransferOffer,
    /// Explicitly re-authorizes an identical interrupted transfer.
    #[serde(default)]
    pub resume: bool,
}

impl OfferCreate {
    /// Applies all offer resource and semantic bounds.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.offer.validate()
    }
}

/// Owner-only offer status query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfferStatusRequest {
    pub transfer_id: TransferId,
}

/// Owner-only status response. A bearer is present only after acceptance and only inside Noise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfferStatusResponse {
    pub transfer_id: TransferId,
    pub status: TransferStatus,
    pub authorization: Option<AuthorizationProof>,
}

impl OfferStatusResponse {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if (self.status == TransferStatus::Accepted) != self.authorization.is_some() {
            return Err(ValidationError::InvalidAuthorizationState);
        }
        Ok(())
    }
}

/// Redacted bearer proof carried only inside authenticated Noise transport.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthorizationProof([u8; 32]);

impl AuthorizationProof {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn with_bytes<R>(&self, callback: impl FnOnce(&[u8; 32]) -> R) -> R {
        callback(&self.0)
    }
}

impl Clone for AuthorizationProof {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl PartialEq for AuthorizationProof {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for AuthorizationProof {}

impl fmt::Debug for AuthorizationProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationProof([REDACTED])")
    }
}

impl Drop for AuthorizationProof {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Owner-authorized transfer status and resume query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferStatusRequest {
    pub transfer_id: TransferId,
    pub authorization: AuthorizationProof,
}

/// Compact missing-chunk bitmap; bit 1 means the chunk must be uploaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MissingChunkBitmap {
    pub chunk_count: u32,
    pub missing_bits: Vec<u8>,
}

impl MissingChunkBitmap {
    pub fn from_missing(
        chunk_count: u32,
        missing: impl IntoIterator<Item = u32>,
    ) -> Result<Self, ValidationError> {
        if u64::from(chunk_count) > MAX_TRANSFER_CHUNKS {
            return Err(ValidationError::TooManyChunks);
        }
        let byte_count = usize::try_from(u64::from(chunk_count).div_ceil(8))
            .map_err(|_| ValidationError::TooManyChunks)?;
        let mut missing_bits = vec![0_u8; byte_count];
        for index in missing {
            if index >= chunk_count {
                return Err(ValidationError::InvalidChunkRange);
            }
            missing_bits[index as usize / 8] |= 1 << (index % 8);
        }
        Ok(Self {
            chunk_count,
            missing_bits,
        })
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if u64::from(self.chunk_count) > MAX_TRANSFER_CHUNKS
            || self.missing_bits.len()
                != usize::try_from(u64::from(self.chunk_count).div_ceil(8))
                    .map_err(|_| ValidationError::TooManyChunks)?
        {
            return Err(ValidationError::InvalidChunkBitmap);
        }
        if !self.chunk_count.is_multiple_of(8)
            && self.missing_bits.last().is_some_and(|last| {
                let used = self.chunk_count % 8;
                *last & !((1_u8 << used) - 1) != 0
            })
        {
            return Err(ValidationError::InvalidChunkBitmap);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_missing(&self, index: u32) -> bool {
        index < self.chunk_count && self.missing_bits[index as usize / 8] & (1 << (index % 8)) != 0
    }
}

/// Resume status for one payload-bearing manifest entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntryTransferStatus {
    pub entry_id: EntryId,
    pub chunks: MissingChunkBitmap,
}

/// Receiver status bound to the accepted immutable manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferStatusResponse {
    pub transfer_id: TransferId,
    pub status: TransferStatus,
    pub manifest_digest: [u8; 32],
    pub entries: Vec<EntryTransferStatus>,
}

impl TransferStatusResponse {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.entries.len() > MAX_MANIFEST_ENTRIES {
            return Err(ValidationError::TooManyEntries);
        }
        let mut ids = BTreeSet::new();
        let mut chunks = 0_u64;
        for entry in &self.entries {
            if !ids.insert(entry.entry_id) {
                return Err(ValidationError::DuplicateEntryId);
            }
            entry.chunks.validate()?;
            chunks = chunks
                .checked_add(u64::from(entry.chunks.chunk_count))
                .ok_or(ValidationError::TooManyChunks)?;
        }
        if chunks > MAX_TRANSFER_CHUNKS {
            return Err(ValidationError::TooManyChunks);
        }
        Ok(())
    }
}

/// One bounded plaintext fragment of a larger fixed-size chunk.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkData {
    pub authorization: AuthorizationProof,
    pub descriptor: ChunkDescriptor,
    pub fragment_offset: u32,
    pub final_fragment: bool,
    pub payload: Vec<u8>,
}

impl fmt::Debug for ChunkData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChunkData")
            .field("authorization", &self.authorization)
            .field("descriptor", &self.descriptor)
            .field("fragment_offset", &self.fragment_offset)
            .field("final_fragment", &self.final_fragment)
            .field("payload", &"[REDACTED]")
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

impl ChunkData {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.payload.is_empty() || self.payload.len() > MAX_CHUNK_FRAME_BYTES {
            return Err(ValidationError::InvalidChunkSize);
        }
        let end = self
            .fragment_offset
            .checked_add(
                u32::try_from(self.payload.len()).map_err(|_| ValidationError::InvalidChunkSize)?,
            )
            .ok_or(ValidationError::InvalidChunkRange)?;
        if self.descriptor.length == 0
            || self.descriptor.length > MAX_CHUNK_SIZE
            || end > self.descriptor.length
            || self.final_fragment != (end == self.descriptor.length)
        {
            return Err(ValidationError::InvalidChunkRange);
        }
        Ok(())
    }
}

/// Idempotent acknowledgement emitted only after data sync and journal update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChunkAck {
    pub transfer_id: TransferId,
    pub entry_id: EntryId,
    pub index: u32,
    pub accepted_length: u32,
    pub duplicate: bool,
}

/// Final verification request for an immutable accepted manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferComplete {
    pub transfer_id: TransferId,
    pub authorization: AuthorizationProof,
    pub manifest_digest: [u8; 32],
}

/// Completion acknowledgement after verified commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferCompleteAck {
    pub transfer_id: TransferId,
    pub status: TransferStatus,
}

/// Stable cancellation origin without attacker-controlled log text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    User,
    Peer,
    Shutdown,
    SourceChanged,
}

/// Authorized transfer cancellation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferCancel {
    pub transfer_id: TransferId,
    pub authorization: AuthorizationProof,
    pub reason: CancelReason,
}

/// Decodes a length-bounded offer and performs semantic validation.
pub fn decode_offer(bytes: &[u8]) -> Result<TransferOffer, DecodeError> {
    if bytes.len() > MAX_OFFER_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let offer: TransferOffer = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    offer.validate().map_err(DecodeError::Validation)?;
    Ok(offer)
}

/// Length-bounded wire decoding failure.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// Frame body exceeded the protocol allocation bound.
    #[error("offer exceeds the maximum encoded size")]
    TooLarge,
    /// JSON structure failed strict decoding.
    #[error("invalid offer JSON: {0}")]
    Json(#[source] serde_json::Error),
    /// Decoded values violated protocol semantics.
    #[error("invalid offer: {0}")]
    Validation(#[source] ValidationError),
}

/// Stable wire error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Peer protocol majors differ.
    IncompatibleProtocol,
    /// Input failed bounded semantic validation.
    InvalidMessage,
    /// Peer is not authorized for the requested state transition.
    Unauthorized,
    /// The receiver rejected the offer.
    Rejected,
    /// Transfer integrity verification failed.
    IntegrityFailed,
    /// A transfer could not be found.
    NotFound,
    /// Peer exceeded a resource or rate limit.
    ResourceLimit,
    /// An internal failure safe to expose only generically.
    Internal,
}

/// Machine-readable protocol error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{code:?}: {message}")]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    /// Stable code used by clients.
    pub code: ErrorCode,
    /// Sanitized human-readable context.
    pub message: String,
}

impl ProtocolError {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.message.is_empty()
            || self.message.chars().count() > 300
            || self.message.chars().any(char::is_control)
        {
            return Err(ValidationError::InvalidErrorMessage);
        }
        Ok(())
    }

    /// Creates a protocol error.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Local validation failures for bounded wire data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    /// Device ID format is invalid.
    #[error("invalid device ID")]
    InvalidDeviceId,
    /// Device display name is empty.
    #[error("device name is empty")]
    InvalidDeviceName,
    /// Device display name exceeds the hard limit.
    #[error("device name exceeds the limit")]
    DeviceNameTooLong,
    /// Manifest has no entries.
    #[error("manifest is empty")]
    EmptyManifest,
    /// Top-level content kind does not match its entries.
    #[error("content kind does not match manifest entries")]
    InvalidContentKind,
    /// Manifest entry count exceeds the hard limit.
    #[error("manifest has too many entries")]
    TooManyEntries,
    /// Serialized offer exceeds the hard byte limit.
    #[error("offer exceeds the byte limit")]
    OfferTooLarge,
    /// Proposed chunk size is outside the allowed range.
    #[error("invalid chunk size")]
    InvalidChunkSize,
    /// Relative path fails basic wire-level limits.
    #[error("invalid relative path")]
    InvalidRelativePath,
    /// Two entries reuse one transfer-scoped identifier.
    #[error("duplicate entry ID")]
    DuplicateEntryId,
    /// Summing entry sizes overflowed u64.
    #[error("total size overflow")]
    TotalSizeOverflow,
    /// Declared total does not equal entry sizes.
    #[error("total size does not match entries")]
    TotalSizeMismatch,
    /// Directory/link digest or payload size is invalid for its kind.
    #[error("manifest entry size or digest is invalid for its kind")]
    InvalidEntrySize,
    /// A payload-bearing entry has no full content digest.
    #[error("file or text entry is missing its full content digest")]
    MissingContentDigest,
    /// Text media type is absent or too long.
    #[error("text media type is invalid")]
    InvalidMediaType,
    /// Accepted offer responses must carry a bearer and all other states must not.
    #[error("offer authorization does not match its status")]
    InvalidAuthorizationState,
    /// Chunk offset, index, length, or entry size are inconsistent.
    #[error("chunk range is invalid")]
    InvalidChunkRange,
    /// A resume bitmap has the wrong length or non-zero padding.
    #[error("missing-chunk bitmap is invalid")]
    InvalidChunkBitmap,
    /// A transfer contains too many chunks for bounded resume state.
    #[error("transfer has too many chunks")]
    TooManyChunks,
    /// Protocol error text is empty, too long, or contains terminal controls.
    #[error("protocol error message is invalid")]
    InvalidErrorMessage,
    /// Remote selection requested an unusable callback port.
    #[error("source-selection callback port must be non-zero")]
    InvalidCallbackPort,
    /// Selection response status and transfer ID are inconsistent.
    #[error("source-selection response has an invalid terminal state")]
    InvalidSelectionState,
    /// Message kind byte is not assigned by QSP/1.
    #[error("unknown QSP message type")]
    UnknownMessageType,
    /// Offer could not be encoded for size validation.
    #[error("offer serialization failed")]
    SerializationFailed,
}
