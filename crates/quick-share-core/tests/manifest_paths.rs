use proptest::prelude::*;
use quick_share_core::{
    config::ConflictPolicy,
    manifest::{
        ManifestBuilder, ManifestEntryKind, ManifestError, SymlinkCommitOutcome,
        SymlinkDisposition, commit_symlink,
    },
    paths::{PathError, RelativePath, resolve_destination},
};
use std::fs;
use tempfile::tempdir;

#[test]
fn relative_path_rejects_cross_platform_escape_and_reserved_names() {
    for invalid in [
        "",
        ".",
        "..",
        "a/../b",
        "/etc/passwd",
        r"C:\Windows\file",
        r"\\server\share",
        "a//b",
        "NUL",
        "con.txt",
        "LPT9.log",
        "COM¹.txt",
        "trailing. ",
        "bad:name",
        "zero\0byte",
    ] {
        assert!(
            RelativePath::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert_eq!(
        RelativePath::parse("目录/hello world.txt")
            .expect("safe path")
            .as_str(),
        "目录/hello world.txt"
    );
}

proptest! {
    #[test]
    fn every_accepted_relative_path_resolves_inside_root(candidate in ".{0,200}") {
        let root = std::path::Path::new("safe-root");
        if let Ok(relative) = RelativePath::parse(&candidate) {
            let resolved = relative.resolve_under(root);
            prop_assert!(resolved.starts_with(root));
            prop_assert!(!resolved.components().any(|part| matches!(part, std::path::Component::ParentDir)));
        }
    }
}

#[test]
fn destination_conflict_policies_are_explicit_and_rename_preserves_extension() {
    let root = tempdir().expect("temporary root");
    fs::write(root.path().join("report.txt"), b"old").expect("existing file");
    let relative = RelativePath::parse("report.txt").expect("relative path");

    let renamed = resolve_destination(root.path(), &relative, ConflictPolicy::Rename)
        .expect("renamed destination")
        .expect("destination");
    assert_eq!(renamed.file_name().expect("file name"), "report (1).txt");
    assert!(
        resolve_destination(root.path(), &relative, ConflictPolicy::Skip)
            .expect("skip")
            .is_none()
    );
    assert!(matches!(
        resolve_destination(root.path(), &relative, ConflictPolicy::Error),
        Err(PathError::DestinationExists(_))
    ));
}

#[cfg(unix)]
#[test]
fn destination_resolution_rejects_existing_symlink_ancestors() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("receive root");
    let outside = tempdir().expect("outside root");
    symlink(outside.path(), root.path().join("redirect")).expect("redirecting ancestor");
    let relative = RelativePath::parse("redirect/escaped.txt").expect("portable path");

    let result = resolve_destination(root.path(), &relative, ConflictPolicy::Overwrite);

    assert!(matches!(result, Err(PathError::SymlinkAncestor(_))));
    assert!(!outside.path().join("escaped.txt").exists());
}

#[test]
fn manifest_contains_files_empty_directories_unicode_and_unique_top_names() {
    let root = tempdir().expect("temporary root");
    let first = root.path().join("first").join("same");
    let second = root.path().join("second").join("same");
    fs::create_dir_all(first.join("空目录")).expect("first tree");
    fs::create_dir_all(&second).expect("second tree");
    fs::write(first.join("你好.txt"), b"hello").expect("unicode file");
    fs::write(second.join("empty.bin"), b"").expect("empty file");

    let manifest = ManifestBuilder::new()
        .build(&[first.clone(), second])
        .expect("build manifest");

    assert!(manifest.entries.iter().any(|entry| {
        entry.relative_path.as_str() == "same/空目录"
            && matches!(entry.kind, ManifestEntryKind::Directory)
    }));
    assert!(manifest.entries.iter().any(|entry| {
        entry.relative_path.as_str() == "same/你好.txt"
            && matches!(entry.kind, ManifestEntryKind::File)
    }));
    assert!(
        manifest.entries.iter().any(|entry| {
            entry.relative_path.as_str() == "same (1)/empty.bin" && entry.size == 0
        })
    );
    let unicode_file = manifest
        .entries
        .iter()
        .find(|entry| entry.relative_path.as_str() == "same/你好.txt")
        .expect("Unicode file entry");
    assert!(unicode_file.source_is_unchanged().expect("snapshot check"));
    fs::write(first.join("你好.txt"), b"changed and larger").expect("mutate source");
    assert!(
        !unicode_file
            .source_is_unchanged()
            .expect("changed snapshot check")
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn manifest_renames_case_collisions_for_windows_portability() {
    let root = tempdir().expect("temporary root");
    let tree = root.path().join("tree");
    fs::create_dir(&tree).expect("tree");
    fs::write(tree.join("Report.txt"), b"one").expect("first case");
    fs::write(tree.join("report.txt"), b"two").expect("second case");

    let manifest = ManifestBuilder::new()
        .build(std::slice::from_ref(&tree))
        .expect("portable manifest");
    let names: Vec<_> = manifest
        .entries
        .iter()
        .map(|entry| entry.relative_path.as_str())
        .collect();

    assert!(names.contains(&"tree/Report.txt"));
    assert!(names.contains(&"tree/report (1).txt"));
}

#[cfg(unix)]
#[test]
fn symlinks_are_metadata_by_default_and_follow_links_detects_cycles() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("temporary root");
    let tree = root.path().join("tree");
    fs::create_dir(&tree).expect("tree");
    fs::write(tree.join("target.txt"), b"target").expect("target");
    symlink("target.txt", tree.join("link.txt")).expect("file link");
    symlink("missing.txt", tree.join("broken.txt")).expect("broken link");

    let manifest = ManifestBuilder::new()
        .build(std::slice::from_ref(&tree))
        .expect("manifest");
    assert!(manifest.entries.iter().any(|entry| matches!(
        &entry.kind,
        ManifestEntryKind::Symlink { target } if target == "target.txt"
    )));
    assert!(manifest.entries.iter().any(|entry| matches!(
        &entry.kind,
        ManifestEntryKind::Symlink { target } if target == "missing.txt"
    )));

    fs::remove_file(tree.join("broken.txt")).expect("remove broken link before follow test");
    symlink(".", tree.join("cycle")).expect("cycle link");
    let followed = ManifestBuilder::new()
        .follow_links(true)
        .build(std::slice::from_ref(&tree));
    assert!(matches!(followed, Err(ManifestError::SymlinkCycle(_))));
}

#[test]
fn unsafe_or_unsupported_symlink_targets_degrade_safely() {
    let link = RelativePath::parse("folder/link").expect("link path");
    assert_eq!(
        SymlinkDisposition::classify(&link, "../safe.txt", true),
        SymlinkDisposition::Create
    );
    assert!(matches!(
        SymlinkDisposition::classify(&link, "../../escape.txt", true),
        SymlinkDisposition::NeedsConfirmation { .. }
    ));
    assert!(matches!(
        SymlinkDisposition::classify(&link, r"C:\outside", true),
        SymlinkDisposition::NeedsConfirmation { .. }
    ));
    assert!(matches!(
        SymlinkDisposition::classify(&link, "target.txt", false),
        SymlinkDisposition::SaveAsNotice { .. }
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn symlink_commit_creates_a_link_or_a_safe_notice() {
    let root = tempdir().expect("temporary root");
    fs::write(root.path().join("target.txt"), b"target").expect("target");
    let link = RelativePath::parse("link.txt").expect("link path");

    let outcome = commit_symlink(root.path(), &link, "target.txt", false, false)
        .expect("link commit or safe fallback");

    println!("symlink outcome: {outcome:?}");
    match outcome {
        SymlinkCommitOutcome::Created(path) => {
            assert!(
                fs::symlink_metadata(path)
                    .expect("link metadata")
                    .file_type()
                    .is_symlink()
            );
        }
        SymlinkCommitOutcome::NoticeSaved(path) => {
            let notice = fs::read_to_string(path).expect("fallback notice");
            assert!(notice.contains("target.txt"));
            assert!(notice.contains("did not create"));
        }
    }
}

#[test]
fn manifest_entry_limit_is_enforced() {
    let root = tempdir().expect("temporary root");
    let tree = root.path().join("many");
    fs::create_dir(&tree).expect("tree");
    for index in 0..5 {
        fs::write(tree.join(format!("{index}.txt")), b"x").expect("entry");
    }

    let result = ManifestBuilder::new()
        .max_entries(5)
        .build(std::slice::from_ref(&tree));

    // The root directory plus five files exceeds the configured five-entry limit.
    assert!(matches!(
        result,
        Err(ManifestError::TooManyEntries { limit: 5 })
    ));
}
