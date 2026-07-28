#!/usr/bin/env bats

setup() {
    export REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    export INSTALL_SCRIPT="$REPO_ROOT/install.sh"
    export TEST_DIR="$(mktemp -d)"
    # shellcheck disable=SC1090
    source "$INSTALL_SCRIPT"
}

teardown() {
    rm -rf "$TEST_DIR"
}

@test "platform mapping uses published Rust target triples" {
    [ "$(resolve_target linux x86_64)" = "x86_64-unknown-linux-musl" ]
    [ "$(resolve_target macos x86_64)" = "x86_64-apple-darwin" ]
    [ "$(resolve_target macos aarch64)" = "aarch64-apple-darwin" ]
    run resolve_target linux aarch64
    [ "$status" -ne 0 ]
}

@test "release URLs stay in the fixed repository and validate versions" {
    [ "$(release_download_root latest)" = "https://github.com/Newbluecake/quick-share/releases/latest/download" ]
    [ "$(release_download_root 2.0.0)" = "https://github.com/Newbluecake/quick-share/releases/download/v2.0.0" ]
    run release_download_root '../../evil'
    [ "$status" -ne 0 ]
}

@test "checksum parsing requires exactly one matching asset" {
    printf 'payload' > "$TEST_DIR/candidate"
    digest="$(sha256_file "$TEST_DIR/candidate")"
    printf '%s  quick-share-target\n' "$digest" > "$TEST_DIR/SHA256SUMS"
    verify_checksum "$TEST_DIR/candidate" "$TEST_DIR/SHA256SUMS" quick-share-target

    printf '%s  quick-share-target\n%s  quick-share-target\n' "$digest" "$digest" > "$TEST_DIR/duplicate"
    run verify_checksum "$TEST_DIR/candidate" "$TEST_DIR/duplicate" quick-share-target
    [ "$status" -ne 0 ]

    printf 'changed' > "$TEST_DIR/candidate"
    run verify_checksum "$TEST_DIR/candidate" "$TEST_DIR/SHA256SUMS" quick-share-target
    [ "$status" -ne 0 ]
}

@test "pinned Ed25519 signature accepts authentic manifest and rejects mutation" {
    cp "$REPO_ROOT/crates/quick-share-update/tests/fixtures/signed-manifest.txt" "$TEST_DIR/manifest"
    cp "$REPO_ROOT/crates/quick-share-update/tests/fixtures/signed-manifest.sig" "$TEST_DIR/signature"
    printf candidate > "$TEST_DIR/candidate"
    verify_signature_if_available "$TEST_DIR/manifest" "$TEST_DIR/signature" "$TEST_DIR/candidate" "$TEST_DIR"
    printf x >> "$TEST_DIR/manifest"
    run verify_signature_if_available "$TEST_DIR/manifest" "$TEST_DIR/signature" "$TEST_DIR/candidate" "$TEST_DIR"
    [ "$status" -ne 0 ]
}

@test "existing sc and rc paths are never overwritten" {
    mkdir -p "$TEST_DIR/bin"
    printf 'existing-sc' > "$TEST_DIR/bin/sc"
    printf 'existing-rc' > "$TEST_DIR/bin/rc"
    create_aliases "$TEST_DIR/bin"
    [ "$(cat "$TEST_DIR/bin/sc")" = "existing-sc" ]
    [ "$(cat "$TEST_DIR/bin/rc")" = "existing-rc" ]
}

@test "failed candidate startup preserves an existing installation" {
    mkdir -p "$TEST_DIR/bin"
    printf '#!/usr/bin/env bash\necho old\n' > "$TEST_DIR/bin/quick-share"
    chmod +x "$TEST_DIR/bin/quick-share"
    printf 'not executable content' > "$TEST_DIR/candidate"
    run install_verified_binary "$TEST_DIR/candidate" "$TEST_DIR/bin"
    [ "$status" -ne 0 ]
    grep -q 'echo old' "$TEST_DIR/bin/quick-share"
    ! find "$TEST_DIR/bin" -name '.quick-share.new.*' | grep -q .
}

@test "successful main installation cleans temporary state and exits zero" {
    run bash -c '
        set -Eeuo pipefail
        source "$1"
        download_file() {
            local destination="$2"
            case "$destination" in
                */quick-share-*)
                    printf "#!/usr/bin/env bash\necho quick-share 9.9.9\n" > "$destination"
                    chmod +x "$destination"
                    ;;
                *) printf placeholder > "$destination" ;;
            esac
        }
        verify_signature_if_available() { :; }
        verify_checksum() { :; }
        main --version 9.9.9 --install-dir "$2" --no-aliases
        "$2/quick-share" --version
    ' _ "$INSTALL_SCRIPT" "$TEST_DIR/bin"
    [ "$status" -eq 0 ]
    [[ "$output" == *"quick-share 9.9.9"* ]]
}

@test "installer has no Python or pip runtime dependency" {
    ! grep -Eqi 'python|pip install|pyinstaller' "$INSTALL_SCRIPT"
}

@test "installer is executable and uses strict Bash mode" {
    [ -x "$INSTALL_SCRIPT" ]
    head -n 1 "$INSTALL_SCRIPT" | grep -q '^#!/usr/bin/env bash$'
    grep -q 'set -Eeuo pipefail' "$INSTALL_SCRIPT"
}
