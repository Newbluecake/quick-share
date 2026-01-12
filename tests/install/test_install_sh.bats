#!/usr/bin/env bats

# Test suite for install.sh script
# These tests verify the installation script works correctly across different platforms

setup() {
    # Source the install script to test individual functions
    INSTALL_SCRIPT="/home/bluecake/ai/chat/quick-share/install.sh"

    # Create a temporary directory for testing
    export TEST_DIR="$(mktemp -d)"
    export TEST_BIN_DIR="$TEST_DIR/bin"
    mkdir -p "$TEST_BIN_DIR"
}

teardown() {
    # Clean up temporary directory
    rm -rf "$TEST_DIR"
}

# Test: detect_os returns linux on Linux
@test "detect_os returns 'linux' on Linux system" {
    # Mock uname to return Linux
    function uname() {
        if [ "$1" = "-s" ]; then
            echo "Linux"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    result=$(detect_os)
    [ "$result" = "linux" ]
}

# Test: detect_os returns macos on Darwin
@test "detect_os returns 'macos' on Darwin system" {
    # Mock uname to return Darwin
    function uname() {
        if [ "$1" = "-s" ]; then
            echo "Darwin"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    result=$(detect_os)
    [ "$result" = "macos" ]
}

# Test: detect_os handles unsupported OS
@test "detect_os fails on unsupported OS" {
    # Mock uname to return Windows (unsupported)
    function uname() {
        if [ "$1" = "-s" ]; then
            echo "MINGW64_NT"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    run detect_os
    [ "$status" -ne 0 ]
}

# Test: detect_arch returns amd64 for x86_64
@test "detect_arch returns 'amd64' for x86_64" {
    # Mock uname to return x86_64
    function uname() {
        if [ "$1" = "-m" ]; then
            echo "x86_64"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    result=$(detect_arch)
    [ "$result" = "amd64" ]
}

# Test: detect_arch returns arm64 for aarch64
@test "detect_arch returns 'arm64' for aarch64" {
    # Mock uname to return aarch64
    function uname() {
        if [ "$1" = "-m" ]; then
            echo "aarch64"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    result=$(detect_arch)
    [ "$result" = "arm64" ]
}

# Test: detect_arch returns arm64 for arm64 (macOS)
@test "detect_arch returns 'arm64' for arm64 (macOS)" {
    # Mock uname to return arm64 (macOS M1/M2)
    function uname() {
        if [ "$1" = "-m" ]; then
            echo "arm64"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    result=$(detect_arch)
    [ "$result" = "arm64" ]
}

# Test: detect_arch handles unsupported architecture
@test "detect_arch fails on unsupported architecture" {
    # Mock uname to return i386 (unsupported)
    function uname() {
        if [ "$1" = "-m" ]; then
            echo "i386"
        fi
    }
    export -f uname

    source "$INSTALL_SCRIPT"
    run detect_arch
    [ "$status" -ne 0 ]
}

# Test: get_download_url constructs correct URL for linux-amd64
@test "get_download_url constructs correct URL for linux-amd64" {
    source "$INSTALL_SCRIPT"
    result=$(get_download_url "linux" "amd64")
    expected="https://github.com/Newbluecake/quick-share/releases/latest/download/quick-share-linux-amd64"
    [ "$result" = "$expected" ]
}

# Test: get_download_url constructs correct URL for macos-arm64
@test "get_download_url constructs correct URL for macos-arm64" {
    source "$INSTALL_SCRIPT"
    result=$(get_download_url "macos" "arm64")
    expected="https://github.com/Newbluecake/quick-share/releases/latest/download/quick-share-macos-arm64"
    [ "$result" = "$expected" ]
}

# Test: select_install_dir prefers /usr/local/bin when writable
@test "select_install_dir prefers /usr/local/bin when writable" {
    # Source the script first to get the original definitions
    source "$INSTALL_SCRIPT"

    # Create a writable /usr/local/bin equivalent in test dir
    local test_usr_local="$TEST_DIR/usr/local/bin"
    mkdir -p "$test_usr_local"
    chmod 755 "$test_usr_local"

    # Mock the check by overriding the function AFTER sourcing
    function check_writable() {
        local dir="$1"
        if [ "$dir" = "/usr/local/bin" ]; then
            return 0  # Simulate writable
        fi
        return 1
    }

    result=$(select_install_dir)
    [ "$result" = "/usr/local/bin" ]
}

# Test: select_install_dir falls back to ~/.local/bin when /usr/local/bin not writable
@test "select_install_dir falls back to ~/.local/bin when /usr/local/bin not writable" {
    # Source the script first
    source "$INSTALL_SCRIPT"

    # Mock the check to simulate non-writable /usr/local/bin
    function check_writable() {
        local dir="$1"
        if [ "$dir" = "/usr/local/bin" ]; then
            return 1  # Simulate not writable
        fi
        return 0
    }

    result=$(select_install_dir)
    expected="$HOME/.local/bin"
    [ "$result" = "$expected" ]
}

# Test: script creates ~/.local/bin if it doesn't exist
@test "script creates ~/.local/bin if it doesn't exist" {
    # Source script first
    source "$INSTALL_SCRIPT"

    # Remove ~/.local/bin if it exists for this test
    local test_local_bin="$TEST_DIR/.local/bin"

    function check_writable() {
        return 1  # Simulate /usr/local/bin not writable
    }

    # We need to capture the real mkdir before overriding if we were calling it,
    # but the script calls 'mkdir'. Since 'mkdir' is external, we can mock it or let it run.
    # The script uses 'mkdir -p'.
    # Let's ensure we are using the system mkdir for the actual creation,
    # OR we mock it to verify it was called.
    # The original test mocked mkdir to command mkdir which is fine,
    # but we need to ensure select_install_dir calls our mocked check_writable.

    HOME="$TEST_DIR" result=$(select_install_dir)

    # Verify directory was created
    [ -d "$test_local_bin" ]
}

# Test: script is executable
@test "install.sh script is executable" {
    [ -x "$INSTALL_SCRIPT" ]
}

# Test: script has proper shebang
@test "install.sh has proper shebang" {
    first_line=$(head -n 1 "$INSTALL_SCRIPT")
    [[ "$first_line" =~ ^#!/bin/(bash|sh) ]]
}

# Test: script sets error handling (set -e)
@test "install.sh sets error handling" {
    grep -q "set -e" "$INSTALL_SCRIPT"
}
