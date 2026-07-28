use quick_share_core::{
    config::ConflictPolicy,
    destination::{ConflictDecision, ConflictSelections, DestinationPlan},
    paths::RelativePath,
};
use quick_share_protocol::{
    Capability, ChunkDescriptor, ContentKind, DeviceId, DeviceInfo, EntryId, ManifestEntry,
    ManifestEntryKind, ProtocolVersion, TransferId, TransferOffer,
};
use quick_share_transfer::{
    ChunkBegin, CommitOutcome, FaultPoint, MetadataState, StagedFile, StoreError, TransferStore,
    hash_file,
};
use std::fs;
use tempfile::tempdir;
use uuid::Uuid;

const CHUNK_SIZE: u32 = 256 * 1024;

fn transfer_id() -> TransferId {
    TransferId::new(Uuid::now_v7())
}
fn entry_id() -> EntryId {
    EntryId::new(1).expect("entry ID")
}
fn sender_id() -> DeviceId {
    DeviceId::parse("qs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("sender ID")
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn destination_offer(id: TransferId, path: &str, bytes: &[u8]) -> TransferOffer {
    TransferOffer {
        protocol_version: ProtocolVersion::V1_0,
        transfer_id: id,
        initiated_by: None,
        sender: DeviceInfo {
            device_id: sender_id(),
            name: "sender".to_owned(),
            capabilities: [Capability::Files].into_iter().collect(),
        },
        content_kind: ContentKind::Files,
        chunk_size: CHUNK_SIZE,
        total_bytes: bytes.len() as u64,
        entries: vec![ManifestEntry {
            id: entry_id(),
            relative_path: path.to_owned(),
            kind: ManifestEntryKind::File,
            size: bytes.len() as u64,
            digest: Some(digest(bytes)),
        }],
    }
}

fn descriptor(transfer_id: TransferId, index: u32, bytes: &[u8]) -> ChunkDescriptor {
    ChunkDescriptor {
        transfer_id,
        entry_id: entry_id(),
        index,
        offset: u64::from(index) * u64::from(CHUNK_SIZE),
        length: u32::try_from(bytes.len()).expect("chunk length"),
        digest: digest(bytes),
    }
}

#[test]
fn final_path_is_hidden_until_all_chunks_and_full_digest_are_verified() {
    let root = tempdir().expect("output root");
    let payload = vec![b'x'; usize::try_from(CHUNK_SIZE).expect("chunk size") + 3];
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("result.bin").expect("relative path"),
        payload.len() as u64,
        CHUNK_SIZE,
        digest(&payload),
    )
    .expect("staged file");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    assert!(store.staging_path().starts_with(root.path()));
    assert_eq!(store.sender_device_id(), &sender_id());

    {
        let mut writer = store.chunk_writer(entry_id()).expect("chunk writer");
        writer
            .write(
                &descriptor(id, 1, &payload[CHUNK_SIZE as usize..]),
                &payload[CHUNK_SIZE as usize..],
                FaultPoint::None,
            )
            .expect("last chunk first");
        assert_eq!(writer.missing_chunks().expect("missing"), vec![0]);
    }
    assert!(!root.path().join("result.bin").exists());
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, &payload[..CHUNK_SIZE as usize]),
            &payload[..CHUNK_SIZE as usize],
            FaultPoint::None,
        )
        .expect("first chunk");
    let outcome = store
        .commit_file(entry_id(), ConflictPolicy::Error, FaultPoint::None)
        .expect("commit");

    assert_eq!(
        outcome,
        CommitOutcome::Committed(root.path().join("result.bin"))
    );
    assert_eq!(
        fs::read(root.path().join("result.bin")).expect("final payload"),
        payload
    );
}

#[test]
fn completed_chunk_boundary_is_recoverable_after_restart() {
    let payload = vec![0x6b; CHUNK_SIZE as usize * 2 + 17];
    let chunks = payload.chunks(CHUNK_SIZE as usize).collect::<Vec<_>>();
    for boundary in 0..=chunks.len() {
        let root = tempdir().expect("output root");
        let id = transfer_id();
        let spec = StagedFile::new(
            entry_id(),
            RelativePath::parse("resume.bin").expect("relative"),
            payload.len() as u64,
            CHUNK_SIZE,
            digest(&payload),
        )
        .expect("spec");
        let mut store =
            TransferStore::create_bound(root.path(), id, sender_id(), vec![spec], [9; 32])
                .expect("store");
        for (index, bytes) in chunks.iter().enumerate().take(boundary) {
            store
                .write_chunk(
                    entry_id(),
                    &descriptor(id, index as u32, bytes),
                    bytes,
                    FaultPoint::None,
                )
                .expect("pre-restart chunk");
        }
        drop(store);

        let mut resumed =
            TransferStore::reopen_bound(root.path(), id, &sender_id(), [9; 32]).expect("reopen");
        assert_eq!(
            resumed.missing_chunks(entry_id()).expect("missing"),
            (boundary as u32..chunks.len() as u32).collect::<Vec<_>>()
        );
        for (index, bytes) in chunks.iter().enumerate().skip(boundary) {
            resumed
                .write_chunk(
                    entry_id(),
                    &descriptor(id, index as u32, bytes),
                    bytes,
                    FaultPoint::None,
                )
                .expect("post-restart chunk");
        }
        resumed
            .commit_file(entry_id(), ConflictPolicy::Error, FaultPoint::None)
            .expect("commit resumed file");
        assert_eq!(
            fs::read(root.path().join("resume.bin")).expect("payload"),
            payload
        );
    }
}

#[test]
fn duplicate_chunks_are_idempotent_but_digest_or_length_mismatch_fails() {
    let root = tempdir().expect("output root");
    let bytes = b"small payload";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("small.bin").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    let good = descriptor(id, 0, bytes);

    store
        .write_chunk(entry_id(), &good, bytes, FaultPoint::None)
        .expect("first write");
    store
        .write_chunk(entry_id(), &good, bytes, FaultPoint::None)
        .expect("duplicate write");
    let mut wrong = good.clone();
    wrong.digest = [9; 32];
    assert!(matches!(
        store.write_chunk(entry_id(), &wrong, bytes, FaultPoint::None),
        Err(StoreError::ChunkDigestMismatch { .. })
    ));
    assert!(matches!(
        store.write_chunk(entry_id(), &good, b"short", FaultPoint::None),
        Err(StoreError::InvalidChunk(_))
    ));
}

#[test]
fn streaming_chunk_fragments_are_ordered_digest_checked_and_idempotent() {
    let root = tempdir().expect("output root");
    let bytes = b"streamed payload";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("stream.bin").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create_bound(root.path(), id, sender_id(), vec![spec], [4; 32])
        .expect("store");
    let descriptor = descriptor(id, 0, bytes);
    let mut upload = match store.begin_chunk(&descriptor).expect("begin") {
        ChunkBegin::Upload(upload) => upload,
        ChunkBegin::Duplicate => panic!("new chunk cannot be duplicate"),
    };
    assert!(upload.write_fragment(1, b"bad order").is_err());
    upload
        .write_fragment(0, &bytes[..4])
        .expect("first fragment");
    upload
        .write_fragment(4, &bytes[4..])
        .expect("last fragment");
    store.finish_chunk(*upload).expect("finish");
    assert!(matches!(
        store.begin_chunk(&descriptor).expect("duplicate"),
        ChunkBegin::Duplicate
    ));
    drop(store);
    assert!(TransferStore::reopen_bound(root.path(), id, &sender_id(), [5; 32]).is_err());
    let different_sender =
        DeviceId::parse("qs_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").expect("different sender");
    assert!(TransferStore::reopen_bound(root.path(), id, &different_sender, [4; 32]).is_err());
    let reopened =
        TransferStore::reopen_bound(root.path(), id, &sender_id(), [4; 32]).expect("bound reopen");
    assert!(
        reopened
            .missing_chunks(entry_id())
            .expect("missing")
            .is_empty()
    );
}

#[test]
fn metadata_commit_intent_survives_restart_and_is_idempotently_finishable() {
    let root = tempdir().expect("output root");
    let id = transfer_id();
    let metadata_id = EntryId::new(9).expect("metadata ID");
    let mut store = TransferStore::create_bound(root.path(), id, sender_id(), Vec::new(), [6; 32])
        .expect("store");
    store.register_metadata(&[metadata_id]).expect("register");
    let destination = RelativePath::parse("empty-dir").expect("relative");
    store
        .begin_metadata_commit(metadata_id, &destination)
        .expect("intent");
    drop(store);

    let mut reopened =
        TransferStore::reopen_bound(root.path(), id, &sender_id(), [6; 32]).expect("reopen");
    assert_eq!(
        reopened.metadata_state(metadata_id).expect("state"),
        MetadataState::Committing {
            destination: destination.clone()
        }
    );
    fs::create_dir(root.path().join("empty-dir")).expect("metadata side effect");
    reopened
        .finish_metadata_commit(metadata_id, false)
        .expect("finish");
    assert!(matches!(
        reopened.metadata_state(metadata_id).expect("state"),
        MetadataState::Committed { .. }
    ));
}

#[test]
fn journal_ordering_recovers_faults_before_and_after_state_replace() {
    let root = tempdir().expect("output root");
    let bytes = b"recoverable";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("recover.bin").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    let chunk = descriptor(id, 0, bytes);

    let storage_full = store.write_chunk(
        entry_id(),
        &chunk,
        bytes,
        FaultPoint::StorageFullBeforeWrite,
    );
    assert!(matches!(
        storage_full,
        Err(StoreError::Io(error)) if error.kind() == std::io::ErrorKind::StorageFull
    ));
    assert_eq!(store.missing_chunks(entry_id()).expect("missing"), vec![0]);
    assert!(matches!(
        store.write_chunk(entry_id(), &chunk, bytes, FaultPoint::AfterDataSync),
        Err(StoreError::InjectedFault(FaultPoint::AfterDataSync))
    ));
    drop(store);
    let wrong_sender =
        DeviceId::parse("qs_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").expect("wrong sender ID");
    assert!(TransferStore::reopen(root.path(), id, &wrong_sender).is_err());
    let mut reopened =
        TransferStore::reopen(root.path(), id, &sender_id()).expect("reopen after data");
    assert_eq!(
        reopened.missing_chunks(entry_id()).expect("missing"),
        vec![0]
    );
    assert!(matches!(
        reopened.write_chunk(entry_id(), &chunk, bytes, FaultPoint::BeforeJournalReplace),
        Err(StoreError::InjectedFault(FaultPoint::BeforeJournalReplace))
    ));
    drop(reopened);
    let mut reopened =
        TransferStore::reopen(root.path(), id, &sender_id()).expect("reopen before journal");
    assert_eq!(
        reopened.missing_chunks(entry_id()).expect("missing"),
        vec![0]
    );
    assert!(matches!(
        reopened.write_chunk(entry_id(), &chunk, bytes, FaultPoint::AfterJournalReplace),
        Err(StoreError::InjectedFault(FaultPoint::AfterJournalReplace))
    ));
    drop(reopened);
    let reopened =
        TransferStore::reopen(root.path(), id, &sender_id()).expect("reopen after journal");
    assert!(
        reopened
            .missing_chunks(entry_id())
            .expect("missing")
            .is_empty()
    );
}

#[test]
fn small_file_chunk_log_and_batch_commit_scale_without_losing_restart_state() {
    let root = tempdir().expect("output root");
    let id = transfer_id();
    let files = (1..=256_u32)
        .map(|raw| {
            let entry_id = EntryId::new(raw).expect("entry ID");
            let bytes = [(raw % 251) as u8];
            StagedFile::new(
                entry_id,
                RelativePath::parse(format!("small-{raw:03}.bin")).expect("relative"),
                1,
                CHUNK_SIZE,
                digest(&bytes),
            )
            .expect("small file")
        })
        .collect::<Vec<_>>();
    let mut store = TransferStore::create(root.path(), id, sender_id(), files).expect("store");
    for raw in 1..=256_u32 {
        let entry_id = EntryId::new(raw).expect("entry ID");
        let bytes = [(raw % 251) as u8];
        store
            .write_chunk(
                entry_id,
                &ChunkDescriptor {
                    transfer_id: id,
                    entry_id,
                    index: 0,
                    offset: 0,
                    length: 1,
                    digest: digest(&bytes),
                },
                &bytes,
                FaultPoint::None,
            )
            .expect("small chunk");
    }
    drop(store);

    let mut reopened = TransferStore::reopen(root.path(), id, &sender_id()).expect("reopen");
    let entries = (1..=256_u32)
        .map(|raw| EntryId::new(raw).expect("entry ID"))
        .collect::<Vec<_>>();
    assert!(entries.iter().all(|entry_id| {
        reopened
            .missing_chunks(*entry_id)
            .expect("missing")
            .is_empty()
    }));
    let outcomes = reopened
        .commit_files_batch(&entries, ConflictPolicy::Error)
        .expect("batch commit");
    assert_eq!(outcomes.len(), entries.len());
    for raw in 1..=256_u32 {
        assert_eq!(
            fs::read(root.path().join(format!("small-{raw:03}.bin"))).expect("committed"),
            [(raw % 251) as u8]
        );
    }
}

#[test]
fn interrupted_commit_intent_rechecks_a_raced_destination() {
    let root = tempdir().expect("output root");
    let bytes = b"verified payload";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("race.txt").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");
    assert!(matches!(
        store.commit_file(
            entry_id(),
            ConflictPolicy::Rename,
            FaultPoint::AfterCommitIntent,
        ),
        Err(StoreError::InjectedFault(FaultPoint::AfterCommitIntent))
    ));
    fs::write(root.path().join("race.txt"), b"racer").expect("raced destination");

    let outcome = store
        .commit_file(entry_id(), ConflictPolicy::Rename, FaultPoint::None)
        .expect("retry commit");

    assert_eq!(
        fs::read(root.path().join("race.txt")).expect("racer"),
        b"racer"
    );
    assert_eq!(
        outcome,
        CommitOutcome::Committed(root.path().join("race (1).txt"))
    );
}

#[cfg(unix)]
#[test]
fn raced_symlink_ancestor_cannot_redirect_capability_bound_final_commit() {
    let root = tempdir().expect("output root");
    let outside = tempdir().expect("outside root");
    let bytes = b"contained";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("nested/file.bin").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");
    assert!(matches!(
        store.commit_file(
            entry_id(),
            ConflictPolicy::Error,
            FaultPoint::AfterCommitIntent,
        ),
        Err(StoreError::InjectedFault(FaultPoint::AfterCommitIntent))
    ));
    std::os::unix::fs::symlink(outside.path(), root.path().join("nested")).expect("raced symlink");
    assert!(
        store
            .commit_file(entry_id(), ConflictPolicy::Error, FaultPoint::None)
            .is_err()
    );
    assert!(!outside.path().join("file.bin").exists());
}

#[test]
fn crash_after_final_rename_is_reconciled_without_exposing_unverified_data() {
    let root = tempdir().expect("output root");
    let bytes = b"verified before rename";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("final.bin").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");

    assert!(matches!(
        store.commit_file(
            entry_id(),
            ConflictPolicy::Error,
            FaultPoint::AfterFileRename
        ),
        Err(StoreError::InjectedFault(FaultPoint::AfterFileRename))
    ));
    assert_eq!(
        hash_file(root.path().join("final.bin")).expect("final digest"),
        digest(bytes)
    );
    let reopened =
        TransferStore::reopen(root.path(), id, &sender_id()).expect("reconcile committed file");
    assert!(reopened.is_committed(entry_id()).expect("commit status"));
}

#[test]
fn conflict_policies_never_silently_overwrite() {
    let root = tempdir().expect("output root");
    fs::write(root.path().join("report.txt"), b"old").expect("existing");
    let bytes = b"new";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("report.txt").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");

    assert!(matches!(
        store.commit_file(entry_id(), ConflictPolicy::Error, FaultPoint::None),
        Err(StoreError::Path(
            quick_share_core::paths::PathError::DestinationExists(_)
        ))
    ));
    let outcome = store
        .commit_file(entry_id(), ConflictPolicy::Rename, FaultPoint::None)
        .expect("renamed commit");
    assert_eq!(
        fs::read(root.path().join("report.txt")).expect("old"),
        b"old"
    );
    assert_eq!(
        outcome,
        CommitOutcome::Committed(root.path().join("report (1).txt"))
    );
}

#[test]
fn planned_commit_uses_exact_rename_and_skip_dispositions() {
    let renamed_root = tempdir().expect("rename root");
    let bytes = b"renamed payload";
    let rename_id = transfer_id();
    fs::write(renamed_root.path().join("report.txt"), b"existing").expect("existing");
    let rename_offer = destination_offer(rename_id, "report.txt", bytes);
    let rename_plan = DestinationPlan::build(
        renamed_root.path(),
        &rename_offer,
        &ConflictSelections::default().with_entry(entry_id(), ConflictDecision::Rename),
    )
    .expect("rename plan");
    let rename_spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("report.txt").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let mut rename_store = TransferStore::create_bound(
        renamed_root.path(),
        rename_id,
        sender_id(),
        vec![rename_spec],
        digest(&serde_json::to_vec(&rename_offer).expect("offer")),
    )
    .expect("store");
    rename_store
        .write_chunk(
            entry_id(),
            &descriptor(rename_id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");
    rename_store
        .commit_files_planned(&[entry_id()], &rename_plan, FaultPoint::None)
        .expect("rename commit");
    assert_eq!(
        fs::read(renamed_root.path().join("report.txt")).expect("existing"),
        b"existing"
    );
    assert_eq!(
        fs::read(renamed_root.path().join("report (1).txt")).expect("renamed"),
        bytes
    );

    let skipped_root = tempdir().expect("skip root");
    fs::write(skipped_root.path().join("report.txt"), b"existing").expect("existing");
    let skip_id = transfer_id();
    let skip_offer = destination_offer(skip_id, "report.txt", bytes);
    let skip_plan = DestinationPlan::build(
        skipped_root.path(),
        &skip_offer,
        &ConflictSelections::default().with_entry(entry_id(), ConflictDecision::Skip),
    )
    .expect("skip plan");
    let skip_spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("report.txt").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let mut skip_store = TransferStore::create_bound(
        skipped_root.path(),
        skip_id,
        sender_id(),
        vec![skip_spec],
        digest(&serde_json::to_vec(&skip_offer).expect("offer")),
    )
    .expect("store");
    skip_store
        .write_chunk(
            entry_id(),
            &descriptor(skip_id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");
    assert_eq!(
        skip_store
            .commit_files_planned(&[entry_id()], &skip_plan, FaultPoint::None)
            .expect("skip commit"),
        vec![CommitOutcome::Skipped]
    );
    assert_eq!(
        fs::read(skipped_root.path().join("report.txt")).expect("existing"),
        b"existing"
    );
}

#[test]
fn planned_commit_reports_races_then_accepts_an_explicit_updated_overwrite_plan() {
    let root = tempdir().expect("output root");
    let bytes = b"planned replacement";
    let id = transfer_id();
    let offer = destination_offer(id, "planned.txt", bytes);
    let initial_plan = DestinationPlan::build(root.path(), &offer, &ConflictSelections::default())
        .expect("initial no-clobber plan");
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("planned.txt").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let mut store = TransferStore::create_bound(
        root.path(),
        id,
        sender_id(),
        vec![spec],
        digest(&serde_json::to_vec(&offer).expect("offer")),
    )
    .expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");
    fs::write(root.path().join("planned.txt"), b"racer").expect("race");

    assert!(matches!(
        store.commit_files_planned(&[entry_id()], &initial_plan, FaultPoint::None),
        Err(StoreError::ConflictPending { entry_id: pending }) if pending == entry_id()
    ));
    let overwrite_plan = DestinationPlan::build(
        root.path(),
        &offer,
        &ConflictSelections::default().with_entry(entry_id(), ConflictDecision::Overwrite),
    )
    .expect("updated overwrite plan");
    let outcomes = store
        .commit_files_planned(&[entry_id()], &overwrite_plan, FaultPoint::None)
        .expect("planned overwrite");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        fs::read(root.path().join("planned.txt")).expect("final"),
        bytes
    );
}

#[test]
fn planned_commit_intent_and_rename_faults_reconcile_after_restart() {
    for fault in [
        FaultPoint::AfterCommitIntent,
        FaultPoint::AfterFileRename,
        FaultPoint::BeforeJournalReplace,
        FaultPoint::AfterJournalReplace,
    ] {
        let root = tempdir().expect("output root");
        let bytes = b"recover planned commit";
        let id = transfer_id();
        let offer = destination_offer(id, "recover-planned.txt", bytes);
        let plan = DestinationPlan::build(root.path(), &offer, &ConflictSelections::default())
            .expect("plan");
        let spec = StagedFile::new(
            entry_id(),
            RelativePath::parse("recover-planned.txt").expect("relative"),
            bytes.len() as u64,
            CHUNK_SIZE,
            digest(bytes),
        )
        .expect("spec");
        let offer_digest = digest(&serde_json::to_vec(&offer).expect("offer"));
        let mut store =
            TransferStore::create_bound(root.path(), id, sender_id(), vec![spec], offer_digest)
                .expect("store");
        store
            .write_chunk(
                entry_id(),
                &descriptor(id, 0, bytes),
                bytes,
                FaultPoint::None,
            )
            .expect("chunk");
        assert!(matches!(
            store.commit_files_planned(&[entry_id()], &plan, fault),
            Err(StoreError::InjectedFault(point)) if point == fault
        ));
        drop(store);

        let mut reopened = TransferStore::reopen_bound(root.path(), id, &sender_id(), offer_digest)
            .expect("reopen");
        reopened
            .commit_files_planned(&[entry_id()], &plan, FaultPoint::None)
            .expect("recover");
        assert_eq!(
            fs::read(root.path().join("recover-planned.txt")).expect("final"),
            bytes
        );
    }
}

#[test]
fn explicit_overwrite_atomically_replaces_existing_file() {
    let root = tempdir().expect("output root");
    fs::write(root.path().join("replace.txt"), b"old").expect("existing");
    let bytes = b"replacement";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("replace.txt").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        digest(bytes),
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");

    store
        .commit_file(entry_id(), ConflictPolicy::Overwrite, FaultPoint::None)
        .expect("explicit overwrite");

    assert_eq!(
        fs::read(root.path().join("replace.txt")).expect("replacement"),
        bytes
    );
}

#[test]
fn empty_and_sparse_large_files_use_bounded_staging() {
    let root = tempdir().expect("output root");
    assert!(
        StagedFile::new(
            EntryId::new(3).expect("oversized ID"),
            RelativePath::parse("impossibly-large.bin").expect("relative"),
            u64::MAX,
            CHUNK_SIZE,
            [0; 32],
        )
        .is_err()
    );
    let empty_digest = digest(b"");
    let empty = StagedFile::new(
        entry_id(),
        RelativePath::parse("empty.bin").expect("relative"),
        0,
        CHUNK_SIZE,
        empty_digest,
    )
    .expect("empty spec");
    let large_id = EntryId::new(2).expect("large ID");
    let large = StagedFile::new(
        large_id,
        RelativePath::parse("large.bin").expect("relative"),
        u64::from(u32::MAX) + 2,
        CHUNK_SIZE,
        [0; 32],
    )
    .expect("large spec");
    let id = transfer_id();
    let mut store =
        TransferStore::create(root.path(), id, sender_id(), vec![empty, large]).expect("store");

    assert_eq!(
        store.part_metadata(large_id).expect("large metadata").len(),
        u64::from(u32::MAX) + 2
    );
    store
        .commit_file(entry_id(), ConflictPolicy::Error, FaultPoint::None)
        .expect("empty commit");
    assert_eq!(
        fs::metadata(root.path().join("empty.bin"))
            .expect("empty final")
            .len(),
        0
    );
    assert!(
        TransferStore::list_staging(root.path())
            .expect("staging list")
            .contains(&id)
    );
}

#[cfg(unix)]
#[test]
fn unwritable_output_fails_without_creating_a_final_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("output root");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o500)).expect("make read-only");
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("denied.bin").expect("relative"),
        0,
        CHUNK_SIZE,
        digest(b""),
    )
    .expect("spec");

    let result = TransferStore::create(root.path(), transfer_id(), sender_id(), vec![spec]);
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
        .expect("restore permissions");

    assert!(result.is_err());
    assert!(!root.path().join("denied.bin").exists());
}

#[test]
fn full_digest_mismatch_keeps_part_file_and_final_hidden() {
    let root = tempdir().expect("output root");
    let bytes = b"payload";
    let spec = StagedFile::new(
        entry_id(),
        RelativePath::parse("bad.bin").expect("relative"),
        bytes.len() as u64,
        CHUNK_SIZE,
        [0; 32],
    )
    .expect("spec");
    let id = transfer_id();
    let mut store = TransferStore::create(root.path(), id, sender_id(), vec![spec]).expect("store");
    store
        .write_chunk(
            entry_id(),
            &descriptor(id, 0, bytes),
            bytes,
            FaultPoint::None,
        )
        .expect("chunk");

    assert!(matches!(
        store.commit_file(entry_id(), ConflictPolicy::Error, FaultPoint::None),
        Err(StoreError::FinalDigestMismatch { .. })
    ));
    assert!(!root.path().join("bad.bin").exists());
    assert!(store.part_metadata(entry_id()).is_ok());
}
