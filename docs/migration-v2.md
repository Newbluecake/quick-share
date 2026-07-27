# Migrating from Quick Share 1.x to 2.x

Quick Share 2 is a Rust rewrite and intentionally breaks the Python 1.x CLI and peer protocol. It is distributed as one native executable and does not require Python or pip.

## Command migration

| Quick Share 1.x | Quick Share 2.x | Notes |
|---|---|---|
| `quick-share file` | `quick-share serve file` or `quick-share send file` | `send` discovers encrypted receivers and falls back to Web only after a definitive empty scan; `serve` always starts browser mode |
| `quick-share file1 dir2` | `quick-share send file1 dir2` | Direct transfer supports files, directories, empty files, large files, and safe symlink metadata |
| `quick-share --upload DIR` | `quick-share serve --upload --output DIR` | HTTPS by default; optional password via `QUICK_SHARE_UPLOAD_PASSWORD` or `--upload-password` |
| `quick-share --serve` | `quick-share receive` | The old shared-secret HTTP peer protocol is replaced by authenticated Noise XX |
| `quick-share config --peer ... --secret ...` | `quick-share devices ...` and `--peer host:port` | Shared secrets are not migrated into device trust |
| `quick-share update --check` | `quick-share update --check` | v2 verifies a pinned Ed25519 release signature and rejects foreign redirects |
| n/a | `quick-share send --text ...` / `--clipboard` | New bounded text and clipboard transfer |
| n/a | `sc` / `rc` | Created only when the names do not conflict with existing commands |

## Configuration migration

The v1 file `~/.quick-share/config.json` is not used as the v2 configuration source. On first start, v2 may migrate only a semantically safe previous output directory. It deliberately drops:

- the old HTTP peer shared secret;
- peer host/IP identity assumptions;
- settings that would weaken HTTPS, confirmation, trust, or path policies.

Use `quick-share config path` to locate the platform-standard v2 TOML file and `quick-share config show` to inspect effective non-secret settings. Device identity, trust records, resumable journals, and ordinary preferences are stored separately.

A v1 shared secret **cannot** become a trusted v2 device. Establish trust by completing a Noise XX handshake and explicitly comparing the six-digit SAS.

## Behavioral changes

- Web mode defaults to a temporary self-signed HTTPS certificate. HTTP requires `--allow-http` and prints a warning.
- Unknown non-interactive device offers are rejected unless `--yes` is explicit; `--yes` never creates durable trust.
- Automatic Web fallback is intentionally narrow. Discovery errors, partial scans, rejection, timeout, and direct-transfer failure return errors instead of changing transport.
- Received files use staging, BLAKE3 verification, bounded resume journals, and atomic final commit. Direct sends print a transfer UUID; after a sender/receiver process restart, rerun the exact content and target with `--resume UUID`.
- Symbolic links are preserved as metadata by default. Following targets requires `--follow-links`.
- Text is never executed or automatically opened. Explicit output files are no-clobber.
- Existing `sc` and `rc` commands are never replaced by the installer.
- Windows PowerShell/external-command output is normalized from UTF-8, UTF-16LE, or legacy GBK into UTF-8. Transferred file/text payload bytes are not transcoded.

## Python-to-Rust coverage matrix

The 1.9.1 suite contained 339 Python tests. Before removing that runtime, the corresponding product behavior was represented by Rust, installer, browser, and release tests:

| v1 behavior | v2 implementation and evidence |
|---|---|
| CLI parsing, limits, version, config | `quick-share-cli/tests/{cli,contract,release_contract}.rs`, `quick-share-core/tests/config_identity.rs` |
| single/multi-file and directory listing/download | `quick-share-web/tests/catalog_download.rs` |
| directory ZIP, Unicode, empty directories | `quick-share-web/tests/ui_zip_tls.rs` |
| browser upload, password, conflict rename, limits | `quick-share-web/tests/upload.rs` |
| path traversal and symlink escape | core manifest/path tests plus Web post-catalog TOCTOU tests |
| download count and timeout shutdown | `quick-share-web/tests/server_lifecycle.rs` |
| live download/upload progress and interruption | bounded Web progress channel tests, ZIP/body-drop cancellation tests, production CLI E2E |
| network/IP filtering | `quick-share-discovery/tests/discovery.rs` and platform network tests |
| peer authentication and remote transfer | Noise/auth/offer/direct/receiver/sender integration tests |
| large-file streaming and interruption | transfer store/resume tests, >4 GiB sparse test, Web streaming-body test |
| update check, corruption, rollback | `quick-share-update/tests/update.rs` |
| Shell and PowerShell installation | `tests/install/test_install_sh.bats`, `tests/install/test_install_ps1.Tests.ps1` |
| standalone release executable | release matrix, SBOM/checksum/signature jobs, Unix/Windows loopback smoke scripts |

Detailed batch evidence is under `docs/dev/rust-rewrite/Batch-2-review.md` through `Batch-8-review.md`.

## Rollback to 1.9.1

Before upgrading, save any v1 configuration you may want for reference:

```bash
cp -a ~/.quick-share ~/.quick-share-v1-backup
```

To roll back:

1. Stop all Quick Share 2 send/receive/Web processes.
2. Remove or rename the v2 executable from its installation directory.
3. Download the v1.9.1 asset from the immutable GitHub release page, or check out tag `v1.9.1` and use its documented Python installation method.
4. Restore `~/.quick-share-v1-backup` only for the v1 application.

Do not copy v2 identity/trust/journal files into v1, and do not convert a v1 shared secret into a v2 trusted key. Files already received and committed are ordinary user files and do not require conversion.

Rollback from an updater failure is automatic when replacement or startup checking fails. If the operating system prevents automatic cleanup, the error reports the retained runnable backup path.
