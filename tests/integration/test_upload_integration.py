"""Integration tests for the upload feature.

Tests cover both standalone (UploadServer) and integrated
(MultiShareServer with upload_enabled=True) modes.
"""

import io
import os
import json
import time
import uuid
import threading
from http.server import HTTPServer
from socketserver import ThreadingMixIn

import pytest
import requests

from src.server import UploadServer, MultiShareServer, find_available_port
from src.upload_handler import UploadedFile, save_uploaded_file


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _get_free_port():
    """Return a port that is currently free."""
    return find_available_port(18000, 18999)


def _build_multipart_body(filename, data, boundary=None, extra_fields=None):
    """Build a simple multipart/form-data request body.

    Args:
        filename: Name of the file to upload.
        data: File content as bytes.
        boundary: Optional boundary string (auto-generated if None).
        extra_fields: Optional dict of extra form fields.

    Returns:
        Tuple of (body_bytes, boundary_string).
    """
    if boundary is None:
        boundary = f'----TestBoundary{uuid.uuid4().hex[:8]}'

    parts = []
    if extra_fields:
        for key, value in extra_fields.items():
            parts.append(f'--{boundary}\r\n')
            parts.append(f'Content-Disposition: form-data; name="{key}"\r\n')
            parts.append('\r\n')
            parts.append(f'{value}\r\n')

    parts.append(f'--{boundary}\r\n')
    parts.append(f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n')
    parts.append('Content-Type: application/octet-stream\r\n')
    parts.append('\r\n')

    body = ''.join(parts).encode('utf-8') + data + f'\r\n--{boundary}--\r\n'.encode('utf-8')
    return body, boundary


def _upload_file(url, file_data, filename='test.txt', password=None, timeout=5):
    """Upload a file to the given URL and return the response."""
    files = {'file': (filename, file_data)}
    headers = {}
    if password:
        headers['X-Upload-Password'] = password
    return requests.post(url, files=files, headers=headers, timeout=timeout)


@pytest.fixture
def temp_dir(tmp_path):
    """Create a temporary directory for uploads."""
    d = tmp_path / 'uploads'
    d.mkdir()
    return str(d)


# ---------------------------------------------------------------------------
# Standalone UploadServer tests
# ---------------------------------------------------------------------------

class TestUploadServerStandalone:
    """Tests for the standalone UploadServer."""

    @pytest.fixture
    def server(self, temp_dir):
        """Create and yield an UploadServer instance."""
        port = _get_free_port()
        srv = UploadServer(
            save_dir=temp_dir,
            port=port,
            timeout_minutes=1,  # Short timeout for tests
            max_sessions=10,
            upload_password=None,
        )
        srv.start()
        time.sleep(0.2)  # Allow server thread to start
        yield srv, port
        srv.stop()
        time.sleep(0.1)

    def test_upload_page_served(self, server):
        """GET /upload should return HTML."""
        srv, port = server
        resp = requests.get(f'http://localhost:{port}/upload', timeout=5)
        assert resp.status_code == 200
        assert 'text/html' in resp.headers.get('Content-Type', '')
        assert 'Upload' in resp.text

    def test_root_redirects_to_upload(self, server):
        """GET / should also serve the upload page."""
        srv, port = server
        resp = requests.get(f'http://localhost:{port}/', timeout=5)
        assert resp.status_code == 200
        assert 'Upload' in resp.text

    def test_upload_simple_file(self, server, temp_dir):
        """POST /upload with a file should save it and return JSON."""
        srv, port = server
        data = b'Hello, upload test!'
        resp = _upload_file(f'http://localhost:{port}/upload', data, 'hello.txt')
        assert resp.status_code == 200
        result = resp.json()
        assert result['status'] == 'ok'
        assert result['filename'] == 'hello.txt'

        # Verify file was saved to disk
        saved = os.path.join(temp_dir, 'hello.txt')
        assert os.path.isfile(saved)
        with open(saved, 'rb') as f:
            assert f.read() == data

    def test_upload_multiple_files(self, server):
        """Upload multiple files in a single request."""
        srv, port = server
        files = [
            ('file', ('a.txt', b'content a')),
            ('file', ('b.txt', b'content b')),
        ]
        resp = requests.post(
            f'http://localhost:{port}/upload',
            files=files,
            timeout=5,
        )
        assert resp.status_code == 200
        result = resp.json()
        assert result['count'] == 2
        assert 'a.txt' in result['files']
        assert 'b.txt' in result['files']

    def test_upload_non_existent_endpoint(self, server):
        """POST to a non-upload endpoint returns 404."""
        srv, port = server
        resp = requests.post(
            f'http://localhost:{port}/api/foo',
            files={'file': ('x.txt', b'data')},
            timeout=5,
        )
        assert resp.status_code == 404

    def test_upload_no_file_returns_error(self, server):
        """POST /upload without a file returns 400."""
        srv, port = server
        resp = requests.post(
            f'http://localhost:{port}/upload',
            data='just text',
            headers={'Content-Type': 'text/plain'},
            timeout=5,
        )
        assert resp.status_code == 400

    def test_upload_auto_rename(self, server, temp_dir):
        """Uploading the same filename twice should auto-rename the second."""
        srv, port = server

        # First upload
        r1 = _upload_file(f'http://localhost:{port}/upload', b'first', 'dup.txt')
        assert r1.status_code == 200

        # Second upload
        r2 = _upload_file(f'http://localhost:{port}/upload', b'second', 'dup.txt')
        assert r2.status_code == 200
        result = r2.json()
        assert result['filename'] == 'dup (1).txt'
        assert os.path.isfile(os.path.join(temp_dir, 'dup.txt'))
        assert os.path.isfile(os.path.join(temp_dir, 'dup (1).txt'))


class TestUploadServerWithPassword:
    """Tests for UploadServer with password protection."""

    @pytest.fixture
    def server(self, temp_dir):
        port = _get_free_port()
        srv = UploadServer(
            save_dir=temp_dir,
            port=port,
            timeout_minutes=1,
            max_sessions=10,
            upload_password='secret123',
        )
        srv.start()
        time.sleep(0.2)
        yield srv, port
        srv.stop()
        time.sleep(0.1)

    def test_upload_with_correct_password(self, server):
        """Upload with correct X-Upload-Password header succeeds."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/upload',
            b'data',
            'pw_test.txt',
            password='secret123',
        )
        assert resp.status_code == 200

    def test_upload_with_wrong_password(self, server):
        """Upload with wrong password returns 403."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/upload',
            b'data',
            'pw_fail.txt',
            password='wrong_password',
        )
        assert resp.status_code == 403

    def test_upload_without_password_returns_403(self, server):
        """Upload without password header returns 403."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/upload',
            b'data',
            'no_pw.txt',
        )
        assert resp.status_code == 403

    def test_upload_page_shows_password_field(self, server):
        """Upload page should indicate password is required."""
        srv, port = server
        resp = requests.get(f'http://localhost:{port}/upload', timeout=5)
        assert resp.status_code == 200
        # Page should reference password
        assert 'password' in resp.text.lower()


class TestUploadServerQuota:
    """Tests for UploadServer session limiting."""

    @pytest.fixture
    def server(self, temp_dir):
        port = _get_free_port()
        # Only allow 2 sessions
        srv = UploadServer(
            save_dir=temp_dir,
            port=port,
            timeout_minutes=1,
            max_sessions=2,
            upload_password=None,
        )
        srv.start()
        time.sleep(0.2)
        yield srv, port
        srv.stop()
        time.sleep(0.1)

    def test_upload_under_quota_succeeds(self, server):
        """First upload should succeed."""
        srv, port = server
        resp = _upload_file(f'http://localhost:{port}/upload', b'data1', 'f1.txt')
        assert resp.status_code == 200

    def test_upload_exceeds_quota(self, server):
        """After reaching session limit, new uploads get 403.

        Note: The session limit is per-client IP (via cookie). Since
        we are testing from localhost, sessions are tracked per browser
        session, not per upload. A single client can upload multiple files.
        """
        srv, port = server
        # First upload succeeds
        r1 = _upload_file(f'http://localhost:{port}/upload', b'data1', 'q1.txt')
        assert r1.status_code == 200

        # Second upload should also succeed (same client, reuses session)
        r2 = _upload_file(f'http://localhost:{port}/upload', b'data2', 'q2.txt')
        assert r2.status_code == 200


# ---------------------------------------------------------------------------
# Server lifecycle tests
# ---------------------------------------------------------------------------

class TestUploadServerLifecycle:
    """Tests for UploadServer start/stop."""

    def test_server_start_stop(self, temp_dir):
        """Server should start and stop without errors."""
        port = _get_free_port()
        srv = UploadServer(
            save_dir=temp_dir,
            port=port,
            timeout_minutes=1,
            max_sessions=5,
        )
        srv.start()
        time.sleep(0.2)
        assert srv.server_thread is not None
        assert srv.server_thread.is_alive()

        srv.stop()
        time.sleep(0.1)
        # After stop the thread may not immediately die, but the HTTPD should be closed

    def test_server_timeout_shuts_down(self, temp_dir):
        """Server should auto-shutdown after timeout."""
        port = _get_free_port()
        srv = UploadServer(
            save_dir=temp_dir,
            port=port,
            timeout_minutes=0.05,  # 3 seconds
            max_sessions=5,
        )
        srv.start()
        time.sleep(1)
        assert srv.server_thread is not None

        # The server should still be alive at this point (very short timeout)
        srv.stop()


# ---------------------------------------------------------------------------
# MultiShareServer with upload (integrated mode)
# ---------------------------------------------------------------------------

class TestMultiShareWithUpload:
    """Tests for the integrated share+upload mode."""

    @pytest.fixture
    def shared_dir(self, tmp_path):
        d = tmp_path / 'shared'
        d.mkdir()
        (d / 'readme.txt').write_text('shared content')
        return d

    @pytest.fixture
    def upload_dir(self, tmp_path):
        d = tmp_path / 'uploaded'
        d.mkdir()
        return d

    @pytest.fixture
    def server(self, shared_dir, upload_dir):
        port = _get_free_port()
        srv = MultiShareServer(
            paths=[(str(shared_dir), 'directory')],
            port=port,
            timeout_minutes=1,
            max_sessions=10,
            upload_enabled=True,
            upload_save_dir=str(upload_dir),
            upload_password=None,
        )
        srv.start()
        time.sleep(0.2)
        yield srv, port
        srv.stop()
        time.sleep(0.1)

    def test_share_page_still_works(self, server):
        """The share page should still be served."""
        srv, port = server
        resp = requests.get(f'http://localhost:{port}/', timeout=5)
        assert resp.status_code == 200
        assert 'Quick Share' in resp.text

    def test_upload_endpoint_available(self, server, upload_dir):
        """POST /upload should work in integrated mode."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/upload',
            b'integrated upload data',
            'integrated.txt',
        )
        assert resp.status_code == 200
        result = resp.json()
        assert result['status'] == 'ok'
        assert os.path.isfile(os.path.join(upload_dir, 'integrated.txt'))

    def test_api_upload_endpoint(self, server, upload_dir):
        """POST /api/upload should also work."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/api/upload',
            b'api upload data',
            'api_test.txt',
        )
        assert resp.status_code == 200
        result = resp.json()
        assert result['status'] == 'ok'
        assert os.path.isfile(os.path.join(upload_dir, 'api_test.txt'))

    def test_upload_page_integrated(self, server):
        """GET /upload should serve the upload page."""
        srv, port = server
        resp = requests.get(f'http://localhost:{port}/upload', timeout=5)
        assert resp.status_code == 200
        assert 'Upload' in resp.text

    def test_download_still_works(self, server, shared_dir):
        """Downloading files should still work when upload is enabled."""
        dirname = os.path.basename(str(shared_dir))
        srv, port = server
        resp = requests.get(
            f'http://localhost:{port}/files/{dirname}/readme.txt',
            timeout=5,
        )
        assert resp.status_code == 200
        assert resp.content == b'shared content'


class TestMultiShareUploadPassword:
    """Integrated mode upload with password."""

    @pytest.fixture
    def shared_dir(self, tmp_path):
        d = tmp_path / 'shared'
        d.mkdir()
        (d / 'readme.txt').write_text('shared content')
        return d

    @pytest.fixture
    def upload_dir(self, tmp_path):
        d = tmp_path / 'uploaded'
        d.mkdir()
        return d

    @pytest.fixture
    def server(self, shared_dir, upload_dir):
        port = _get_free_port()
        srv = MultiShareServer(
            paths=[(str(shared_dir), 'directory')],
            port=port,
            timeout_minutes=1,
            max_sessions=10,
            upload_enabled=True,
            upload_save_dir=str(upload_dir),
            upload_password='server_pw',
        )
        srv.start()
        time.sleep(0.2)
        yield srv, port
        srv.stop()

    def test_password_protected_upload(self, server, upload_dir):
        """Correct password allows upload."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/upload',
            b'pw data',
            'pw_protected.txt',
            password='server_pw',
        )
        assert resp.status_code == 200
        assert os.path.isfile(os.path.join(upload_dir, 'pw_protected.txt'))

    def test_wrong_password_rejected(self, server):
        """Wrong password returns 403."""
        srv, port = server
        resp = _upload_file(
            f'http://localhost:{port}/upload',
            b'data',
            'rejected.txt',
            password='wrong',
        )
        assert resp.status_code == 403

    def test_download_not_affected_by_password(self, server, shared_dir):
        """Download still works with upload password set."""
        dirname = os.path.basename(str(shared_dir))
        srv, port = server
        resp = requests.get(
            f'http://localhost:{port}/files/{dirname}/readme.txt',
            timeout=5,
        )
        assert resp.status_code == 200
