use quick_share_platform::{AppDirs, FileSensitivity, atomic_write};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_directories_are_isolated_under_the_supplied_root() {
    // Arrange
    let root = tempdir().expect("temporary root");

    // Act
    let dirs = AppDirs::for_test(root.path());

    // Assert
    assert_eq!(dirs.config_dir(), root.path().join("config"));
    assert_eq!(dirs.data_dir(), root.path().join("data"));
    assert_eq!(dirs.cache_dir(), root.path().join("cache"));
    assert_eq!(dirs.download_dir(), root.path().join("Downloads"));
}

#[test]
fn atomic_write_replaces_complete_content_without_leaving_temp_files() {
    // Arrange
    let root = tempdir().expect("temporary root");
    let path = root.path().join("config").join("config.toml");
    atomic_write(&path, b"old", FileSensitivity::Normal).expect("initial write");

    // Act
    atomic_write(&path, b"new-complete-value", FileSensitivity::Normal).expect("replace write");

    // Assert
    assert_eq!(fs::read(&path).expect("read result"), b"new-complete-value");
    let siblings: Vec<_> = fs::read_dir(path.parent().expect("parent"))
        .expect("read parent")
        .collect();
    assert_eq!(siblings.len(), 1);
}

#[test]
fn failed_atomic_replace_does_not_damage_existing_destination() {
    // Arrange: a non-empty directory cannot be replaced by the temporary file.
    let root = tempdir().expect("temporary root");
    let destination = root.path().join("config.toml");
    fs::create_dir(&destination).expect("destination directory");
    fs::write(destination.join("old-value"), b"preserved").expect("old value");

    // Act
    let result = atomic_write(&destination, b"new", FileSensitivity::Normal);

    // Assert
    assert!(result.is_err());
    assert_eq!(
        fs::read(destination.join("old-value")).expect("preserved old value"),
        b"preserved"
    );
}

#[cfg(windows)]
#[test]
fn private_atomic_write_applies_windows_acl() {
    // Arrange: an explicit path lets the true-host verification inspect the ACL afterwards.
    let root = tempdir().expect("temporary root");
    let path = std::env::var_os("QUICK_SHARE_ACL_PROBE_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.path().join("identity.key"));

    // Act
    atomic_write(&path, b"secret", FileSensitivity::Private).expect("private write");

    // Assert
    assert_eq!(fs::read(path).expect("private value"), b"secret");
}

#[cfg(unix)]
#[test]
fn private_atomic_write_sets_mode_0600() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange
    let root = tempdir().expect("temporary root");
    let path = root.path().join("identity.key");

    // Act
    atomic_write(&path, b"secret", FileSensitivity::Private).expect("private write");

    // Assert
    let mode = fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
