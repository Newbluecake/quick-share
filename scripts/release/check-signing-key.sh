#!/usr/bin/env bash
set -Eeuo pipefail

public_key="security/release-signing-key.pem"
der_base64="$(openssl pkey -pubin -in "$public_key" -outform DER | openssl base64 -A)"
raw_hex="$(openssl pkey -pubin -in "$public_key" -outform DER | tail -c 32 | xxd -p -c 64)"

grep -Fq "$der_base64" install.sh
grep -Fq "$der_base64" install.ps1
grep -Fq "$raw_hex" crates/quick-share-update/src/signature.rs
openssl pkeyutl -verify -rawin -pubin -inkey "$public_key" \
    -in crates/quick-share-update/tests/fixtures/signed-manifest.txt \
    -sigfile crates/quick-share-update/tests/fixtures/signed-manifest.sig \
    >/dev/null

echo "Pinned release key consistency PASS"
