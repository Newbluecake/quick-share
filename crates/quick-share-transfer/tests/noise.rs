use proptest::prelude::*;
use quick_share_core::identity::{DeviceIdentity, IdentityStore};
use quick_share_protocol::{
    AuthorizationProof, Capability, ChunkData, ChunkDescriptor, DeviceInfo, EntryId, ErrorCode,
    InfoRequest, InfoResponse, MAX_CHUNK_FRAME_BYTES, MessageType, ProtocolError, ProtocolVersion,
    RequestId, TransferId,
};
use quick_share_transfer::network::{
    NetworkError, NetworkSession, read_handshake_packet as read_network_handshake_packet,
    write_handshake_packet as write_network_handshake_packet,
};
use quick_share_transfer::noise::{
    ApplicationFrame, HandshakeEvidence, NoiseError, NoiseHandshake, NoiseRole, SasCode,
    SecureChannel, decode_control_frame, encode_control_frame, read_handshake_packet, read_record,
    write_handshake_packet, write_record,
};
use std::{
    io::Cursor,
    time::{Duration, Instant},
};
use tempfile::tempdir;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn identities() -> (tempfile::TempDir, DeviceIdentity, DeviceIdentity) {
    let root = tempdir().expect("identity root");
    let initiator = IdentityStore::new(root.path().join("initiator.json"))
        .load_or_create()
        .expect("initiator identity");
    let responder = IdentityStore::new(root.path().join("responder.json"))
        .load_or_create()
        .expect("responder identity");
    (root, initiator, responder)
}

fn establish(
    initiator_identity: &DeviceIdentity,
    responder_identity: &DeviceIdentity,
) -> (
    SecureChannel,
    HandshakeEvidence,
    SecureChannel,
    HandshakeEvidence,
) {
    let timeout = Duration::from_secs(5);
    let mut initiator = NoiseHandshake::initiator(initiator_identity, timeout).expect("initiator");
    let mut responder = NoiseHandshake::responder(responder_identity, timeout).expect("responder");

    let message_one = initiator.write_message().expect("XX message one");
    responder
        .read_message(&message_one)
        .expect("read message one");
    let message_two = responder.write_message().expect("XX message two");
    initiator
        .read_message(&message_two)
        .expect("read message two");
    let message_three = initiator.write_message().expect("XX message three");
    responder
        .read_message(&message_three)
        .expect("read message three");

    let expected_responder = responder_identity.public_key();
    let expected_initiator = initiator_identity.public_key();
    let (initiator_channel, initiator_evidence) = initiator
        .finish(Some(&expected_responder))
        .expect("finish initiator");
    let (responder_channel, responder_evidence) = responder
        .finish(Some(&expected_initiator))
        .expect("finish responder");
    (
        initiator_channel,
        initiator_evidence,
        responder_channel,
        responder_evidence,
    )
}

fn request_id() -> RequestId {
    RequestId::new(Uuid::now_v7())
}

#[tokio::test]
async fn kernel_tcp_stream_carries_bounded_encrypted_application_frames() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (initiator_channel, _, responder_channel, _) =
        establish(&initiator_identity, &responder_identity);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind loopback");
    let address = listener.local_addr().expect("listener address");
    let (initiator_stream, accepted) =
        tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
    let initiator_stream = initiator_stream.expect("connect");
    let (responder_stream, _) = accepted.expect("accept");
    let mut initiator =
        NetworkSession::new(initiator_stream, initiator_channel, Duration::from_secs(1))
            .expect("initiator session");
    let mut responder =
        NetworkSession::new(responder_stream, responder_channel, Duration::from_secs(1))
            .expect("responder session");
    let frame = ApplicationFrame {
        message_type: MessageType::InfoRequest,
        request_id: request_id(),
        payload: b"kernel TCP".to_vec(),
    };
    let cancellation = CancellationToken::new();
    let (sent, received) = tokio::join!(
        initiator.send_frame(&frame, &cancellation),
        responder.receive_frame(&cancellation)
    );
    sent.expect("send TCP frame");
    assert_eq!(received.expect("receive TCP frame"), frame);
}

#[tokio::test]
async fn async_network_session_round_trips_and_fails_closed_on_slow_or_invalid_records() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (mut slow_handshake_peer, mut slow_handshake_stream) = tokio::io::duplex(64);
    slow_handshake_peer
        .write_all(&[0, 0])
        .await
        .expect("partial handshake header");
    assert!(matches!(
        read_network_handshake_packet(
            &mut slow_handshake_stream,
            Duration::from_millis(20),
            &CancellationToken::new(),
        )
        .await,
        Err(NetworkError::Timeout)
    ));
    let (mut invalid_handshake_stream, _peer) = tokio::io::duplex(64);
    assert!(matches!(
        write_network_handshake_packet(
            &mut invalid_handshake_stream,
            &[],
            Duration::from_secs(1),
            &CancellationToken::new(),
        )
        .await,
        Err(NetworkError::InvalidRecord)
    ));

    let (initiator_channel, _, responder_channel, _) =
        establish(&initiator_identity, &responder_identity);
    let (initiator_stream, responder_stream) = tokio::io::duplex(4096);
    let mut initiator =
        NetworkSession::new(initiator_stream, initiator_channel, Duration::from_secs(1))
            .expect("initiator session");
    let mut responder =
        NetworkSession::new(responder_stream, responder_channel, Duration::from_secs(1))
            .expect("responder session");
    let frame = ApplicationFrame {
        message_type: MessageType::InfoRequest,
        request_id: request_id(),
        payload: b"bounded".to_vec(),
    };
    let cancellation = CancellationToken::new();
    let (sent, received) = tokio::join!(
        initiator.send_frame(&frame, &cancellation),
        responder.receive_frame(&cancellation)
    );
    sent.expect("send frame");
    assert_eq!(received.expect("receive frame"), frame);

    let (mut slow_peer, slow_stream) = tokio::io::duplex(64);
    let (_, _, slow_channel, _) = establish(&initiator_identity, &responder_identity);
    let mut slow_session =
        NetworkSession::new(slow_stream, slow_channel, Duration::from_millis(20))
            .expect("slow session");
    slow_peer.write_all(&[0, 0]).await.expect("partial header");
    assert!(matches!(
        slow_session.receive_frame(&CancellationToken::new()).await,
        Err(NetworkError::Timeout)
    ));
    assert!(slow_session.is_closed());
    assert!(matches!(
        slow_session.receive_frame(&CancellationToken::new()).await,
        Err(NetworkError::Closed)
    ));

    let (mut invalid_peer, invalid_stream) = tokio::io::duplex(64);
    let (_, _, invalid_channel, _) = establish(&initiator_identity, &responder_identity);
    let mut invalid_session =
        NetworkSession::new(invalid_stream, invalid_channel, Duration::from_secs(1))
            .expect("invalid session");
    invalid_peer
        .write_all(&(u32::MAX).to_be_bytes())
        .await
        .expect("invalid length");
    assert!(matches!(
        invalid_session
            .receive_frame(&CancellationToken::new())
            .await,
        Err(NetworkError::InvalidRecord)
    ));
    assert!(invalid_session.is_closed());
}

#[test]
fn xx_proves_static_keys_derives_matching_sas_and_transports_segmented_frames() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (mut initiator, initiator_evidence, mut responder, responder_evidence) =
        establish(&initiator_identity, &responder_identity);
    let payload = vec![0x5a; 200_000];
    let frame = ApplicationFrame {
        message_type: MessageType::OfferCreate,
        request_id: request_id(),
        payload: payload.clone(),
    };

    let records = initiator.seal_frame(&frame).expect("seal segmented frame");
    let mut observed = None;
    for record in records {
        observed = responder
            .open_record(&record)
            .expect("open record")
            .or(observed);
    }

    assert_eq!(
        initiator_evidence.remote_static(),
        responder_identity.public_key()
    );
    assert_eq!(
        responder_evidence.remote_static(),
        initiator_identity.public_key()
    );
    assert_eq!(initiator_evidence.sas, responder_evidence.sas);
    assert_eq!(observed.expect("reassembled frame"), frame);
    assert!(format!("{initiator_evidence:?}").contains("[REDACTED]"));
    assert!(
        !format!("{initiator_evidence:?}").contains(&hex::encode(responder_identity.public_key()))
    );
}

#[test]
fn logical_frame_accepts_exact_eight_mib_and_rejects_one_byte_more() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (mut initiator, _, mut responder, _) = establish(&initiator_identity, &responder_identity);
    let frame = ApplicationFrame {
        message_type: MessageType::OfferCreate,
        request_id: request_id(),
        payload: vec![0x41; 8 * 1024 * 1024],
    };

    let records = initiator.seal_frame(&frame).expect("8 MiB frame");
    let mut decoded = None;
    for record in records {
        if let Some(value) = responder.open_record(&record).expect("bounded record") {
            decoded = Some(value);
        }
    }
    assert_eq!(decoded.expect("complete frame"), frame);
    let too_large = ApplicationFrame {
        message_type: MessageType::OfferCreate,
        request_id: request_id(),
        payload: vec![0; 8 * 1024 * 1024 + 1],
    };
    assert!(matches!(
        initiator.seal_frame(&too_large),
        Err(NoiseError::LogicalFrameTooLarge)
    ));
}

#[test]
fn wrong_static_pin_fails_closed_before_transport() {
    let (_root, initiator_identity, responder_identity) = identities();
    let mut initiator =
        NoiseHandshake::initiator(&initiator_identity, Duration::from_secs(5)).expect("initiator");
    let mut responder =
        NoiseHandshake::responder(&responder_identity, Duration::from_secs(5)).expect("responder");
    let one = initiator.write_message().expect("one");
    responder.read_message(&one).expect("read one");
    let two = responder.write_message().expect("two");
    initiator.read_message(&two).expect("read two");
    let three = initiator.write_message().expect("three");
    responder.read_message(&three).expect("read three");

    assert!(matches!(
        initiator.finish(Some(&[0_u8; 32])),
        Err(NoiseError::RemoteIdentityMismatch)
    ));
}

#[test]
fn handshake_debug_and_errors_never_include_private_key_material() {
    let (root, initiator_identity, _responder_identity) = identities();
    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.path().join("initiator.json")).expect("identity file"),
    )
    .expect("identity JSON");
    let private_key = persisted["privateKey"].as_str().expect("private key");
    let handshake =
        NoiseHandshake::initiator(&initiator_identity, Duration::from_secs(5)).expect("handshake");
    let debug = format!("{handshake:?}");
    let error = NoiseError::CryptographicFailure.to_string();

    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(private_key));
    assert!(!error.contains(private_key));
}

#[test]
fn handshake_deadline_order_and_replayed_handshake_messages_fail() {
    let (_root, initiator_identity, responder_identity) = identities();
    let now = Instant::now();
    let mut expired = NoiseHandshake::new_at(
        NoiseRole::Initiator,
        &initiator_identity,
        now,
        Duration::from_millis(1),
    )
    .expect("expired handshake");
    assert!(matches!(
        expired.write_message_at(now + Duration::from_millis(1)),
        Err(NoiseError::HandshakeExpired)
    ));

    let mut initiator =
        NoiseHandshake::initiator(&initiator_identity, Duration::from_secs(5)).expect("initiator");
    let mut responder =
        NoiseHandshake::responder(&responder_identity, Duration::from_secs(5)).expect("responder");
    let one = initiator.write_message().expect("one");
    responder.read_message(&one).expect("first read");
    assert!(matches!(
        responder.read_message(&one),
        Err(NoiseError::HandshakeOrder)
    ));
}

#[test]
fn ciphertext_replay_truncation_and_oversized_records_fail_closed() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (mut initiator, _, mut responder, _) = establish(&initiator_identity, &responder_identity);
    let frame = ApplicationFrame {
        message_type: MessageType::InfoRequest,
        request_id: request_id(),
        payload: b"bounded".to_vec(),
    };
    let record = initiator.seal_frame(&frame).expect("seal").remove(0);
    assert_eq!(
        responder.open_record(&record).expect("first open"),
        Some(frame)
    );
    assert!(matches!(
        responder.open_record(&record),
        Err(NoiseError::CryptographicFailure)
    ));

    let mut truncated = Cursor::new([0, 0, 0, 10, 1, 2, 3]);
    assert!(matches!(
        read_record(&mut truncated),
        Err(NoiseError::TruncatedFrame("record body"))
    ));
    let mut oversized = Cursor::new(u32::MAX.to_be_bytes());
    assert!(matches!(
        read_record(&mut oversized),
        Err(NoiseError::CiphertextRecordTooLarge)
    ));
    let mut oversized_handshake = Cursor::new(u32::MAX.to_be_bytes());
    assert!(matches!(
        read_handshake_packet(&mut oversized_handshake),
        Err(NoiseError::InvalidHandshakeFrame)
    ));
}

#[test]
fn strict_control_codec_binds_payload_to_assigned_message_type() {
    let info = InfoRequest {
        protocol_version: ProtocolVersion::V1_0,
        capabilities: std::collections::BTreeSet::from([Capability::Files]),
    };
    let frame = encode_control_frame(MessageType::InfoRequest, request_id(), &info)
        .expect("encode INFO_REQUEST");

    let decoded: InfoRequest =
        decode_control_frame(&frame, MessageType::InfoRequest).expect("decode INFO_REQUEST");

    assert_eq!(decoded, info);
    assert!(encode_control_frame(MessageType::OfferCreate, request_id(), &info).is_err());
    assert!(decode_control_frame::<InfoRequest>(&frame, MessageType::OfferCreate).is_err());
    let mut unknown: serde_json::Value = serde_json::from_slice(&frame.payload).expect("JSON");
    unknown["admin"] = serde_json::Value::Bool(true);
    let malformed = ApplicationFrame {
        payload: serde_json::to_vec(&unknown).expect("malformed JSON"),
        ..frame
    };
    assert!(decode_control_frame::<InfoRequest>(&malformed, MessageType::InfoRequest).is_err());

    let (_root, identity, _) = identities();
    let invalid_info = InfoResponse {
        protocol_version: ProtocolVersion::V1_0,
        device: DeviceInfo {
            device_id: identity.device_id(),
            name: "terminal\u{1b}injection".to_owned(),
            capabilities: std::collections::BTreeSet::new(),
        },
    };
    assert!(encode_control_frame(MessageType::InfoResponse, request_id(), &invalid_info).is_err());
    let invalid_frame = ApplicationFrame {
        message_type: MessageType::InfoResponse,
        request_id: request_id(),
        payload: serde_json::to_vec(&invalid_info).expect("invalid INFO JSON"),
    };
    assert!(
        decode_control_frame::<InfoResponse>(&invalid_frame, MessageType::InfoResponse).is_err()
    );

    let chunk = ChunkData {
        authorization: AuthorizationProof::from_bytes([4; 32]),
        descriptor: ChunkDescriptor {
            transfer_id: TransferId::new(Uuid::now_v7()),
            entry_id: EntryId::new(1).expect("entry"),
            index: 0,
            offset: 0,
            length: MAX_CHUNK_FRAME_BYTES as u32,
            digest: [5; 32],
        },
        fragment_offset: 0,
        final_fragment: true,
        payload: vec![0; MAX_CHUNK_FRAME_BYTES],
    };
    let chunk_frame = encode_control_frame(MessageType::ChunkData, request_id(), &chunk)
        .expect("bounded CHUNK_DATA");
    let decoded_chunk: ChunkData =
        decode_control_frame(&chunk_frame, MessageType::ChunkData).expect("decode CHUNK_DATA");
    assert_eq!(decoded_chunk.payload.len(), MAX_CHUNK_FRAME_BYTES);

    let remote_error = ProtocolError::new(ErrorCode::IntegrityFailed, "integrity failed");
    let error_frame = encode_control_frame(MessageType::ProtocolError, request_id(), &remote_error)
        .expect("bounded protocol error");
    let decoded_error: ProtocolError =
        decode_control_frame(&error_frame, MessageType::ProtocolError).expect("decode error");
    assert_eq!(decoded_error.code, ErrorCode::IntegrityFailed);
    assert!(
        encode_control_frame(
            MessageType::ProtocolError,
            request_id(),
            &ProtocolError::new(ErrorCode::Internal, "bad\u{1b}[31m"),
        )
        .is_err()
    );
}

#[test]
fn network_record_helpers_round_trip_without_accepting_trailing_or_empty_lengths() {
    let mut bytes = Vec::new();
    write_record(&mut bytes, b"ciphertext").expect("write record");
    assert_eq!(
        read_record(&mut Cursor::new(bytes)).expect("read record"),
        b"ciphertext"
    );

    let mut handshake = Vec::new();
    write_handshake_packet(&mut handshake, b"handshake").expect("write handshake");
    assert_eq!(
        read_handshake_packet(&mut Cursor::new(handshake)).expect("read handshake"),
        b"handshake"
    );
    assert!(write_record(&mut Vec::new(), b"").is_err());
    assert!(write_handshake_packet(&mut Vec::new(), b"").is_err());
}

#[test]
fn coordinated_rekey_preserves_transport_and_uncoordinated_key_fails() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (mut initiator, _, mut responder, _) = establish(&initiator_identity, &responder_identity);
    initiator.rekey_outgoing();
    responder.rekey_incoming();
    let frame = ApplicationFrame {
        message_type: MessageType::InfoResponse,
        request_id: request_id(),
        payload: b"after rekey".to_vec(),
    };
    let record = initiator
        .seal_frame(&frame)
        .expect("seal rekeyed")
        .remove(0);
    assert_eq!(
        responder.open_record(&record).expect("open rekeyed"),
        Some(frame)
    );

    initiator.rekey_outgoing();
    let record = initiator
        .seal_frame(&ApplicationFrame {
            message_type: MessageType::InfoRequest,
            request_id: request_id(),
            payload: vec![],
        })
        .expect("seal uncoordinated")
        .remove(0);
    assert!(matches!(
        responder.open_record(&record),
        Err(NoiseError::CryptographicFailure)
    ));
}

#[test]
fn terminating_mitm_sessions_produce_different_endpoint_sas_codes() {
    let root = tempdir().expect("identity root");
    let client = IdentityStore::new(root.path().join("client.json"))
        .load_or_create()
        .expect("client");
    let server = IdentityStore::new(root.path().join("server.json"))
        .load_or_create()
        .expect("server");
    let mitm_left = IdentityStore::new(root.path().join("mitm-left.json"))
        .load_or_create()
        .expect("MITM left");
    let mitm_right = IdentityStore::new(root.path().join("mitm-right.json"))
        .load_or_create()
        .expect("MITM right");

    let (_, client_evidence, _, _) = establish(&client, &mitm_left);
    let (_, _, _, server_evidence) = establish(&mitm_right, &server);

    assert_ne!(client_evidence.sas, server_evidence.sas);
}

proptest! {
    #[test]
    fn arbitrary_length_prefixed_records_never_panic(input in proptest::collection::vec(any::<u8>(), 0..100_000)) {
        let result = std::panic::catch_unwind(|| read_record(&mut Cursor::new(input)));
        prop_assert!(result.is_ok());
    }
}

#[test]
fn sas_is_always_six_decimal_digits() {
    let (_root, initiator_identity, responder_identity) = identities();
    let (_, evidence, _, _) = establish(&initiator_identity, &responder_identity);
    let rendered = evidence.sas.to_string();
    assert_eq!(rendered.len(), 6);
    assert!(rendered.bytes().all(|byte| byte.is_ascii_digit()));
    let _: SasCode = evidence.sas;
}
