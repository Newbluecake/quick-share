# Rust Rewrite Feasibility Spikes

These programs are disposable experiments for Batch 0 of the Rust rewrite.
They are **not production Quick Share code** and are not part of the final
application workspace.

## Rules

- Do not copy spike code into production crates without rewriting it under TDD.
- Do not treat a successful local run as cross-platform proof.
- Record exact commands, environment, failures, and unverified cases in
  `docs/dev/rust-rewrite/spikes/`.
- No spike may weaken certificate, identity, path, or integrity checks merely
  to make a demo pass.

## Packages

- `identity`: Noise XX identity/SAS/pinning experiment and attack checks.
- `discovery`: mDNS advertiser/scanner for two-machine testing.
- `web-tls`: temporary self-signed HTTPS page and QR experiment.
- `resume`: chunk journal, crash injection, resume, and atomic-finalize experiment.
- `clipboard`: native clipboard capability probe with safe fallback reporting.
- `windows-desktop`: Windows native dialog, tray, owner-window, and UI-thread probe.

Build all experiments:

```bash
cargo test --manifest-path spikes/Cargo.toml --workspace
```
