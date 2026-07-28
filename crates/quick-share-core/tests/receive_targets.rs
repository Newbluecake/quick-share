use proptest::prelude::*;
use quick_share_core::{
    destination::{
        ConflictDecision, ConflictKind, ConflictSelections, DestinationDisposition, DestinationPlan,
    },
    identity::{TrustStatus, TrustedDevice, device_id_from_public_key},
    paths::RelativePath,
    receive::{ReceiveBinding, ReceiveBindingStore, ReceiveDestinationStore},
};
use quick_share_platform::AppDirs;
use quick_share_protocol::{
    Capability, ContentKind, DeviceId, DeviceInfo, EntryId, ManifestEntry, ManifestEntryKind,
    ProtocolVersion, TransferId, TransferOffer,
};
use std::{collections::BTreeSet, fs};
use tempfile::tempdir;
use uuid::Uuid;

fn device(key: u8, name: &str) -> TrustedDevice {
    let public_key = [key; 32];
    TrustedDevice {
        device_id: device_id_from_public_key(&public_key).expect("derived device ID"),
        name: name.to_owned(),
        public_key,
    }
}

fn trusted(device: &TrustedDevice) -> TrustStatus {
    TrustStatus::Trusted(device.clone())
}

fn entry(id: u32, path: &str, kind: ManifestEntryKind) -> ManifestEntry {
    ManifestEntry {
        id: EntryId::new(id).expect("entry ID"),
        relative_path: path.to_owned(),
        size: 0,
        digest: matches!(kind, ManifestEntryKind::File).then_some([id as u8; 32]),
        kind,
    }
}

fn offer(entries: Vec<ManifestEntry>) -> TransferOffer {
    let total_bytes = entries.iter().map(|entry| entry.size).sum();
    TransferOffer {
        protocol_version: ProtocolVersion::V1_1,
        transfer_id: TransferId::new(Uuid::now_v7()),
        initiated_by: None,
        sender: DeviceInfo {
            device_id: DeviceId::parse("qs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("sender"),
            name: "sender".to_owned(),
            capabilities: BTreeSet::from([
                Capability::Files,
                Capability::Directories,
                Capability::RemoteSelection,
            ]),
        },
        content_kind: ContentKind::Files,
        chunk_size: 4 * 1024 * 1024,
        total_bytes,
        entries,
    }
}

fn committed_path(plan: &DestinationPlan, id: u32) -> (&str, bool) {
    match plan
        .entry(EntryId::new(id).expect("entry ID"))
        .expect("planned entry")
    {
        DestinationDisposition::Commit {
            relative_path,
            replace_existing,
        } => (relative_path.as_str(), *replace_existing),
        DestinationDisposition::Skip => panic!("entry was unexpectedly skipped"),
    }
}

#[test]
fn trusted_device_destinations_survive_restart_and_bind_to_the_complete_key() {
    // Arrange
    let root = tempdir().expect("root");
    let first_dir = root.path().join("first");
    let second_dir = root.path().join("second");
    fs::create_dir_all(&first_dir).expect("first directory");
    fs::create_dir_all(&second_dir).expect("second directory");
    let path = root.path().join("receive-destinations.toml");
    let store = ReceiveDestinationStore::new(&path);
    let dirs = AppDirs::for_test(root.path().join("app-dirs"));
    let first = device(1, "laptop");
    let renamed = device(1, "renamed laptop");
    let changed_identity = device(9, "laptop");
    let mut malformed_identity = first.clone();
    malformed_identity.public_key = [9; 32];
    let second = device(2, "desktop");

    // Act
    assert_eq!(
        store
            .preferred_directory(&first, &dirs)
            .expect("default directory"),
        dirs.download_dir()
    );
    assert!(store.remember(&TrustStatus::Unknown, &first_dir).is_err());
    store
        .remember(&trusted(&first), &first_dir)
        .expect("remember first");
    store
        .remember(&trusted(&second), &second_dir)
        .expect("remember second");
    let reopened = ReceiveDestinationStore::new(&path);

    // Assert
    assert_eq!(
        reopened.get(&renamed).expect("lookup"),
        Some(first_dir.clone())
    );
    assert_eq!(reopened.get(&second).expect("lookup"), Some(second_dir));
    assert_eq!(
        reopened
            .get(&changed_identity)
            .expect("new identity lookup"),
        None
    );
    assert!(reopened.get(&malformed_identity).is_err());
    assert!(
        reopened
            .remember(&trusted(&malformed_identity), &first_dir)
            .is_err()
    );
    assert!(
        reopened
            .remember(&trusted(&first), root.path().join("missing"))
            .is_err()
    );
    assert!(!format!("{reopened:?}").contains(&first_dir.display().to_string()));
}

#[test]
fn concurrent_destination_updates_from_shared_store_handles_preserve_every_device() {
    let root = tempdir().expect("root");
    let store = ReceiveDestinationStore::new(root.path().join("destinations.toml"));
    let devices = (1_u8..=8)
        .map(|key| {
            (
                device(key, &format!("peer-{key}")),
                root.path().join(format!("dir-{key}")),
            )
        })
        .collect::<Vec<_>>();
    for (_, directory) in &devices {
        fs::create_dir(directory).expect("directory");
    }

    let handles = devices
        .clone()
        .into_iter()
        .map(|(device, directory)| {
            let store = store.clone();
            std::thread::spawn(move || store.remember(&trusted(&device), directory))
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("writer thread").expect("remember");
    }

    for (device, directory) in devices {
        assert_eq!(store.get(&device).expect("lookup"), Some(directory));
    }
}

#[test]
fn destination_store_corruption_fails_closed_without_losing_the_previous_file() {
    let root = tempdir().expect("root");
    let path = root.path().join("receive-destinations.toml");
    fs::write(&path, "version = 99\n").expect("corrupt store");
    let store = ReceiveDestinationStore::new(&path);

    assert!(store.get(&device(1, "peer")).is_err());
    assert_eq!(
        fs::read_to_string(path).expect("source retained"),
        "version = 99\n"
    );
}

#[test]
fn rename_avoids_paths_reserved_by_other_manifest_entries() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("file.txt"), b"old").expect("existing file");
    let offered = offer(vec![
        entry(1, "file.txt", ManifestEntryKind::File),
        entry(2, "file (1).txt", ManifestEntryKind::File),
    ]);
    let selections = ConflictSelections::default()
        .with_entry(EntryId::new(1).expect("entry"), ConflictDecision::Rename);

    let plan = DestinationPlan::build(root.path(), &offered, &selections).expect("plan");
    assert!(matches!(
        plan.entry(EntryId::new(1).expect("entry")),
        Some(DestinationDisposition::Commit { relative_path, replace_existing: false })
            if relative_path.as_str() == "file (2).txt"
    ));
}

#[test]
fn apply_all_is_scoped_to_subsequent_conflicts_of_the_same_kind() {
    let root = tempdir().expect("root");
    fs::create_dir(root.path().join("existing-dir")).expect("existing directory");
    fs::write(root.path().join("existing.txt"), b"old").expect("existing file");
    let offered = offer(vec![
        entry(1, "existing-dir", ManifestEntryKind::Directory),
        entry(2, "existing.txt", ManifestEntryKind::File),
    ]);

    let file_only =
        ConflictSelections::default().apply_to_all(ConflictKind::File, ConflictDecision::Skip);
    assert!(matches!(
        DestinationPlan::build(root.path(), &offered, &file_only),
        Err(quick_share_core::destination::DestinationPlanError::UnresolvedConflict {
            entry_id,
            ..
        }) if entry_id == EntryId::new(1).expect("entry")
    ));

    let both_kinds = file_only.apply_to_all(ConflictKind::Directory, ConflictDecision::Skip);
    let plan = DestinationPlan::build(root.path(), &offered, &both_kinds).expect("complete plan");
    assert!(matches!(
        plan.entry(EntryId::new(1).expect("entry")),
        Some(DestinationDisposition::Skip)
    ));
    assert!(matches!(
        plan.entry(EntryId::new(2).expect("entry")),
        Some(DestinationDisposition::Skip)
    ));
}

#[test]
fn directory_rename_and_skip_rewrite_or_remove_the_complete_subtree() {
    // Arrange
    let root = tempdir().expect("root");
    fs::create_dir(root.path().join("photos")).expect("existing root");
    let transfer = offer(vec![
        entry(1, "photos", ManifestEntryKind::Directory),
        entry(2, "photos/2026", ManifestEntryKind::Directory),
        entry(3, "photos/2026/image.jpg", ManifestEntryKind::File),
    ]);
    let renamed = ConflictSelections::default()
        .with_entry(EntryId::new(1).expect("entry"), ConflictDecision::Rename);
    let skipped = ConflictSelections::default()
        .with_entry(EntryId::new(1).expect("entry"), ConflictDecision::Skip);

    // Act
    let rename_plan = DestinationPlan::build(root.path(), &transfer, &renamed).expect("rename");
    let skip_plan = DestinationPlan::build(root.path(), &transfer, &skipped).expect("skip");

    // Assert
    assert_eq!(committed_path(&rename_plan, 1), ("photos (1)", false));
    assert_eq!(committed_path(&rename_plan, 2), ("photos (1)/2026", false));
    assert_eq!(
        committed_path(&rename_plan, 3),
        ("photos (1)/2026/image.jpg", false)
    );
    for id in 1..=3 {
        assert!(matches!(
            skip_plan.entry(EntryId::new(id).expect("entry")),
            Some(DestinationDisposition::Skip)
        ));
    }
}

#[test]
fn file_conflicts_support_per_entry_and_apply_all_without_silent_overwrite() {
    // Arrange
    let root = tempdir().expect("root");
    fs::write(root.path().join("a.txt"), b"old").expect("conflict a");
    fs::write(root.path().join("b.txt"), b"old").expect("conflict b");
    let transfer = offer(vec![
        entry(1, "a.txt", ManifestEntryKind::File),
        entry(2, "b.txt", ManifestEntryKind::File),
    ]);

    // Act + Assert
    assert!(
        DestinationPlan::build(root.path(), &transfer, &ConflictSelections::default()).is_err()
    );
    let selections = ConflictSelections::default()
        .with_entry(EntryId::new(1).expect("entry"), ConflictDecision::Overwrite)
        .apply_to_all(ConflictKind::File, ConflictDecision::Rename);
    let plan = DestinationPlan::build(root.path(), &transfer, &selections).expect("plan");
    assert_eq!(committed_path(&plan, 1), ("a.txt", true));
    assert_eq!(committed_path(&plan, 2), ("b (1).txt", false));
}

#[test]
fn receive_binding_is_idempotent_but_rejects_sender_manifest_or_root_changes() {
    // Arrange
    let root = tempdir().expect("root");
    let output = root.path().join("output");
    fs::create_dir(&output).expect("output");
    let transfer = offer(vec![entry(1, "file.txt", ManifestEntryKind::File)]);
    let plan =
        DestinationPlan::build(&output, &transfer, &ConflictSelections::default()).expect("plan");
    let binding = ReceiveBinding {
        transfer_id: transfer.transfer_id,
        sender_device_id: transfer.sender.device_id.clone(),
        manifest_digest: [7; 32],
        output_root: output.clone(),
        destination_plan: plan,
    };
    let path = root.path().join("receive-bindings.json");
    let store = ReceiveBindingStore::new(&path);

    // Act + Assert
    store.bind(binding.clone()).expect("bind");
    store.bind(binding.clone()).expect("idempotent bind");
    let diagnostic = format!("{binding:?}");
    assert!(!diagnostic.contains(&output.display().to_string()));
    assert!(diagnostic.contains("REDACTED"));
    assert_eq!(
        store.get(binding.transfer_id).expect("get"),
        Some(binding.clone())
    );
    let mut changed = binding.clone();
    changed.manifest_digest = [8; 32];
    assert!(store.bind(changed).is_err());
    let reopened = ReceiveBindingStore::new(&path);
    assert_eq!(
        reopened.get(binding.transfer_id).expect("reopen"),
        Some(binding.clone())
    );
    assert!(reopened.remove(binding.transfer_id).expect("remove"));
    assert_eq!(reopened.get(binding.transfer_id).expect("removed"), None);
}

#[test]
fn destination_plan_rejects_portable_case_collisions() {
    let root = tempdir().expect("root");
    let transfer = offer(vec![
        entry(1, "Report.txt", ManifestEntryKind::File),
        entry(2, "report.TXT", ManifestEntryKind::File),
    ]);

    assert!(
        DestinationPlan::build(root.path(), &transfer, &ConflictSelections::default()).is_err()
    );
}

proptest! {
    #[test]
    fn generated_conflict_free_plans_keep_every_commit_under_the_output_root(
        names in prop::collection::vec("[a-z][a-z0-9]{0,7}", 1..32)
    ) {
        let root = tempdir().expect("root");
        let unique = names
            .into_iter()
            .filter(|name| RelativePath::parse(format!("{name}.txt")).is_ok())
            .collect::<BTreeSet<_>>();
        prop_assume!(!unique.is_empty());
        let entries = unique
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                entry(
                    u32::try_from(index + 1).expect("bounded entry ID"),
                    &format!("{name}.txt"),
                    ManifestEntryKind::File,
                )
            })
            .collect();
        let plan = DestinationPlan::build(
            root.path(),
            &offer(entries),
            &ConflictSelections::default(),
        )
        .expect("safe generated plan");

        for (_, disposition) in plan.entries() {
            if let DestinationDisposition::Commit { relative_path, .. } = disposition {
                prop_assert!(relative_path.resolve_under(root.path()).starts_with(root.path()));
                prop_assert!(RelativePath::parse(relative_path.as_str()).is_ok());
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn destination_plan_rejects_symlink_ancestors_instead_of_planning_through_them() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("root");
    let outside = tempdir().expect("outside");
    symlink(outside.path(), root.path().join("redirect")).expect("symlink");
    let transfer = offer(vec![entry(1, "redirect/file.txt", ManifestEntryKind::File)]);

    assert!(
        DestinationPlan::build(root.path(), &transfer, &ConflictSelections::default()).is_err()
    );
    assert!(!std::path::Path::new(outside.path().join("file.txt").as_os_str()).exists());
}
