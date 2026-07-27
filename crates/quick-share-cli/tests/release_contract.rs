use quick_share_cli::{IntentCommand, parse_intent_from};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn cargo_workspace_is_the_only_runtime_and_version_source() {
    let root = root();
    for removed in [
        "src",
        "setup.py",
        "pyproject.toml",
        "requirements.txt",
        "requirements-dev.txt",
    ] {
        assert!(
            !root.join(removed).exists(),
            "legacy runtime remains: {removed}"
        );
    }
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("workspace Cargo.toml");
    assert!(cargo.contains("[workspace.package]"));
    let metadata: toml::Value = toml::from_str(&cargo).expect("workspace TOML");
    let version = metadata["workspace"]["package"]["version"]
        .as_str()
        .expect("workspace version");
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    let workflow =
        fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release workflow");
    let lowercase = workflow.to_ascii_lowercase();
    for forbidden in ["setup-python", "pyinstaller", "pip install", "src/main.py"] {
        assert!(
            !lowercase.contains(forbidden),
            "release contains {forbidden}"
        );
    }
}

#[test]
fn release_pipeline_contains_all_assets_integrity_and_smoke_gates() {
    let root = root();
    let release =
        fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release workflow");
    for required in [
        "x86_64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "SHA256SUMS",
        "SHA256SUMS.sig",
        "RELEASE_SIGNING_KEY_PEM",
        "actions/attest-build-provenance@",
        "anchore/sbom-action@",
        "smoke-unix.sh",
        "smoke-windows.ps1",
        "check-signing-key.sh",
        "cargo-deny-action@",
    ] {
        assert!(release.contains(required), "release is missing {required}");
    }
    let release_lines: Vec<_> = release.lines().map(str::trim).collect();
    assert!(
        release_lines.contains(&"path: release-assets")
            && release_lines.contains(&"upload-release-assets: false")
            && !release_lines.contains(&"path: release-assets/${{ matrix.asset }}"),
        "SBOM action must scan the staged directory rather than treat one executable as a directory"
    );
    for line in release
        .lines()
        .filter(|line| line.trim_start().starts_with("uses:"))
    {
        let revision = line
            .split_once('@')
            .map(|(_, value)| value.split_whitespace().next().unwrap_or_default())
            .expect("action revision");
        assert_eq!(revision.len(), 40, "action is not commit-pinned: {line}");
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    assert!(root.join("security/release-signing-key.pem").is_file());
}

#[test]
fn fuzz_smoke_forces_the_pinned_nightly_over_the_workspace_toolchain() {
    let ci = fs::read_to_string(root().join(".github/workflows/ci.yml")).expect("CI workflow");
    assert!(ci.contains("RUSTUP_TOOLCHAIN: nightly-2026-07-01"));
}

#[test]
fn readme_commands_match_the_rust_cli_contract() {
    let readme = fs::read_to_string(root().join("README.md")).expect("README");
    for required in [
        "quick-share send",
        "quick-share receive",
        "quick-share serve",
        "quick-share devices",
        "quick-share update --check",
        "install.sh",
        "install.ps1",
    ] {
        assert!(readme.contains(required), "README is missing {required}");
    }
    for arguments in [
        vec!["quick-share", "send", "file.txt"],
        vec!["quick-share", "receive", "--once"],
        vec!["quick-share", "serve", "file.txt"],
        vec!["quick-share", "devices", "list"],
        vec!["quick-share", "update", "--check"],
    ] {
        parse_intent_from(arguments).expect("documented command must parse");
    }
    let shortcut = parse_intent_from(["sc", "file.txt"]).expect("sc command");
    assert!(matches!(shortcut.command, IntentCommand::Send(_)));
}
