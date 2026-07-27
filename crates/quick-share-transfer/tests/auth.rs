use quick_share_core::identity::{DeviceIdentity, IdentityStore, TrustedDeviceStore};
use quick_share_transfer::{
    auth::{IdentityChange, PeerAuthContext, PeerClaim, PreAuthorizationPermission, classify_peer},
    noise::{HandshakeEvidence, NoiseHandshake},
};
use std::time::Duration;
use tempfile::tempdir;

fn evidence(local: &DeviceIdentity, remote: &DeviceIdentity) -> HandshakeEvidence {
    let mut initiator =
        NoiseHandshake::initiator(local, Duration::from_secs(5)).expect("initiator");
    let mut responder =
        NoiseHandshake::responder(remote, Duration::from_secs(5)).expect("responder");
    let one = initiator.write_message().expect("one");
    responder.read_message(&one).expect("read one");
    let two = responder.write_message().expect("two");
    initiator.read_message(&two).expect("read two");
    let three = initiator.write_message().expect("three");
    responder.read_message(&three).expect("read three");
    initiator.finish(None).expect("finish").1
}

#[test]
fn unknown_trusted_changed_and_name_collision_use_full_static_keys() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let peer = IdentityStore::new(root.path().join("peer.json"))
        .load_or_create()
        .expect("peer");
    let replacement = IdentityStore::new(root.path().join("replacement.json"))
        .load_or_create()
        .expect("replacement");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer_evidence = evidence(&local, &peer);
    let claim = PeerClaim {
        device_id: peer.device_id(),
        name: "workstation".to_owned(),
    };

    let unknown = classify_peer(claim.clone(), &peer_evidence, &trust).expect("unknown");
    assert!(matches!(unknown, PeerAuthContext::Unknown { .. }));
    assert!(!unknown.is_trusted());
    assert!(unknown.allows_pre_authorization(PreAuthorizationPermission::Info));
    assert!(unknown.allows_pre_authorization(PreAuthorizationPermission::OfferCreate));

    trust
        .trust_peer(peer.device_id(), "workstation", peer.public_key())
        .expect("pin peer");
    let trusted = classify_peer(claim.clone(), &peer_evidence, &trust).expect("trusted");
    assert!(trusted.is_trusted());
    let renamed_claim = classify_peer(
        PeerClaim {
            device_id: peer.device_id(),
            name: "network supplied alias".to_owned(),
        },
        &peer_evidence,
        &trust,
    )
    .expect("trusted key with changed wire name");
    assert!(renamed_claim.is_trusted());
    assert_eq!(renamed_claim.name(), "workstation");
    assert_eq!(renamed_claim.claimed_name(), "network supplied alias");

    let replacement_evidence = evidence(&local, &replacement);
    let key_changed = classify_peer(claim, &replacement_evidence, &trust).expect("changed key");
    assert_eq!(
        key_changed.change_reason(),
        Some(IdentityChange::PinnedKeyChanged)
    );

    let name_collision = classify_peer(
        PeerClaim {
            device_id: replacement.device_id(),
            name: "workstation".to_owned(),
        },
        &replacement_evidence,
        &trust,
    )
    .expect("name collision");
    assert_eq!(
        name_collision.change_reason(),
        Some(IdentityChange::TrustedNameUsesDifferentKey)
    );
    assert!(!name_collision.is_trusted());
}

#[test]
fn claim_id_must_match_authenticated_static_key_and_debug_redacts_key() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let peer = IdentityStore::new(root.path().join("peer.json"))
        .load_or_create()
        .expect("peer");
    let unrelated = IdentityStore::new(root.path().join("unrelated.json"))
        .load_or_create()
        .expect("unrelated");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let evidence = evidence(&local, &peer);

    let context = classify_peer(
        PeerClaim {
            device_id: unrelated.device_id(),
            name: "peer".to_owned(),
        },
        &evidence,
        &trust,
    )
    .expect("claim mismatch");
    let debug = format!("{context:?}");

    assert_eq!(
        context.change_reason(),
        Some(IdentityChange::ClaimedIdDoesNotMatchKey)
    );
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(&hex::encode(peer.public_key())));
}
