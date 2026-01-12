#!/bin/bash

# Quick Share Installation Script
# Installs Quick Share using pip from GitHub repository

set -e

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# GitHub repository information
GITHUB_REPO="Newbluecake/quick-share"

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

# Check if Python is installed
check_python() {
    if command -v python3 >/dev/null 2>&1; then
        echo "python3"
    elif command -v python >/dev/null 2>&1; then
        echo "python"
    else
        return 1
    fi
}

# Main installation function
main() {
    print_info "Quick Share Installation Script"
    echo ""

    # Check for Python
    print_info "Checking for Python..."
    local python_cmd=$(check_python)
    if [ $? -ne 0 ]; then
        print_error "Python is not installed"
        print_error "Please install Python 3.8 or later from https://www.python.org/"
        exit 1
    fi
    print_info "Found: $python_cmd"
    echo ""

    # Install using pip
    print_info "Installing Quick Share from GitHub..."
    $python_cmd -m pip install --user "git+https://github.com/${GITHUB_REPO}.git" || {
        print_error "Failed to install Quick Share"
        exit 1
    }
    print_info "Installation complete"
    echo ""

    # Verify installation
    print_info "Verifying installation..."
    if command -v quick-share >/dev/null 2>&1; then
        print_info "✓ Installation successful!"
        echo ""
        quick-share --version
    else
        print_warn "Installation completed but quick-share command not found in PATH"
        print_warn "You may need to add Python's user bin directory to your PATH"
        echo ""
        echo "Try running:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    fi
}

# Run main function only if script is executed (not sourced for testing)
if ! (return 0 2>/dev/null); then
    main "$@"
fi
