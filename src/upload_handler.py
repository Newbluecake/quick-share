"""Upload handling module for Quick Share.

Provides core upload processing logic including multipart form-data parsing,
file saving, auto-rename on filename conflicts, and upload progress tracking.

All functions use pure Python standard library with no external dependencies.
"""

import os
import cgi
import uuid
import time
import tempfile
from typing import Optional, NamedTuple, List, BinaryIO
from urllib.parse import urlparse


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------

class UploadedFile(NamedTuple):
    """Represents a single uploaded file parsed from a multipart request."""

    field_name: str
    """Name of the form field."""

    filename: str
    """Original filename provided by the client."""

    content_type: str
    """MIME type of the uploaded file."""

    data: bytes
    """Raw file content."""

    size: int
    """File size in bytes."""


class UploadProgressTracker:
    """Track upload progress for a single client connection.

    Each connection gets its own instance so no thread synchronisation
    is needed.
    """

    CHUNK_SIZE = 8192  # 8 KB, matches DownloadProgressTracker

    def __init__(self, client_ip: str, filename: str, file_size: int):
        """
        Args:
            client_ip: IP address of the uploading client.
            filename: Name of the file being uploaded.
            file_size: Total expected file size in bytes (0 if unknown).
        """
        self.client_ip = client_ip
        self.filename = filename
        self.file_size = file_size
        self.bytes_transferred = 0
        self.start_time = time.time()
        self.is_complete = False

    def update(self, chunk_size: int) -> bool:
        """Advance progress by *chunk_size* bytes.

        Returns:
            True if the caller should print a progress log line
            (approximately every 80 KiB or at completion).
        """
        self.bytes_transferred += chunk_size
        return (
            self.bytes_transferred % (self.CHUNK_SIZE * 10) == 0
            or (self.file_size > 0
                and self.bytes_transferred >= self.file_size)
        )

    def complete(self):
        """Mark the upload as finished."""
        self.is_complete = True

    def get_progress_percentage(self) -> float:
        """Return progress as a float between 0 and 100."""
        if self.file_size <= 0:
            return 0.0
        return min((self.bytes_transferred / self.file_size) * 100, 100.0)


# ---------------------------------------------------------------------------
# Multipart request parsing
# ---------------------------------------------------------------------------

def parse_multipart_request(
    content_type: str,
    content_length: int,
    rfile: BinaryIO,
    max_form_memory: int = 1024 * 1024,
    include_form_fields: bool = False,
):
    """Parse a ``multipart/form-data`` POST body using stdlib facilities.

    Args:
        content_type: Value of the ``Content-Type`` header (must contain
            ``multipart/form-data`` with a ``boundary`` parameter).
        content_length: Value of the ``Content-Length`` header.
        rfile: The input stream (e.g. ``handler.rfile``).
        max_form_memory: Maximum in-memory buffer per field before falling
            back to a temporary file (see :class:`cgi.FieldStorage`).
        include_form_fields: If True, returns a tuple ``(files, form_fields)``
            where *form_fields* is a dict of non-file form field values.

    Returns:
        A list of :class:`UploadedFile` namedtuples, or a tuple
        ``(files, form_fields)`` if *include_form_fields* is True.

    Raises:
        ValueError: If *content_type* is not a valid multipart content type
            or the boundary parameter is missing.
    """
    if not content_type or 'multipart/form-data' not in content_type:
        raise ValueError(
            "Content-Type must be multipart/form-data with a boundary parameter"
        )

    # cgi.FieldStorage needs the environment-like dict for its internal
    # query string parsing.  We provide the minimum required entries.
    env = {
        'REQUEST_METHOD': 'POST',
        'CONTENT_TYPE': content_type,
        'CONTENT_LENGTH': str(content_length),
    }

    try:
        form = cgi.FieldStorage(
            fp=rfile,
            environ=env,
            keep_blank_values=False,
            strict_parsing=False,
        )
    except Exception as exc:
        raise ValueError(f"Failed to parse multipart request: {exc}") from exc

    uploaded: List[UploadedFile] = []
    form_fields: dict = {}

    # FieldStorage can yield a single item or a list of items.
    items: list = form.list if form.list is not None else \
        ([form] if form.filename else [])

    for field in items:
        if not field.filename:
            # Non-file field – collect for later use (password, metadata).
            value = field.value
            if isinstance(value, bytes):
                value = value.decode('utf-8', errors='replace')
            form_fields[field.name or ''] = value or ''
            continue

        raw_data: bytes = b''
        if field.file is not None:
            # Data was written to a (possibly temporary) file on disk.
            field.file.seek(0)
            raw_data = field.file.read()
        elif field.value is not None:
            if isinstance(field.value, str):
                raw_data = field.value.encode('utf-8')
            else:
                raw_data = field.value

        # Basic security: reject filenames with path separators.
        filename = field.filename
        _reject_path_traversal(filename)

        uploaded.append(UploadedFile(
            field_name=field.name or '',
            filename=os.path.basename(filename),
            content_type=field.type or 'application/octet-stream',
            data=raw_data,
            size=len(raw_data),
        ))

    if include_form_fields:
        return uploaded, form_fields
    return uploaded


# ---------------------------------------------------------------------------
# File saving
# ---------------------------------------------------------------------------

def save_uploaded_file(
    uploaded: UploadedFile,
    save_dir: str,
    tracker: Optional[UploadProgressTracker] = None,
) -> str:
    """Write an uploaded file to *save_dir*.

    The file is first written to a temporary path inside *save_dir* and then
    atomically renamed to the final (potentially conflict-resolved) name.
    This prevents partial writes from being visible to concurrent readers.

    Args:
        uploaded: The parsed uploaded file descriptor.
        save_dir: Absolute path of the directory to save into.
        tracker: Optional progress tracker to update during the write.

    Returns:
        The **absolute path** of the saved file.

    Raises:
        ValueError: If the resolved path would escape *save_dir*.
        OSError: If the write fails.
    """
    save_dir = os.path.abspath(save_dir)
    if not os.path.isdir(save_dir):
        raise NotADirectoryError(f"Save directory not found: {save_dir}")

    # Resolve the final filename (with conflict handling).
    final_name = resolve_filename_conflict(uploaded.filename, save_dir)
    final_path = os.path.join(save_dir, final_name)

    # Verify the final path stays inside save_dir.
    real_final = os.path.realpath(final_path)
    real_save = os.path.realpath(save_dir)
    try:
        if os.path.commonpath([real_final, real_save]) != real_save:
            raise ValueError(
                f"Path traversal detected: {uploaded.filename}"
            )
    except ValueError:
        raise ValueError(
            f"Path traversal detected: {uploaded.filename}"
        )

    # Write via a temporary file then atomically rename.
    tmp_path = os.path.join(
        save_dir,
        f'.upload_tmp_{uuid.uuid4().hex}_{uploaded.filename}',
    )
    try:
        write_size = 0
        chunk_size = 8192
        with open(tmp_path, 'wb') as f:
            # Write content in chunks for large files.
            data = uploaded.data
            if len(data) <= chunk_size:
                f.write(data)
                write_size = len(data)
            else:
                offset = 0
                while offset < len(data):
                    chunk = data[offset:offset + chunk_size]
                    f.write(chunk)
                    write_size += len(chunk)
                    offset += chunk_size
                    if tracker:
                        if tracker.update(len(chunk)):
                            pass  # Caller can check percentage externally.

        # If we have a tracker and it was updated, mark complete.
        if tracker:
            tracker.complete()

        os.replace(tmp_path, final_path)
    except BaseException:
        # Clean up the temp file on any error.
        try:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)
        except OSError:
            pass
        raise

    return final_path


# ---------------------------------------------------------------------------
# Conflict resolution
# ---------------------------------------------------------------------------

def resolve_filename_conflict(filename: str, save_dir: str) -> str:
    """Return a filename that does not exist in *save_dir*.

    If *filename* already exists, a numeric suffix is inserted before the
    extension, incrementing until a free name is found.

    Examples::

        report.pdf   -> report (1).pdf   (when report.pdf exists)
        photo.jpg    -> photo (1).jpg    (when photo.jpg exists)
        archive.tar.gz -> archive (1).tar.gz
        notes.txt    -> notes (1).txt    (when notes.txt exists)
    """
    name, ext = _split_ext(filename)
    candidate = filename
    counter = 1

    while os.path.exists(os.path.join(save_dir, candidate)):
        candidate = f"{name} ({counter}){ext}"
        counter += 1

    return candidate


def _split_ext(filename: str):
    """Split *filename* into ``(base, extension)``.

    Unlike ``os.path.splitext`` this handles compound extensions such as
    ``.tar.gz`` by splitting on the **first** dot rather than the last.
    """
    dot_index = filename.find('.')
    if dot_index == -1:
        return filename, ''
    return filename[:dot_index], filename[dot_index:]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _reject_path_traversal(filename: str):
    """Raise :class:`ValueError` if *filename* contains path separators.

    This is a basic sanity check; the real sandbox is enforced by the
    ``commonpath`` check in :func:`save_uploaded_file`.
    """
    if not filename:
        raise ValueError("Empty filename rejected")
    if '/' in filename or '\\' in filename or '..' in filename:
        raise ValueError(
            f"Path traversal detected in filename: {filename}"
        )
