# Contributing to Quick Share

Quick Share is a security-sensitive Rust workspace. Changes to identity, Noise framing, path handling, Web upload, updater, installers, or release workflows require tests and an explicit security review.

## Development setup

Install Rust 1.92.0 with rustfmt and Clippy. The repository pins toolchains in `rust-toolchain.toml`.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo audit
```

The product has one binary target: `quick-share`. The workspace crates separate CLI, core, protocol, discovery, transfer, Web, platform, and update responsibilities.

## Test expectations

Use Red → Green → Refactor:

- unit tests for parsing and state transitions;
- integration tests for module boundaries and fault paths;
- real-process tests for transport, shutdown, installer, and release behavior;
- bounded fuzzing for untrusted protocol framing.

Tests must cover failure behavior, not only successful paths. Never weaken production validation to make a test pass.

Shell installer changes require Bats coverage. PowerShell installer changes require Pester coverage. Release workflow changes must pass `actionlint` and preserve checksum, Ed25519 signature, SBOM, provenance, and released-binary smoke gates.

## Security rules

- Do not invent cryptographic primitives or make cipher suites runtime-configurable.
- Do not log private keys, full static public keys, bearer tokens, upload passwords, or text bodies.
- Do not use device names, IP addresses, short fingerprints, or SAS as durable identity.
- Do not turn discovery failure into automatic Web fallback.
- Do not silently enable HTTP, follow symlinks, overwrite destinations, alter firewalls, or trust unknown devices.
- Keep all untrusted frames, paths, manifests, uploads, downloads, ZIP streams, and update assets bounded.
- Product crates keep `#![forbid(unsafe_code)]`.

Report security issues through the private process in `SECURITY.md`.

## Versioning and release

`Cargo.toml` under `[workspace.package]` is the only version source. Crates inherit it with `version.workspace = true`.

A release tag triggers `.github/workflows/release.yml`, which builds native binaries, runs loopback smoke tests, emits SBOMs and checksums, signs `SHA256SUMS` using the protected Actions secret, publishes GitHub provenance, and creates the release.

Never place release private keys in source, workflow YAML, logs, artifacts, documentation, or local project memory. Public release trust is recorded in `security/release-signing-key.pem`.

## Pull requests

Keep changes focused and describe:

- user-visible behavior;
- security boundaries affected;
- tests and platforms exercised;
- migration or rollback considerations;
- any remaining release gate.

Use Conventional Commits where practical. Do not tag or publish a release without the final acceptance approval described in `docs/dev/rust-rewrite/rust-rewrite-tasks.md`.
