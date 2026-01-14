# Changelog

All notable changes to Quick Share will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[1.0.7]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.7
[1.0.6]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.6
[1.0.0]: https://github.com/Newbluecake/quick-share/releases/tag/v1.0.0
