# Changelog

All notable changes to Quick Share will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.7.0] - 2026-04-28

### Added
- **Auto-trigger peer file dialog on hello handshake** — when a configured peer is connected:
  - Share mode (`quick-share <file>`) automatically triggers a save dialog on the peer side
  - Upload mode (`quick-share --upload`) automatically triggers a file selection dialog on the peer side
  - Both flows complete the transfer without requiring a separate peer command

## [1.6.1] - 2026-04-27

### Fixed
- **Commit hash detection** now uses a 3-step fallback for all install methods
  - Build-time `_commit.py` (PyInstaller releases)
  - Runtime `git rev-parse` (dev / editable installs)
  - pip `direct_url.json` metadata (git and local path installs)
  - Previously `_commit.py` was always "unknown" because PEP 517 build isolation strips `.git`

## [1.6.0] - 2026-04-27

### Added
- **`--serve` mode** for peer-only server startup (`quick-share --serve`)
  - Starts a minimal HTTP server exposing only `/api/peer/*` endpoints
  - Enables inter-instance communication without sharing files or requiring an upload directory
  - New classes: `ServeHandler`, `ServeServer`
- **`--secret` CLI flag** for passing peer authentication secret directly on the command line
  - `quick-share --serve --secret KEY` starts server with secret without requiring prior config
  - CLI `--secret` takes precedence over config file value
- **Short commit ID in `--version` output** — `quick-share --version` now shows version with commit hash
  - Example: `quick-share 1.6.0 (d954d40)`
  - Fallback chain: build-time `_commit.py` → runtime `git rev-parse` → "unknown"
- **`pyproject.toml`** for modern Python build system compatibility

## [1.5.0] - 2026-04-27

### Added
- **Peer-to-peer remote trigger** - lszrz-style bidirectional file transfer between quick-share instances
  - `quick-share config --peer HOST:PORT --secret KEY` to configure remote peer
  - `quick-share config --show` to view current config (with masked secret)
  - Auto-connect to configured peer on startup with terminal status message
  - Peer API endpoints (`/api/peer/hello`, `/api/peer/request-upload`, `/api/peer/request-download`, `/api/peer/receive`)
  - Shared-secret authentication via `X-Peer-Secret` HTTP header
  - Remote-triggered upload (rz-like): peer triggers file dialog on this instance, files sent back to peer
  - Remote-triggered download (sz-like): peer sends file list, this instance picks save path
- Cross-platform native file dialogs (`src/file_dialog.py`)
  - Linux: zenity (GNOME/XFCE) and kdialog (KDE)
  - macOS: osascript / AppleScript dialogs
  - Windows: PowerShell file/folder browser dialogs
  - Graceful fallback to terminal path input when no GUI available
- Peer HTTP client (`src/peer_client.py`) for inter-instance communication
- Configuration file management (`src/config.py`) at `~/.quick-share/config.json`
- Peer event logging with dedicated emoji indicators (✓ ⚠ ↗ ↘ ✗)

## [1.4.0] - 2026-04-27

### Added
- **Bidirectional file upload support** - Upload files to the sharing server
  - `quick-share --upload [dir]` starts a standalone upload-only server
  - `quick-share <paths> --upload` adds upload capability to existing share pages
  - Pure HTML5 upload page with drag-and-drop and progress bar
  - Upload form integrated into both single-directory and multi-share SPA views
- Optional password protection for uploads via `--upload-password <password>`
  - Supports `X-Upload-Password` HTTP header (curl) and form field (browser)
- Upload progress logging in terminal with ⬆️ emoji indicator
- Auto-rename on filename conflict: `file.txt` → `file (1).txt` → `file (2).txt`
- Upload and download share the same `-n` / `-t` quota limits
- New `src/upload_handler.py` module with multipart parsing, file saving, and path traversal protection
- `UploadServer` class for standalone mode, `UploadHandler` for HTTP request handling
- SPA upload form with drag-drop, progress bar, and password support

### Fixed
- Synced `__version__` to 1.3.0 and added version bump checklist to CLAUDE.md

## [1.3.0] - 2026-03-30

### Added
- **Multi-path sharing** - Support passing multiple files and directories in a single command
  - `quick-share file1.txt file2.pdf ./docs` shares all items at once
  - Shell glob patterns naturally expand (e.g., `quick-share *.pdf`)
  - Unified file list page for all scenarios (including single file)
  - Per-item download and "Download All" as ZIP
  - Folder expansion and browsing in the web UI
- New `MultiShareServer` and `MultiShareHandler` classes with full route dispatch
  - `/api/tree` and `/api/content` JSON API endpoints
  - `/files/<path>` for individual file downloads
  - `/download/all.zip` for streaming ZIP of all shared items
- Vue 3 SPA template (`generate_multi_share_spa_html()`) with tree expansion
- Server-rendered legacy template (`generate_multi_share_legacy_html()`)
- Name conflict detection: CLI reports error when duplicate basenames are passed
- Path security: `validate_multi_share_path()` with commonpath sandboxing and traversal defence

### Changed
- CLI argument `file_path` changed to `file_paths` with `nargs='+'` for multi-path support
- `main()` now always routes through `MultiShareServer` (unified experience)
- Session-based counting (`-n`) for multi-path sharing, consistent with directory sharing

### Fixed
- Fixed test suite to match `MultiShareServer` refactor (test_integration, test_server, test_updater)
- Fixed `DirectoryShareHandler` test helper missing `client_address` attribute

## [1.2.0] - 2026-02-05

### Added
- **Real-time download progress tracking** - Console now displays live download progress with detailed information
  - ⬇️ Download start notification with client IP, filename, and file size
  - ⬇️ Real-time progress updates (transferred bytes/total bytes + percentage)
  - ✅ Completion notification with total bytes transferred and duration
  - ⚠️ Interruption notification when client disconnects
  - ❌ Error messages for failed downloads
- Progress logging for all file downloads (single files and directory ZIPs)
- Thread-safe progress tracking with per-connection instances
- Optimized logging frequency (every 80KB) to avoid I/O overhead
- Support for concurrent downloads with independent progress tracking (10+ connections)
- Graceful error handling for client disconnections (BrokenPipeError, ConnectionResetError)
- Emoji indicators for visual feedback (⬇️ ✅ ⚠️ ❌)

### Changed
- Enhanced file streaming with progress callback integration
- Enhanced ZIP streaming with approximate progress tracking based on file sizes
- Improved directory size calculation (inline tree walking instead of non-existent utility)

### Fixed
- Fixed directory ZIP download progress tracking by calculating size inline

[1.6.1]: https://github.com/Newbluecake/quick-share/compare/v1.6.0...v1.6.1
[1.6.0]: https://github.com/Newbluecake/quick-share/compare/v1.5.0...v1.6.0
[1.5.0]: https://github.com/Newbluecake/quick-share/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/Newbluecake/quick-share/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/Newbluecake/quick-share/releases/tag/v1.3.0
[1.2.0]: https://github.com/Newbluecake/quick-share/releases/tag/v1.2.0
[1.1.0]: https://github.com/Newbluecake/quick-share/releases/tag/v1.1.0

## [1.1.0] - 2026-02-05

### Added
- Symlink handling with interactive user confirmation
  - Automatic detection when sharing symlinked files or directories
  - Shows symlink source and target paths before sharing
  - Interactive prompt (y/n) to confirm following symlinks
  - Broken symlink detection with clear error messages
  - Proper exit codes: 0 for user cancellation, 1 for errors
- Full backward compatibility: normal file/directory sharing unchanged

### Changed
- Enhanced `validate_path()` function to detect symlinks before processing
- Added `handle_symlink()` function for symlink-specific logic
- Improved error handling in `main()` for symlink-specific error types

[1.0.13]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.13

### Added
- New `update` command: check and update quick-share to the latest version
  - `quick-share update --check`: check for updates without installing
  - `quick-share update`: update to latest version with confirmation
  - `quick-share update -y`: skip confirmation prompt
- Smart update source detection: automatically uses pip, exe replacement, or git based on installation method
- Rollback mechanism: automatic rollback on update failure
- Full backward compatibility: existing `quick-share <file>` usage unchanged

## [1.0.12] - 2026-01-27

### Added
- Multi-IP detection: display all available LAN IPs with interface names
- Virtual network filtering: automatically exclude Docker, VirtualBox, VMware, WSL, and container bridge interfaces
- Cross-platform support for IP detection (Linux, macOS, Windows)

### Changed
- Improved startup message to show all available URLs when multiple network interfaces are detected

## [1.0.11] - 2026-01-27

### Fixed
- URL encode Chinese filenames in download commands for wget/curl compatibility
- Wrap download commands in single quotes for shell safety with special characters

## [1.0.10] - 2026-01-18

### Fixed
- Fixed directory tree scrolling issue in SPA view (now scrolls independently from preview)

## [1.0.9] - 2026-01-18

### Changed
- improved startup message to include Browse URL for directory sharing

## [1.0.8] - 2026-01-18

### Added
- Implement text-format-preview SPA

### Fixed
- Update legacy view link to use query parameter

### Changed
- Add logs to gitignore

## [1.0.7] - 2026-01-14

### Fixed
- Fixed critical bug in directory zip download causing curl/wget failures with "Illegal or missing hexadecimal sequence" error
- Removed improper Transfer-Encoding: chunked header (was not correctly implemented)
- Added exception handling for client disconnections during zip streaming
- Fixed path validation logic for RESTful zip download URLs (/download/{name}.zip)

## [1.0.6] - 2026-01-14

### Changed
- Directory sharing now uses RESTful URL format: `http://IP:PORT/download/DIR_NAME.zip`
- Updated wget/curl command examples to match the new URL format

### Removed
- Removed "Scan QR code to download:" output (QR code functionality was never implemented)

### Fixed
- Fixed directory download URL format to be more intuitive and RESTful

## [1.0.0] - 2026-01-12

### Added
- Initial release of Quick Share
- Automatic LAN IP detection with multi-interface support
- Smart port selection (8000-8099 range with auto-increment)
- Path security validation to prevent traversal attacks
- File streaming for large file support
- Download counter with configurable limits (default: 10)
- Automatic timeout with configurable duration (default: 5 minutes)
- Real-time download logging with client IP tracking
- Support for curl/wget compatible downloads
- Browser-friendly file downloads
- Command-line interface with comprehensive options:
  - `-p/--port`: Custom port specification
  - `-n/--max-downloads`: Download limit configuration
  - `-t/--timeout`: Timeout configuration (supports s/m/h units)
- Graceful shutdown on Ctrl+C
- Cross-platform support (Linux/macOS/Windows)
- Comprehensive test suite with 99% coverage
- PyInstaller packaging support for standalone executables

### Security
- Path traversal attack prevention
- URL encoding attack detection
- Basename-only file access enforcement
- No directory listing exposure

[1.0.12]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.12
[1.0.11]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.11
[1.0.10]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.10
[1.0.9]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.9
[1.0.8]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.8
[1.0.7]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.7
[1.0.6]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.6
[1.0.0]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.0
