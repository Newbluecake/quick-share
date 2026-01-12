#!/bin/bash

# Quick Share Installation Script
# Automatically detects OS and architecture, downloads the appropriate binary,
# and installs it to the system.

set -e

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# GitHub repository information
GITHUB_REPO="Newbluecake/quick-share"
BINARY_NAME="quick-share"

# Print colored message
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Detect operating system
detect_os() {
    local os_type=$(uname -s)
    case "$os_type" in
        Linux*)
            echo "linux"
            ;;
        Darwin*)
            echo "macos"
            ;;
        *)
            print_error "Unsupported operating system: $os_type"
            print_error "Supported: Linux, macOS"
            exit 1
            ;;
    esac
}

# Detect CPU architecture
detect_arch() {
    local arch=$(uname -m)
    case "$arch" in
        x86_64)
            echo "amd64"
            ;;
        aarch64)
            echo "arm64"
            ;;
        arm64)
            echo "arm64"
            ;;
        *)
            print_error "Unsupported architecture: $arch"
            print_error "Supported: x86_64 (amd64), aarch64/arm64"
            exit 1
            ;;
    esac
}

# Construct download URL
get_download_url() {
    local os="$1"
    local arch="$2"
    echo "https://github.com/${GITHUB_REPO}/releases/latest/download/${BINARY_NAME}-${os}-${arch}"
}

# Check if directory is writable
check_writable() {
    local dir="$1"
    [ -w "$dir" ] 2>/dev/null
}

# Select installation directory
select_install_dir() {
    # Prefer /usr/local/bin if writable
    if check_writable "/usr/local/bin"; then
        echo "/usr/local/bin"
    else
        # Fallback to ~/.local/bin
        local local_bin="$HOME/.local/bin"
        mkdir -p "$local_bin"
        echo "$local_bin"
    fi
}

# Check if PATH includes directory
check_path() {
    local dir="$1"
    if [[ ":$PATH:" == *":$dir:"* ]]; then
        return 0
    else
        return 1
    fi
}

# Main installation function
main() {
    print_info "Quick Share Installation Script"
    echo ""

    # Detect system information
    print_info "Detecting system information..."
    local os=$(detect_os)
    local arch=$(detect_arch)
    print_info "Detected: $os-$arch"
    echo ""

    # Construct download URL
    local download_url=$(get_download_url "$os" "$arch")
    print_info "Download URL: $download_url"
    echo ""

    # Select installation directory
    local install_dir=$(select_install_dir)
    print_info "Installation directory: $install_dir"
    echo ""

    # Download binary
    local temp_file=$(mktemp)
    print_info "Downloading binary..."
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$temp_file" "$download_url" || {
            print_error "Failed to download binary"
            rm -f "$temp_file"
            exit 1
        }
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$temp_file" "$download_url" || {
            print_error "Failed to download binary"
            rm -f "$temp_file"
            exit 1
        }
    else
        print_error "Neither curl nor wget found. Please install one of them."
        rm -f "$temp_file"
        exit 1
    fi
    print_info "Download complete"
    echo ""

    # Install binary
    print_info "Installing to $install_dir/$BINARY_NAME..."
    chmod +x "$temp_file"
    mv "$temp_file" "$install_dir/$BINARY_NAME" || {
        print_error "Failed to install binary to $install_dir"
        rm -f "$temp_file"
        exit 1
    }
    print_info "Installation complete"
    echo ""

    # Check PATH configuration
    if ! check_path "$install_dir"; then
        print_warn "⚠️  $install_dir is not in your PATH"
        echo ""
        echo "To use quick-share, add the following line to your shell configuration:"
        echo ""
        if [ "$os" = "macos" ]; then
            echo "  echo 'export PATH=\"$install_dir:\$PATH\"' >> ~/.zshrc"
            echo "  source ~/.zshrc"
        else
            echo "  echo 'export PATH=\"$install_dir:\$PATH\"' >> ~/.bashrc"
            echo "  source ~/.bashrc"
        fi
        echo ""
    fi

    # Verify installation
    print_info "Verifying installation..."
    if "$install_dir/$BINARY_NAME" --version >/dev/null 2>&1; then
        print_info "✓ Installation successful!"
        echo ""
        "$install_dir/$BINARY_NAME" --version
    else
        print_warn "Installation completed but version check failed"
        print_warn "You may need to add $install_dir to your PATH"
    fi
}

# Run main function only if script is executed (not sourced for testing)
if ! (return 0 2>/dev/null); then
    main "$@"
fi
