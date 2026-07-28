use quick_share_core::{
    config::{ConfigError, ConfigLoader, ConfigOverrides},
    identity::{IdentityError, IdentityStore, TrustStatus, TrustedDeviceStore},
};
use quick_share_platform::AppDirs;
use std::{collections::BTreeMap, fs};
use tempfile::tempdir;

#[test]
fn configuration_priority_is_cli_then_environment_then_toml_then_defaults() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let dirs = AppDirs::for_test(root.path());
    let loader = ConfigLoader::new(dirs.clone());
    let source = r#"
        [device]
        name = "from-toml"
        [discovery]
        timeout_ms = 4000
        peers = ["192.168.32.38:4242", "windows.local:4242"]
        [web]
        max_downloads = 5
    "#;
    let environment = BTreeMap::from([
        (
            "QUICK_SHARE__DISCOVERY__TIMEOUT_MS".to_owned(),
            "2500".to_owned(),
        ),
        ("QUICK_SHARE__WEB__MAX_DOWNLOADS".to_owned(), "8".to_owned()),
    ]);
    let overrides = ConfigOverrides {
        discovery_timeout_ms: Some(900),
        ..ConfigOverrides::default()
    };

    // Act
    let config = loader
        .load_from_str(Some(source), &environment, &overrides)
        .expect("load merged configuration");

    // Assert
    assert_eq!(config.device.name, "from-toml");
    assert_eq!(config.discovery.timeout_ms, 900);
    assert_eq!(
        config.discovery.peers,
        ["192.168.32.38:4242", "windows.local:4242"]
    );
    assert_eq!(config.web.max_downloads, 8);
    assert_eq!(config.receive.output, dirs.download_dir());
}

#[test]
fn configured_discovery_peers_reject_empty_duplicate_and_unbounded_lists() {
    let root = tempdir().expect("temporary root");
    let loader = ConfigLoader::new(AppDirs::for_test(root.path()));
    let load = |peers: &str| {
        loader.load_from_str(
            Some(&format!("[discovery]\npeers = {peers}")),
            &BTreeMap::new(),
            &ConfigOverrides::default(),
        )
    };

    assert!(load("[\"\"]").is_err());
    assert!(load("[\"host:4242\", \"HOST:4242\"]").is_err());
    let too_many = format!(
        "[{}]",
        (0..33)
            .map(|index| format!("\"host{index}:4242\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(load(&too_many).is_err());
}

#[test]
fn configuration_is_saved_and_loaded_from_the_standard_path() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let loader = ConfigLoader::new(AppDirs::for_test(root.path()));
    let mut config = loader
        .load(&BTreeMap::new(), &ConfigOverrides::default())
        .expect("default configuration");
    config.device.name = "persisted-device".to_owned();

    // Act
    loader.save(&config).expect("save configuration");
    let loaded = loader
        .load(&BTreeMap::new(), &ConfigOverrides::default())
        .expect("load configuration");

    // Assert
    assert_eq!(loaded, config);
    assert!(root.path().join("config/config.toml").is_file());
}

#[test]
fn unknown_or_invalid_configuration_fields_fail_with_context() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let loader = ConfigLoader::new(AppDirs::for_test(root.path()));

    // Act
    let unknown = loader.load_from_str(
        Some("[web]\nallow_everything = true"),
        &BTreeMap::new(),
        &ConfigOverrides::default(),
    );
    let invalid_env = loader.load_from_str(
        None,
        &BTreeMap::from([(
            "QUICK_SHARE__DISCOVERY__TIMEOUT_MS".to_owned(),
            "not-a-number".to_owned(),
        )]),
        &ConfigOverrides::default(),
    );

    // Assert
    assert!(matches!(unknown, Err(ConfigError::Toml(_))));
    assert!(matches!(invalid_env, Err(ConfigError::Environment { .. })));
}

#[test]
fn identity_is_created_once_reloaded_and_debug_redacts_private_key() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let identity_path = root.path().join("identity.json");
    let store = IdentityStore::new(identity_path.clone());

    // Act
    let first = store.load_or_create().expect("create identity");
    let second = store.load_or_create().expect("reload identity");
    let debug = format!("{first:?}");
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(identity_path).expect("read identity"))
            .expect("parse persisted identity");
    let private_key = persisted["privateKey"].as_str().expect("private key field");

    // Assert
    assert_eq!(first.public_key(), second.public_key());
    assert_eq!(first.device_id(), second.device_id());
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(private_key));
}

#[test]
fn concurrent_first_start_converges_on_one_identity() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let path = root.path().join("identity.json");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                IdentityStore::new(path)
                    .load_or_create()
                    .expect("concurrent identity")
                    .public_key()
            })
        })
        .collect();

    // Act
    let keys: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("identity thread"))
        .collect();

    // Assert
    assert!(keys.iter().all(|key| key == &keys[0]));
}

#[cfg(unix)]
#[test]
fn identity_file_is_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange
    let root = tempdir().expect("temporary root");
    let path = root.path().join("identity.json");
    let store = IdentityStore::new(path.clone());

    // Act
    store.load_or_create().expect("create identity");

    // Assert
    let mode = fs::metadata(path)
        .expect("identity metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn malformed_identity_fails_closed_instead_of_regenerating() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let path = root.path().join("identity.json");
    fs::write(&path, b"{broken").expect("write malformed identity");
    let store = IdentityStore::new(path);

    // Act
    let result = store.load_or_create();

    // Assert
    assert!(matches!(result, Err(IdentityError::InvalidFile(_))));
}

#[test]
fn trusted_devices_can_be_added_renamed_and_removed_atomically() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let first = IdentityStore::new(root.path().join("first.json"))
        .load_or_create()
        .expect("first identity");
    let second = IdentityStore::new(root.path().join("second.json"))
        .load_or_create()
        .expect("second identity");
    let store = TrustedDeviceStore::new(root.path().join("trusted-devices.toml"));

    // Act
    store.trust(&first, "first device").expect("trust first");
    store.trust(&second, "second device").expect("trust second");
    assert!(matches!(
        store
            .check(&first.device_id(), &first.public_key())
            .expect("matching pin"),
        TrustStatus::Trusted(_)
    ));
    assert_eq!(
        store
            .check(&first.device_id(), &second.public_key())
            .expect("mismatched pin"),
        TrustStatus::KeyMismatch
    );
    store
        .rename(&first.device_id(), "renamed device")
        .expect("rename first");
    let removed = store.remove(&second.device_id()).expect("remove second");
    let devices = store.list().expect("list devices");

    // Assert
    assert!(removed);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "renamed device");
    assert_eq!(devices[0].public_key, first.public_key());
}

#[test]
fn trusted_store_rejects_a_public_key_that_does_not_match_its_device_id() {
    let root = tempdir().expect("temporary root");
    let identity = IdentityStore::new(root.path().join("identity.json"))
        .load_or_create()
        .expect("identity");
    let path = root.path().join("trusted-devices.toml");
    let store = TrustedDeviceStore::new(path.clone());
    store.trust(&identity, "device").expect("trust device");
    let original_key = hex::encode(identity.public_key());
    let tampered = fs::read_to_string(&path)
        .expect("trust store")
        .replace(&original_key, &"00".repeat(32));
    fs::write(path, tampered).expect("tamper trust store");

    assert!(store.list().is_err());
}

#[test]
fn legacy_migration_keeps_safe_last_directory_but_drops_shared_secret() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let loader = ConfigLoader::new(AppDirs::for_test(root.path()));
    let legacy = r#"{
        "last_dir": "/safe/previous",
        "peer": {"address": "192.0.2.10:8000", "secret": "must-not-migrate"}
    }"#;

    // Act
    let migration = loader
        .migrate_legacy_json(legacy)
        .expect("migrate legacy config");
    let serialized = toml::to_string(&migration).expect("serialize migrated config");

    // Assert
    assert_eq!(migration.receive.output.to_string_lossy(), "/safe/previous");
    assert!(!serialized.contains("secret"));
    assert!(!serialized.contains("192.0.2.10"));
}
