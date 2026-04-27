"""Unit tests for upload_handler module."""

import io
import os
import pytest
from src.upload_handler import (
    UploadedFile,
    UploadProgressTracker,
    parse_multipart_request,
    save_uploaded_file,
    resolve_filename_conflict,
    _split_ext,
    _reject_path_traversal,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _build_multipart(boundary, parts):
    """Build a multipart/form-data body.

    Args:
        boundary: The boundary string.
        parts: List of (field_name, filename, content_type, data_bytes).

    Returns:
        bytes: The complete multipart body.
    """
    body = b''
    for name, filename, content_type, data in parts:
        body += f'--{boundary}\r\n'.encode('utf-8')
        body += (
            f'Content-Disposition: form-data; name="{name}"; '
            f'filename="{filename}"\r\n'
        ).encode('utf-8')
        if content_type:
            body += f'Content-Type: {content_type}\r\n'.encode('utf-8')
        body += b'\r\n'
        body += data
        body += b'\r\n'
    body += f'--{boundary}--\r\n'.encode('utf-8')
    return body


# ---------------------------------------------------------------------------
# parse_multipart_request tests
# ---------------------------------------------------------------------------

class TestParseMultipartRequest:

    def test_parse_multipart_simple_file(self):
        """Parse a single file from a multipart request."""
        boundary = '----TestBoundary123'
        content = b'Hello, this is test file content.'
        body = _build_multipart(boundary, [
            ('file', 'test.txt', 'text/plain', content),
        ])
        content_type = f'multipart/form-data; boundary={boundary}'
        rfile = io.BytesIO(body)

        results = parse_multipart_request(
            content_type=content_type,
            content_length=len(body),
            rfile=rfile,
        )

        assert len(results) == 1
        f = results[0]
        assert f.field_name == 'file'
        assert f.filename == 'test.txt'
        assert f.content_type == 'text/plain'
        assert f.data == content
        assert f.size == len(content)

    def test_parse_multipart_multiple_files(self):
        """Parse multiple files in one multipart request."""
        boundary = '----MultiBoundary'
        content_a = b'Content of file A.'
        content_b = b'Content of file B.'
        body = _build_multipart(boundary, [
            ('files', 'a.txt', 'text/plain', content_a),
            ('files', 'b.txt', 'text/plain', content_b),
        ])
        content_type = f'multipart/form-data; boundary={boundary}'
        rfile = io.BytesIO(body)

        results = parse_multipart_request(
            content_type=content_type,
            content_length=len(body),
            rfile=rfile,
        )

        assert len(results) == 2
        assert results[0].filename == 'a.txt'
        assert results[0].data == content_a
        assert results[1].filename == 'b.txt'
        assert results[1].data == content_b

    def test_parse_multipart_binary_content(self):
        """Parse a file with binary content."""
        boundary = '----BinaryBoundary'
        content = bytes(range(256))  # All byte values
        body = _build_multipart(boundary, [
            ('file', 'data.bin', 'application/octet-stream', content),
        ])
        content_type = f'multipart/form-data; boundary={boundary}'
        rfile = io.BytesIO(body)

        results = parse_multipart_request(
            content_type=content_type,
            content_length=len(body),
            rfile=rfile,
        )

        assert len(results) == 1
        assert results[0].data == content

    def test_parse_multipart_no_content_type_header(self):
        """Field without explicit Content-Type defaults to text/plain (cgi.FieldStorage default)."""
        boundary = '----NoCT'
        # Build body without Content-Type header for the part
        body = b''
        body += f'--{boundary}\r\n'.encode('utf-8')
        body += f'Content-Disposition: form-data; name="file"; filename="data.bin"\r\n'.encode('utf-8')
        body += b'\r\n'
        body += b'some binary data'
        body += b'\r\n'
        body += f'--{boundary}--\r\n'.encode('utf-8')

        content_type = f'multipart/form-data; boundary={boundary}'
        rfile = io.BytesIO(body)

        results = parse_multipart_request(
            content_type=content_type,
            content_length=len(body),
            rfile=rfile,
        )

        assert len(results) == 1
        assert results[0].content_type == 'text/plain'

    def test_parse_multipart_rejects_path_traversal(self):
        """parse_multipart_request rejects filenames containing ../."""
        boundary = '----TestTraversal'
        content = b'malicious'
        body = _build_multipart(boundary, [
            ('file', '../etc/passwd', 'text/plain', content),
        ])
        content_type = f'multipart/form-data; boundary={boundary}'
        rfile = io.BytesIO(body)

        with pytest.raises(ValueError, match='Path traversal'):
            parse_multipart_request(
                content_type=content_type,
                content_length=len(body),
                rfile=rfile,
            )

    def test_invalid_content_type_raises(self):
        """ValueError when Content-Type is not multipart/form-data."""
        rfile = io.BytesIO(b'')
        with pytest.raises(ValueError, match='multipart/form-data'):
            parse_multipart_request(
                content_type='text/plain',
                content_length=0,
                rfile=rfile,
            )

    def test_empty_content_type_raises(self):
        """Empty Content-Type raises ValueError."""
        rfile = io.BytesIO(b'')
        with pytest.raises(ValueError, match='multipart/form-data'):
            parse_multipart_request(
                content_type='',
                content_length=0,
                rfile=rfile,
            )


# ---------------------------------------------------------------------------
# save_uploaded_file tests
# ---------------------------------------------------------------------------

class TestSaveUploadedFile:

    def test_save_file(self, tmp_path):
        """Save an uploaded file to disk and verify content."""
        save_dir = str(tmp_path)
        data = b'Hello, this is saved file content!'
        uploaded = UploadedFile(
            field_name='file',
            filename='saved.txt',
            content_type='text/plain',
            data=data,
            size=len(data),
        )

        result_path = save_uploaded_file(uploaded, save_dir)

        assert os.path.isfile(result_path)
        assert result_path == os.path.join(save_dir, 'saved.txt')
        with open(result_path, 'rb') as f:
            assert f.read() == data

    def test_save_file_missing_directory_raises(self, tmp_path):
        """NotADirectoryError raised when save_dir doesn't exist."""
        save_dir = os.path.join(str(tmp_path), 'nonexistent')
        uploaded = UploadedFile(
            field_name='file',
            filename='test.txt',
            content_type='text/plain',
            data=b'data',
            size=4,
        )

        with pytest.raises(NotADirectoryError):
            save_uploaded_file(uploaded, save_dir)

    def test_save_file_with_tracker(self, tmp_path):
        """save_uploaded_file updates and completes the progress tracker."""
        save_dir = str(tmp_path)
        data = b'x' * 20000  # Large enough to trigger chunked writing
        uploaded = UploadedFile(
            field_name='file',
            filename='large.bin',
            content_type='application/octet-stream',
            data=data,
            size=len(data),
        )
        tracker = UploadProgressTracker('127.0.0.1', 'large.bin', len(data))

        result_path = save_uploaded_file(uploaded, save_dir, tracker=tracker)

        assert os.path.isfile(result_path)
        assert tracker.is_complete is True
        assert tracker.bytes_transferred >= len(data)

    def test_save_file_with_conflict(self, tmp_path):
        """save_uploaded_file auto-renames when file exists."""
        save_dir = str(tmp_path)
        # Pre-create the file
        (tmp_path / 'test.txt').write_text('existing')

        uploaded = UploadedFile(
            field_name='file',
            filename='test.txt',
            content_type='text/plain',
            data=b'new content',
            size=11,
        )

        result_path = save_uploaded_file(uploaded, save_dir)

        assert os.path.isfile(result_path)
        assert os.path.basename(result_path) == 'test (1).txt'
        with open(result_path, 'rb') as f:
            assert f.read() == b'new content'


# ---------------------------------------------------------------------------
# resolve_filename_conflict tests
# ---------------------------------------------------------------------------

class TestResolveFilenameConflict:

    def test_resolve_filename_no_conflict(self, tmp_path):
        """Returns original filename when no conflict exists."""
        result = resolve_filename_conflict('file.txt', str(tmp_path))
        assert result == 'file.txt'

    def test_resolve_filename_first_conflict(self, tmp_path):
        """file.txt -> file (1).txt when file.txt exists."""
        (tmp_path / 'file.txt').write_text('existing')
        result = resolve_filename_conflict('file.txt', str(tmp_path))
        assert result == 'file (1).txt'

    def test_resolve_filename_second_conflict(self, tmp_path):
        """file.txt -> file (2).txt when file.txt and file (1).txt exist."""
        (tmp_path / 'file.txt').write_text('existing')
        (tmp_path / 'file (1).txt').write_text('also existing')
        result = resolve_filename_conflict('file.txt', str(tmp_path))
        assert result == 'file (2).txt'

    def test_resolve_filename_complex_ext(self, tmp_path):
        """file.tar.gz -> file (1).tar.gz (preserves compound extension)."""
        (tmp_path / 'file.tar.gz').write_text('existing')
        result = resolve_filename_conflict('file.tar.gz', str(tmp_path))
        assert result == 'file (1).tar.gz'

    def test_resolve_filename_no_extension(self, tmp_path):
        """File with no extension gets suffix appended."""
        (tmp_path / 'README').write_text('existing')
        result = resolve_filename_conflict('README', str(tmp_path))
        assert result == 'README (1)'

    def test_resolve_filename_higher_counter(self, tmp_path):
        """Increments counter until a free name is found."""
        (tmp_path / 'notes.txt').write_text('v0')
        (tmp_path / 'notes (1).txt').write_text('v1')
        (tmp_path / 'notes (2).txt').write_text('v2')
        result = resolve_filename_conflict('notes.txt', str(tmp_path))
        assert result == 'notes (3).txt'


# ---------------------------------------------------------------------------
# _split_ext tests
# ---------------------------------------------------------------------------

class TestSplitExt:

    def test_simple_extension(self):
        assert _split_ext('file.txt') == ('file', '.txt')

    def test_compound_extension(self):
        assert _split_ext('archive.tar.gz') == ('archive', '.tar.gz')

    def test_no_extension(self):
        assert _split_ext('README') == ('README', '')

    def test_dotfile(self):
        assert _split_ext('.gitignore') == ('', '.gitignore')

    def test_multiple_dots(self):
        assert _split_ext('my.file.name.txt') == ('my', '.file.name.txt')


# ---------------------------------------------------------------------------
# _reject_path_traversal tests
# ---------------------------------------------------------------------------

class TestRejectPathTraversal:

    def test_reject_path_traversal_double_dot(self):
        """Filename with ../ raises ValueError."""
        with pytest.raises(ValueError, match='Path traversal'):
            _reject_path_traversal('../etc/passwd')

    def test_reject_path_traversal_forward_slash(self):
        """Filename with / raises ValueError."""
        with pytest.raises(ValueError, match='Path traversal'):
            _reject_path_traversal('etc/passwd')

    def test_reject_path_traversal_backslash(self):
        """Filename with \\ raises ValueError."""
        with pytest.raises(ValueError, match='Path traversal'):
            _reject_path_traversal('etc\\passwd')

    def test_reject_empty_filename(self):
        """Empty filename raises ValueError."""
        with pytest.raises(ValueError, match='Empty filename'):
            _reject_path_traversal('')

    def test_accept_normal_filename(self):
        """Normal filenames do not raise."""
        _reject_path_traversal('test.txt')
        _reject_path_traversal('photo.jpg')
        _reject_path_traversal('archive.tar.gz')


# ---------------------------------------------------------------------------
# UploadProgressTracker tests
# ---------------------------------------------------------------------------

class TestUploadProgressTracker:

    def test_tracker_initialization(self):
        """UploadProgressTracker initializes with correct defaults."""
        tracker = UploadProgressTracker(
            client_ip='192.168.1.100',
            filename='test.zip',
            file_size=2500000,
        )

        assert tracker.client_ip == '192.168.1.100'
        assert tracker.filename == 'test.zip'
        assert tracker.file_size == 2500000
        assert tracker.bytes_transferred == 0
        assert tracker.is_complete is False
        assert isinstance(tracker.start_time, float)

    def test_tracker_update_returns_logging_flag(self):
        """update() returns True every 10 chunks or at completion."""
        tracker = UploadProgressTracker('192.168.1.100', 'test.bin', 100000)

        # 9 chunks of 8KB: no logging
        for _ in range(9):
            assert tracker.update(8192) is False

        # 10th chunk triggers logging (10 * 8192 = 81920)
        assert tracker.update(8192) is True
        assert tracker.bytes_transferred == 81920

    def test_tracker_update_at_completion(self):
        """update() returns True when bytes reach or exceed file_size."""
        tracker = UploadProgressTracker('127.0.0.1', 'small.txt', 100)
        should_log = tracker.update(100)
        assert tracker.bytes_transferred == 100
        assert should_log is True

    def test_tracker_complete(self):
        """complete() marks the upload as finished."""
        tracker = UploadProgressTracker('127.0.0.1', 'file.zip', 1000)
        assert tracker.is_complete is False
        tracker.complete()
        assert tracker.is_complete is True

    def test_tracker_progress_percentage(self):
        """get_progress_percentage() returns correct values."""
        tracker = UploadProgressTracker('127.0.0.1', 'file.zip', 2500000)

        assert tracker.get_progress_percentage() == 0.0

        tracker.bytes_transferred = 1250000
        assert tracker.get_progress_percentage() == 50.0

        tracker.bytes_transferred = 2500000
        assert tracker.get_progress_percentage() == 100.0

    def test_tracker_progress_percentage_zero_size(self):
        """Zero file size returns 0.0% without division error."""
        tracker = UploadProgressTracker('127.0.0.1', 'empty.txt', 0)
        assert tracker.get_progress_percentage() == 0.0

    def test_tracker_progress_percentage_negative_size(self):
        """Negative file size returns 0.0%."""
        tracker = UploadProgressTracker('127.0.0.1', 'unknown.bin', -1)
        assert tracker.get_progress_percentage() == 0.0
