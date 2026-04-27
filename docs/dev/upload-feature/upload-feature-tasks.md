---
feature: upload-feature
complexity: standard
generated_by: architect-planner
generated_at: 2026-04-27T00:00:00+08:00
version: 1
based_on_design: upload-feature-design.md
---

# Task Breakdown: Upload Feature

## Task Groups

### Group 0: Foundation (P0, standalone)

| ID | Name | Dependencies | Parallel | Files |
|----|------|-------------|----------|-------|
| T-001 | Create upload_handler.py with multipart parsing, save, and auto-rename | None | Yes | `src/upload_handler.py` |
| T-002 | Add upload progress logging to logger.py | None | Yes | `src/logger.py` |

### Group 1: CLI & Standalone Upload (P0, after T-001, T-002)

| ID | Name | Dependencies | Parallel | Files |
|----|------|-------------|----------|-------|
| T-003 | Add --upload and --upload-password CLI arguments | T-001 | No | `src/cli.py` |
| T-004 | Create UploadServer class and integrate into main.py | T-001, T-002, T-003 | No | `src/server.py`, `src/main.py` |
| T-005 | Create standalone upload page HTML in templates.py | T-001, T-004 | No | `src/templates.py` |

### Group 2: Integrated Upload (P1, after T-004)

| ID | Name | Dependencies | Parallel | Files |
|----|------|-------------|----------|-------|
| T-006 | Add POST /upload endpoint to MultiShareHandler | T-001, T-004 | Yes | `src/server.py` |
| T-007 | Add upload form to SPA templates | T-005, T-006 | Yes | `src/templates.py` |

### Group 3: Security & Quota (P1, after T-003)

| ID | Name | Dependencies | Parallel | Files |
|----|------|-------------|----------|-------|
| T-008 | Implement upload password protection | T-001 | Yes | `src/upload_handler.py`, `src/server.py` |
| T-009 | Integrate upload with session/quota limits | T-004 | Yes | `src/server.py`, `src/upload_handler.py` |

### Group 4: Tests (P2, parallel with Groups 1-3)

| ID | Name | Dependencies | Parallel | Files |
|----|------|-------------|----------|-------|
| T-010 | Write unit tests for upload_handler | T-001 | Yes | `tests/test_upload_handler.py` |
| T-011 | Write integration tests for upload server | T-004, T-006 | Yes | `tests/test_server_upload.py` |

---

## Execution Order (serialized for execution=batch)

Based on dependency resolution, the execution order is:

```
Parallel Group A (P0):
  T-001: Create upload_handler.py
  T-002: Add upload logging
     │
Parallel Group B (P0, after A):
  T-003: CLI arguments
     │
  T-004: UploadServer + main.py integration (after T-001,T-002,T-003)
     │
Parallel Group C (P1, after T-004):
  T-005: Standalone upload page HTML
  T-006: POST /upload in MultiShareHandler
  T-008: Password protection (after T-001)
  T-009: Quota integration (after T-004)
  T-010: Unit tests for upload_handler (after T-001)
     │
Parallel Group D (P1, after C):
  T-007: Upload form in SPA templates (after T-005,T-006)
  T-011: Integration tests (after T-004,T-006)
```

---

## Task Details

### T-001: Create upload_handler.py

**Description**: Create new module `src/upload_handler.py` with core upload processing logic.

**Acceptance Criteria**:
- [ ] `UploadedFile` namedtuple defined with fields: filename, data, content_type, size
- [ ] `parse_multipart_request(handler)` parses multipart/form-data POST body using `cgi.FieldStorage`
- [ ] `save_uploaded_file(file, save_dir)` writes file to disk
- [ ] `resolve_filename_conflict(filename, save_dir)` appends ` (N)` suffix on conflict
- [ ] `UploadProgressTracker` class for tracking upload progress
- [ ] Path traversal protection: reject filenames with `/` or `..`
- [ ] Chunked streaming via TemporaryFile for large uploads
- [ ] Pure stdlib, no external dependencies
- [ ] All functions have docstrings and type annotations

**Skill Injections**: dev-backend-standards, test-governance

**Files**: `src/upload_handler.py`

---

### T-002: Add upload progress logging to logger.py

**Description**: Add upload-specific log formatters to `src/logger.py`, mirroring existing download log functions.

**Acceptance Criteria**:
- [ ] `format_upload_start(timestamp, client_ip, filename, file_size)` -- format: `[ts] ⬆️  client_ip - filename (size)`
- [ ] `format_upload_progress(timestamp, client_ip, bytes_transferred, total_bytes, percentage)` -- format: `[ts] ⬆️  client_ip - xfer / total (pct%)`
- [ ] `format_upload_complete(timestamp, client_ip, filename, total_bytes, duration_sec)` -- format: `[ts] ✅ client_ip - Completed: filename (size in Xs)`
- [ ] `format_upload_interrupted(timestamp, client_ip, filename, bytes_transferred, total_bytes)` -- format: `[ts] ⚠️  client_ip - Interrupted: filename (xfer / total)`
- [ ] `format_upload_error(timestamp, client_ip, filename, error_message)` -- format: `[ts] ❌ client_ip - Error: filename - msg`
- [ ] All upload formatters use ⬆️  emoji to distinguish from download ⬇️
- [ ] `get_timestamp()` reused (no new timestamp function)
- [ ] Lazy imports to avoid circular dependencies (same pattern as download functions)

**Skill Injections**: dev-backend-standards

**Files**: `src/logger.py`

---

### T-003: Add --upload and --upload-password CLI arguments

**Description**: Add `--upload` and `--upload-password` arguments to CLI.

**Acceptance Criteria**:
- [ ] `--upload [SAVE_DIR]` argument added to argparse: optional value, defaults to None
- [ ] `--upload-password <password>` argument added to argparse
- [ ] `--upload` works as standalone flag and with optional save directory
- [ ] `--upload` can be combined with file path arguments (integrated mode)
- [ ] `--upload-password` only valid when `--upload` is used
- [ ] `validate_arguments()` checks new args, raises ValueError on invalid combinations
- [ ] Existing tests for CLI still pass

**Skill Injections**: dev-backend-standards

**Files**: `src/cli.py`

---

### T-004: Create UploadServer class and integrate into main.py

**Description**: Create `UploadServer` class in `server.py` for standalone upload mode. Update `main.py` to detect upload mode and create upload server.

**Acceptance Criteria**:
- [ ] `UploadServer` class in `server.py`:
  - Constructor: `save_dir`, `port`, `timeout_minutes`, `max_sessions`, `upload_password`
  - `start()`, `stop()`, `_shutdown_server()` methods (same pattern as MultiShareServer)
  - Session tracking via `track_session()` (reuse pattern)
  - Uses `ThreadingHTTPServer` and custom `UploadHandler`
- [ ] `UploadHandler` class:
  - `do_GET`: serves upload page at `GET /upload`, redirects `/` to `/upload`
  - `do_POST`: handles file upload via `upload_handler.parse_multipart_request` + `save_uploaded_file`
  - Uses `UploadProgressTracker` and upload log formatters for terminal logging
- [ ] `main.py` updated:
  - Detect `--upload` mode
  - If no file paths: create `UploadServer` (standalone)
  - If file paths + `--upload`: create `MultiShareServer` with `upload_enabled=True`
  - If no `--upload`: existing behavior unchanged
- [ ] Terminal startup message includes upload URL when upload mode is active
- [ ] Combined share+upload mode shows both browse URL and upload URL

**Skill Injections**: dev-backend-standards, test-governance

**Files**: `src/server.py`, `src/main.py`

---

### T-005: Create standalone upload page HTML in templates.py

**Description**: Create the HTML upload page served at `GET /upload` for standalone mode.

**Acceptance Criteria**:
- [ ] `generate_upload_page(upload_password_set: bool) -> str` function in `templates.py`
- [ ] File input with single and multiple file selection
- [ ] Drag-and-drop zone
- [ ] Password input field (shown only when `upload_password_set=True`)
- [ ] Progress bar showing upload progress
- [ ] Status message area for success/error feedback
- [ ] Clean styling consistent with project aesthetic (no external deps)
- [ ] Client-side JavaScript: XMLHttpRequest with FormData, upload progress events
- [ ] Displays uploaded filename and size on completion
- [ ] Title: "Quick Share - Upload"
- [ ] Link back to browse page when in integrated mode (optional, for standalone it shows self)

**Skill Injections**: dev-frontend-standards

**Files**: `src/templates.py`

---

### T-006: Add POST /upload endpoint to MultiShareHandler

**Description**: Add `do_POST` and upload handling to `MultiShareHandler` for integrated share+upload mode.

**Acceptance Criteria**:
- [ ] `MultiShareHandler.do_POST` method added
- [ ] Route `/upload` or `/api/upload` to upload handler
- [ ] Only active when `self.server.upload_enabled` is True, else return 404
- [ ] Uses `upload_handler.parse_multipart_request` and `save_uploaded_file`
- [ ] Returns JSON `{"status": "ok", "filename": "..."}` on success
- [ ] Returns JSON `{"error": "..."}` on failure
- [ ] Terminal logging via upload log formatters
- [ ] `MultiShareServer.start()` passes `upload_enabled`, `upload_save_dir`, `upload_password` to handler

**Skill Injections**: dev-backend-standards

**Files**: `src/server.py`

---

### T-007: Add upload form to SPA templates

**Description**: Add upload button and inline upload form to both `generate_spa_html()` and `generate_multi_share_spa_html()`.

**Acceptance Criteria**:
- [ ] Upload button in header action area (only when upload is enabled -- but since templates don't know server config, add a query parameter check or always show)
- [ ] Clicking upload button toggles an inline upload form
- [ ] Form supports drag-and-drop and file selection
- [ ] Upload progress bar in the form
- [ ] Success/error feedback display
- [ ] Password field shown when `?upload_password=1` query param is present
- [ ] Sends POST to `/api/upload` with FormData
- [ ] Styling consistent with existing SPA design
- [ ] Works in both `generate_spa_html()` and `generate_multi_share_spa_html()`

**Skill Injections**: dev-frontend-standards

**Files**: `src/templates.py`

---

### T-008: Implement upload password protection

**Description**: Add password check to upload endpoint.

**Acceptance Criteria**:
- [ ] Password check in `UploadHandler.do_POST` (standalone mode)
- [ ] Password check in `MultiShareHandler._handle_upload` (integrated mode)
- [ ] Check `X-Upload-Password` header first
- [ ] Fall back to `password` form field in multipart body
- [ ] Return 403 with "Invalid upload password" if password doesn't match
- [ ] Password config stored in server instance (`self.server.upload_password`)
- [ ] No password set = no check (all uploads allowed)
- [ ] Download/browse not affected by upload password
- [ ] Standalone upload page shows password field when `upload_password` is set

**Skill Injections**: dev-backend-standards

**Files**: `src/server.py`, `src/upload_handler.py`, `src/templates.py`

---

### T-009: Integrate upload with session/quota limits

**Description**: Make upload sessions count against the same `-n`/`-t` limits as downloads.

**Acceptance Criteria**:
- [ ] `UploadServer` calls `track_session()` for each upload request
- [ ] When max sessions reached (`-n`), upload is rejected with 403
- [ ] Upload connects to existing `session` tracking (reuses or extends `track_session`)
- [ ] Existing `-t` timeout shuts down both download and upload
- [ ] `max_sessions` renamed or aliased to `max_sessions` (not just `max_downloads`) -- but for backward compat, `-n` label stays
- [ ] Session counter includes both upload and download sessions

**Skill Injections**: dev-backend-standards

**Files**: `src/server.py`

---

### T-010: Write unit tests for upload_handler

**Description**: Comprehensive unit tests for `src/upload_handler.py`.

**Acceptance Criteria**:
- [ ] `test_parse_multipart_simple_file`: Parse a simple multipart request
- [ ] `test_parse_multipart_multiple_files`: Parse multiple files in one request
- [ ] `test_save_file`: Save file to disk verifies content
- [ ] `test_resolve_filename_no_conflict`: No rename when file doesn't exist
- [ ] `test_resolve_filename_first_conflict`: `file.txt` -> `file (1).txt`
- [ ] `test_resolve_filename_second_conflict`: `file.txt` -> `file (1).txt` -> `file (2).txt`
- [ ] `test_resolve_filename_complex_ext`: `file.tar.gz` -> `file (1).tar.gz`
- [ ] `test_reject_path_traversal`: Filename with `../` raises ValueError
- [ ] `test_upload_progress_tracker`: Verify UploadProgressTracker tracking
- [ ] All tests follow project test style (pytest or unittest)

**Skill Injections**: test-governance

**Files**: `tests/test_upload_handler.py`

---

### T-011: Write integration tests for upload server

**Description**: Integration tests for upload HTTP endpoints.

**Acceptance Criteria**:
- [ ] `test_standalone_upload_server_start_stop`: Server starts and stops
- [ ] `test_standalone_upload_page`: `GET /upload` returns HTML
- [ ] `test_upload_file_via_http`: POST multipart file, verify saved to disk
- [ ] `test_upload_password_correct`: Correct password succeeds
- [ ] `test_upload_password_incorrect`: Wrong password returns 403
- [ ] `test_upload_quota_limit`: Exceed `-n` limit returns 403
- [ ] `test_integrated_upload`: MultiShareServer with upload option works
- [ ] `test_upload_then_download_quota_sharing`: Both count against same limit
- [ ] All tests follow project test style

**Skill Injections**: test-governance

**Files**: `tests/test_server_upload.py`

---

## Dependency Summary

```
T-001 (upload_handler) ──┬── T-003 (CLI args)
                          ├── T-004 (UploadServer) ──┬── T-005 (upload page)
                          │                          ├── T-006 (POST endpoint) ──┐
                          │                          ├── T-009 (quota)           │
                          │                          └── T-011 (integration tests)
                          ├── T-008 (password)
                          └── T-010 (unit tests)

T-002 (logger) ────────── T-004 (needs logging)
T-003 (CLI) ───────────── T-004 (main.py integration)
T-005 (page) ──────────── T-007 (SPA upload form)
T-006 (POST) ──────────── T-007 (SPA upload form)
```

## Task Count

- **Total tasks**: 11
- **P0 (critical)**: 5 (T-001, T-002, T-003, T-004, T-005)
- **P1 (important)**: 4 (T-006, T-007, T-008, T-009)
- **P2 (tests)**: 2 (T-010, T-011)
- **Estimated implementation time**: Medium (standard complexity)
