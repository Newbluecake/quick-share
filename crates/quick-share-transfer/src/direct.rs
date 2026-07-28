//! Concrete TCP + Noise client/server request dispatch for direct transfer orchestration.

use crate::{
    auth::{PeerClaim, classify_peer},
    expected_offer::ExpectedOfferRegistry,
    network::{
        NetworkError, NetworkSession, read_handshake_packet as read_network_handshake,
        write_handshake_packet as write_network_handshake,
    },
    noise::{
        ApplicationFrame, ControlPayload, HandshakeEvidence, NoiseError, NoiseHandshake,
        decode_control_frame, decode_negotiated_control_frame, encode_control_frame,
        encode_negotiated_control_frame,
    },
    offer::{AuthorizationToken, OfferManager, OfferResolution, OfferView},
    receiver::{ReceiverEndpoint, ReceiverError, ReceiverService},
    selection::{CallbackTarget, SelectionManager, SelectionResult},
    sender::{TransferTransport, TransportError},
};
use async_trait::async_trait;
use quick_share_core::identity::{DeviceIdentity, TrustedDeviceStore};
use quick_share_protocol::{
    AuthorizationProof, Capabilities, Capability, ChunkAck, ChunkData, DeviceId, ErrorCode,
    InfoRequest, InfoResponse, ManifestEntryKind, MessageType, NegotiatedProtocol, OfferCreate,
    OfferDecision, OfferStatusResponse, ProtocolError, ProtocolVersion, RequestId,
    SourceSelectionRequest, SourceSelectionResponse, SourceSelectionStatus, TransferCancel,
    TransferComplete, TransferCompleteAck, TransferStatus, TransferStatusRequest,
    TransferStatusResponse, compatible_info_response, negotiate,
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
const SELECTION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(11 * 60);

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
        stream.set_nodelay(true).map_err(DirectError::Connect)?;
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
        let negotiated = negotiate(
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
            negotiated,
        })
    }
}

pub struct ConnectedClient {
    pub session: NetworkSession<TcpStream>,
    pub evidence: HandshakeEvidence,
    pub remote_info: InfoResponse,
    pub negotiated: NegotiatedProtocol,
}

/// One serialized Noise request channel. File reads/hashes remain concurrently scheduled.
pub struct NoiseClientTransport {
    connector: ClientConnector,
    session: Mutex<NetworkSession<TcpStream>>,
    negotiated: NegotiatedProtocol,
    cancellation: CancellationToken,
}

impl NoiseClientTransport {
    #[must_use]
    pub fn new(connector: ClientConnector, connected: ConnectedClient) -> Self {
        Self {
            connector,
            session: Mutex::new(connected.session),
            negotiated: connected.negotiated,
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
        self.authorize_offer(offer, false).await
    }

    /// Exchanges one authenticated QSP/1.1 source-selection request with a separate UI timeout.
    pub async fn request_source_selection(
        &self,
        request: SourceSelectionRequest,
    ) -> Result<SelectionExchange, DirectError> {
        let request_id = random_request_id()?;
        let frame = encode_negotiated_control_frame(
            &self.negotiated,
            MessageType::SourceSelectionRequest,
            request_id,
            &request,
        )?;
        let mut session = self.session.lock().await;
        session.set_operation_timeout(SELECTION_RESPONSE_TIMEOUT)?;
        let exchange = async {
            session.send_frame(&frame, &self.cancellation).await?;
            session.receive_frame(&self.cancellation).await
        }
        .await;
        if !session.is_closed() {
            session.set_operation_timeout(self.connector.operation_timeout)?;
        }
        let response = exchange?;
        check_remote_error(&response, request_id)?;
        verify_response(&response, request_id, MessageType::SourceSelectionResponse)?;
        let response = decode_negotiated_control_frame(
            &self.negotiated,
            &response,
            MessageType::SourceSelectionResponse,
        )?;
        Ok(SelectionExchange {
            request_id,
            response,
        })
    }

    /// Re-authorizes the exact same transfer ID and manifest after an interrupted sender exits.
    pub async fn resume_offer(
        &self,
        offer: quick_share_protocol::TransferOffer,
    ) -> Result<AuthorizationToken, DirectError> {
        self.authorize_offer(offer, true).await
    }

    async fn authorize_offer(
        &self,
        offer: quick_share_protocol::TransferOffer,
        resume: bool,
    ) -> Result<AuthorizationToken, DirectError> {
        let response: OfferStatusResponse = self
            .exchange(
                MessageType::OfferCreate,
                &OfferCreate { offer, resume },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionExchange {
    pub request_id: RequestId,
    pub response: SourceSelectionResponse,
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
    /// Prepares an already expected callback before its one-shot authorization grant.
    async fn prepare_expected(
        &self,
        peer: &crate::auth::PeerAuthContext,
        view: &OfferView,
        offer: &quick_share_protocol::TransferOffer,
    ) -> Result<(), DirectError>;

    /// Collects authorization and persists receiver-local routing before returning Accept.
    async fn decide(
        &self,
        peer: &crate::auth::PeerAuthContext,
        view: &OfferView,
        offer: &quick_share_protocol::TransferOffer,
    ) -> Result<(OfferDecision, bool), DirectError>;
}

#[async_trait]
pub trait SelectionDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        request_id: RequestId,
        peer: crate::auth::PeerAuthContext,
        observed_source_ip: std::net::IpAddr,
        request: SourceSelectionRequest,
    ) -> Result<SelectionResult, DirectError>;
}

#[async_trait]
impl<H> SelectionDispatcher for SelectionManager<H>
where
    H: crate::selection::SelectionHandler + 'static,
{
    async fn dispatch(
        &self,
        request_id: RequestId,
        peer: crate::auth::PeerAuthContext,
        observed_source_ip: std::net::IpAddr,
        request: SourceSelectionRequest,
    ) -> Result<SelectionResult, DirectError> {
        self.handle(
            request_id,
            peer,
            observed_source_ip,
            request,
            std::time::Instant::now(),
        )
        .await
        .map_err(DirectError::Selection)
    }
}

pub struct ServerContext<P, R = ReceiverService> {
    pub identity: Arc<DeviceIdentity>,
    pub trust_store: TrustedDeviceStore,
    pub local_info: InfoResponse,
    pub offers: Arc<OfferManager>,
    pub receiver: Arc<R>,
    pub prompt: Arc<P>,
    pub selection: Option<Arc<dyn SelectionDispatcher>>,
    pub expected_offers: Option<Arc<ExpectedOfferRegistry>>,
    pub operation_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSessionOutcome {
    TransferCompleted {
        peer: DeviceId,
        transfer_id: quick_share_protocol::TransferId,
    },
    TransferCancelled {
        peer: DeviceId,
        transfer_id: quick_share_protocol::TransferId,
    },
    OfferRejected,
    SelectionReady {
        peer: DeviceId,
        request_id: RequestId,
        transfer_id: quick_share_protocol::TransferId,
        callback: CallbackTarget,
    },
    SelectionFinished {
        peer: DeviceId,
        request_id: RequestId,
        status: SourceSelectionStatus,
    },
    Disconnected,
}

impl<P, R> ServerContext<P, R>
where
    P: OfferPrompt + 'static,
    R: ReceiverEndpoint + 'static,
{
    pub async fn serve_stream(
        &self,
        mut stream: TcpStream,
        peer_address: SocketAddr,
        cancellation: CancellationToken,
    ) -> Result<ServerSessionOutcome, DirectError> {
        stream.set_nodelay(true).map_err(DirectError::Connect)?;
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
        let mut negotiated_protocol = None;

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
                    let response = compatible_info_response(
                        self.local_info.protocol_version,
                        self.local_info.device.clone(),
                        &request,
                    )
                    .map_err(|_| DirectError::IncompatibleProtocol)?;
                    send_response(
                        &mut session,
                        frame.request_id,
                        MessageType::InfoResponse,
                        &response,
                        &cancellation,
                    )
                    .await?;
                    negotiated_protocol = Some(negotiated);
                    None
                }
                MessageType::SourceSelectionRequest => {
                    let negotiated = negotiated_protocol
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let dispatcher = self
                        .selection
                        .as_ref()
                        .ok_or(DirectError::IncompatibleProtocol)?;
                    let request: SourceSelectionRequest = decode_negotiated_control_frame(
                        negotiated,
                        &frame,
                        MessageType::SourceSelectionRequest,
                    )?;
                    let peer = classify_peer(
                        PeerClaim {
                            device_id: request.requester.device_id.clone(),
                            name: request.requester.name.clone(),
                        },
                        &evidence,
                        &self.trust_store,
                    )?;
                    let selection = dispatcher
                        .dispatch(frame.request_id, peer, peer_address.ip(), request)
                        .await?;
                    send_negotiated_response(
                        &mut session,
                        negotiated,
                        frame.request_id,
                        MessageType::SourceSelectionResponse,
                        &selection.response,
                        &cancellation,
                    )
                    .await?;
                    debug_assert_eq!(
                        selection.response.status == SourceSelectionStatus::Ready,
                        selection.callback.is_some()
                    );
                    Some(match (selection.response.transfer_id, selection.callback) {
                        (Some(transfer_id), Some(callback)) => {
                            ServerSessionOutcome::SelectionReady {
                                peer: authenticated_peer.clone(),
                                request_id: frame.request_id,
                                transfer_id,
                                callback,
                            }
                        }
                        _ => ServerSessionOutcome::SelectionFinished {
                            peer: authenticated_peer.clone(),
                            request_id: frame.request_id,
                            status: selection.response.status,
                        },
                    })
                }
                MessageType::OfferCreate => {
                    let capabilities = &negotiated_protocol
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?
                        .capabilities;
                    let create: OfferCreate =
                        decode_control_frame(&frame, MessageType::OfferCreate)?;
                    if create.offer.initiated_by.is_some()
                        && (negotiated_protocol
                            .as_ref()
                            .is_none_or(|protocol| protocol.version < ProtocolVersion::V1_1)
                            || !capabilities.contains(&Capability::RemoteSelection))
                    {
                        return close_error(&mut session, DirectError::IncompatibleProtocol).await;
                    }
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
                    let now = std::time::Instant::now();
                    let submitted_offer = create.offer.clone();
                    let expected_callback = !create.resume && create.offer.initiated_by.is_some();
                    if expected_callback {
                        self.expected_offers
                            .as_ref()
                            .ok_or(DirectError::Unauthorized)?
                            .consume(
                                peer.device_id(),
                                &peer.public_key(),
                                peer_address.ip(),
                                &create.offer,
                                now,
                            )?;
                    }
                    let mut submission = if create.resume {
                        self.offers
                            .resume_offer(&peer, Some(peer_address), create.offer, now)?
                    } else {
                        self.offers
                            .create_offer(&peer, Some(peer_address), create.offer, now)?
                    };
                    if expected_callback {
                        self.prompt
                            .prepare_expected(&peer, &submission.view, &submitted_offer)
                            .await?;
                        self.offers.decide(
                            submission.view.transfer_id,
                            OfferDecision::AcceptOnce,
                            false,
                            now,
                        )?;
                    }
                    let resolution = match submission.resolution.try_recv() {
                        Ok(resolution) => resolution,
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                            match timeout(
                                self.offers.confirmation_timeout(),
                                self.prompt
                                    .decide(&peer, &submission.view, &submitted_offer),
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
                    negotiated_protocol
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
                    negotiated_protocol
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
                    negotiated_protocol
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
                    negotiated_protocol
                        .as_ref()
                        .ok_or(DirectError::InvalidResponse)?;
                    let request: TransferCancel =
                        decode_control_frame(&frame, MessageType::TransferCancel)?;
                    let transfer_id = request.transfer_id;
                    match self.receiver.cancel(
                        &authenticated_peer,
                        request,
                        std::time::Instant::now(),
                    ) {
                        Ok(()) => Some(ServerSessionOutcome::TransferCancelled {
                            peer: authenticated_peer.clone(),
                            transfer_id,
                        }),
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

struct DisconnectGuard<R: ReceiverEndpoint> {
    receiver: Arc<R>,
    peer: StdMutex<Option<DeviceId>>,
}

impl<R> DisconnectGuard<R>
where
    R: ReceiverEndpoint,
{
    fn new(receiver: Arc<R>) -> Self {
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

impl<R> Drop for DisconnectGuard<R>
where
    R: ReceiverEndpoint,
{
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
        ReceiverError::Store(crate::StoreError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::StorageFull
                    | std::io::ErrorKind::QuotaExceeded
                    | std::io::ErrorKind::FileTooLarge
                    | std::io::ErrorKind::PermissionDenied
            ) =>
        {
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
        | ReceiverError::ConflictPending { .. }
        | ReceiverError::Terminal(_)
        | ReceiverError::SkippedMetadata
        | ReceiverError::Path(_)
        | ReceiverError::DestinationPlan(_) => ErrorCode::InvalidMessage,
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

async fn send_negotiated_response<T: ControlPayload + Sync>(
    session: &mut NetworkSession<TcpStream>,
    negotiated: &NegotiatedProtocol,
    request_id: RequestId,
    message_type: MessageType,
    response: &T,
    cancellation: &CancellationToken,
) -> Result<(), DirectError> {
    let response = encode_negotiated_control_frame(negotiated, message_type, request_id, response)?;
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
    #[error("source selection failed: {0}")]
    Selection(#[from] crate::selection::SelectionError),
    #[error("expected callback offer failed: {0}")]
    ExpectedOffer(#[from] crate::expected_offer::ExpectedOfferError),
}
