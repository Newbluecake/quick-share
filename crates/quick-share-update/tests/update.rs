use async_trait::async_trait;
use quick_share_update::{
    CHECKSUM_SIGNATURE_ASSET, CHECKSUMS_ASSET, Ed25519ReleaseVerifier, ExecutableReplacer,
    ReleaseAsset, ReleaseInfo, ReleaseProvider, ReleaseSignatureVerifier, UpdateCheck, UpdateError,
    UpdateService, asset_name_for_target, expected_sha256, is_allowed_redirect, target_for,
    validate_asset_url,
};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
enum DownloadMode {
    Valid,
    Corrupt,
    Interrupted,
}

struct FakeProvider {
    release: ReleaseInfo,
    payload: Vec<u8>,
    mode: DownloadMode,
    downloads: Mutex<Vec<String>>,
}

#[async_trait]
impl ReleaseProvider for FakeProvider {
    async fn release(&self, _requested: Option<&Version>) -> Result<ReleaseInfo, UpdateError> {
        Ok(self.release.clone())
    }

    async fn download(
        &self,
        asset: &ReleaseAsset,
        destination: &Path,
        _limit: u64,
        _cancellation: &CancellationToken,
    ) -> Result<(), UpdateError> {
        self.downloads
            .lock()
            .expect("downloads")
            .push(asset.name.clone());
        if asset.name == CHECKSUMS_ASSET {
            let digest = hex::encode(Sha256::digest(&self.payload));
            fs::write(
                destination,
                format!("{digest}  {}\n", self.release.binary.name),
            )?;
            return Ok(());
        }
        if asset.name == CHECKSUM_SIGNATURE_ASSET {
            fs::write(destination, [0_u8; 64])?;
            return Ok(());
        }
        match self.mode {
            DownloadMode::Valid => fs::write(destination, &self.payload)?,
            DownloadMode::Corrupt => fs::write(destination, b"corrupt")?,
            DownloadMode::Interrupted => {
                fs::write(destination, b"partial")?;
                return Err(UpdateError::DownloadInterrupted);
            }
        }
        Ok(())
    }
}

struct FakeVerifier {
    valid: bool,
}

impl ReleaseSignatureVerifier for FakeVerifier {
    fn verify(&self, _checksums: &[u8], _signature: &[u8]) -> Result<(), UpdateError> {
        if self.valid {
            Ok(())
        } else {
            Err(UpdateError::SignatureInvalid)
        }
    }
}

struct FakeReplacer {
    succeed: bool,
    calls: Mutex<usize>,
}

#[async_trait]
impl ExecutableReplacer for FakeReplacer {
    async fn replace(
        &self,
        current: &Path,
        candidate: &Path,
        _expected_version: &Version,
    ) -> Result<(), UpdateError> {
        *self.calls.lock().expect("calls") += 1;
        if !self.succeed {
            assert_eq!(fs::read(current).expect("old executable"), b"old");
            return Err(UpdateError::ReplacementFailed("injected".to_owned()));
        }
        fs::copy(candidate, current)?;
        Ok(())
    }
}

fn release(version: &str) -> ReleaseInfo {
    let version = Version::parse(version).expect("version");
    let tag = format!("v{version}");
    let binary_name = asset_name_for_target("x86_64-unknown-linux-gnu");
    ReleaseInfo {
        version,
        tag: tag.clone(),
        binary: ReleaseAsset {
            name: binary_name.clone(),
            download_url: format!(
                "https://github.com/Newbluecake/quick-share/releases/download/{tag}/{binary_name}"
            ),
            size: 7,
        },
        checksums: ReleaseAsset {
            name: CHECKSUMS_ASSET.to_owned(),
            download_url: format!(
                "https://github.com/Newbluecake/quick-share/releases/download/{tag}/{CHECKSUMS_ASSET}"
            ),
            size: 128,
        },
        signature: ReleaseAsset {
            name: CHECKSUM_SIGNATURE_ASSET.to_owned(),
            download_url: format!(
                "https://github.com/Newbluecake/quick-share/releases/download/{tag}/{CHECKSUM_SIGNATURE_ASSET}"
            ),
            size: 64,
        },
    }
}

fn service(
    root: &Path,
    version: &str,
    mode: DownloadMode,
    signature_valid: bool,
    replace_succeeds: bool,
) -> (
    UpdateService<FakeProvider, FakeVerifier, FakeReplacer>,
    Arc<FakeProvider>,
    Arc<FakeReplacer>,
) {
    let current = root.join(if cfg!(windows) {
        "quick-share.exe"
    } else {
        "quick-share"
    });
    fs::write(&current, b"old").expect("old executable");
    let provider = Arc::new(FakeProvider {
        release: release(version),
        payload: b"new-bin".to_vec(),
        mode,
        downloads: Mutex::new(Vec::new()),
    });
    let verifier = Arc::new(FakeVerifier {
        valid: signature_valid,
    });
    let replacer = Arc::new(FakeReplacer {
        succeed: replace_succeeds,
        calls: Mutex::new(0),
    });
    (
        UpdateService::new(
            Arc::clone(&provider),
            verifier,
            Arc::clone(&replacer),
            Version::parse("2.0.0-alpha.0").expect("current version"),
            current,
        ),
        provider,
        replacer,
    )
}

#[test]
fn release_asset_diagnostics_redact_download_urls() {
    let asset = ReleaseAsset {
        name: "quick-share-target".to_owned(),
        download_url: "https://release-assets.githubusercontent.com/object?token=secret".to_owned(),
        size: 7,
    };
    let diagnostic = format!("{asset:?}");
    assert!(!diagnostic.contains("token=secret"));
    assert!(!diagnostic.contains("release-assets.githubusercontent.com"));
}

#[test]
fn platform_asset_mapping_is_explicit_and_rejects_unknown_targets() {
    assert_eq!(
        target_for("linux", "x86_64", true),
        Some("x86_64-unknown-linux-musl")
    );
    assert_eq!(
        target_for("linux", "x86_64", false),
        Some("x86_64-unknown-linux-gnu")
    );
    assert_eq!(
        target_for("macos", "aarch64", false),
        Some("aarch64-apple-darwin")
    );
    assert_eq!(
        target_for("windows", "x86_64", false),
        Some("x86_64-pc-windows-msvc")
    );
    assert_eq!(target_for("plan9", "mips", false), None);
}

#[test]
fn checksum_manifest_is_strict_exact_and_duplicate_safe() {
    let digest = "11".repeat(32);
    let expected = expected_sha256(
        format!("{digest}  quick-share-target\n").as_bytes(),
        "quick-share-target",
    )
    .expect("checksum");
    assert_eq!(hex::encode(expected), digest);
    assert!(expected_sha256(b"not-a-checksum\n", "quick-share-target").is_err());
    assert!(
        expected_sha256(
            format!("{digest}  quick-share-target\n{digest}  quick-share-target\n").as_bytes(),
            "quick-share-target"
        )
        .is_err()
    );
}

#[test]
fn pinned_release_key_accepts_the_fixture_and_rejects_mutation() {
    let verifier = Ed25519ReleaseVerifier;
    let manifest = include_bytes!("fixtures/signed-manifest.txt");
    let signature = include_bytes!("fixtures/signed-manifest.sig");
    verifier
        .verify(manifest, signature)
        .expect("pinned release signature");
    let mut changed = manifest.to_vec();
    changed.push(b'!');
    assert!(verifier.verify(&changed, signature).is_err());
    assert!(verifier.verify(manifest, b"short").is_err());
}

#[test]
fn fixed_repository_and_redirect_policy_reject_downgrade_or_foreign_hosts() {
    let valid = "https://github.com/Newbluecake/quick-share/releases/download/v2.0.0/quick-share-x";
    assert!(validate_asset_url(valid, Some("v2.0.0")).is_ok());
    for invalid in [
        "http://github.com/Newbluecake/quick-share/releases/download/v2.0.0/quick-share-x",
        "https://github.com/attacker/quick-share/releases/download/v2.0.0/quick-share-x",
        "https://evil.example/Newbluecake/quick-share/releases/download/v2.0.0/quick-share-x",
        "https://github.com/Newbluecake/quick-share/releases/download/v1.0.0/quick-share-x",
    ] {
        assert!(validate_asset_url(invalid, Some("v2.0.0")).is_err());
    }
    let github = reqwest::Url::parse(valid).expect("GitHub URL");
    let assets = reqwest::Url::parse("https://release-assets.githubusercontent.com/object?sig=x")
        .expect("asset URL");
    let evil = reqwest::Url::parse("https://evil.example/object").expect("evil URL");
    let plaintext = reqwest::Url::parse("http://release-assets.githubusercontent.com/object")
        .expect("plaintext URL");
    assert!(is_allowed_redirect(&github, &assets));
    assert!(!is_allowed_redirect(&github, &evil));
    assert!(!is_allowed_redirect(&github, &plaintext));
}

#[tokio::test]
async fn semver_check_does_not_download_and_refuses_downgrade() {
    let root = tempdir().expect("root");
    let (up_to_date, provider, _) = service(
        root.path(),
        "2.0.0-alpha.0",
        DownloadMode::Valid,
        true,
        true,
    );
    assert!(matches!(
        up_to_date.check(None).await,
        Ok(UpdateCheck::UpToDate { .. })
    ));
    assert!(provider.downloads.lock().expect("downloads").is_empty());
    assert!(matches!(
        up_to_date
            .check(Some(&Version::parse("2.1.0").expect("requested")))
            .await,
        Err(UpdateError::UnexpectedVersion { .. })
    ));

    let (older, _, _) = service(root.path(), "1.9.1", DownloadMode::Valid, true, true);
    assert!(matches!(
        older
            .check(Some(&Version::parse("1.9.1").expect("old")))
            .await,
        Err(UpdateError::DowngradeRefused { .. })
    ));
}

#[tokio::test]
async fn valid_checksum_signature_and_replacement_install_once() {
    let root = tempdir().expect("root");
    let (service, provider, replacer) =
        service(root.path(), "2.0.0", DownloadMode::Valid, true, true);
    let UpdateCheck::Available(plan) = service.check(None).await.expect("check") else {
        panic!("update must be available");
    };
    let receipt = service
        .install(&plan, &CancellationToken::new())
        .await
        .expect("install");
    assert_eq!(receipt.installed, Version::parse("2.0.0").expect("version"));
    assert_eq!(provider.downloads.lock().expect("downloads").len(), 3);
    assert_eq!(*replacer.calls.lock().expect("calls"), 1);
    assert_no_candidates(root.path());
}

#[tokio::test]
async fn checksum_signature_interruption_and_replacement_fail_closed() {
    for (mode, signature, replacement, expected) in [
        (DownloadMode::Corrupt, true, true, "checksum"),
        (DownloadMode::Valid, false, true, "signature"),
        (DownloadMode::Interrupted, true, true, "interrupted"),
        (DownloadMode::Valid, true, false, "replacement"),
    ] {
        let root = tempdir().expect("root");
        let (service, _, replacer) = service(root.path(), "2.0.0", mode, signature, replacement);
        let UpdateCheck::Available(plan) = service.check(None).await.expect("check") else {
            panic!("update must be available");
        };
        let error = service
            .install(&plan, &CancellationToken::new())
            .await
            .expect_err("fault must fail");
        assert!(error.to_string().contains(expected), "{error}");
        let current = root.path().join(if cfg!(windows) {
            "quick-share.exe"
        } else {
            "quick-share"
        });
        assert_eq!(fs::read(current).expect("old"), b"old");
        if replacement {
            assert_eq!(*replacer.calls.lock().expect("calls"), 0);
        }
        assert_no_candidates(root.path());
    }
}

fn assert_no_candidates(root: &Path) {
    assert!(fs::read_dir(root).expect("root entries").all(|entry| {
        !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .starts_with(".quick-share-update-")
    }));
}
