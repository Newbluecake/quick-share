use proptest::prelude::*;
use quick_share_protocol::{
    AuthorizationProof, Capability, ChunkData, ChunkDescriptor, ContentKind, DecodeError, DeviceId,
    DeviceInfo, EntryId, EntryTransferStatus, ErrorCode, InfoRequest, InfoResponse,
    MAX_CHUNK_FRAME_BYTES, MAX_MANIFEST_ENTRIES, ManifestEntry, ManifestEntryKind, MessageType,
    MissingChunkBitmap, OfferCreate, OfferDecision, OfferStatusRequest, OfferStatusResponse,
    ProtocolError, ProtocolVersion, RejectionReason, RequestId, SourceSelectionRequest,
    SourceSelectionResponse, SourceSelectionStatus, TransferComplete, TransferId, TransferOffer,
    TransferStatus, TransferStatusResponse, ValidationError, compatible_info_response,
    decode_offer, negotiate,
};
use std::collections::BTreeSet;
use uuid::Uuid;

fn valid_offer() -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id: TransferId::new(Uuid::now_v7()),
        initiated_by: None,
        sender: DeviceInfo {
            device_id: DeviceId::parse("qs_0123456789abcdef0123456789abcdef")
                .expect("valid device ID"),
            name: "test-device".to_owned(),
            capabilities: BTreeSet::from([
                Capability::Files,
                Capability::Directories,
                Capability::Resume,
            ]),
        },
        content_kind: ContentKind::Files,
        chunk_size: 4 * 1024 * 1024,
        total_bytes: 12,
        entries: vec![ManifestEntry {
            id: EntryId::new(1).expect("nonzero entry ID"),
            relative_path: "docs/readme.txt".to_owned(),
            kind: ManifestEntryKind::File,
            size: 12,
            digest: Some([7; 32]),
        }],
    }
}

#[test]
fn fixture_round_trips_without_losing_wire_fields() {
    // Arrange
    let fixture = include_str!("fixtures/protocol/v1/offer.json");

    // Act
    let offer: TransferOffer = serde_json::from_str(fixture).expect("fixture should deserialize");
    offer.validate().expect("fixture should validate");
    let encoded = serde_json::to_string(&offer).expect("offer should serialize");
    let decoded: TransferOffer = serde_json::from_str(&encoded).expect("round trip");

    // Assert
    assert_eq!(decoded, offer);
    assert_eq!(offer.entries[0].relative_path, "docs/readme.txt");
}

#[test]
fn negotiation_rejects_different_major_versions() {
    // Arrange
    let local = ProtocolVersion::new(1, 3);
    let remote = ProtocolVersion::new(2, 0);

    // Act
    let result = negotiate(local, &BTreeSet::new(), remote, &BTreeSet::new());

    // Assert
    assert!(matches!(
        result,
        Err(ProtocolError {
            code: ErrorCode::IncompatibleProtocol,
            ..
        })
    ));
}

#[test]
fn negotiation_uses_lower_minor_and_capability_intersection() {
    // Arrange
    let local = BTreeSet::from([
        Capability::Files,
        Capability::Resume,
        Capability::RemoteSelection,
    ]);
    let remote = BTreeSet::from([
        Capability::Files,
        Capability::Text,
        Capability::RemoteSelection,
    ]);

    // Act
    let negotiated = negotiate(
        ProtocolVersion::new(1, 3),
        &local,
        ProtocolVersion::new(1, 1),
        &remote,
    )
    .expect("same major should negotiate");

    // Assert
    assert_eq!(negotiated.version, ProtocolVersion::new(1, 1));
    assert_eq!(
        negotiated.capabilities,
        BTreeSet::from([Capability::Files, Capability::RemoteSelection])
    );

    let downgraded = negotiate(
        ProtocolVersion::V1_1,
        &local,
        ProtocolVersion::V1_0,
        &remote,
    )
    .expect("QSP/1.0 should downgrade");
    assert_eq!(downgraded.version, ProtocolVersion::V1_0);
    assert_eq!(downgraded.capabilities, BTreeSet::from([Capability::Files]));
}

#[test]
fn validation_rejects_oversized_device_names_and_manifests() {
    // Arrange
    let mut long_name = valid_offer();
    long_name.sender.name = "x".repeat(65);
    let mut control_name = valid_offer();
    control_name.sender.name = "trusted\u{1b}[2J".to_owned();
    let mut wrong_content_kind = valid_offer();
    wrong_content_kind.content_kind = ContentKind::Text;
    let mut too_many_entries = valid_offer();
    too_many_entries.entries = (1..=MAX_MANIFEST_ENTRIES + 1)
        .map(|id| ManifestEntry {
            id: EntryId::new(u32::try_from(id).expect("valid ID")).expect("valid ID"),
            relative_path: format!("file-{id}"),
            kind: ManifestEntryKind::File,
            size: 0,
            digest: Some([0; 32]),
        })
        .collect();

    // Act + Assert
    assert_eq!(
        long_name.validate(),
        Err(ValidationError::DeviceNameTooLong)
    );
    assert_eq!(
        control_name.validate(),
        Err(ValidationError::InvalidDeviceName)
    );
    assert_eq!(
        wrong_content_kind.validate(),
        Err(ValidationError::InvalidContentKind)
    );
    assert_eq!(
        too_many_entries.validate(),
        Err(ValidationError::TooManyEntries)
    );
}

#[test]
fn chunk_fragments_resume_bitmaps_and_authorization_are_strictly_bounded() {
    let offer = valid_offer();
    let descriptor = ChunkDescriptor {
        transfer_id: offer.transfer_id,
        entry_id: EntryId::new(1).expect("entry"),
        index: 0,
        offset: 0,
        length: 4,
        digest: [3; 32],
    };
    let proof = AuthorizationProof::from_bytes([9; 32]);
    assert!(!format!("{proof:?}").contains("9, 9"));
    let frame = ChunkData {
        authorization: proof,
        descriptor: descriptor.clone(),
        fragment_offset: 0,
        final_fragment: true,
        payload: vec![1; 4],
    };
    assert_eq!(frame.validate(), Ok(()));
    let debug = format!("{frame:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("1, 1, 1, 1"));
    let oversized = ChunkData {
        authorization: AuthorizationProof::from_bytes([8; 32]),
        descriptor,
        fragment_offset: 0,
        final_fragment: false,
        payload: vec![0; MAX_CHUNK_FRAME_BYTES + 1],
    };
    assert_eq!(oversized.validate(), Err(ValidationError::InvalidChunkSize));

    let bitmap = MissingChunkBitmap::from_missing(10, [0, 3, 9]).expect("bitmap");
    assert!(bitmap.is_missing(0));
    assert!(!bitmap.is_missing(1));
    assert!(bitmap.is_missing(9));
    assert_eq!(bitmap.validate(), Ok(()));
    let response = TransferStatusResponse {
        transfer_id: offer.transfer_id,
        status: TransferStatus::Paused,
        manifest_digest: [4; 32],
        entries: vec![EntryTransferStatus {
            entry_id: EntryId::new(1).expect("entry"),
            chunks: bitmap,
        }],
    };
    assert_eq!(response.validate(), Ok(()));

    let completion = TransferComplete {
        transfer_id: offer.transfer_id,
        authorization: AuthorizationProof::from_bytes([7; 32]),
        manifest_digest: [4; 32],
    };
    let mut value = serde_json::to_value(completion).expect("completion JSON");
    value["bypassDigest"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<TransferComplete>(value).is_err());
}

#[test]
fn security_sensitive_structs_reject_unknown_fields() {
    // Arrange
    let fixture = include_str!("fixtures/protocol/v1/offer.json");
    let value: serde_json::Value = serde_json::from_str(fixture).expect("JSON fixture");
    let mut object = value.as_object().expect("fixture object").clone();
    object.insert(
        "bypassAuthorization".to_owned(),
        serde_json::Value::Bool(true),
    );

    // Act
    let result = serde_json::from_value::<TransferOffer>(serde_json::Value::Object(object));

    // Assert
    assert!(result.is_err());
}

#[test]
fn info_and_offer_control_types_are_strict_and_message_codes_are_stable() {
    let offer = valid_offer();
    let info_request = InfoRequest {
        protocol_version: ProtocolVersion::V1_0,
        capabilities: BTreeSet::from([Capability::Files]),
    };
    let info_response = InfoResponse {
        protocol_version: ProtocolVersion::V1_0,
        device: offer.sender.clone(),
    };
    let create = OfferCreate {
        offer: offer.clone(),
        resume: false,
    };
    let legacy_create: OfferCreate = serde_json::from_value(serde_json::json!({
        "offer": offer.clone()
    }))
    .expect("legacy create defaults to a fresh offer");
    assert!(!legacy_create.resume);
    let status_request = OfferStatusRequest {
        transfer_id: offer.transfer_id,
    };
    let status_response = OfferStatusResponse {
        transfer_id: offer.transfer_id,
        status: TransferStatus::Offered,
        authorization: None,
    };
    assert!(status_response.validate().is_ok());
    assert!(
        OfferStatusResponse {
            transfer_id: offer.transfer_id,
            status: TransferStatus::Accepted,
            authorization: None,
        }
        .validate()
        .is_err()
    );

    assert_eq!(
        MessageType::try_from(1).expect("INFO_REQUEST"),
        MessageType::InfoRequest
    );
    assert_eq!(MessageType::OfferCreate as u8, 3);
    assert!(MessageType::try_from(255).is_err());
    for value in [
        serde_json::to_value(info_request).expect("info request"),
        serde_json::to_value(info_response).expect("info response"),
        serde_json::to_value(create).expect("offer create"),
        serde_json::to_value(status_request).expect("status request"),
        serde_json::to_value(status_response).expect("status response"),
    ] {
        assert!(value.is_object());
    }
    let unknown = serde_json::json!({
        "protocolVersion": {"major": 1, "minor": 0},
        "capabilities": ["files"],
        "bypass": true
    });
    assert!(serde_json::from_value::<InfoRequest>(unknown).is_err());
}

#[test]
fn qsp_1_1_selection_messages_are_strict_correlated_and_stable() {
    // Arrange
    let offer = valid_offer();
    let request_id = RequestId::new(Uuid::now_v7());
    let request = SourceSelectionRequest {
        requester: offer.sender.clone(),
        callback_port: 4242,
    };
    let ready = SourceSelectionResponse {
        status: SourceSelectionStatus::Ready,
        transfer_id: Some(offer.transfer_id),
    };

    // Act + Assert
    assert_eq!(ProtocolVersion::V1_1, ProtocolVersion::new(1, 1));
    assert_eq!(request.validate(), Ok(()));
    assert_eq!(ready.validate(), Ok(()));
    assert_eq!(MessageType::SourceSelectionRequest as u8, 11);
    assert_eq!(MessageType::SourceSelectionResponse as u8, 12);
    assert_eq!(
        MessageType::try_from(11).expect("selection request"),
        MessageType::SourceSelectionRequest
    );
    let request_value = serde_json::to_value(&request).expect("request JSON");
    assert_eq!(request_value["callbackPort"], 4242);
    let ready_value = serde_json::to_value(&ready).expect("response JSON");
    assert_eq!(ready_value["status"], "ready");
    assert_eq!(
        serde_json::from_value::<SourceSelectionResponse>(ready_value).expect("response"),
        ready
    );

    let mut correlated = offer.clone();
    correlated.initiated_by = Some(request_id);
    let correlated_value = serde_json::to_value(&correlated).expect("correlated offer JSON");
    assert!(correlated_value.get("initiatedBy").is_some());
    let ordinary_value = serde_json::to_value(&offer).expect("ordinary offer JSON");
    assert!(ordinary_value.get("initiatedBy").is_none());

    assert_eq!(
        SourceSelectionRequest {
            callback_port: 0,
            ..request.clone()
        }
        .validate(),
        Err(ValidationError::InvalidCallbackPort)
    );
    assert_eq!(
        SourceSelectionResponse {
            status: SourceSelectionStatus::Ready,
            transfer_id: None,
        }
        .validate(),
        Err(ValidationError::InvalidSelectionState)
    );
    for status in [
        SourceSelectionStatus::Cancelled,
        SourceSelectionStatus::Rejected,
        SourceSelectionStatus::Busy,
        SourceSelectionStatus::UiUnavailable,
        SourceSelectionStatus::Expired,
        SourceSelectionStatus::Failed,
    ] {
        assert_eq!(
            SourceSelectionResponse {
                status,
                transfer_id: Some(offer.transfer_id),
            }
            .validate(),
            Err(ValidationError::InvalidSelectionState)
        );
    }
}

#[test]
fn info_response_hides_remote_selection_from_qsp_1_0_peers() {
    // Arrange
    let offer = valid_offer();
    let mut local = offer.sender;
    local.capabilities.insert(Capability::RemoteSelection);
    let old_request = InfoRequest {
        protocol_version: ProtocolVersion::V1_0,
        capabilities: BTreeSet::from([Capability::Files]),
    };
    let new_request = InfoRequest {
        protocol_version: ProtocolVersion::V1_1,
        capabilities: BTreeSet::from([Capability::Files]),
    };

    // Act
    let old = compatible_info_response(ProtocolVersion::V1_1, local.clone(), &old_request)
        .expect("QSP/1.0 response");
    let new = compatible_info_response(ProtocolVersion::V1_1, local, &new_request)
        .expect("QSP/1.1 response");

    // Assert
    assert_eq!(old.protocol_version, ProtocolVersion::V1_0);
    assert!(
        !old.device
            .capabilities
            .contains(&Capability::RemoteSelection)
    );
    assert_eq!(new.protocol_version, ProtocolVersion::V1_1);
    assert!(
        new.device
            .capabilities
            .contains(&Capability::RemoteSelection)
    );
}

#[test]
fn decisions_statuses_and_chunks_have_stable_bounded_wire_forms() {
    // Arrange
    let decision = OfferDecision::Reject {
        reason: RejectionReason::ConfirmationTimeout,
    };
    let chunk = ChunkDescriptor {
        transfer_id: TransferId::new(Uuid::nil()),
        entry_id: EntryId::new(1).expect("entry ID"),
        index: 1,
        offset: 4 * 1024 * 1024,
        length: 1,
        digest: [7; 32],
    };

    // Act
    let decision_value = serde_json::to_value(decision).expect("decision JSON");
    let status_value = serde_json::to_value(TransferStatus::Verifying).expect("status JSON");

    // Assert
    assert_eq!(decision_value["decision"], "reject");
    assert_eq!(decision_value["reason"], "confirmation_timeout");
    assert_eq!(status_value, "verifying");
    chunk
        .validate(4 * 1024 * 1024, 4 * 1024 * 1024 + 1)
        .expect("last one-byte block");
    assert_eq!(
        ChunkDescriptor { offset: 0, ..chunk }.validate(4 * 1024 * 1024, 8 * 1024 * 1024),
        Err(ValidationError::InvalidChunkRange)
    );
}

#[test]
fn bounded_decoder_rejects_oversized_input_before_json_decoding() {
    let bytes = vec![b' '; quick_share_protocol::MAX_OFFER_BYTES + 1];
    assert!(matches!(decode_offer(&bytes), Err(DecodeError::TooLarge)));
}

#[test]
fn error_codes_have_stable_machine_readable_serialization() {
    // Arrange
    let error = ProtocolError::new(ErrorCode::IntegrityFailed, "digest mismatch");

    // Act
    let value = serde_json::to_value(error).expect("serialize protocol error");

    // Assert
    assert_eq!(value["code"], "integrity_failed");
    assert_eq!(value["message"], "digest mismatch");
}

proptest! {
    #[test]
    fn arbitrary_json_never_panics_while_decoding(input in ".{0,4096}") {
        // Arrange + Act
        let result = std::panic::catch_unwind(|| serde_json::from_str::<TransferOffer>(&input));

        // Assert
        prop_assert!(result.is_ok());
    }
}
