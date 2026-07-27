#!/usr/bin/env bash
set -Eeuo pipefail

toolchain_bin="$(dirname "$(rustup which --toolchain 1.92.0 rustc)")"
PATH="$toolchain_bin:$PATH"

echo "Checking Quick Share Rust workspace..."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features --locked

echo "Building the single release executable..."
cargo build --workspace --release --locked

binary="target/release/quick-share"
if [[ "${OS:-}" == "Windows_NT" ]]; then binary="${binary}.exe"; fi
"$binary" --version
"$binary" --help >/dev/null
echo "Build complete: $binary"
