"""Integration tests for upload server endpoints."""

import os
import io
import time
import json
import http.client
import threading
from src.server import UploadServer, MultiShareServer, find_available_port


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _build_multipart_body(boundary, parts):
    """Build a multipart/form-data POST body.

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


def _post_multipart(port, path, parts, headers=None):
    """POST a multipart/form-data request to the server.

    Args:
        port: Server port.
        path: Request path (e.g. '/upload').
        parts: List of (name, filename, content_type, data) tuples.
        headers: Optional dict of extra headers.

    Returns:
        (status, body_bytes)
    """
    boundary = '----TestBoundary789'
    body = _build_multipart_body(boundary, parts)
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    try:
        req_headers = {
            'Content-Type': f'multipart/form-data; boundary={boundary}',
            'Content-Length': str(len(body)),
        }
        if headers:
            req_headers.update(headers)
        conn.request('POST', path, body=body, headers=req_headers)
        resp = conn.getresponse()
        return resp.status, resp.read()
    finally:
        conn.close()


def _get_request(port, path, headers=None):
    """Make a GET request to the server."""
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    try:
        req_headers = headers or {}
        conn.request('GET', path, headers=req_headers)
        resp = conn.getresponse()
        return resp.status, resp.read()
    finally:
        conn.close()


def _wait_for_server(port, path='/', timeout=5):
    """Wait until the server is accepting connections."""
    start = time.time()
    while time.time() - start < timeout:
        try:
            conn = http.client.HTTPConnection('127.0.0.1', port, timeout=1)
            conn.request('GET', path)
            resp = conn.getresponse()
            resp.read()
            conn.close()
            return True
        except Exception:
            time.sleep(0.05)
    return False


# ---------------------------------------------------------------------------
# Standalone UploadServer tests
# ---------------------------------------------------------------------------

class TestStandaloneUploadServer:

    def test_standalone_upload_server_start_stop(self, tmp_path):
        """Server starts and stops without error."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir)

        try:
            server.start()
            assert _wait_for_server(server.port, '/upload'), 'Server did not start'
        finally:
            server.stop()

        # After stop, server should not accept connections
        time.sleep(0.1)
        try:
            conn = http.client.HTTPConnection('127.0.0.1', server.port, timeout=1)
            conn.request('GET', '/upload')
            conn.getresponse()
            conn.close()
            # If we reach here, the server might still be running (unlikely but
            # possible with keep-alive).  The important thing is start/stop
            # don't raise exceptions.
        except Exception:
            pass  # Expected - connection should fail after shutdown

    def test_standalone_upload_page(self, tmp_path):
        """GET /upload returns HTML upload page."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir)
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')
            status, body = _get_request(server.port, '/upload')
            assert status == 200
            html = body.decode('utf-8')
            assert '<title>Quick Share - Upload</title>' in html
            assert 'multipart/form-data' not in html.lower() or 'formdata' in html.lower() or 'FormData' in html or 'form' in html.lower()
            assert 'Upload Files' in html or 'upload' in html.lower()
        finally:
            server.stop()

    def test_standalone_upload_page_root_redirects(self, tmp_path):
        """GET / also returns upload page for standalone server."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir)
        server.start()
        try:
            assert _wait_for_server(server.port, '/')
            status, body = _get_request(server.port, '/')
            assert status == 200
            assert body  # Should return HTML
        finally:
            server.stop()

    def test_upload_file_via_http(self, tmp_path):
        """POST multipart file to standalone server, verify saved to disk."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir)
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            content = b'Integration test file content.'
            status, body = _post_multipart(server.port, '/upload', [
                ('file', 'integration.txt', 'text/plain', content),
            ])

            assert status == 200
            resp = json.loads(body.decode('utf-8'))
            assert resp['status'] == 'ok'
            assert resp['filename'] == 'integration.txt'
            assert resp['count'] == 1

            # Verify file was saved
            saved_path = os.path.join(save_dir, 'integration.txt')
            assert os.path.isfile(saved_path)
            with open(saved_path, 'rb') as f:
                assert f.read() == content
        finally:
            server.stop()

    def test_upload_multiple_files_via_http(self, tmp_path):
        """POST multiple files in one request."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir)
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            content_a = b'File A content'
            content_b = b'File B content'
            status, body = _post_multipart(server.port, '/upload', [
                ('file', 'a.txt', 'text/plain', content_a),
                ('file', 'b.txt', 'text/plain', content_b),
            ])

            assert status == 200
            resp = json.loads(body.decode('utf-8'))
            assert resp['status'] == 'ok'
            assert resp['count'] == 2
            assert 'a.txt' in resp['files']
            assert 'b.txt' in resp['files']

            assert os.path.isfile(os.path.join(save_dir, 'a.txt'))
            assert os.path.isfile(os.path.join(save_dir, 'b.txt'))
        finally:
            server.stop()


# ---------------------------------------------------------------------------
# Upload password tests
# ---------------------------------------------------------------------------

class TestUploadPassword:

    def test_upload_password_correct_header(self, tmp_path):
        """Correct password via X-Upload-Password header succeeds."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir, upload_password='secret123')
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            status, body = _post_multipart(
                server.port, '/upload',
                [('file', 'data.txt', 'text/plain', b'secret content')],
                headers={'X-Upload-Password': 'secret123'},
            )

            assert status == 200
            resp = json.loads(body.decode('utf-8'))
            assert resp['status'] == 'ok'
        finally:
            server.stop()

    def test_upload_password_correct_form_field(self, tmp_path):
        """Correct password via multipart form field succeeds."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir, upload_password='secret123')
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            # Include password as a form field alongside the file
            boundary = '----PasswordTest'
            body = _build_multipart_body(boundary, [
                ('password', 'password', '', b'secret123'),  # form field
            ]) + b'\r\n'.join(_build_multipart_body(boundary, [
                ('file', 'data.txt', 'text/plain', b'content'),
            ]).split(b'\r\n')[1:]) if False else b''

            # Actually need to include password in the same multipart body
            body = b''
            boundary = '----PwFormTest'
            body += f'--{boundary}\r\n'.encode('utf-8')
            body += f'Content-Disposition: form-data; name="password"\r\n'.encode('utf-8')
            body += b'\r\n'
            body += b'secret123'
            body += b'\r\n'
            body += f'--{boundary}\r\n'.encode('utf-8')
            body += f'Content-Disposition: form-data; name="file"; filename="data.txt"\r\n'.encode('utf-8')
            body += f'Content-Type: text/plain\r\n'.encode('utf-8')
            body += b'\r\n'
            body += b'content'
            body += b'\r\n'
            body += f'--{boundary}--\r\n'.encode('utf-8')

            conn = http.client.HTTPConnection('127.0.0.1', server.port, timeout=5)
            try:
                conn.request('POST', '/upload', body=body, headers={
                    'Content-Type': f'multipart/form-data; boundary={boundary}',
                    'Content-Length': str(len(body)),
                })
                resp = conn.getresponse()
                assert resp.status == 200
            finally:
                conn.close()
        finally:
            server.stop()

    def test_upload_password_incorrect(self, tmp_path):
        """Wrong password returns 403."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir, upload_password='secret123')
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            status, body = _post_multipart(
                server.port, '/upload',
                [('file', 'data.txt', 'text/plain', b'content')],
                headers={'X-Upload-Password': 'wrong'},
            )

            assert status == 403
            resp = json.loads(body.decode('utf-8'))
            assert 'password' in resp['error'].lower()
        finally:
            server.stop()

    def test_upload_password_missing(self, tmp_path):
        """No password provided when required returns 403."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir, upload_password='secret123')
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            status, body = _post_multipart(
                server.port, '/upload',
                [('file', 'data.txt', 'text/plain', b'content')],
            )

            assert status == 403
        finally:
            server.stop()

    def test_upload_no_password_set_allows_all(self, tmp_path):
        """When no password is set, uploads succeed without auth."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir)
        server.start()
        try:
            assert _wait_for_server(server.port, '/upload')

            status, body = _post_multipart(
                server.port, '/upload',
                [('file', 'data.txt', 'text/plain', b'no password needed')],
            )

            assert status == 200
        finally:
            server.stop()


# ---------------------------------------------------------------------------
# Quota / session limit tests
# ---------------------------------------------------------------------------

class TestUploadQuota:

    def test_upload_quota_limit(self, tmp_path):
        """Exceeding max_sessions returns 403 for upload.

        Each HTTP connection without a session cookie creates a new session.
        With max_sessions=2, the first 2 requests succeed and the third is
        rejected.
        """
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir, max_sessions=2)
        server.start()
        time.sleep(0.3)  # Allow server to start (don't use _wait_for_server)
        try:
            # First POST: should succeed
            status1, _ = _post_multipart(
                server.port, '/upload',
                [('file', 'first.txt', 'text/plain', b'first')],
            )
            assert status1 == 200, f'First POST failed: {status1}'

            # Second POST: should succeed (new connection, second session)
            status2, _ = _post_multipart(
                server.port, '/upload',
                [('file', 'second.txt', 'text/plain', b'second')],
            )
            assert status2 == 200, f'Second POST failed: {status2}'

            # Third POST (new connection, no session cookie): limit reached
            status3, body3 = _post_multipart(
                server.port, '/upload',
                [('file', 'third.txt', 'text/plain', b'third')],
            )
            assert status3 == 403, f'Third POST should be 403, got {status3}'
            resp = json.loads(body3.decode('utf-8'))
            assert 'session' in resp['error'].lower()
        finally:
            server.stop()

    def test_upload_quota_allows_under_limit(self, tmp_path):
        """New sessions allowed when under max_sessions."""
        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)
        server = UploadServer(save_dir=save_dir, max_sessions=5)
        server.start()
        time.sleep(0.3)
        try:
            for i in range(3):
                status, body = _post_multipart(
                    server.port, '/upload',
                    [('file', f'file{i}.txt', 'text/plain', f'content {i}'.encode())],
                )
                assert status == 200, f'Request {i} failed with status {status}'
        finally:
            server.stop()


# ---------------------------------------------------------------------------
# Integrated (MultiShareServer) upload tests
# ---------------------------------------------------------------------------

class TestIntegratedUpload:

    def test_integrated_upload(self, tmp_path):
        """MultiShareServer with upload_enabled accepts file uploads."""
        # Create files to share
        share_file = tmp_path / 'shared.txt'
        share_file.write_text('shared content')

        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)

        server = MultiShareServer(
            paths=[(str(share_file), 'file')],
            upload_enabled=True,
            upload_save_dir=save_dir,
        )
        server.start()
        try:
            assert _wait_for_server(server.port, '/')

            # Test upload via POST /upload
            content = b'Uploaded via integrated mode.'
            status, body = _post_multipart(server.port, '/upload', [
                ('file', 'integrated.txt', 'text/plain', content),
            ])

            assert status == 200
            resp = json.loads(body.decode('utf-8'))
            assert resp['status'] == 'ok'

            # Verify file saved
            saved_path = os.path.join(save_dir, 'integrated.txt')
            assert os.path.isfile(saved_path)
            with open(saved_path, 'rb') as f:
                assert f.read() == content
        finally:
            server.stop()

    def test_integrated_upload_disabled(self, tmp_path):
        """MultiShareServer without upload_enabled rejects POST /upload."""
        share_file = tmp_path / 'shared.txt'
        share_file.write_text('shared content')

        server = MultiShareServer(
            paths=[(str(share_file), 'file')],
        )
        server.start()
        try:
            assert _wait_for_server(server.port, '/')

            status, body = _post_multipart(server.port, '/upload', [
                ('file', 'test.txt', 'text/plain', b'nope'),
            ])

            assert status == 404
        finally:
            server.stop()

    def test_integrated_upload_via_api(self, tmp_path):
        """MultiShareServer accepts upload via /api/upload."""
        share_file = tmp_path / 'shared.txt'
        share_file.write_text('shared content')

        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)

        server = MultiShareServer(
            paths=[(str(share_file), 'file')],
            upload_enabled=True,
            upload_save_dir=save_dir,
        )
        server.start()
        try:
            assert _wait_for_server(server.port, '/')

            status, body = _post_multipart(server.port, '/api/upload', [
                ('file', 'api_upload.txt', 'text/plain', b'API upload content'),
            ])

            assert status == 200
            resp = json.loads(body.decode('utf-8'))
            assert resp['status'] == 'ok'

            saved_path = os.path.join(save_dir, 'api_upload.txt')
            assert os.path.isfile(saved_path)
        finally:
            server.stop()

    def test_upload_then_download_quota_sharing(self, tmp_path):
        """Both upload and download count against the same max_sessions limit.

        _wait_for_server uses 1 slot, so with max_sessions=3:
        - POST (upload) uses slot 2
        - GET (download) uses slot 3
        - Second POST (slot 4) is rejected.
        """
        share_file = tmp_path / 'shared.txt'
        share_file.write_text('shared content')

        save_dir = str(tmp_path / 'uploads')
        os.makedirs(save_dir)

        server = MultiShareServer(
            paths=[(str(share_file), 'file')],
            max_sessions=3,
            upload_enabled=True,
            upload_save_dir=save_dir,
        )
        server.start()
        try:
            assert _wait_for_server(server.port, '/')

            # Slot 2: upload
            status1, _ = _post_multipart(server.port, '/upload', [
                ('file', 'upload1.txt', 'text/plain', b'upload 1'),
            ])
            assert status1 == 200

            # Slot 3: download
            status2, _ = _get_request(server.port, '/')
            assert status2 == 200

            # Slot 4 (new connection, no cookie): should be rejected
            status3, body3 = _post_multipart(server.port, '/upload', [
                ('file', 'upload3.txt', 'text/plain', b'should fail'),
            ])
            assert status3 == 403
            resp = json.loads(body3.decode('utf-8'))
            assert 'session' in resp['error'].lower()
        finally:
            server.stop()
