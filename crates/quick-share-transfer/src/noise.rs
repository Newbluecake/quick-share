//! Minimal audited surface around the fixed Quick Share Noise XX pattern.
//!
//! Security invariants:
//! - the algorithm suite is a compile-time constant and is never peer-negotiated;
//! - every handshake and transport record has a hard byte bound;
//! - the complete remote static key is extracted only after XX completes;
//! - transport records are ordered/stateful, so replayed ciphertext fails closed;
//! - logical payload reassembly permits one bounded, non-interleaved message at a time.

use quick_share_core::identity::{DeviceIdentity, device_id_from_public_key};
use quick_share_protocol::{
    Capability, ChunkAck, ChunkData, DeviceId, InfoRequest, InfoResponse, MessageType,
    NegotiatedProtocol, OfferCreate, OfferStatusRequest, OfferStatusResponse, ProtocolError,
    ProtocolVersion, RequestId, SourceSelectionRequest, SourceSelectionResponse, TransferCancel,
    TransferComplete, TransferCompleteAck, TransferStatusRequest, TransferStatusResponse,
    ValidationError,
};
use serde::{Serialize, de::DeserializeOwned};
use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};
use std::{
    fmt,
    io::{self, Read, Write},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::Zeroizing;

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const HANDSHAKE_MAX_BYTES: usize = 4 * 1024;
const NOISE_TAG_BYTES: usize = 16;
const PLAINTEXT_RECORD_MAX: usize = 60 * 1024;
const CIPHERTEXT_RECORD_MAX: usize = PLAINTEXT_RECORD_MAX + NOISE_TAG_BYTES;
const SEGMENT_HEADER_BYTES: usize = 32;
const SEGMENT_PAYLOAD_MAX: usize = PLAINTEXT_RECORD_MAX - SEGMENT_HEADER_BYTES;
const MAX_LOGICAL_PAYLOAD: usize = 8 * 1024 * 1024;
const MAX_SEGMENTS: usize = 256;
const FRAME_MAGIC: [u8; 2] = *b"QS";
const FRAME_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseRole {
    Initiator,
    Responder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandshakePhase {
    WriteOne,
    ReadOne,
    WriteTwo,
    ReadTwo,
    WriteThree,
    ReadThree,
    Complete,
}

/// Three-message XX handshake with deterministic deadline checks.
pub struct NoiseHandshake {
    role: NoiseRole,
    phase: HandshakePhase,
    expires_at: Instant,
    state: HandshakeState,
}

impl fmt::Debug for NoiseHandshake {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoiseHandshake")
            .field("role", &self.role)
            .field("phase", &self.phase)
            .field("cryptographic_state", &"[REDACTED]")
            .finish()
    }
}

impl NoiseHandshake {
    pub fn initiator(identity: &DeviceIdentity, timeout: Duration) -> Result<Self, NoiseError> {
        Self::new_at(NoiseRole::Initiator, identity, Instant::now(), timeout)
    }

    pub fn responder(identity: &DeviceIdentity, timeout: Duration) -> Result<Self, NoiseError> {
        Self::new_at(NoiseRole::Responder, identity, Instant::now(), timeout)
    }

    pub fn new_at(
        role: NoiseRole,
        identity: &DeviceIdentity,
        now: Instant,
        timeout: Duration,
    ) -> Result<Self, NoiseError> {
        let params: NoiseParams = NOISE_PATTERN
            .parse()
            .map_err(|_| NoiseError::Configuration)?;
        let mut private_key = Zeroizing::new(identity.with_private_key(|key| key.to_bytes()));
        let builder = Builder::new(params)
            .local_private_key(private_key.as_ref())
            .map_err(snow_error)?;
        let state = match role {
            NoiseRole::Initiator => builder.build_initiator(),
            NoiseRole::Responder => builder.build_responder(),
        }
        .map_err(snow_error)?;
        private_key.fill(0);
        Ok(Self {
            role,
            phase: match role {
                NoiseRole::Initiator => HandshakePhase::WriteOne,
                NoiseRole::Responder => HandshakePhase::ReadOne,
            },
            expires_at: now.checked_add(timeout).unwrap_or(now),
            state,
        })
    }

    pub fn write_message(&mut self) -> Result<Vec<u8>, NoiseError> {
        self.write_message_at(Instant::now())
    }

    pub fn write_message_at(&mut self, now: Instant) -> Result<Vec<u8>, NoiseError> {
        self.check_deadline(now)?;
        let next = match (self.role, self.phase) {
            (NoiseRole::Initiator, HandshakePhase::WriteOne) => HandshakePhase::ReadTwo,
            (NoiseRole::Initiator, HandshakePhase::WriteThree) => HandshakePhase::Complete,
            (NoiseRole::Responder, HandshakePhase::WriteTwo) => HandshakePhase::ReadThree,
            _ => return Err(NoiseError::HandshakeOrder),
        };
        let mut output = vec![0_u8; HANDSHAKE_MAX_BYTES];
        let count = self
            .state
            .write_message(&[], &mut output)
            .map_err(snow_error)?;
        if count == 0 || count > HANDSHAKE_MAX_BYTES {
            return Err(NoiseError::InvalidHandshakeFrame);
        }
        output.truncate(count);
        self.phase = next;
        Ok(output)
    }

    pub fn read_message(&mut self, message: &[u8]) -> Result<(), NoiseError> {
        self.read_message_at(Instant::now(), message)
    }

    pub fn read_message_at(&mut self, now: Instant, message: &[u8]) -> Result<(), NoiseError> {
        self.check_deadline(now)?;
        if message.is_empty() || message.len() > HANDSHAKE_MAX_BYTES {
            return Err(NoiseError::InvalidHandshakeFrame);
        }
        let next = match (self.role, self.phase) {
            (NoiseRole::Initiator, HandshakePhase::ReadTwo) => HandshakePhase::WriteThree,
            (NoiseRole::Responder, HandshakePhase::ReadOne) => HandshakePhase::WriteTwo,
            (NoiseRole::Responder, HandshakePhase::ReadThree) => HandshakePhase::Complete,
            _ => return Err(NoiseError::HandshakeOrder),
        };
        let mut payload = [0_u8; 1];
        let count = self
            .state
            .read_message(message, &mut payload)
            .map_err(snow_error)?;
        if count != 0 {
            return Err(NoiseError::UnexpectedHandshakePayload);
        }
        self.phase = next;
        Ok(())
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.phase == HandshakePhase::Complete
    }

    pub fn finish(
        self,
        expected_remote_static: Option<&[u8; 32]>,
    ) -> Result<(SecureChannel, HandshakeEvidence), NoiseError> {
        self.finish_at(Instant::now(), expected_remote_static)
    }

    pub fn finish_at(
        self,
        now: Instant,
        expected_remote_static: Option<&[u8; 32]>,
    ) -> Result<(SecureChannel, HandshakeEvidence), NoiseError> {
        self.check_deadline(now)?;
        if !self.is_complete() {
            return Err(NoiseError::HandshakeIncomplete);
        }
        let remote: [u8; 32] = self
            .state
            .get_remote_static()
            .ok_or(NoiseError::MissingRemoteStatic)?
            .try_into()
            .map_err(|_| NoiseError::MissingRemoteStatic)?;
        if let Some(expected) = expected_remote_static
            && !bool::from(remote.ct_eq(expected))
        {
            return Err(NoiseError::RemoteIdentityMismatch);
        }
        let handshake_hash = self.state.get_handshake_hash();
        let evidence = HandshakeEvidence {
            remote_device_id: device_id_from_public_key(&remote)
                .map_err(|_| NoiseError::MissingRemoteStatic)?,
            remote_static: remote,
            static_key_fingerprint: *blake3::hash(&remote).as_bytes(),
            sas: SasCode::from_handshake_hash(handshake_hash),
        };
        let state = self.state.into_transport_mode().map_err(snow_error)?;
        Ok((
            SecureChannel {
                state,
                reassembly: None,
            },
            evidence,
        ))
    }

    fn check_deadline(&self, now: Instant) -> Result<(), NoiseError> {
        if now >= self.expires_at {
            Err(NoiseError::HandshakeExpired)
        } else {
            Ok(())
        }
    }
}

/// Human-comparable code derived from the authenticated XX transcript.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SasCode(u32);

impl SasCode {
    /// Constructs a display code from an already bounded six-digit value.
    #[must_use]
    pub const fn from_value(value: u32) -> Option<Self> {
        if value < 1_000_000 {
            Some(Self(value))
        } else {
            None
        }
    }

    fn from_handshake_hash(hash: &[u8]) -> Self {
        let derived = blake3::derive_key("quick-share/qsp1/noise-sas", hash);
        let mut prefix = [0_u8; 4];
        prefix.copy_from_slice(&derived[..4]);
        let value = u32::from_be_bytes(prefix) % 1_000_000;
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for SasCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SasCode({self})")
    }
}

impl fmt::Display for SasCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:06}", self.0)
    }
}

/// Evidence available only after the XX handshake proves possession of both static keys.
pub struct HandshakeEvidence {
    pub remote_device_id: DeviceId,
    remote_static: [u8; 32],
    pub static_key_fingerprint: [u8; 32],
    pub sas: SasCode,
}

impl HandshakeEvidence {
    #[must_use]
    pub const fn remote_static(&self) -> [u8; 32] {
        self.remote_static
    }
}

impl fmt::Debug for HandshakeEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandshakeEvidence")
            .field("remote_device_id", &self.remote_device_id)
            .field("remote_static", &"[REDACTED]")
            .field("static_key_fingerprint", &"[REDACTED]")
            .field("sas", &self.sas)
            .finish()
    }
}

/// One logical QSP application frame after bounded reassembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationFrame {
    pub message_type: MessageType,
    pub request_id: RequestId,
    pub payload: Vec<u8>,
}

/// Semantic validation required before a control payload crosses the secure boundary.
pub trait ControlPayload: Serialize + DeserializeOwned {
    const MESSAGE_TYPE: MessageType;

    fn validate_control(&self) -> Result<(), ValidationError>;
}

impl ControlPayload for InfoRequest {
    const MESSAGE_TYPE: MessageType = MessageType::InfoRequest;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for InfoResponse {
    const MESSAGE_TYPE: MessageType = MessageType::InfoResponse;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for OfferCreate {
    const MESSAGE_TYPE: MessageType = MessageType::OfferCreate;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for OfferStatusRequest {
    const MESSAGE_TYPE: MessageType = MessageType::OfferStatus;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for OfferStatusResponse {
    const MESSAGE_TYPE: MessageType = MessageType::OfferStatus;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for TransferStatusRequest {
    const MESSAGE_TYPE: MessageType = MessageType::TransferStatus;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for TransferStatusResponse {
    const MESSAGE_TYPE: MessageType = MessageType::TransferStatus;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for ChunkData {
    const MESSAGE_TYPE: MessageType = MessageType::ChunkData;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for ChunkAck {
    const MESSAGE_TYPE: MessageType = MessageType::ChunkAck;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for TransferComplete {
    const MESSAGE_TYPE: MessageType = MessageType::TransferComplete;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for TransferCompleteAck {
    const MESSAGE_TYPE: MessageType = MessageType::TransferComplete;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for TransferCancel {
    const MESSAGE_TYPE: MessageType = MessageType::TransferCancel;

    fn validate_control(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

impl ControlPayload for ProtocolError {
    const MESSAGE_TYPE: MessageType = MessageType::ProtocolError;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for SourceSelectionRequest {
    const MESSAGE_TYPE: MessageType = MessageType::SourceSelectionRequest;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

impl ControlPayload for SourceSelectionResponse {
    const MESSAGE_TYPE: MessageType = MessageType::SourceSelectionResponse;

    fn validate_control(&self) -> Result<(), ValidationError> {
        self.validate()
    }
}

/// Strictly serializes a validated baseline control payload.
/// QSP/1.1 selection messages require `encode_negotiated_control_frame`.
pub fn encode_control_frame<T: ControlPayload>(
    message_type: MessageType,
    request_id: RequestId,
    value: &T,
) -> Result<ApplicationFrame, NoiseError> {
    if is_selection_message(message_type) {
        return Err(NoiseError::InvalidControlPayload);
    }
    encode_control_frame_inner(message_type, request_id, value)
}

fn encode_control_frame_inner<T: ControlPayload>(
    message_type: MessageType,
    request_id: RequestId,
    value: &T,
) -> Result<ApplicationFrame, NoiseError> {
    if message_type != T::MESSAGE_TYPE {
        return Err(NoiseError::InvalidControlPayload);
    }
    value
        .validate_control()
        .map_err(|_| NoiseError::InvalidControlPayload)?;
    let payload = serde_json::to_vec(value).map_err(|_| NoiseError::InvalidControlPayload)?;
    if payload.len() > MAX_LOGICAL_PAYLOAD {
        return Err(NoiseError::LogicalFrameTooLarge);
    }
    Ok(ApplicationFrame {
        message_type,
        request_id,
        payload,
    })
}

/// Encodes only control messages enabled by the authenticated INFO negotiation.
pub fn encode_negotiated_control_frame<T: ControlPayload>(
    negotiated: &NegotiatedProtocol,
    message_type: MessageType,
    request_id: RequestId,
    value: &T,
) -> Result<ApplicationFrame, NoiseError> {
    require_negotiated_message(negotiated, message_type)?;
    encode_control_frame_inner(message_type, request_id, value)
}

/// Decodes only control messages enabled by the authenticated INFO negotiation.
pub fn decode_negotiated_control_frame<T: ControlPayload>(
    negotiated: &NegotiatedProtocol,
    frame: &ApplicationFrame,
    expected_type: MessageType,
) -> Result<T, NoiseError> {
    require_negotiated_message(negotiated, expected_type)?;
    decode_control_frame_inner(frame, expected_type)
}

const fn is_selection_message(message_type: MessageType) -> bool {
    matches!(
        message_type,
        MessageType::SourceSelectionRequest | MessageType::SourceSelectionResponse
    )
}

fn require_negotiated_message(
    negotiated: &NegotiatedProtocol,
    message_type: MessageType,
) -> Result<(), NoiseError> {
    if is_selection_message(message_type)
        && (negotiated.version < ProtocolVersion::V1_1
            || !negotiated
                .capabilities
                .contains(&Capability::RemoteSelection))
    {
        return Err(NoiseError::InvalidControlPayload);
    }
    Ok(())
}

/// Strictly decodes a baseline control frame.
/// QSP/1.1 selection messages require `decode_negotiated_control_frame`.
pub fn decode_control_frame<T: ControlPayload>(
    frame: &ApplicationFrame,
    expected_type: MessageType,
) -> Result<T, NoiseError> {
    if is_selection_message(expected_type) {
        return Err(NoiseError::InvalidControlPayload);
    }
    decode_control_frame_inner(frame, expected_type)
}

fn decode_control_frame_inner<T: ControlPayload>(
    frame: &ApplicationFrame,
    expected_type: MessageType,
) -> Result<T, NoiseError> {
    if frame.message_type != expected_type
        || expected_type != T::MESSAGE_TYPE
        || frame.payload.len() > MAX_LOGICAL_PAYLOAD
    {
        return Err(NoiseError::InvalidControlPayload);
    }
    let value: T =
        serde_json::from_slice(&frame.payload).map_err(|_| NoiseError::InvalidControlPayload)?;
    value
        .validate_control()
        .map_err(|_| NoiseError::InvalidControlPayload)?;
    Ok(value)
}

/// Stateful ordered Noise transport. Cryptographic internals never implement Debug.
pub struct SecureChannel {
    state: TransportState,
    reassembly: Option<Reassembly>,
}

impl fmt::Debug for SecureChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureChannel")
            .field("cryptographic_state", &"[REDACTED]")
            .field("reassembly_active", &self.reassembly.is_some())
            .finish()
    }
}

impl SecureChannel {
    /// Encrypts one bounded logical frame into ordered Noise records.
    pub fn seal_frame(&mut self, frame: &ApplicationFrame) -> Result<Vec<Vec<u8>>, NoiseError> {
        if frame.payload.len() > MAX_LOGICAL_PAYLOAD {
            return Err(NoiseError::LogicalFrameTooLarge);
        }
        let segment_count = frame.payload.len().div_ceil(SEGMENT_PAYLOAD_MAX).max(1);
        if segment_count > MAX_SEGMENTS {
            return Err(NoiseError::TooManySegments);
        }
        let mut records = Vec::with_capacity(segment_count);
        for index in 0..segment_count {
            let start = index * SEGMENT_PAYLOAD_MAX;
            let end = (start + SEGMENT_PAYLOAD_MAX).min(frame.payload.len());
            let fragment = &frame.payload[start..end];
            let plaintext = encode_segment(frame, index, segment_count, fragment)?;
            let mut ciphertext = vec![0_u8; plaintext.len() + NOISE_TAG_BYTES];
            let count = self
                .state
                .write_message(&plaintext, &mut ciphertext)
                .map_err(snow_error)?;
            if count == 0 || count > CIPHERTEXT_RECORD_MAX {
                return Err(NoiseError::CiphertextRecordTooLarge);
            }
            ciphertext.truncate(count);
            records.push(ciphertext);
        }
        Ok(records)
    }

    /// Decrypts one ordered Noise record. Returns a frame only after all segments arrive.
    pub fn open_record(&mut self, record: &[u8]) -> Result<Option<ApplicationFrame>, NoiseError> {
        if record.is_empty() || record.len() > CIPHERTEXT_RECORD_MAX {
            return Err(NoiseError::CiphertextRecordTooLarge);
        }
        let mut plaintext = vec![0_u8; PLAINTEXT_RECORD_MAX];
        let count = self
            .state
            .read_message(record, &mut plaintext)
            .map_err(snow_error)?;
        plaintext.truncate(count);
        let segment = decode_segment(&plaintext)?;
        self.accept_segment(segment)
    }

    pub fn rekey_outgoing(&mut self) {
        self.state.rekey_outgoing();
    }
    pub fn rekey_incoming(&mut self) {
        self.state.rekey_incoming();
    }

    fn accept_segment(&mut self, segment: Segment) -> Result<Option<ApplicationFrame>, NoiseError> {
        if segment.index == 0 {
            if self.reassembly.is_some() {
                return Err(NoiseError::InterleavedFrame);
            }
            self.reassembly = Some(Reassembly {
                message_type: segment.message_type,
                request_id: segment.request_id,
                next_index: 0,
                segment_count: segment.segment_count,
                total_len: segment.total_len,
                payload: Vec::with_capacity(segment.total_len),
            });
        }
        let state = self
            .reassembly
            .as_mut()
            .ok_or(NoiseError::UnexpectedSegment)?;
        if state.message_type != segment.message_type
            || state.request_id != segment.request_id
            || state.segment_count != segment.segment_count
            || state.total_len != segment.total_len
            || state.next_index != segment.index
        {
            return Err(NoiseError::UnexpectedSegment);
        }
        state.payload.extend_from_slice(&segment.payload);
        if state.payload.len() > state.total_len {
            return Err(NoiseError::InvalidSegment);
        }
        state.next_index += 1;
        if state.next_index == state.segment_count {
            let state = self
                .reassembly
                .take()
                .ok_or(NoiseError::UnexpectedSegment)?;
            if state.payload.len() != state.total_len {
                return Err(NoiseError::InvalidSegment);
            }
            return Ok(Some(ApplicationFrame {
                message_type: state.message_type,
                request_id: state.request_id,
                payload: state.payload,
            }));
        }
        Ok(None)
    }
}

struct Segment {
    message_type: MessageType,
    request_id: RequestId,
    index: usize,
    segment_count: usize,
    total_len: usize,
    payload: Vec<u8>,
}

struct Reassembly {
    message_type: MessageType,
    request_id: RequestId,
    next_index: usize,
    segment_count: usize,
    total_len: usize,
    payload: Vec<u8>,
}

fn encode_segment(
    frame: &ApplicationFrame,
    index: usize,
    segment_count: usize,
    payload: &[u8],
) -> Result<Vec<u8>, NoiseError> {
    let index = u16::try_from(index).map_err(|_| NoiseError::TooManySegments)?;
    let segment_count = u16::try_from(segment_count).map_err(|_| NoiseError::TooManySegments)?;
    let total_len =
        u32::try_from(frame.payload.len()).map_err(|_| NoiseError::LogicalFrameTooLarge)?;
    let fragment_len = u16::try_from(payload.len()).map_err(|_| NoiseError::InvalidSegment)?;
    let mut output = Vec::with_capacity(SEGMENT_HEADER_BYTES + payload.len());
    output.extend_from_slice(&FRAME_MAGIC);
    output.push(FRAME_VERSION);
    output.push(frame.message_type as u8);
    output.extend_from_slice(&frame.request_id.to_bytes());
    output.extend_from_slice(&index.to_be_bytes());
    output.extend_from_slice(&segment_count.to_be_bytes());
    output.extend_from_slice(&total_len.to_be_bytes());
    output.extend_from_slice(&fragment_len.to_be_bytes());
    output.extend_from_slice(&[0_u8; 2]);
    output.extend_from_slice(payload);
    Ok(output)
}

fn decode_segment(bytes: &[u8]) -> Result<Segment, NoiseError> {
    if bytes.len() < SEGMENT_HEADER_BYTES || bytes[..2] != FRAME_MAGIC || bytes[2] != FRAME_VERSION
    {
        return Err(NoiseError::InvalidSegment);
    }
    let message_type = MessageType::try_from(bytes[3]).map_err(|_| NoiseError::InvalidSegment)?;
    let mut request = [0_u8; 16];
    request.copy_from_slice(&bytes[4..20]);
    let index = usize::from(u16::from_be_bytes([bytes[20], bytes[21]]));
    let segment_count = usize::from(u16::from_be_bytes([bytes[22], bytes[23]]));
    let total_len = u32::from_be_bytes(
        bytes[24..28]
            .try_into()
            .map_err(|_| NoiseError::InvalidSegment)?,
    ) as usize;
    let fragment_len = usize::from(u16::from_be_bytes([bytes[28], bytes[29]]));
    let expected_segments = total_len.div_ceil(SEGMENT_PAYLOAD_MAX).max(1);
    let expected_fragment_len = if index + 1 < segment_count {
        SEGMENT_PAYLOAD_MAX
    } else {
        total_len.saturating_sub(index * SEGMENT_PAYLOAD_MAX)
    };
    if bytes[30..32] != [0, 0]
        || segment_count == 0
        || segment_count > MAX_SEGMENTS
        || segment_count != expected_segments
        || index >= segment_count
        || total_len > MAX_LOGICAL_PAYLOAD
        || fragment_len != expected_fragment_len
        || bytes.len() != SEGMENT_HEADER_BYTES + fragment_len
    {
        return Err(NoiseError::InvalidSegment);
    }
    Ok(Segment {
        message_type,
        request_id: RequestId::from_bytes(request),
        index,
        segment_count,
        total_len,
        payload: bytes[SEGMENT_HEADER_BYTES..].to_vec(),
    })
}

/// Fuzz-only entry point for authenticated plaintext segment parsing.
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub fn fuzz_decode_segment(bytes: &[u8]) -> bool {
    decode_segment(bytes).is_ok()
}

/// Writes a network length prefix followed by one bounded ciphertext record.
pub fn write_record(writer: &mut impl Write, record: &[u8]) -> Result<(), NoiseError> {
    if record.is_empty() || record.len() > CIPHERTEXT_RECORD_MAX {
        return Err(NoiseError::CiphertextRecordTooLarge);
    }
    let length = u32::try_from(record.len()).map_err(|_| NoiseError::CiphertextRecordTooLarge)?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(record)?;
    writer.flush()?;
    Ok(())
}

/// Reads exactly one bounded ciphertext record and rejects truncation before decryption.
pub fn read_record(reader: &mut impl Read) -> Result<Vec<u8>, NoiseError> {
    let mut prefix = [0_u8; 4];
    reader
        .read_exact(&mut prefix)
        .map_err(|error| map_truncation(error, "record prefix"))?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > CIPHERTEXT_RECORD_MAX {
        return Err(NoiseError::CiphertextRecordTooLarge);
    }
    let mut record = vec![0_u8; length];
    reader
        .read_exact(&mut record)
        .map_err(|error| map_truncation(error, "record body"))?;
    Ok(record)
}

/// Handshake packets use the same prefix with a smaller independent bound.
pub fn write_handshake_packet(writer: &mut impl Write, packet: &[u8]) -> Result<(), NoiseError> {
    if packet.is_empty() || packet.len() > HANDSHAKE_MAX_BYTES {
        return Err(NoiseError::InvalidHandshakeFrame);
    }
    writer.write_all(&(packet.len() as u32).to_be_bytes())?;
    writer.write_all(packet)?;
    writer.flush()?;
    Ok(())
}

pub fn read_handshake_packet(reader: &mut impl Read) -> Result<Vec<u8>, NoiseError> {
    let mut prefix = [0_u8; 4];
    reader
        .read_exact(&mut prefix)
        .map_err(|error| map_truncation(error, "handshake prefix"))?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > HANDSHAKE_MAX_BYTES {
        return Err(NoiseError::InvalidHandshakeFrame);
    }
    let mut packet = vec![0_u8; length];
    reader
        .read_exact(&mut packet)
        .map_err(|error| map_truncation(error, "handshake body"))?;
    Ok(packet)
}

fn map_truncation(error: io::Error, context: &'static str) -> NoiseError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        NoiseError::TruncatedFrame(context)
    } else {
        NoiseError::Io(error)
    }
}

fn snow_error(_error: snow::Error) -> NoiseError {
    NoiseError::CryptographicFailure
}

#[derive(Debug, Error)]
pub enum NoiseError {
    #[error("fixed Noise configuration is unavailable")]
    Configuration,
    #[error("Noise cryptographic operation failed")]
    CryptographicFailure,
    #[error("Noise handshake operation is out of order")]
    HandshakeOrder,
    #[error("Noise handshake expired")]
    HandshakeExpired,
    #[error("Noise handshake is incomplete")]
    HandshakeIncomplete,
    #[error("invalid bounded handshake frame")]
    InvalidHandshakeFrame,
    #[error("handshake messages cannot carry application payload")]
    UnexpectedHandshakePayload,
    #[error("completed XX handshake did not expose a 32-byte remote static key")]
    MissingRemoteStatic,
    #[error("remote static identity does not match the pinned key")]
    RemoteIdentityMismatch,
    #[error("ciphertext record exceeds the Noise bound")]
    CiphertextRecordTooLarge,
    #[error("logical QSP frame exceeds the 8 MiB bound")]
    LogicalFrameTooLarge,
    #[error("logical QSP frame requires too many segments")]
    TooManySegments,
    #[error("invalid or mismatched QSP control payload")]
    InvalidControlPayload,
    #[error("invalid QSP segment")]
    InvalidSegment,
    #[error("QSP frames cannot be interleaved on one ordered channel")]
    InterleavedFrame,
    #[error("unexpected QSP segment order or identity")]
    UnexpectedSegment,
    #[error("truncated {0}")]
    TruncatedFrame(&'static str),
    #[error("Noise transport I/O failed: {0}")]
    Io(#[from] io::Error),
}
