use quick_share_core::{
    destination::{ConflictDecision, ConflictSelections, DestinationPlan},
    identity::{IdentityStore, TrustedDeviceStore},
    receive::{ReceiveBinding, ReceiveBindingStore},
};
use quick_share_protocol::{
    AuthorizationProof, Capability, ContentKind, DeviceInfo, EntryId, ManifestEntry,
    ManifestEntryKind, OfferDecision, ProtocolVersion, TransferComplete, TransferId, TransferOffer,
    TransferStatus, TransferStatusRequest,
};
use quick_share_transfer::{
    auth::{PeerAuthContext, PeerClaim, classify_peer},
    noise::NoiseHandshake,
    offer::{AuthorizationToken, OfferManager, OfferPolicy},
    receiver::ReceiverPolicy,
    receiver_router::{ReceiverRouter, ReceiverRouterError},
};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::tempdir;
use uuid::Uuid;

fn peer_context(root: &Path) -> (PeerAuthContext, TrustedDeviceStore) {
    let local = IdentityStore::new(root.join("local.json"))
        .load_or_create()
        .expect("local identity");
    let remote = IdentityStore::new(root.join("remote.json"))
        .load_or_create()
        .expect("remote identity");
    let mut initiator = NoiseHandshake::initiator(&local, Duration::from_secs(5)).expect("init");
    let mut responder = NoiseHandshake::responder(&remote, Duration::from_secs(5)).expect("resp");
    let one = initiator.write_message().expect("one");
    responder.read_message(&one).expect("read one");
    let two = responder.write_message().expect("two");
    initiator.read_message(&two).expect("read two");
    let three = initiator.write_message().expect("three");
    responder.read_message(&three).expect("read three");
    let (_, evidence) = initiator.finish(None).expect("finish");
    let trust = TrustedDeviceStore::new(root.join("trusted.json"));
    let peer = classify_peer(
        PeerClaim {
            device_id: remote.device_id(),
            name: "sender".to_owned(),
        },
        &evidence,
        &trust,
    )
    .expect("peer");
    (peer, trust)
}

fn offer(peer: &PeerAuthContext, name: &str) -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_1,
        transfer_id: TransferId::new(Uuid::now_v7()),
        initiated_by: None,
        sender: DeviceInfo {
            device_id: peer.device_id().clone(),
            name: peer.claimed_name().to_owned(),
            capabilities: BTreeSet::from([Capability::Files, Capability::Resume]),
        },
        content_kind: ContentKind::Files,
        chunk_size: 256 * 1024,
        total_bytes: 0,
        entries: vec![ManifestEntry {
            id: EntryId::new(1).expect("entry"),
            relative_path: name.to_owned(),
            kind: ManifestEntryKind::File,
            size: 0,
            digest: Some(*blake3::hash(b"").as_bytes()),
        }],
    }
}

fn directory_offer(peer: &PeerAuthContext, child: &str) -> TransferOffer {
    let mut offer = offer(peer, &format!("docs/{child}"));
    offer.sender.capabilities.insert(Capability::Directories);
    offer.entries.insert(
        0,
        ManifestEntry {
            id: EntryId::new(2).expect("directory"),
            relative_path: "docs".to_owned(),
            kind: ManifestEntryKind::Directory,
            size: 0,
            digest: None,
        },
    );
    offer
}

fn digest(offer: &TransferOffer) -> [u8; 32] {
    *blake3::hash(&serde_json::to_vec(offer).expect("offer JSON")).as_bytes()
}

fn binding(root: &Path, offer: &TransferOffer, plan: DestinationPlan) -> ReceiveBinding {
    ReceiveBinding {
        transfer_id: offer.transfer_id,
        sender_device_id: offer.sender.device_id.clone(),
        manifest_digest: digest(offer),
        output_root: root.to_path_buf(),
        destination_plan: plan,
    }
}

async fn submit_and_accept(
    router: &ReceiverRouter,
    manager: &OfferManager,
    peer: &PeerAuthContext,
    root: &Path,
    offer: TransferOffer,
    selections: ConflictSelections,
) -> AuthorizationToken {
    let now = Instant::now();
    let submission = manager
        .create_offer(peer, None, offer.clone(), now)
        .expect("offer submission");
    let plan = DestinationPlan::build(root, &offer, &selections).expect("destination plan");
    router
        .bind(binding(root, &offer, plan), &offer)
        .expect("binding before authorization");
    manager
        .decide(offer.transfer_id, OfferDecision::AcceptOnce, false, now)
        .expect("accept");
    submission
        .resolution
        .await
        .expect("resolution")
        .authorization
        .expect("token")
}

fn proof(token: &AuthorizationToken) -> AuthorizationProof {
    token.with_bytes(|bytes| AuthorizationProof::from_bytes(*bytes))
}

#[tokio::test]
async fn router_commits_two_transfers_to_distinct_bound_roots() {
    let temp = tempdir().expect("root");
    let identity_root = temp.path().join("identity");
    fs::create_dir(&identity_root).expect("identity root");
    let (peer, trust) = peer_context(&identity_root);
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let binding_path = temp.path().join("bindings.json");
    let router = ReceiverRouter::new(
        Arc::clone(&manager),
        ReceiveBindingStore::new(&binding_path),
        ReceiverPolicy {
            max_receive_tasks: 2,
            ..ReceiverPolicy::default()
        },
        None,
    )
    .expect("router");
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    fs::create_dir(&first_root).expect("first root");
    fs::create_dir(&second_root).expect("second root");
    let first_offer = offer(&peer, "first.txt");
    let second_offer = offer(&peer, "second.txt");
    let first_token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &first_root,
        first_offer.clone(),
        ConflictSelections::default(),
    )
    .await;
    let second_token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &second_root,
        second_offer.clone(),
        ConflictSelections::default(),
    )
    .await;

    for (offer, token) in [(&first_offer, &first_token), (&second_offer, &second_token)] {
        let status = router
            .status(
                peer.device_id(),
                TransferStatusRequest {
                    transfer_id: offer.transfer_id,
                    authorization: proof(token),
                },
                Instant::now(),
            )
            .expect("status");
        assert_eq!(status.status, TransferStatus::Accepted);
        router
            .complete(
                peer.device_id(),
                TransferComplete {
                    transfer_id: offer.transfer_id,
                    authorization: proof(token),
                    manifest_digest: digest(offer),
                },
                Instant::now(),
            )
            .expect("complete");
    }

    assert_eq!(fs::read(first_root.join("first.txt")).expect("first"), b"");
    assert_eq!(
        fs::read(second_root.join("second.txt")).expect("second"),
        b""
    );
    assert!(!first_root.join("second.txt").exists());
    assert!(!second_root.join("first.txt").exists());
}

#[tokio::test]
async fn planned_directory_rename_merge_and_skip_apply_to_descendants() {
    let temp = tempdir().expect("root");
    let identity_root = temp.path().join("identity");
    fs::create_dir(&identity_root).expect("identity root");
    let (peer, trust) = peer_context(&identity_root);
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let root = temp.path().join("output");
    fs::create_dir(&root).expect("output");
    fs::create_dir(root.join("docs")).expect("existing directory");
    let router = ReceiverRouter::new(
        Arc::clone(&manager),
        ReceiveBindingStore::new(temp.path().join("bindings.json")),
        ReceiverPolicy::default(),
        None,
    )
    .expect("router");
    let offer = directory_offer(&peer, "empty.txt");
    let token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &root,
        offer.clone(),
        ConflictSelections::default().with_entry(
            EntryId::new(2).expect("directory"),
            ConflictDecision::Rename,
        ),
    )
    .await;

    router
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: offer.transfer_id,
                authorization: proof(&token),
            },
            Instant::now(),
        )
        .expect("status");
    router
        .complete(
            peer.device_id(),
            TransferComplete {
                transfer_id: offer.transfer_id,
                authorization: proof(&token),
                manifest_digest: digest(&offer),
            },
            Instant::now(),
        )
        .expect("complete");
    assert!(root.join("docs (1)").is_dir());
    assert_eq!(
        fs::read(root.join("docs (1)/empty.txt")).expect("file"),
        b""
    );
    assert!(!root.join("docs/empty.txt").exists());

    let merge_offer = directory_offer(&peer, "merged.txt");
    let merge_token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &root,
        merge_offer.clone(),
        ConflictSelections::default().with_entry(
            EntryId::new(2).expect("directory"),
            ConflictDecision::Overwrite,
        ),
    )
    .await;
    router
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: merge_offer.transfer_id,
                authorization: proof(&merge_token),
            },
            Instant::now(),
        )
        .expect("merge status");
    router
        .complete(
            peer.device_id(),
            TransferComplete {
                transfer_id: merge_offer.transfer_id,
                authorization: proof(&merge_token),
                manifest_digest: digest(&merge_offer),
            },
            Instant::now(),
        )
        .expect("merge complete");
    assert_eq!(fs::read(root.join("docs/merged.txt")).expect("merged"), b"");

    let skip_offer = directory_offer(&peer, "skipped.txt");
    let skip_token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &root,
        skip_offer.clone(),
        ConflictSelections::default()
            .with_entry(EntryId::new(2).expect("directory"), ConflictDecision::Skip),
    )
    .await;
    router
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: skip_offer.transfer_id,
                authorization: proof(&skip_token),
            },
            Instant::now(),
        )
        .expect("skip status");
    router
        .complete(
            peer.device_id(),
            TransferComplete {
                transfer_id: skip_offer.transfer_id,
                authorization: proof(&skip_token),
                manifest_digest: digest(&skip_offer),
            },
            Instant::now(),
        )
        .expect("skip complete");
    assert!(!root.join("docs/skipped.txt").exists());
}

#[tokio::test]
async fn paused_transfer_reopens_only_at_original_root_and_missing_root_fails_explicitly() {
    let temp = tempdir().expect("root");
    let identity_root = temp.path().join("identity");
    fs::create_dir(&identity_root).expect("identity root");
    let (peer, trust) = peer_context(&identity_root);
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let binding_path = temp.path().join("bindings.json");
    let output = temp.path().join("output");
    fs::create_dir(&output).expect("output");
    let offer = offer(&peer, "paused.txt");
    let router = ReceiverRouter::new(
        Arc::clone(&manager),
        ReceiveBindingStore::new(&binding_path),
        ReceiverPolicy::default(),
        None,
    )
    .expect("router");
    let token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &output,
        offer.clone(),
        ConflictSelections::default(),
    )
    .await;
    router
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: offer.transfer_id,
                authorization: proof(&token),
            },
            Instant::now(),
        )
        .expect("initialize");
    assert_eq!(router.pause_all().expect("pause"), 1);
    drop(router);

    let restarted = ReceiverRouter::new(
        Arc::clone(&manager),
        ReceiveBindingStore::new(&binding_path),
        ReceiverPolicy::default(),
        None,
    )
    .expect("restart");
    assert_eq!(
        restarted
            .status(
                peer.device_id(),
                TransferStatusRequest {
                    transfer_id: offer.transfer_id,
                    authorization: proof(&token),
                },
                Instant::now(),
            )
            .expect("resume")
            .status,
        TransferStatus::Paused
    );
    drop(restarted);
    fs::rename(&output, temp.path().join("detached-output")).expect("detach root");
    let missing = ReceiverRouter::new(
        manager,
        ReceiveBindingStore::new(&binding_path),
        ReceiverPolicy::default(),
        None,
    )
    .expect("missing-root router");
    assert!(matches!(
        missing.status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: offer.transfer_id,
                authorization: proof(&token),
            },
            Instant::now(),
        ),
        Err(ReceiverRouterError::BindingMismatch)
    ));
}

#[tokio::test]
async fn global_task_limit_resume_original_root_plan_update_and_cleanup_fail_closed() {
    let temp = tempdir().expect("root");
    let identity_root = temp.path().join("identity");
    fs::create_dir(&identity_root).expect("identity root");
    let (peer, trust) = peer_context(&identity_root);
    let manager = Arc::new(OfferManager::new(trust, OfferPolicy::default()));
    let binding_path = temp.path().join("bindings.json");
    let root_a = temp.path().join("a");
    let root_b = temp.path().join("b");
    fs::create_dir(&root_a).expect("root a");
    fs::create_dir(&root_b).expect("root b");
    let first = offer(&peer, "raced.txt");
    let second = offer(&peer, "blocked.txt");
    let router = ReceiverRouter::new(
        Arc::clone(&manager),
        ReceiveBindingStore::new(&binding_path),
        ReceiverPolicy {
            max_receive_tasks: 1,
            ..ReceiverPolicy::default()
        },
        None,
    )
    .expect("router");
    let first_token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &root_a,
        first.clone(),
        ConflictSelections::default(),
    )
    .await;
    let second_token = submit_and_accept(
        &router,
        &manager,
        &peer,
        &root_b,
        second.clone(),
        ConflictSelections::default(),
    )
    .await;
    router
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: first.transfer_id,
                authorization: proof(&first_token),
            },
            Instant::now(),
        )
        .expect("first reserves global task");
    assert!(matches!(
        router.status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: second.transfer_id,
                authorization: proof(&second_token),
            },
            Instant::now(),
        ),
        Err(ReceiverRouterError::ReceiveTaskLimit)
    ));

    fs::write(root_a.join("raced.txt"), b"racer").expect("race");
    assert!(matches!(
        router.complete(
            peer.device_id(),
            TransferComplete {
                transfer_id: first.transfer_id,
                authorization: proof(&first_token),
                manifest_digest: digest(&first),
            },
            Instant::now(),
        ),
        Err(ReceiverRouterError::Receiver(
            quick_share_transfer::receiver::ReceiverError::ConflictPending { .. }
        ))
    ));
    let overwrite = DestinationPlan::build(
        &root_a,
        &first,
        &ConflictSelections::default()
            .with_entry(EntryId::new(1).expect("entry"), ConflictDecision::Overwrite),
    )
    .expect("overwrite plan");
    router
        .update_plan(first.transfer_id, &first, overwrite)
        .expect("update plan");
    router
        .complete(
            peer.device_id(),
            TransferComplete {
                transfer_id: first.transfer_id,
                authorization: proof(&first_token),
                manifest_digest: digest(&first),
            },
            Instant::now(),
        )
        .expect("retry complete");
    assert_eq!(fs::read(root_a.join("raced.txt")).expect("final"), b"");

    router.pause_all().expect("pause");
    drop(router);
    let restarted = ReceiverRouter::new(
        Arc::clone(&manager),
        ReceiveBindingStore::new(&binding_path),
        ReceiverPolicy::default(),
        None,
    )
    .expect("restarted router");
    let status = restarted
        .status(
            peer.device_id(),
            TransferStatusRequest {
                transfer_id: first.transfer_id,
                authorization: proof(&first_token),
            },
            Instant::now(),
        )
        .expect("reopen original root");
    assert_eq!(status.status, TransferStatus::Completed);
    restarted.cleanup(first.transfer_id).expect("cleanup");
    assert!(
        ReceiveBindingStore::new(&binding_path)
            .get(first.transfer_id)
            .expect("binding lookup")
            .is_none()
    );
    assert!(
        !root_a
            .join(".quick-share-staging")
            .join(first.transfer_id.as_uuid().to_string())
            .exists()
    );
    assert!(
        !root_a.join(".quick-share-staging").exists(),
        "staging root must be removed with the last transfer"
    );
}
