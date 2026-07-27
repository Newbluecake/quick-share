//! Concrete TCP + Noise client/server request dispatch for direct transfer orchestration.

use crate::{
    auth::{PeerClaim, classify_peer},
    network::{
        NetworkError, NetworkSession, read_handshake_packet as read_network_handshake,
        write_handshake_packet as write_network_handshake,
    },
    noise::{
        ApplicationFrame, ControlPayload, HandshakeEvidence, NoiseError, NoiseHandshake,
        decode_control_frame, encode_control_frame,
    },
    offer::{AuthorizationToken, OfferManager, OfferResolution, OfferView},
    receiver::{ReceiverError, ReceiverService},
    sender::{TransferTransport, TransportError},
};
use async_trait::async_trait;
use quick_share_core::identity::{DeviceIdentity, TrustedDeviceStore};
use quick_share_protocol::{
    AuthorizationProof, Capabilities, Capability, ChunkAck, ChunkData, DeviceId, ErrorCode,
    InfoRequest, InfoResponse, ManifestEntryKind, MessageType, OfferCreate, OfferDecision,
    OfferStatusResponse, ProtocolError, RequestId, TransferCancel, TransferComplete,
    TransferCompleteAck, TransferStatus, TransferStatusRequest, TransferStatusResponse, negotiate,
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use thiserror::Error;
use tokio::{net::TcpStream, sync::Mutex, time::timeout};
use tokio_util::sync::CancellationToken;

const OFFER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(130);

/// Reconnectable authenticated peer connector.
#[derive(Clone)]
pub struct ClientConnector {
    endpoint: SocketAddr,
    identity: Arc<DeviceIdentity>,
    expected_device_id: Option<DeviceId>,
    expected_remote_key: Option<[u8; 32]>,
    local_info: InfoRequest,
    operation_timeout: Duration,
}

impl ClientConnector {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        endpoint: SocketAddr,
        identity: Arc<DeviceIdentity>,
        expected_device_id: Option<DeviceId>,
        expected_remote_key: Option<[u8; 32]>,
        local_info: InfoRequest,
        operation_timeout: Duration,
    ) -> Self {
        Self {
            endpoint,
            identity,
            expected_device_id,
            expected_remote_key,
            local_info,
            operation_timeout,
        }
    }

    pub async fn connect(&self) -> Result<ConnectedClient, DirectError> {
        if self.operation_timeout.is_zero() {
            return Err(DirectError::Network(NetworkError::InvalidTimeout));
        }
        let connect_timeout = self.operation_timeout.min(Duration::from_secs(5));
        let mut stream = timeout(connect_timeout, TcpStream::connect(self.endpoint))
            .await
            .map_err(|_| DirectError::ConnectTimeout)?
            .map_err(DirectError::Connect)?;
        let cancellation = CancellationToken::new();
        let mut handshake = NoiseHandshake::initiator(&self.identity, self.operation_timeout)?;
        let one = handshake.write_message()?;
        write_network_handshake(&mut stream, &one, self.operation_timeout, &cancellation).await?;
        let two =
            read_network_handshake(&mut stream, self.operation_timeout, &cancellation).await?;
        handshake.read_message(&two)?;
        let three = handshake.write_message()?;
        write_network_handshake(&mut stream, &three, self.operation_timeout, &cancellation).await?;
        let (channel, evidence) = handshake.finish(self.expected_remote_key.as_ref())?;
        if self
            .expected_device_id
            .as_ref()
            .is_some_and(|expected| &evidence.remote_device_id != expected)
        {
            return Err(DirectError::IdentityMismatch);
        }
        let mut session = NetworkSession::new(stream, channel, self.operation_timeout)?;
        let request_id = random_request_id()?;
        let request = encode_control_frame(MessageType::InfoRequest, request_id, &self.local_info)?;
        session.send_frame(&request, &cancellation).await?;
        let response = session.receive_frame(&cancellation).await?;
        verify_response(&response, request_id, MessageType::InfoResponse)?;
        let info: InfoResponse = decode_control_frame(&response, MessageType::InfoResponse)?;
        negotiate(
            self.local_info.protocol_version,
            &self.local_info.capabilities,
            info.protocol_version,
            &info.device.capabilities,
        )
        .map_err(|_| DirectError::IncompatibleProtocol)?;
        if self
            .expected_device_id
            .as_ref()
            .is_some_and(|expected| &info.device.device_id != expected)
            || info.device.device_id != evidence.remote_device_id
            || info.protocol_version.major != self.local_info.protocol_version.major
        {
            session.close().await;
            return Err(DirectError::IdentityMismatch);
        }
        Ok(ConnectedClient {
            session,
            evidence,
            remote_info: info,
        })
    }
}

pub struct ConnectedClient {
    pub session: NetworkSession<TcpStream>,
    pub evidence: HandshakeEvidence,
    pub remote_info: InfoResponse,
}

/// One serialized Noise request channel. File reads/hashes remain concurrently scheduled.
pub struct NoiseClientTransport {
    connector: ClientConnector,
    session: Mutex<NetworkSession<TcpStream>>,
    cancellation: CancellationToken,
}

impl NoiseClientTransport {
    #[must_use]
    pub fn new(connector: ClientConnector, connected: ConnectedClient) -> Self {
        Self {
            connector,
            session: Mutex::new(connected.session),
            cancellation: CancellationToken::new(),
        }
    }

    /// Closes the current socket while retaining connector and authorization state for resume.
    pub async fn disconnect(&self) {
        self.session.lock().await.close().await;
    }

    pub async fn create_offer(
        &self,
        offer: quick_share_protocol::TransferOffer,
    ) -> Result<AuthorizationToken, DirectError> {
        let response: OfferStatusResponse = self
            .exchange(
                MessageType::OfferCreate,
                &OfferCreate { offer },
                MessageType::OfferStatus,
                Some(OFFER_RESPONSE_TIMEOUT),
            )
            .await?;
        match response.status {
            TransferStatus::Accepted => response
                .authorization
                .ok_or(DirectError::InvalidResponse)
                .map(|proof| proof.with_bytes(|bytes| AuthorizationToken::from_bytes(*bytes))),
            TransferStatus::Rejected | TransferStatus::Expired => Err(DirectError::Rejected),
            _ => Err(DirectError::InvalidResponse),
        }
    }

    async fn exchange<Q, R>(
        &self,
        request_type: MessageType,
        request: &Q,
        response_type: MessageType,
        timeout_override: Option<Duration>,
    ) -> Result<R, DirectError>
    where
        Q: ControlPayload + Sync,
        R: ControlPayload,
    {
        let request_id = random_request_id()?;
        let frame = encode_control_frame(request_type, request_id, request)?;
        let mut session = self.session.lock().await;
        if let Some(operation_timeout) = timeout_override {
            session.set_operation_timeout(operation_timeout)?;
        }
        let exchange = async {
            session.send_frame(&frame, &self.cancellation).await?;
            session.receive_frame(&self.cancellation).await
        }
        .await;
        if timeout_override.is_some() && !session.is_closed() {
            session.set_operation_timeout(self.connector.operation_timeout)?;
        }
        let response = exchange?;
        check_remote_error(&response, request_id)?;
        verify_response(&response, request_id, response_type)?;
        decode_control_frame(&response, response_type).map_err(DirectError::Noise)
    }
}

#[async_trait]
impl TransferTransport for NoiseClientTransport {
    async fn status(
        &self,
        request: TransferStatusRequest,
    ) -> Result<TransferStatusResponse, TransportError> {
        self.exchange(
            MessageType::TransferStatus,
            &request,
            MessageType::TransferStatus,
            None,
        )
        .await
        .map_err(map_transport)
    }

    async fn send_fragment(
        &self,
        request_id: RequestId,
        frame: ChunkData,
    ) -> Result<Option<ChunkAck>, TransportError> {
        let request = encode_control_frame(MessageType::ChunkData, request_id, &frame)
            .map_err(|_| TransportError::Fatal)?;
        let mut session = self.session.lock().await;
        session
            .send_frame(&request, &self.cancellation)
            .await
            .map_err(|_| TransportError::Retryable)?;
        if !frame.final_fragment {
            return Ok(None);
        }
        let response = session
            .receive_frame(&self.cancellation)
            .await
            .map_err(|_| TransportError::Retryable)?;
        check_remote_error(&response, request_id).map_err(map_transport)?;
        verify_response(&response, request_id, MessageType::ChunkAck)
            .map_err(|_| TransportError::Fatal)?;
        decode_control_frame(&response, MessageType::ChunkAck)
            .map(Some)
            .map_err(|_| TransportError::Fatal)
    }

    async fn complete(
        &self,
        request: TransferComplete,
    ) -> Result<TransferCompleteAck, TransportError> {
        self.exchange(
            MessageType::TransferComplete,
            &request,
            MessageType::TransferComplete,
            None,
        )
        .await
        .map_err(map_transport)
    }

    async fn cancel(&self, request: TransferCancel) -> Result<(), TransportError> {
        let request_id = random_request_id().map_err(|_| TransportError::Fatal)?;
        let frame = encode_control_frame(MessageType::TransferCancel, request_id, &request)
            .map_err(|_| TransportError::Fatal)?;
        self.session
            .lock()
            .await
            .send_frame(&frame, &self.cancellation)
            .await
            .map_err(|_| TransportError::Retryable)
    }

    async fn reconnect(&self) -> Result<(), TransportError> {
        let connected = self.connector.connect().await.map_err(map_transport)?;
        *self.session.lock().await = connected.session;
        Ok(())
    }
}

#[async_trait]
pub trait OfferPrompt: Send + Sync {
    async fn decide(&self, view: &OfferView) -> Result<(OfferDecision, bool), DirectError>;
}

pub struct ServerContext<P> {
    pub identity: Arc<DeviceIdentity>,
    pub trust_store: TrustedDeviceStore,
    pub local_info: InfoResponse,
    pub offers: Arc<OfferManager>,
    pub receiver: Arc<ReceiverService>,
    pub prompt: Arc<P>,
    pub operation_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSessionOutcome {
    TransferCompleted {
        peer: DeviceId,
        transfer_id: quick_share_protocol::TransferId,
    },
    TransferCancelled,
    OfferRejected,
    Disconnected,
}

impl<P> ServerContext<P>
where
    P: OfferPrompt + 'static,
{
    pub async fn serve_stream(
        &self,
        mut stream: TcpStream,
        peer_address: SocketAddr,
        cancellation: CancellationToken,
    ) -> Result<ServerSessionOutcome, DirectError> {
        let mut handshake = NoiseHandshake::responder(&self.identity, self.operation_timeout)?;
        let one =
            read_network_handshake(&mut stream, self.operation_timeout, &cancellation).await?;
        handshake.read_message(&one)?;
        let two = handshake.write_message()?;
        write_network_handshake(&mut stream, &two, self.operation_timeout, &cancellation).await?;
        let three =
            read_network_handshake(&mut stream, self.operation_timeout, &cancellation).await?;
        handshake.read_message(&three)?;
        let (channel, evidence) = handshake.finish(None)?;
        let mut session = NetworkSession::new(stream, channel, self.operation_timeout)?;
        let authenticated_peer = evidence.remote_device_id.clone();
        let disconnect_guard = DisconnectGuard::new(Arc::clone(&self.receiver));
        disconnect_guard.set_peer(authenticated_peer.clone());
        let mut negotiated_capabilities = None;

        loop {
            let frame = match session.receive_frame(&cancellation).await {
                Ok(frame) => frame,
                Err(
                    NetworkError::Cancelled
                    | NetworkError::Closed
                    | NetworkError::Io(_)
                    | NetworkError::Timeout,
                ) => return Ok(ServerSessionOutcome::Disconnected),
                Err(error) => return Err(error.into()),
            };
            let result = match frame.message_type {
                MessageType::InfoRequest => {
                    let request: InfoRequest =
                        decode_control_frame(&frame, MessageType::InfoRequest)?;
                    let negotiated = negotiate(
                        self.local_info.protocol_version,
                        &self.local_info.device.capabilities,
                        request.protocol_version,
                        &request.capabilities,
                    )
                    .map_err(|_| DirectError::IncompatibleProtocol)?;
                    send_response(
                        &mut session,
                        frame.request_id,
                        MessageType::InfoResponse,
                        &self.local_info,
                        &cancellation,
                    )
                    .await?;
                    negotiated_capabilities = Some(negotiated.capabilities);
                    None
                }
                MessageType::OfferCreate => {
                    let capabilities = negotiated_capabilities
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let create: OfferCreate =
                        decode_control_frame(&frame, MessageType::OfferCreate)?;
                    if !offer_supported(&create.offer, capabilities) {
                        return close_error(&mut session, DirectError::IncompatibleProtocol).await;
                    }
                    let peer = classify_peer(
                        PeerClaim {
                            device_id: create.offer.sender.device_id.clone(),
                            name: create.offer.sender.name.clone(),
                        },
                        &evidence,
                        &self.trust_store,
                    )?;
                    let mut submission = self.offers.create_offer(
                        &peer,
                        Some(peer_address),
                        create.offer,
                        std::time::Instant::now(),
                    )?;
                    let resolution = match submission.resolution.try_recv() {
                        Ok(resolution) => resolution,
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                            match timeout(
                                self.offers.confirmation_timeout(),
                                self.prompt.decide(&submission.view),
                            )
                            .await
                            {
                                Ok(result) => {
                                    let (decision, sas_verified) = result?;
                                    self.offers.decide(
                                        submission.view.transfer_id,
                                        decision,
                                        sas_verified,
                                        std::time::Instant::now(),
                                    )?;
                                }
                                Err(_) => {
                                    self.offers.expire(std::time::Instant::now())?;
                                }
                            }
                            submission
                                .resolution
                                .await
                                .map_err(|_| DirectError::OfferChannel)?
                        }
                        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                            return close_error(&mut session, DirectError::OfferChannel).await;
                        }
                    };
                    let status = resolution.status;
                    let response = offer_response(submission.view.transfer_id, resolution);
                    send_response(
                        &mut session,
                        frame.request_id,
                        MessageType::OfferStatus,
                        &response,
                        &cancellation,
                    )
                    .await?;
                    debug_assert_eq!(peer.device_id(), &authenticated_peer);
                    if matches!(status, TransferStatus::Rejected | TransferStatus::Expired) {
                        Some(ServerSessionOutcome::OfferRejected)
                    } else {
                        None
                    }
                }
                MessageType::TransferStatus => {
                    negotiated_capabilities
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let request: TransferStatusRequest =
                        decode_control_frame(&frame, MessageType::TransferStatus)?;
                    let response = match self.receiver.status(
                        &authenticated_peer,
                        request,
                        std::time::Instant::now(),
                    ) {
                        Ok(response) => response,
                        Err(error) => {
                            send_receiver_error(
                                &mut session,
                                frame.request_id,
                                &error,
                                &cancellation,
                            )
                            .await?;
                            continue;
                        }
                    };
                    send_response(
                        &mut session,
                        frame.request_id,
                        MessageType::TransferStatus,
                        &response,
                        &cancellation,
                    )
                    .await?;
                    None
                }
                MessageType::ChunkData => {
                    negotiated_capabilities
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let request: ChunkData = decode_control_frame(&frame, MessageType::ChunkData)?;
                    let response = match self.receiver.receive_chunk(
                        &authenticated_peer,
                        frame.request_id,
                        request,
                        std::time::Instant::now(),
                    ) {
                        Ok(response) => response,
                        Err(error) => {
                            send_receiver_error(
                                &mut session,
                                frame.request_id,
                                &error,
                                &cancellation,
                            )
                            .await?;
                            continue;
                        }
                    };
                    if let Some(response) = response {
                        send_response(
                            &mut session,
                            frame.request_id,
                            MessageType::ChunkAck,
                            &response,
                            &cancellation,
                        )
                        .await?;
                    }
                    None
                }
                MessageType::TransferComplete => {
                    negotiated_capabilities
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let request: TransferComplete =
                        decode_control_frame(&frame, MessageType::TransferComplete)?;
                    let transfer_id = request.transfer_id;
                    let response = match self.receiver.complete(
                        &authenticated_peer,
                        request,
                        std::time::Instant::now(),
                    ) {
                        Ok(response) => response,
                        Err(error) => {
                            send_receiver_error(
                                &mut session,
                                frame.request_id,
                                &error,
                                &cancellation,
                            )
                            .await?;
                            continue;
                        }
                    };
                    send_response(
                        &mut session,
                        frame.request_id,
                        MessageType::TransferComplete,
                        &response,
                        &cancellation,
                    )
                    .await?;
                    Some(ServerSessionOutcome::TransferCompleted {
                        peer: authenticated_peer.clone(),
                        transfer_id,
                    })
                }
                MessageType::TransferCancel => {
                    negotiated_capabilities
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let request: TransferCancel =
                        decode_control_frame(&frame, MessageType::TransferCancel)?;
                    match self.receiver.cancel(
                        &authenticated_peer,
                        request,
                        std::time::Instant::now(),
                    ) {
                        Ok(()) => Some(ServerSessionOutcome::TransferCancelled),
                        Err(error) => {
                            send_receiver_error(
                                &mut session,
                                frame.request_id,
                                &error,
                                &cancellation,
                            )
                            .await?;
                            Some(ServerSessionOutcome::Disconnected)
                        }
                    }
                }
                _ => return close_error(&mut session, DirectError::InvalidResponse).await,
            };
            if let Some(outcome) = result {
                return Ok(outcome);
            }
        }
    }
}

struct DisconnectGuard {
    receiver: Arc<ReceiverService>,
    peer: StdMutex<Option<DeviceId>>,
}

impl DisconnectGuard {
    fn new(receiver: Arc<ReceiverService>) -> Self {
        Self {
            receiver,
            peer: StdMutex::new(None),
        }
    }

    fn set_peer(&self, peer: DeviceId) {
        if let Ok(mut stored) = self.peer.lock() {
            *stored = Some(peer);
        }
    }
}

impl Drop for DisconnectGuard {
    fn drop(&mut self) {
        if let Ok(peer) = self.peer.lock()
            && let Some(peer) = peer.as_ref()
        {
            let _ = self.receiver.disconnect(peer);
        }
    }
}

fn offer_supported(
    offer: &quick_share_protocol::TransferOffer,
    capabilities: &Capabilities,
) -> bool {
    let content = match offer.content_kind {
        quick_share_protocol::ContentKind::Files => Capability::Files,
        quick_share_protocol::ContentKind::Text => Capability::Text,
    };
    capabilities.contains(&content)
        && offer.entries.iter().all(|entry| match &entry.kind {
            ManifestEntryKind::Directory => capabilities.contains(&Capability::Directories),
            ManifestEntryKind::Symlink { .. } => capabilities.contains(&Capability::Symlinks),
            ManifestEntryKind::File | ManifestEntryKind::Text { .. } => true,
        })
}

fn offer_response(
    transfer_id: quick_share_protocol::TransferId,
    resolution: OfferResolution,
) -> OfferStatusResponse {
    OfferStatusResponse {
        transfer_id,
        status: resolution.status,
        authorization: resolution
            .authorization
            .map(|token| token.with_bytes(|bytes| AuthorizationProof::from_bytes(*bytes))),
    }
}

async fn send_receiver_error(
    session: &mut NetworkSession<TcpStream>,
    request_id: RequestId,
    error: &ReceiverError,
    cancellation: &CancellationToken,
) -> Result<(), DirectError> {
    let code = match error {
        ReceiverError::Unauthorized | ReceiverError::Offer(_) => ErrorCode::Unauthorized,
        ReceiverError::FileStreamLimit | ReceiverError::ReceiveTaskLimit => {
            ErrorCode::ResourceLimit
        }
        ReceiverError::NotFound => ErrorCode::NotFound,
        ReceiverError::Store(
            crate::StoreError::ChunkDigestMismatch { .. }
            | crate::StoreError::ConflictingChunk { .. }
            | crate::StoreError::FinalDigestMismatch { .. }
            | crate::StoreError::InconsistentCommit(_),
        ) => ErrorCode::IntegrityFailed,
        ReceiverError::InvalidLimits
        | ReceiverError::InvalidFrame(_)
        | ReceiverError::ManifestChanged
        | ReceiverError::InvalidText(_)
        | ReceiverError::FragmentOrder
        | ReceiverError::FragmentDescriptorChanged
        | ReceiverError::ChunkAlreadyActive
        | ReceiverError::UploadsActive
        | ReceiverError::Incomplete
        | ReceiverError::Terminal(_)
        | ReceiverError::SkippedMetadata
        | ReceiverError::Path(_) => ErrorCode::InvalidMessage,
        ReceiverError::Store(_) | ReceiverError::Io(_) | ReceiverError::Internal => {
            ErrorCode::Internal
        }
    };
    let message = match code {
        ErrorCode::Unauthorized => "request is not authorized",
        ErrorCode::ResourceLimit => "receiver resource limit reached",
        ErrorCode::NotFound => "transfer was not found",
        ErrorCode::IntegrityFailed => "transfer integrity verification failed",
        ErrorCode::InvalidMessage => "transfer request is invalid for the current state",
        ErrorCode::Internal => "receiver could not complete the operation",
        ErrorCode::IncompatibleProtocol | ErrorCode::Rejected => "transfer request failed",
    };
    send_response(
        session,
        request_id,
        MessageType::ProtocolError,
        &ProtocolError::new(code, message),
        cancellation,
    )
    .await
}

async fn send_response<T: ControlPayload + Sync>(
    session: &mut NetworkSession<TcpStream>,
    request_id: RequestId,
    message_type: MessageType,
    response: &T,
    cancellation: &CancellationToken,
) -> Result<(), DirectError> {
    let response = encode_control_frame(message_type, request_id, response)?;
    session.send_frame(&response, cancellation).await?;
    Ok(())
}

async fn close_error<T>(
    session: &mut NetworkSession<TcpStream>,
    error: DirectError,
) -> Result<T, DirectError> {
    session.close().await;
    Err(error)
}

fn check_remote_error(
    response: &ApplicationFrame,
    request_id: RequestId,
) -> Result<(), DirectError> {
    if response.request_id == request_id && response.message_type == MessageType::ProtocolError {
        let error: ProtocolError = decode_control_frame(response, MessageType::ProtocolError)?;
        return Err(DirectError::Remote(error));
    }
    Ok(())
}

fn verify_response(
    response: &ApplicationFrame,
    request_id: RequestId,
    message_type: MessageType,
) -> Result<(), DirectError> {
    if response.request_id != request_id || response.message_type != message_type {
        return Err(DirectError::InvalidResponse);
    }
    Ok(())
}

fn random_request_id() -> Result<RequestId, DirectError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| DirectError::Random)?;
    Ok(RequestId::from_bytes(bytes))
}

fn map_transport(error: DirectError) -> TransportError {
    match error {
        DirectError::Network(_)
        | DirectError::Connect(_)
        | DirectError::ConnectTimeout
        | DirectError::OfferChannel => TransportError::Retryable,
        DirectError::Rejected => TransportError::Cancelled,
        DirectError::Remote(error) => match error.code {
            ErrorCode::IntegrityFailed => TransportError::Integrity,
            ErrorCode::Unauthorized => TransportError::Unauthorized,
            ErrorCode::ResourceLimit => TransportError::ResourceLimit,
            ErrorCode::Rejected => TransportError::Cancelled,
            ErrorCode::IncompatibleProtocol
            | ErrorCode::InvalidMessage
            | ErrorCode::NotFound
            | ErrorCode::Internal => TransportError::Fatal,
        },
        _ => TransportError::Fatal,
    }
}

#[derive(Debug, Error)]
pub enum DirectError {
    #[error("direct connection timed out")]
    ConnectTimeout,
    #[error("direct TCP connection failed: {0}")]
    Connect(#[source] std::io::Error),
    #[error("Noise network session failed: {0}")]
    Network(#[from] NetworkError),
    #[error("Noise protocol failed: {0}")]
    Noise(#[from] NoiseError),
    #[error("authenticated peer identity does not match the selected receiver")]
    IdentityMismatch,
    #[error("peer has no compatible QSP/1 protocol or capabilities")]
    IncompatibleProtocol,
    #[error("peer rejected or expired the transfer offer")]
    Rejected,
    #[error("peer returned {0}")]
    Remote(ProtocolError),
    #[error("peer returned an invalid or mismatched response")]
    InvalidResponse,
    #[error("operation requires an accepted authenticated peer")]
    Unauthorized,
    #[error("offer confirmation channel closed")]
    OfferChannel,
    #[error("secure request ID generation failed")]
    Random,
    #[error("peer authentication failed: {0}")]
    Authentication(#[from] crate::auth::AuthError),
    #[error("offer state failed: {0}")]
    Offer(#[from] crate::offer::OfferError),
    #[error("receiver state failed: {0}")]
    Receiver(#[from] ReceiverError),
}
