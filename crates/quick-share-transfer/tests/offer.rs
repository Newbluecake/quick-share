use quick_share_core::{
    config::TrustedPolicy,
    identity::{DeviceIdentity, IdentityStore, TrustStatus, TrustedDeviceStore},
};
use quick_share_protocol::{
    Capability, ContentKind, DeviceInfo, EntryId, ManifestEntry, ManifestEntryKind, OfferDecision,
    ProtocolVersion, RejectionReason, TransferId, TransferOffer, TransferStatus,
};
use quick_share_transfer::{
    auth::{PeerAuthContext, PeerClaim, classify_peer},
    noise::{HandshakeEvidence, NoiseHandshake},
    offer::{AuthorizationPermission, AuthorizationToken, OfferError, OfferManager, OfferPolicy},
};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::tempdir;
use uuid::Uuid;

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

fn context(
    local: &DeviceIdentity,
    remote: &DeviceIdentity,
    name: &str,
    trust: &TrustedDeviceStore,
) -> PeerAuthContext {
    classify_peer(
        PeerClaim {
            device_id: remote.device_id(),
            name: name.to_owned(),
        },
        &evidence(local, remote),
        trust,
    )
    .expect("peer context")
}

fn offer(peer: &PeerAuthContext, transfer_id: TransferId, size: u64) -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id,
        sender: DeviceInfo {
            device_id: peer.device_id().clone(),
            name: peer.claimed_name().to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::Resume]),
        },
        content_kind: ContentKind::Files,
        chunk_size: 4 * 1024 * 1024,
        total_bytes: size,
        entries: vec![ManifestEntry {
            id: EntryId::new(1).expect("entry ID"),
            relative_path: "docs/readme.txt".to_owned(),
            kind: ManifestEntryKind::File,
            size,
            digest: Some([7; 32]),
        }],
    }
}

fn id() -> TransferId {
    TransferId::new(Uuid::now_v7())
}

#[tokio::test]
async fn accept_once_grant_binds_token_peer_transfer_permission_expiry_and_manifest() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let other = IdentityStore::new(root.path().join("other.json"))
        .load_or_create()
        .expect("other");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer = context(&local, &remote, "sender", &trust);
    let manager = OfferManager::new(trust, OfferPolicy::default());
    let transfer_id = id();
    let mut original = offer(&peer, transfer_id, 12);
    let authorized_snapshot = original.clone();
    let now = Instant::now();
    let submission = manager
        .create_offer(&peer, None, original.clone(), now)
        .expect("offer");
    original.entries[0].relative_path = "mutated-after-submit.txt".to_owned();

    assert_eq!(submission.view.entry_count, 1);
    assert_eq!(submission.view.total_bytes, 12);
    assert_eq!(submission.view.entries[0].relative_path, "docs/readme.txt");
    assert_eq!(
        manager
            .status(peer.device_id(), transfer_id, now)
            .expect("status"),
        TransferStatus::Offered
    );
    assert!(matches!(
        manager.authorize(
            &AuthorizationToken::from_bytes([0; 32]),
            peer.device_id(),
            transfer_id,
            AuthorizationPermission::ChunkUpload,
            now,
        ),
        Err(OfferError::Unauthorized)
    ));
    manager
        .decide(transfer_id, OfferDecision::AcceptOnce, false, now)
        .expect("accept once");
    let resolution = submission.resolution.await.expect("resolution");
    assert_eq!(resolution.status, TransferStatus::Accepted);
    let token = resolution.authorization.expect("authorization token");
    let authorized = manager
        .authorize(
            &token,
            peer.device_id(),
            transfer_id,
            AuthorizationPermission::ChunkUpload,
            now,
        )
        .expect("authorized upload");

    assert_eq!(authorized.offer, authorized_snapshot);
    assert_ne!(authorized.offer, original);
    assert!(
        manager
            .authorize(
                &token,
                &other.device_id(),
                transfer_id,
                AuthorizationPermission::ChunkUpload,
                now,
            )
            .is_err()
    );
    assert!(
        manager
            .authorize(
                &token,
                peer.device_id(),
                transfer_id,
                AuthorizationPermission::ChunkUpload,
                now + Duration::from_secs(60 * 60),
            )
            .is_err()
    );
    let token_hex = token.with_bytes(|bytes| hex::encode(bytes));
    let debug = format!("{token:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(&token_hex));
}

#[tokio::test]
async fn explicit_resume_reauthorizes_only_the_identical_accepted_offer() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer = context(&local, &remote, "sender", &trust);
    let manager = OfferManager::new(trust, OfferPolicy::default());
    let transfer_id = id();
    let original = offer(&peer, transfer_id, 12);
    let now = Instant::now();
    let submission = manager
        .create_offer(&peer, None, original.clone(), now)
        .expect("offer");
    manager
        .decide(transfer_id, OfferDecision::AcceptOnce, false, now)
        .expect("accept");
    let original_token = submission
        .resolution
        .await
        .expect("resolution")
        .authorization
        .expect("token");

    let resumed = manager
        .resume_offer(&peer, None, original.clone(), now + Duration::from_secs(1))
        .expect("resume");
    let resumed_token = resumed
        .resolution
        .await
        .expect("resume resolution")
        .authorization
        .expect("resume token");
    assert_ne!(
        original_token.with_bytes(|bytes| *bytes),
        resumed_token.with_bytes(|bytes| *bytes)
    );
    assert!(
        manager
            .authorize(
                &resumed_token,
                peer.device_id(),
                transfer_id,
                AuthorizationPermission::ChunkUpload,
                now + Duration::from_secs(1),
            )
            .is_ok()
    );
    assert!(
        manager
            .authorize(
                &original_token,
                peer.device_id(),
                transfer_id,
                AuthorizationPermission::ChunkUpload,
                now + Duration::from_secs(1),
            )
            .is_err()
    );

    let mut changed = original;
    changed.entries[0].digest = Some([9; 32]);
    assert!(matches!(
        manager.resume_offer(&peer, None, changed, now + Duration::from_secs(2)),
        Err(OfferError::ResumeMismatch)
    ));
}

#[tokio::test]
async fn accept_and_trust_requires_sas_and_atomically_pins_authenticated_key() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer = context(&local, &remote, "new device", &trust);
    let manager = OfferManager::new(trust.clone(), OfferPolicy::default());
    let transfer_id = id();
    let now = Instant::now();
    let submission = manager
        .create_offer(&peer, None, offer(&peer, transfer_id, 1), now)
        .expect("offer");

    assert!(matches!(
        manager.decide(transfer_id, OfferDecision::AcceptAndTrust, false, now),
        Err(OfferError::SasVerificationRequired)
    ));
    manager
        .decide(transfer_id, OfferDecision::AcceptAndTrust, true, now)
        .expect("verified trust");
    let resolution = submission.resolution.await.expect("resolution");

    assert_eq!(resolution.status, TransferStatus::Accepted);
    assert!(matches!(
        trust
            .check(&remote.device_id(), &remote.public_key())
            .expect("pin"),
        TrustStatus::Trusted(_)
    ));
}

#[tokio::test]
async fn reject_and_timeout_wake_channels_without_authorization() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer = context(&local, &remote, "sender", &trust);
    let policy = OfferPolicy {
        confirmation_timeout: Duration::from_millis(10),
        ..OfferPolicy::default()
    };
    let manager = OfferManager::new(trust, policy);
    let now = Instant::now();

    let rejected_id = id();
    let rejected = manager
        .create_offer(&peer, None, offer(&peer, rejected_id, 1), now)
        .expect("rejected offer");
    manager
        .decide(
            rejected_id,
            OfferDecision::Reject {
                reason: RejectionReason::UserRejected,
            },
            false,
            now,
        )
        .expect("reject");
    let rejected_resolution = rejected.resolution.await.expect("rejected resolution");
    assert_eq!(rejected_resolution.status, TransferStatus::Rejected);
    assert!(rejected_resolution.authorization.is_none());

    let expired_id = id();
    let expired = manager
        .create_offer(&peer, None, offer(&peer, expired_id, 1), now)
        .expect("expiring offer");
    assert_eq!(
        manager
            .status(
                peer.device_id(),
                expired_id,
                now + Duration::from_millis(10),
            )
            .expect("status triggers timeout"),
        TransferStatus::Expired
    );
    assert_eq!(
        manager
            .expire(now + Duration::from_millis(10))
            .expect("already expired"),
        0
    );
    let expired_resolution = expired.resolution.await.expect("expired resolution");
    assert_eq!(expired_resolution.status, TransferStatus::Expired);
    assert!(expired_resolution.authorization.is_none());
}

#[tokio::test]
async fn trusted_auto_accept_respects_confirm_policy_and_resource_thresholds() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    trust
        .trust_peer(remote.device_id(), "trusted", remote.public_key())
        .expect("trust");
    let peer = context(&local, &remote, "trusted", &trust);
    assert!(peer.is_trusted());
    let now = Instant::now();

    let auto = OfferManager::new(trust.clone(), OfferPolicy::default());
    let auto_submission = auto
        .create_offer(&peer, None, offer(&peer, id(), 1), now)
        .expect("auto");
    assert_eq!(
        auto_submission
            .resolution
            .await
            .expect("auto result")
            .status,
        TransferStatus::Accepted
    );

    trust.remove(&remote.device_id()).expect("revoke trust");
    let revoked_manager = OfferManager::new(trust.clone(), OfferPolicy::default());
    let revoked_id = id();
    let revoked = revoked_manager
        .create_offer(&peer, None, offer(&peer, revoked_id, 1), now)
        .expect("stale trusted context becomes pending");
    assert!(!revoked.view.trusted);
    assert_eq!(
        revoked_manager
            .status(peer.device_id(), revoked_id, now)
            .expect("revoked pending"),
        TransferStatus::Offered
    );
    manager_reject(&revoked_manager, revoked_id, now);
    assert_eq!(
        revoked.resolution.await.expect("revoked result").status,
        TransferStatus::Rejected
    );
    trust
        .trust_peer(remote.device_id(), "trusted", remote.public_key())
        .expect("restore trust");

    let confirm_policy = OfferPolicy {
        trusted_policy: TrustedPolicy::Confirm,
        ..OfferPolicy::default()
    };
    let confirm = OfferManager::new(trust.clone(), confirm_policy);
    let confirm_id = id();
    let confirm_submission = confirm
        .create_offer(&peer, None, offer(&peer, confirm_id, 1), now)
        .expect("confirm");
    assert_eq!(
        confirm
            .status(peer.device_id(), confirm_id, now)
            .expect("pending"),
        TransferStatus::Offered
    );
    manager_reject(&confirm, confirm_id, now);
    assert_eq!(
        confirm_submission
            .resolution
            .await
            .expect("confirm result")
            .status,
        TransferStatus::Rejected
    );

    let limited_policy = OfferPolicy {
        trusted_auto_max_bytes: 0,
        ..OfferPolicy::default()
    };
    let limited = OfferManager::new(trust, limited_policy);
    let limited_id = id();
    let limited_submission = limited
        .create_offer(&peer, None, offer(&peer, limited_id, 1), now)
        .expect("limited");
    assert_eq!(
        limited
            .status(peer.device_id(), limited_id, now)
            .expect("limited pending"),
        TransferStatus::Offered
    );
    manager_reject(&limited, limited_id, now);
    assert_eq!(
        limited_submission
            .resolution
            .await
            .expect("limited result")
            .status,
        TransferStatus::Rejected
    );
}

fn manager_reject(manager: &OfferManager, transfer_id: TransferId, now: Instant) {
    manager
        .decide(
            transfer_id,
            OfferDecision::Reject {
                reason: RejectionReason::Policy,
            },
            false,
            now,
        )
        .expect("reject pending offer");
}

#[test]
fn replay_owner_isolation_rate_limit_and_queue_limit_fail_closed() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let other = IdentityStore::new(root.path().join("other.json"))
        .load_or_create()
        .expect("other");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer = context(&local, &remote, "sender", &trust);
    let now = Instant::now();
    let policy = OfferPolicy {
        max_pending: 1,
        max_offers_per_window: 1,
        ..OfferPolicy::default()
    };
    let manager = OfferManager::new(trust, policy);
    let transfer_id = id();
    let first = offer(&peer, transfer_id, 1);

    manager
        .create_offer(&peer, None, first.clone(), now)
        .expect("first");
    assert!(matches!(
        manager.create_offer(&peer, None, first, now),
        Err(OfferError::Replay)
    ));
    assert!(matches!(
        manager.create_offer(&peer, None, offer(&peer, id(), 1), now),
        Err(OfferError::RateLimited)
    ));
    assert!(matches!(
        manager.status(&other.device_id(), transfer_id, now),
        Err(OfferError::Unauthorized)
    ));
}

#[test]
fn concurrent_offer_creation_obeys_global_queue_bound() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let remote = IdentityStore::new(root.path().join("remote.json"))
        .load_or_create()
        .expect("remote");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    let peer = context(&local, &remote, "sender", &trust);
    let policy = OfferPolicy {
        max_pending: 2,
        max_offers_per_window: 100,
        ..OfferPolicy::default()
    };
    let manager = Arc::new(OfferManager::new(trust, policy));
    let now = Instant::now();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let manager = manager.clone();
            let peer = peer.clone();
            std::thread::spawn(move || {
                manager
                    .create_offer(&peer, None, offer(&peer, id(), 1), now)
                    .is_ok()
            })
        })
        .collect();
    let accepted = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .filter(|accepted| *accepted)
        .count();
    assert_eq!(accepted, 2);
}

#[test]
fn malformed_or_changed_identity_offers_never_auto_accept() {
    let root = tempdir().expect("root");
    let local = IdentityStore::new(root.path().join("local.json"))
        .load_or_create()
        .expect("local");
    let pinned = IdentityStore::new(root.path().join("pinned.json"))
        .load_or_create()
        .expect("pinned");
    let changed = IdentityStore::new(root.path().join("changed.json"))
        .load_or_create()
        .expect("changed");
    let trust = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    trust
        .trust_peer(pinned.device_id(), "same-name", pinned.public_key())
        .expect("pin");
    let changed_evidence = evidence(&local, &changed);
    let changed_context = classify_peer(
        PeerClaim {
            device_id: pinned.device_id(),
            name: "same-name".to_owned(),
        },
        &changed_evidence,
        &trust,
    )
    .expect("changed context");
    let manager = OfferManager::new(trust, OfferPolicy::default());
    let now = Instant::now();
    let changed_id = id();
    let submission = manager
        .create_offer(
            &changed_context,
            None,
            offer(&changed_context, changed_id, 1),
            now,
        )
        .expect("changed offer");
    assert!(submission.view.identity_changed);
    assert_eq!(
        manager
            .status(changed_context.device_id(), changed_id, now)
            .expect("pending"),
        TransferStatus::Offered
    );

    let mut malformed = offer(&changed_context, id(), 1);
    malformed.entries[0].relative_path = "../escape".to_owned();
    assert!(matches!(
        manager.create_offer(&changed_context, None, malformed, now),
        Err(OfferError::InvalidOffer(_))
    ));
}
