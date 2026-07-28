# Quick Share

[简体中文](README.md) | **English**

[![CI](https://github.com/Newbluecake/quick-share/actions/workflows/ci.yml/badge.svg)](https://github.com/Newbluecake/quick-share/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Quick Share is a secure, single-binary LAN sharing CLI for Linux, macOS, and Windows. It discovers nearby receivers and transfers files, directories, text, or clipboard content over authenticated Noise XX encryption. If a successful scan finds no compatible receiver, it can start a browser-oriented HTTPS share instead.

Quick Share 2.0 is the production Rust implementation. It replaces the Python 1.x runtime and peer protocol; see the [migration guide](docs/migration-v2.md) before upgrading.

## Install

No Python, pip, Node.js, or other language runtime is required.

### Linux and macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Newbluecake/quick-share/master/install.sh | bash
```

The installer downloads one executable from the fixed GitHub repository, verifies `SHA256SUMS` plus the pinned Ed25519 release signature when OpenSSL supports it, installs to `~/.local/bin`, and creates `sc`/`rc` only when those names are free.

To inspect the installer before running it:

```bash
curl -fsSLO https://raw.githubusercontent.com/Newbluecake/quick-share/master/install.sh
less install.sh
bash install.sh
```

### Windows PowerShell

```powershell
iwr -useb https://raw.githubusercontent.com/Newbluecake/quick-share/master/install.ps1 | iex
```

The default destination is `%LOCALAPPDATA%\QuickShare\bin`. Existing `sc` or `rc` commands are never overwritten. A Private-profile, program-scoped inbound firewall rule is added only when the installer is explicitly run with `-AddPrivateFirewallRule`.

Prebuilt assets, `SHA256SUMS`, `SHA256SUMS.sig`, SBOMs, and GitHub build provenance are published on [GitHub Releases](https://github.com/Newbluecake/quick-share/releases).

## Quick start

On the receiving computer:

```bash
quick-share receive
```

On the sending computer:

```bash
quick-share send report.pdf photos/
# shortcut:
sc report.pdf photos/
```

The first unknown connection displays a six-digit SAS. Compare it on both terminals before choosing “accept and trust”. `--yes` permits one-time TOFU only; it never creates durable trust.

## Commands

### Send files and directories

```bash
quick-share send file.txt directory/
quick-share send --peer 192.168.1.20:4242 file.txt
quick-share send --resume 018f47b4-5b3a-7c9d-8123-0123456789ab --peer 192.168.1.20:4242 file.txt
quick-share send --web file.txt
quick-share send --follow-links symlink
```

Automatic Web fallback occurs **only** when discovery succeeds and finds zero compatible receivers. Discovery errors, partial scans, rejection, timeout, identity change, connection failure, and transfer failure do not fall back to Web.

### Send text or clipboard content

```bash
quick-share send --text 'hello from Quick Share'
quick-share send --clipboard
```

Received text is never executed or opened. Without an explicit output file, the receiver tries the native clipboard and safely falls back to stdout.

### Receive

```bash
quick-share receive
quick-share receive --output ~/Downloads/received
quick-share receive --once --yes
```

Unknown non-interactive offers are rejected unless `--yes` is supplied, and `--yes` can accept only once. The first Ctrl+C persists resumable state; a second interrupt forces termination.

### Cross-device remote selection (Windows desktop)

The first-phase native picker is provided only on Windows. Start the resident agent on Windows; it runs in the system tray and shows native windows when a request arrives:

```powershell
quick-share agent
quick-share agent --bind 192.168.1.20 --port 4242
```

- The agent requires an interactive Windows desktop session; without an available desktop, requests fail closed rather than silently continuing.
- The tray menu offers "Open Quick Share" and "Exit"; closing the hidden owner window does not stop the agent.
- Paired devices go straight to selection; unpaired devices first get an authorization window (device name, stable ID, and verification code), and devices whose identity changed are rejected by default.

From Linux, request that Windows choose the content to send and receive the callback transfer securely:

```bash
quick-share receive --request
rc
quick-share receive --request --peer 192.168.1.20:4242
quick-share receive --request --peer 192.168.1.20:4242 --output ~/Downloads/received
```

Without `--output`, an active remote-selection request saves into the command's current working directory; `--output` overrides it. `rc` is the shortcut for `quick-share receive --request`. Use the full `quick-share receive` command for ordinary passive receive; it and the Windows agent receiving a peer's `send` continue to use the configured Downloads directory.

- Auto-discovery lists only agents that negotiated remote selection; non-interactive use must name the target with `--peer`.
- Windows defaults to that device's last saved directory and offers "Change directory"; the directory is remembered only for a fully trusted identity.
- On a destination conflict, Windows prompts Overwrite / Skip / Rename and This entry / All remaining.
- The callback dials only the source IP observed on the authenticated control connection with the pinned complete remote key, and exactly matches the request and transfer identifiers.

### Traditional browser sharing

```bash
quick-share serve file.txt directory/
quick-share serve --upload --output ~/Downloads/received
QUICK_SHARE_UPLOAD_PASSWORD='choose-a-password' \
  quick-share serve --upload --output ~/Downloads/received
```

Web mode uses a temporary self-signed HTTPS certificate by default. The terminal prints the actual listener, token URL, certificate fingerprint, QR code, expiration, and download limit. Browsers will warn about the temporary certificate; do not install it as a CA.

Plaintext HTTP requires an explicit opt-in:

```bash
quick-share serve --allow-http file.txt
```

### Trusted devices

```bash
quick-share devices list
quick-share devices list --json
quick-share devices rename <DEVICE_ID> laptop
quick-share devices remove <DEVICE_ID>
```

Trust is pinned to the complete authenticated static public key, not an IP address, device name, mDNS record, or short SAS.

### Configuration

```bash
quick-share config show
quick-share config path
quick-share config set device.name workstation
quick-share config set receive.output ~/Downloads/received
quick-share config set discovery.peers 192.168.1.20:4242
# Separate multiple static peers with commas; an empty string clears the list:
quick-share config set discovery.peers 'host-a:4242,host-b:4242'
```

Configuration precedence is command line, environment, TOML file, then built-in defaults. Identity and trust state are stored separately with private permissions.

The `host:port` entries in `discovery.peers` are probed in parallel with mDNS, which is useful when a Public-profile firewall or routed network blocks multicast discovery. Probes perform a complete Noise/QSP handshake, add only reachable protocol-compatible devices, and de-duplicate them against mDNS by authenticated identity; they neither establish trust nor bypass the later identity confirmation.

### Signed self-update

```bash
quick-share update --check
quick-share update
quick-share update --yes
quick-share update --version 2.0.0
```

Updates are fetched only from `Newbluecake/quick-share`, bounded while streaming, verified against the signed SHA-256 manifest and pinned Ed25519 release key, startup-checked, and replaced with rollback protection. Redirects to plaintext or foreign hosts are rejected.

## Security model

- Direct transfers use fixed `Noise_XX_25519_ChaChaPoly_BLAKE2s` with full static-key pinning.
- Unknown peers may submit only a bounded offer before approval; transfer operations require short-lived, peer-bound authorization.
- Files use BLAKE3 chunk and final integrity verification, staging, durable journals, and atomic commit.
- Web mode uses random 128-bit access tokens, path-independent catalog IDs, canonical containment checks, strict limits, no-store/no-referrer headers, and default HTTPS.
- Product crates forbid unsafe Rust. Dependencies are checked with RustSec and protocol framing is fuzzed in CI.
- The release updater pins an Ed25519 public key in the binary. The private key exists only as the protected `RELEASE_SIGNING_KEY_PEM` GitHub Actions secret.

See [`SECURITY.md`](SECURITY.md) for reporting issues and [`docs/migration-v2.md`](docs/migration-v2.md) for v1 migration and rollback guidance.

## Build and test

Requirements:

- Rust 1.92.0, as pinned by `rust-toolchain.toml`
- platform C toolchain required by Rust dependencies

```bash
git clone https://github.com/Newbluecake/quick-share.git
cd quick-share
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --release --locked
```

Or run:

```bash
./build.sh
```

The only product executable is `target/release/quick-share`. The installer creates aliases; Cargo does not build separate `sc` or `rc` programs.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for development and review requirements.

## Platform notes

- Discovery is link-local mDNS. For routed or multicast-restricted networks, use `--peer host:port`.
- Current macOS artifacts are unsigned and not notarized because the project has no Apple Developer account; Gatekeeper may require an explicit user override. This does not weaken the Ed25519 release-manifest verification.
- Quick Share never silently changes firewall or network-profile settings.
- Windows Public-profile inbound rules may block connections; diagnostics provide guidance without modifying the system.
- Every direct send prints a transfer UUID. After an interrupted sender or receiver process, rerun the same content and target with `--resume UUID`; the receiver accepts only the exact authenticated sender and immutable manifest, then requests missing chunks.
- Windows subprocess output is normalized from UTF-8, UTF-16LE, or legacy GBK into internal UTF-8; PowerShell scripts explicitly select UTF-8. File contents and redirected transfer payloads are never transcoded.
- Headless Linux clipboard access safely falls back to stdout/file output.
- Symbolic links are transferred as links by default; `--follow-links` must be explicit.

## License

MIT
