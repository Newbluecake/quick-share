---
feature: upload-feature
complexity: standard
generated_by: architect-planner
generated_at: 2026-04-27T00:00:00+08:00
version: 1
based_on_requirements: upload-feature-requirements.md
---

# Design Document: Upload Feature

## 1. Architecture Overview

### 1.1 Server Mode Matrix

| Mode | CLI Command | Server Class | Upload Support |
|------|------------|-------------|----------------|
| Share only | `quick-share <paths>` | MultiShareServer | No (unchanged) |
| Upload only | `quick-share --upload [dir]` | UploadServer (new) | Yes |
| Share + Upload | `quick-share <paths> --upload` | MultiShareServer | Yes |

### 1.2 New Module: upload_handler.py

Central upload processing module containing multipart parsing, file saving, and auto-rename.

```
+-------------------+       +---------------------+
|    cli.py         | --->  |    main.py          |
| --upload          |       | --upload mode       |
| --upload-password |       | detect & route      |
+-------------------+       +----------+----------+
                                       |
                  +--------------------+--------------------+
                  |                                         |
          +-------v--------+                       +--------v--------+
          | UploadServer   |                       | MultiShareServer|
          | (standalone)   |                       | (integrated)    |
          +-------+--------+                       +--------+--------+
                  |                                         |
                  +------------------+----------------------+
                                     |
                            +--------v--------+
                            | upload_handler  |
                            | .parse_request  |
                            | .save_file      |
                            | .resolve_name   |
                            +--------+--------+
                                     |
                    +----------------+----------------+
                    |                |                |
             +------v-----+  +------v-----+  +------v-----+
             | logger.py  |  | security.py|  |templates.py|
             | upload log |  | path check |  | upload UI  |
             +------------+  +------------+  +------------+
```

### 1.3 New Classes

```
UploadServer(ThreadingHTTPServer)
  - Standalone mode upload server
  - Serves GET /upload (upload page)
  - Accepts POST /upload (file upload)
  - Session/limit tracking (reused from DirectoryShareServer pattern)

UploadHandler(BaseHTTPRequestHandler)
  - do_GET: serve upload page
  - do_POST: receive uploaded file(s)
```

### 1.4 Existing Class Extensions

```
MultiShareServer
  + upload_enabled: bool
  + upload_save_dir: str
  + upload_password: Optional[str]

MultiShareHandler
  + do_POST (new): handle POST /upload requests
```

### 1.5 Data Flow

```
Browser                          Server
   |                                |
   |-- GET /upload ---------------->|  Upload page (standalone)
   |<-- HTML upload form -----------|
   |                                |
   |-- POST /upload --------------->|  multipart/form-data
   |   Content-Type: multipart      |  1. Parse boundary + fields
   |   {file: report.pdf}           |  2. Check password (if set)
   |                                |  3. Validate path (security)
   |                                |  4. Resolve filename conflict
   |                                |  5. Save file to disk
   |                                |  6. Log progress to terminal
   |<-- 200 {"status":"ok"} --------|
```

## 2. Route Design

| Route | Method | Purpose | Server Mode |
|-------|--------|---------|-------------|
| `/upload` | GET | Upload page HTML | Standalone only |
| `/upload` | POST | Upload file(s) | Both |
| `/api/upload` | POST | Upload via SPA | Integrated only |

**Standalone**: `GET /upload` returns a full HTML upload page.
**Integrated**: SPA contains its own upload form that sends `POST /api/upload`.

## 3. Upload Handler Specification

### 3.1 `upload_handler.parse_multipart_request(handler, save_dir)`

Parse a multipart/form-data POST request body using `cgi.FieldStorage` (Python stdlib, available in 3.x).

```python
def parse_multipart_request(handler, save_dir: str) -> List[UploadedFile]:
    """
    Parse multipart POST request.

    Returns list of UploadedFile namedtuples:
      (field_name, filename, content_type, data: bytes, size: int)

    Falls back to streaming for large files via TemporaryFile.
    """
```

### 3.2 `upload_handler.save_uploaded_file(file, save_dir, password=None) -> str`

```python
def save_uploaded_file(file: UploadedFile, save_dir: str) -> str:
    """
    Save uploaded file to save_dir with auto-rename.

    Returns: The absolute path of the saved file.

    Raises: ValueError if path traversal detected.
    """
```

### 3.3 `upload_handler.resolve_filename_conflict(filename: str, save_dir: str) -> str`

```python
def resolve_filename_conflict(filename: str, save_dir: str) -> str:
    """
    If filename exists, append (N) before extension.

    Examples:
      report.pdf -> report (1).pdf -> report (2).pdf
      photo.jpg  -> photo (1).jpg
      notes.txt  -> notes (1).txt
    """
```

### 3.4 Security: Path Validation

Reuse `security.validate_directory_path()` logic to ensure uploaded filename stays within save_dir. Reject filenames containing `/` or `..`.

## 4. CLI Changes

```python
# New arguments in cli.py parse_arguments():

parser.add_argument(
    "--upload",
    nargs="?",
    const=None,  # --upload with no value = current dir
    default=None,  # Not set = no upload mode
    help="Enable upload. Optionally specify save directory"
)

parser.add_argument(
    "--upload-password",
    type=str,
    default=None,
    help="Password required for uploading"
)
```

**Behavior matrix:**

| Command | Mode | Upload Save Dir |
|---------|------|----------------|
| `quick-share file.txt` | Share only | N/A |
| `quick-share --upload` | Upload only | CWD |
| `quick-share --upload /tmp` | Upload only | /tmp |
| `quick-share dir/ --upload` | Share + Upload | CWD |
| `quick-share dir/ --upload /tmp` | Share + Upload | /tmp |

## 5. Main Entry Point Changes

```python
def main() -> None:
    args = parse_arguments()

    if args.upload is not None:  # Upload enabled
        upload_save_dir = args.upload or os.getcwd()
        upload_save_dir = os.path.abspath(upload_save_dir)

        if not args.file_paths:
            # Standalone upload mode
            server = UploadServer(
                save_dir=upload_save_dir,
                port=..., timeout_minutes=...,
                max_sessions=...,
                upload_password=args.upload_password,
            )
        else:
            # Integrated share + upload
            # ... validate paths as usual ...
            server = MultiShareServer(
                paths=resolved_paths,
                port=..., timeout_minutes=...,
                max_sessions=...,
                upload_enabled=True,
                upload_save_dir=upload_save_dir,
                upload_password=args.upload_password,
            )
    else:
        # Share only (existing behavior)
        server = MultiShareServer(
            paths=resolved_paths,
            port=..., timeout_minutes=...,
            max_sessions=...,
        )
```

## 6. Logger Extension

Add upload-specific log formatters following the same pattern as download:

```python
format_upload_start(timestamp, client_ip, filename, file_size)   # [ts] ⬆️  ip - file (size)
format_upload_progress(timestamp, client_ip, transferred, total)  # [ts] ⬆️  ip - 1.2MB / 2.5MB (48%)
format_upload_complete(timestamp, client_ip, filename, total, duration)  # [ts] ✅ ip - Completed: file (2.5MB in 2.3s)
format_upload_interrupted(...)    # [ts] ⚠️  ip - Interrupted: file (1.2MB / 2.5MB)
format_upload_error(...)          # [ts] ❌ ip - Error: file - msg
```

Use upward arrow emoji (⬆️) to distinguish from download (⬇️).

## 7. Upload ProgressTracker

New class `UploadProgressTracker` (similar to `DownloadProgressTracker`):

```python
class UploadProgressTracker:
    def __init__(self, client_ip, filename, file_size):
        ...
    def update(self, chunk_size) -> bool:  # True if should log
    def complete(self):
```

## 8. Password Protection (R3)

```python
# UploadHandler.do_POST / MultiShareHandler._handle_upload():
password = self.server.upload_password
if password:
    # Check in order:
    # 1. X-Upload-Password header
    # 2. multipart form field "password"
    provided = (self.headers.get("X-Upload-Password")
                or self._get_form_field("password"))
    if provided != password:
        self.send_error(403, "Invalid upload password")
        return
```

Download/browse is NOT affected by upload password.

## 9. Quota Integration (R4)

The existing `max_sessions` in `DirectoryShareServer` and `MultiShareServer` counts unique browser sessions. Upload operations count as sessions too.

Implementation: `UploadServer.track_session()` uses the same `max_sessions` limit pattern. When `-n` is reached, new sessions (both upload and download) are rejected.

## 10. Upload Page HTML (Standalone)

Simple pure-HTML5 upload page with:
- File input (single select)
- Drag-and-drop zone
- Password field (if upload_password is set)
- Upload progress bar (XMLHttpRequest.upload.onprogress)
- Status message area

```html
<!DOCTYPE html>
<html>
<head>
  <title>Quick Share - Upload</title>
  <style>/* clean, minimal styling matching project aesthetic */</style>
</head>
<body>
  <h1>Upload Files</h1>
  <div id="dropzone">
    <p>Drag & drop files here or click to select</p>
    <input type="file" id="fileInput" multiple />
  </div>
  <div id="passwordSection" style="display:none">
    <input type="password" id="password" placeholder="Upload password" />
  </div>
  <progress id="progress" value="0" max="100"></progress>
  <div id="status"></div>
</body>
</html>
```

## 11. SPA Upload Integration (R2)

### Upload button in MultiShare SPA

Add to the SPA header actions area:
- An "Upload" button that toggles an inline upload form
- The form appears below the file list
- JavaScript sends POST /api/upload with FormData

### Upload button in DirectoryShare SPA

Similarly add upload form to the single-directory SPA page.

## 12. File List

### Documents generated

| File | Description |
|------|-------------|
| `src/upload_handler.py` | Core upload logic (new module) |
| `src/logger.py` | Upload log formatters (modified) |
| `src/cli.py` | --upload / --upload-password args (modified) |
| `src/main.py` | Upload mode detection, upload server creation (modified) |
| `src/server.py` | UploadServer class + MultiShareHandler.do_POST (modified) |
| `src/templates.py` | Upload page + SPA upload form (modified) |
| `src/upload_progress.html` | Standalone upload page template (new, or inline in templates.py) |

### Test files

| File | Description |
|------|-------------|
| `tests/test_upload_handler.py` | Tests for upload parsing, saving, conflict resolution |
| `tests/test_server_upload.py` | Tests for upload HTTP endpoints |

## 13. Performance Considerations

- **Large file uploads**: `cgi.FieldStorage` with `max_file_size` and `TemporaryFile` fallback for files > some threshold (stdlib handles this via `maxfbuf`)
- **Memory**: Files streamed in chunks (8KB, matching existing CHUNK_SIZE)
- **Concurrency**: Each upload in its own thread (ThreadingMixIn already in use)

## 14. Design Decisions & Trade-offs

1. **New UploadServer vs extending MultiShareServer**: New class for standalone mode keeps concerns separated. MultiShareServer gets upload options injected via constructor.

2. **cgi.FieldStorage vs manual multipart parsing**: `cgi.FieldStorage` is stdlib and handles boundary parsing correctly. Note: `cgi` module is deprecated in Python 3.11+ but still available. Future-proofing: we can switch to `multipart` module when needed.

3. **Upload via same port vs separate port**: Same port is simpler UX (one URL). Upload is a POST endpoint on the existing server.

4. **Password in header vs form field**: Both supported: `X-Upload-Password` header for programmatic access (curl), form field for browser uploads.

5. **SPA vs standalone HTML**: Standalone upload mode uses a simple pure-HTML5 page (no Vue dependency). Integrated mode adds upload UI to the existing Vue 3 SPA.
