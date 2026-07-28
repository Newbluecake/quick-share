use quick_share_core::identity::IdentityStore;
use quick_share_protocol::{
    Capability, ContentKind, DeviceId, DeviceInfo, EntryId, ManifestEntry, ManifestEntryKind,
    ProtocolVersion, RequestId, TransferId, TransferOffer,
};
use quick_share_transfer::expected_offer::{
    ExpectedCallback, ExpectedOfferError, ExpectedOfferPolicy, ExpectedOfferRegistry,
};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr},
    time::{Duration, Instant},
};
use tempfile::tempdir;
use uuid::Uuid;

fn offer(sender: DeviceId, request_id: RequestId, transfer_id: TransferId) -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_1,
        transfer_id,
        initiated_by: Some(request_id),
        sender: DeviceInfo {
            device_id: sender,
            name: "windows".to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::RemoteSelection]),
        },
        content_kind: ContentKind::Files,
        chunk_size: 256 * 1024,
        total_bytes: 0,
        entries: vec![ManifestEntry {
            id: EntryId::new(1).expect("entry"),
            relative_path: "empty.txt".to_owned(),
            kind: ManifestEntryKind::File,
            size: 0,
            digest: Some(*blake3::hash(b"").as_bytes()),
        }],
    }
}

#[test]
fn expected_callback_matches_complete_identity_ip_request_and_transfer_once() {
    let root = tempdir().expect("root");
    let sender = IdentityStore::new(root.path().join("sender.json"))
        .load_or_create()
        .expect("sender");
    let registry = ExpectedOfferRegistry::new(ExpectedOfferPolicy::default()).expect("registry");
    let request_id = RequestId::new(Uuid::now_v7());
    let transfer_id = TransferId::new(Uuid::now_v7());
    let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 8));
    let now = Instant::now();
    let expected = ExpectedCallback {
        request_id,
        transfer_id,
        sender_device_id: sender.device_id(),
        sender_public_key: sender.public_key(),
        source_ip: ip,
        expires_at: now + Duration::from_secs(30),
    };
    registry.register(expected.clone(), now).expect("register");
    registry
        .register(expected, now)
        .expect("idempotent register");
    let callback = offer(sender.device_id(), request_id, transfer_id);

    let grant = registry
        .consume(
            &sender.device_id(),
            &sender.public_key(),
            ip,
            &callback,
            now,
        )
        .expect("consume");
    assert_eq!(grant.request_id, request_id);
    assert_eq!(grant.transfer_id, transfer_id);
    assert!(matches!(
        registry.consume(
            &sender.device_id(),
            &sender.public_key(),
            ip,
            &callback,
            now,
        ),
        Err(ExpectedOfferError::Replay)
    ));
    assert!(!format!("{registry:?}").contains(&hex::encode(sender.public_key())));
}

#[test]
fn expected_callback_rejects_every_correlation_or_identity_mismatch_and_expiry() {
    let root = tempdir().expect("root");
    let sender = IdentityStore::new(root.path().join("sender.json"))
        .load_or_create()
        .expect("sender");
    let other = IdentityStore::new(root.path().join("other.json"))
        .load_or_create()
        .expect("other");
    let now = Instant::now();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);

    for mismatch in 0..5 {
        let registry =
            ExpectedOfferRegistry::new(ExpectedOfferPolicy::default()).expect("registry");
        let request_id = RequestId::new(Uuid::now_v7());
        let transfer_id = TransferId::new(Uuid::now_v7());
        registry
            .register(
                ExpectedCallback {
                    request_id,
                    transfer_id,
                    sender_device_id: sender.device_id(),
                    sender_public_key: sender.public_key(),
                    source_ip: ip,
                    expires_at: now + Duration::from_secs(30),
                },
                now,
            )
            .expect("register");
        let mut callback = offer(sender.device_id(), request_id, transfer_id);
        let (device_id, key, source_ip) = match mismatch {
            0 => (other.device_id(), sender.public_key(), ip),
            1 => (sender.device_id(), other.public_key(), ip),
            2 => (
                sender.device_id(),
                sender.public_key(),
                IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            ),
            3 => {
                callback.initiated_by = Some(RequestId::new(Uuid::now_v7()));
                (sender.device_id(), sender.public_key(), ip)
            }
            _ => {
                callback.transfer_id = TransferId::new(Uuid::now_v7());
                (sender.device_id(), sender.public_key(), ip)
            }
        };
        assert!(matches!(
            registry.consume(&device_id, &key, source_ip, &callback, now),
            Err(ExpectedOfferError::Mismatch | ExpectedOfferError::NotExpected)
        ));
    }

    let registry = ExpectedOfferRegistry::new(ExpectedOfferPolicy::default()).expect("registry");
    let request_id = RequestId::new(Uuid::now_v7());
    let transfer_id = TransferId::new(Uuid::now_v7());
    registry
        .register(
            ExpectedCallback {
                request_id,
                transfer_id,
                sender_device_id: sender.device_id(),
                sender_public_key: sender.public_key(),
                source_ip: ip,
                expires_at: now + Duration::from_millis(1),
            },
            now,
        )
        .expect("register");
    assert!(matches!(
        registry.consume(
            &sender.device_id(),
            &sender.public_key(),
            ip,
            &offer(sender.device_id(), request_id, transfer_id),
            now + Duration::from_millis(1),
        ),
        Err(ExpectedOfferError::Expired)
    ));
}
