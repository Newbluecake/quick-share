use quick_share_cli::{
    DevicesIntent, InteractionPolicy,
    devices::{DeviceCommandOutcome, run_devices},
};
use quick_share_core::identity::{IdentityStore, TrustedDeviceStore};
use tempfile::tempdir;

#[test]
fn devices_list_rename_remove_are_atomic_and_never_print_full_keys() {
    let root = tempdir().expect("root");
    let first = IdentityStore::new(root.path().join("first.json"))
        .load_or_create()
        .expect("first");
    let second = IdentityStore::new(root.path().join("second.json"))
        .load_or_create()
        .expect("second");
    let store = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    store.trust(&first, "Laptop").expect("trust first");
    store.trust(&second, "Phone").expect("trust second");

    let mut json = Vec::new();
    assert_eq!(
        run_devices(
            &store,
            &DevicesIntent::List { json: true },
            InteractionPolicy::new(false, false),
            &mut json,
        )
        .expect("list"),
        DeviceCommandOutcome::Listed(2)
    );
    let json = String::from_utf8(json).expect("UTF-8");
    assert!(json.contains("fingerprint"));
    assert!(!json.contains(&hex::encode(first.public_key())));
    assert!(!json.contains(&hex::encode(second.public_key())));

    let first_id = first.device_id();
    run_devices(
        &store,
        &DevicesIntent::Rename {
            device: first_id.to_string(),
            name: "Work laptop".to_owned(),
        },
        InteractionPolicy::new(false, false),
        &mut Vec::new(),
    )
    .expect("rename");
    assert_eq!(
        store
            .list()
            .expect("list")
            .into_iter()
            .find(|item| item.device_id == first_id)
            .expect("first")
            .name,
        "Work laptop"
    );

    assert!(
        run_devices(
            &store,
            &DevicesIntent::Remove {
                device: first_id.to_string(),
                assume_yes: false,
            },
            InteractionPolicy::new(false, false),
            &mut Vec::new(),
        )
        .is_err()
    );
    assert_eq!(store.list().expect("still present").len(), 2);
    assert!(matches!(
        run_devices(
            &store,
            &DevicesIntent::Remove {
                device: first_id.to_string(),
                assume_yes: true,
            },
            InteractionPolicy::new(false, true),
            &mut Vec::new(),
        )
        .expect("remove"),
        DeviceCommandOutcome::Removed(_)
    ));
    assert_eq!(store.list().expect("removed").len(), 1);
}

#[test]
fn trusted_device_names_reject_terminal_control_characters() {
    let root = tempdir().expect("root");
    let identity = IdentityStore::new(root.path().join("identity.json"))
        .load_or_create()
        .expect("identity");
    let store = TrustedDeviceStore::new(root.path().join("trusted.toml"));
    assert!(store.trust(&identity, "evil\u{1b}[31m").is_err());
}
