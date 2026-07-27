#!/usr/bin/env bash
# Install the signed Rust Quick Share executable from the fixed GitHub repository.
set -Eeuo pipefail

readonly QS_REPOSITORY="Newbluecake/quick-share"
readonly QS_RELEASE_ROOT="https://github.com/${QS_REPOSITORY}/releases"
readonly QS_RELEASE_PUBLIC_KEY_DER_BASE64="MCowBQYDK2VwAyEAcXGzr1dl2fQcyFJBD044/DrWlgc5rYjQTozO3yzr8Q0="

info() { printf '[INFO] %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*" >&2; }
die() { printf '[ERROR] %s\n' "$*" >&2; return 1; }

detect_os() {
    case "${QS_TEST_UNAME_S:-$(uname -s)}" in
        Linux) printf 'linux\n' ;;
        Darwin) printf 'macos\n' ;;
        *) die "unsupported operating system" ;;
    esac
}

detect_arch() {
    case "${QS_TEST_UNAME_M:-$(uname -m)}" in
        x86_64|amd64) printf 'x86_64\n' ;;
        arm64|aarch64) printf 'aarch64\n' ;;
        *) die "unsupported CPU architecture" ;;
    esac
}

resolve_target() {
    local os="$1" arch="$2"
    case "${os}/${arch}" in
        linux/x86_64) printf 'x86_64-unknown-linux-musl\n' ;;
        macos/x86_64) printf 'x86_64-apple-darwin\n' ;;
        macos/aarch64) printf 'aarch64-apple-darwin\n' ;;
        *) die "no Quick Share release is published for ${os}/${arch}" ;;
    esac
}

asset_name() {
    printf 'quick-share-%s\n' "$1"
}

release_download_root() {
    local version="$1"
    if [[ "$version" == "latest" ]]; then
        printf '%s/latest/download\n' "$QS_RELEASE_ROOT"
        return
    fi
    [[ "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] \
        || { die "invalid release version: $version"; return 1; }
    version="${version#v}"
    printf '%s/download/v%s\n' "$QS_RELEASE_ROOT" "$version"
}

download_file() {
    local url="$1" destination="$2"
    [[ "$url" == https://github.com/Newbluecake/quick-share/releases/* ]] \
        || { die "refusing download outside the fixed Quick Share GitHub repository"; return 1; }
    command -v curl >/dev/null 2>&1 || { die "curl is required"; return 1; }
    local effective
    if ! effective="$(curl --fail --silent --show-error --location \
        --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 2 \
        --write-out '%{url_effective}' --output "$destination" "$url")"; then
        rm -f "$destination"
        die "release download failed"
        return 1
    fi
    case "$effective" in
        https://github.com/*|https://release-assets.githubusercontent.com/*) ;;
        *) rm -f "$destination"; die "release redirect left the allowed GitHub asset origins" ;;
    esac
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print tolower($1)}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print tolower($1)}'
    else
        die "sha256sum or shasum is required"
    fi
}

expected_checksum() {
    local manifest="$1" wanted="$2"
    awk -v wanted="$wanted" '
        BEGIN { count = 0 }
        /^[0-9A-Fa-f]{64}[[:space:]]+[*]?[^/\\]+$/ {
            name = $2; sub(/^\*/, "", name)
            if (name == wanted) { value = tolower($1); count++ }
        }
        END { if (count != 1) exit 1; print value }
    ' "$manifest" || {
        die "checksum manifest must contain exactly one entry for $wanted"
        return 1
    }
}

verify_checksum() {
    local candidate="$1" manifest="$2" name="$3"
    local expected actual
    expected="$(expected_checksum "$manifest" "$name")"
    actual="$(sha256_file "$candidate")"
    [[ "$actual" == "$expected" ]] || {
        die "SHA-256 verification failed for $name"
        return 1
    }
}

verify_signature_if_available() {
    local manifest="$1" signature="$2" candidate="$3" temporary="$4"
    if command -v openssl >/dev/null 2>&1 \
        && openssl list -public-key-algorithms 2>/dev/null | grep -qi 'ED25519'; then
        local public_key="${temporary}/release-signing-key.der"
        printf '%s' "$QS_RELEASE_PUBLIC_KEY_DER_BASE64" \
            | openssl base64 -d -A > "$public_key"
        openssl pkeyutl -verify -rawin -pubin -keyform DER \
            -inkey "$public_key" -in "$manifest" -sigfile "$signature" \
            >/dev/null 2>&1 || {
                die "Ed25519 release signature verification failed"
                return 1
            }
        info "Verified pinned Ed25519 release signature"
        return
    fi
    if command -v gh >/dev/null 2>&1 \
        && gh auth status >/dev/null 2>&1 \
        && gh attestation verify --help >/dev/null 2>&1; then
        gh attestation verify "$candidate" \
            --repo "$QS_REPOSITORY" \
            --signer-workflow "${QS_REPOSITORY}/.github/workflows/release.yml" \
            >/dev/null || {
                die "GitHub build provenance verification failed"
                return 1
            }
        info "Verified GitHub build provenance"
        return
    fi
    warn "Ed25519/GitHub attestation verification is unavailable; SHA-256 was still verified"
}

command_conflicts() {
    local name="$1" destination="$2" found
    [[ -e "$destination" || -L "$destination" ]] && return 0
    found="$(command -v "$name" 2>/dev/null || true)"
    [[ -n "$found" && "$found" != "$destination" ]]
}

create_aliases() {
    local install_dir="$1" name destination
    for name in sc rc; do
        destination="${install_dir}/${name}"
        if command_conflicts "$name" "$destination"; then
            warn "not overwriting existing command or path: $name"
            continue
        fi
        if [[ -e "$destination" || -L "$destination" ]]; then
            warn "not overwriting existing shortcut: $destination"
            continue
        fi
        ln -s quick-share "$destination"
        info "Created shortcut: $destination"
    done
}

install_verified_binary() {
    local candidate="$1" install_dir="$2"
    local current="${install_dir}/quick-share"
    local staged backup
    mkdir -p "$install_dir"
    staged="$(mktemp "${install_dir}/.quick-share.new.XXXXXX")"
    backup="${staged}.old"
    cp "$candidate" "$staged"
    chmod 0755 "$staged"
    "$staged" --version >/dev/null 2>&1 || {
        rm -f "$staged"
        die "downloaded executable failed its startup check"
        return 1
    }
    if [[ -e "$current" || -L "$current" ]]; then
        mv "$current" "$backup"
    fi
    if ! mv "$staged" "$current"; then
        [[ -e "$backup" ]] && mv "$backup" "$current"
        rm -f "$staged"
        die "failed to atomically install quick-share"
        return 1
    fi
    if ! "$current" --version >/dev/null 2>&1; then
        rm -f "$current"
        [[ -e "$backup" ]] && mv "$backup" "$current"
        die "new executable failed after installation; previous version restored"
        return 1
    fi
    rm -f "$backup"
}

usage() {
    cat <<'EOF'
Usage: install.sh [--version VERSION] [--install-dir PATH] [--no-aliases]

Downloads one Rust executable, verifies SHA-256 and a pinned Ed25519 signature
(or GitHub provenance when available), then atomically installs it. Existing sc
or rc commands are never overwritten.
EOF
}

main() {
    local version="latest"
    local install_dir="${QUICK_SHARE_INSTALL_DIR:-$HOME/.local/bin}"
    local aliases=1
    while (($#)); do
        case "$1" in
            --version) [[ $# -ge 2 ]] || { die "--version requires a value"; return 1; }; version="$2"; shift 2 ;;
            --install-dir) [[ $# -ge 2 ]] || { die "--install-dir requires a value"; return 1; }; install_dir="$2"; shift 2 ;;
            --no-aliases) aliases=0; shift ;;
            -h|--help) usage; return 0 ;;
            *) die "unknown installer option: $1"; return 1 ;;
        esac
    done
    [[ "$install_dir" = /* ]] || {
        die "installation directory must be absolute"
        return 1
    }

    local os arch target name root temporary candidate checksums signature
    os="$(detect_os)"
    arch="$(detect_arch)"
    target="$(resolve_target "$os" "$arch")"
    name="$(asset_name "$target")"
    root="$(release_download_root "$version")"
    temporary="$(mktemp -d "${TMPDIR:-/tmp}/quick-share-install.XXXXXX")"
    trap 'rm -rf "$temporary"' EXIT
    candidate="${temporary}/${name}"
    checksums="${temporary}/SHA256SUMS"
    signature="${temporary}/SHA256SUMS.sig"

    info "Downloading $name"
    download_file "${root}/${name}" "$candidate"
    download_file "${root}/SHA256SUMS" "$checksums"
    download_file "${root}/SHA256SUMS.sig" "$signature"
    verify_signature_if_available "$checksums" "$signature" "$candidate" "$temporary"
    verify_checksum "$candidate" "$checksums" "$name"
    install_verified_binary "$candidate" "$install_dir"
    if ((aliases)); then
        create_aliases "$install_dir"
    fi
    info "Installed $($install_dir/quick-share --version) at $install_dir/quick-share"
}

if [[ -z "${BASH_SOURCE[0]:-}" || "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
