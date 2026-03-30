"""
Integration tests for multi-path sharing (T-009).

Uses a real ThreadingHTTPServer started on an ephemeral port; all HTTP
requests are made with urllib.request from the standard library only.
"""

import io
import json
import os
import tempfile
import threading
import time
import urllib.error
import urllib.request
import zipfile
from http.cookiejar import CookieJar
from pathlib import Path

import pytest

from src.server import MultiShareServer


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _wait_for_port(port: int, timeout: float = 5.0) -> None:
    """Block until the server port is accepting connections."""
    import socket
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError(f"Server did not start on port {port} within {timeout}s")


def _get(url: str, *, cookies: dict | None = None) -> tuple[int, bytes, dict]:
    """
    Perform a GET request and return (status, body, headers).
    Optionally send cookies.
    """
    req = urllib.request.Request(url)
    if cookies:
        cookie_str = "; ".join(f"{k}={v}" for k, v in cookies.items())
        req.add_header("Cookie", cookie_str)
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, resp.read(), dict(resp.headers)
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(), {}


def _extract_session_cookie(headers: dict) -> str | None:
    """Extract quick_share_session value from Set-Cookie header."""
    set_cookie = headers.get("Set-Cookie", "")
    for part in set_cookie.split(";"):
        part = part.strip()
        if part.startswith("quick_share_session="):
            return part.split("=", 1)[1]
    return None


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture()
def tmp_share(tmp_path):
    """
    Create a temporary file layout:
        file1.txt    – 'hello file1'
        file2.pdf    – 'fake pdf content'
        mydir/
            sub.txt  – 'subfile content'
            inner/
                deep.txt – 'deep content'
    Returns the tmp_path root.
    """
    (tmp_path / "file1.txt").write_text("hello file1")
    (tmp_path / "file2.pdf").write_bytes(b"fake pdf content")
    d = tmp_path / "mydir"
    d.mkdir()
    (d / "sub.txt").write_text("subfile content")
    inner = d / "inner"
    inner.mkdir()
    (inner / "deep.txt").write_text("deep content")
    return tmp_path


@pytest.fixture()
def server_multi_files(tmp_share):
    """MultiShareServer sharing 2 top-level files."""
    paths = [
        (str(tmp_share / "file1.txt"), "file"),
        (str(tmp_share / "file2.pdf"), "file"),
    ]
    srv = MultiShareServer(paths=paths, max_sessions=10, timeout_minutes=1)
    srv.start()
    _wait_for_port(srv.port)
    yield srv
    srv.stop()


@pytest.fixture()
def server_mixed(tmp_share):
    """MultiShareServer sharing 1 file + 1 directory."""
    paths = [
        (str(tmp_share / "file1.txt"), "file"),
        (str(tmp_share / "mydir"), "directory"),
    ]
    srv = MultiShareServer(paths=paths, max_sessions=10, timeout_minutes=1)
    srv.start()
    _wait_for_port(srv.port)
    yield srv
    srv.stop()


@pytest.fixture()
def server_single_file(tmp_share):
    """MultiShareServer sharing exactly 1 file (R-007 unified experience)."""
    paths = [(str(tmp_share / "file1.txt"), "file")]
    srv = MultiShareServer(paths=paths, max_sessions=10, timeout_minutes=1)
    srv.start()
    _wait_for_port(srv.port)
    yield srv
    srv.stop()


@pytest.fixture()
def server_session_limit(tmp_share):
    """MultiShareServer with max_sessions=1."""
    paths = [(str(tmp_share / "file1.txt"), "file")]
    srv = MultiShareServer(paths=paths, max_sessions=1, timeout_minutes=1)
    srv.start()
    _wait_for_port(srv.port)
    yield srv
    srv.stop()


# ---------------------------------------------------------------------------
# Scenario 1: Multi-file sharing
# ---------------------------------------------------------------------------

class TestMultiFileSharing:
    """Scenario 1 – share multiple top-level files."""

    def test_root_returns_html(self, server_multi_files):
        port = server_multi_files.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/")
        assert status == 200
        assert b"Quick Share" in body
        assert b"<!DOCTYPE html>" in body.lower() or b"<!doctype html>" in body.lower()

    def test_api_tree_root_lists_both_files(self, server_multi_files):
        port = server_multi_files.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/api/tree?path=/")
        assert status == 200
        data = json.loads(body)
        names = [i["name"] for i in data["items"]]
        assert "file1.txt" in names
        assert "file2.pdf" in names

    def test_file_download_correct_content(self, server_multi_files, tmp_share):
        port = server_multi_files.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/files/file1.txt")
        assert status == 200
        assert body == b"hello file1"

    def test_all_zip_download_contains_both_files(self, server_multi_files):
        port = server_multi_files.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/download/all.zip")
        assert status == 200
        zf = zipfile.ZipFile(io.BytesIO(body))
        names = zf.namelist()
        assert "file1.txt" in names
        assert "file2.pdf" in names
        assert zf.read("file1.txt") == b"hello file1"


# ---------------------------------------------------------------------------
# Scenario 2: Mixed file + directory sharing
# ---------------------------------------------------------------------------

class TestMixedSharing:
    """Scenario 2 – share 1 file + 1 directory."""

    def test_root_api_tree_shows_two_items(self, server_mixed):
        port = server_mixed.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/api/tree?path=/")
        assert status == 200
        data = json.loads(body)
        names = [i["name"] for i in data["items"]]
        assert "file1.txt" in names
        assert "mydir" in names

    def test_directory_subtree(self, server_mixed):
        port = server_mixed.port
        status, body, _ = _get(
            f"http://127.0.0.1:{port}/api/tree?path=/mydir"
        )
        assert status == 200
        data = json.loads(body)
        names = [i["name"] for i in data["items"]]
        assert "sub.txt" in names
        assert "inner" in names

    def test_download_file_inside_directory(self, server_mixed):
        port = server_mixed.port
        status, body, _ = _get(
            f"http://127.0.0.1:{port}/files/mydir/sub.txt"
        )
        assert status == 200
        assert body == b"subfile content"

    def test_download_nested_file_inside_directory(self, server_mixed):
        port = server_mixed.port
        status, body, _ = _get(
            f"http://127.0.0.1:{port}/files/mydir/inner/deep.txt"
        )
        assert status == 200
        assert body == b"deep content"

    def test_zip_contains_file_and_directory_contents(self, server_mixed):
        port = server_mixed.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/download/all.zip")
        assert status == 200
        zf = zipfile.ZipFile(io.BytesIO(body))
        names = zf.namelist()
        assert "file1.txt" in names
        assert "mydir/sub.txt" in names
        assert "mydir/inner/deep.txt" in names


# ---------------------------------------------------------------------------
# Scenario 3: Single-file unified experience (R-007)
# ---------------------------------------------------------------------------

class TestSingleFileUnifiedExperience:
    """Scenario 3 – even a single shared file shows the list page."""

    def test_root_returns_html_not_download(self, server_single_file):
        port = server_single_file.port
        status, body, headers = _get(f"http://127.0.0.1:{port}/")
        assert status == 200
        # Must be HTML, not octet-stream
        ct = headers.get("Content-Type", "")
        assert "text/html" in ct
        assert b"Quick Share" in body

    def test_file_still_downloadable_via_files_prefix(self, server_single_file):
        port = server_single_file.port
        status, body, _ = _get(f"http://127.0.0.1:{port}/files/file1.txt")
        assert status == 200
        assert body == b"hello file1"


# ---------------------------------------------------------------------------
# Scenario 4: Session counting (R-008)
# ---------------------------------------------------------------------------

class TestSessionCounting:
    """Scenario 4 – max_sessions=1 limits access to one session."""

    def test_first_session_allowed(self, server_session_limit):
        port = server_session_limit.port
        status, body, headers = _get(f"http://127.0.0.1:{port}/")
        assert status == 200
        # Get our session cookie
        session_id = _extract_session_cookie(headers)
        assert session_id is not None

    def test_second_session_rejected(self, server_session_limit):
        port = server_session_limit.port
        # First session establishes the limit
        _get(f"http://127.0.0.1:{port}/")
        # Second request with no cookie → new session → should be rejected
        # We need to force a new session by NOT sending the existing cookie.
        # Make request without cookie header.
        status, _, _ = _get(f"http://127.0.0.1:{port}/")
        assert status == 403


# ---------------------------------------------------------------------------
# Scenario 5: Security – path traversal rejected
# ---------------------------------------------------------------------------

class TestSecurity:
    """Scenario 5 – security checks are enforced."""

    def test_path_traversal_rejected(self, server_multi_files):
        port = server_multi_files.port
        status, _, _ = _get(f"http://127.0.0.1:{port}/files/../etc/passwd")
        assert status == 403

    def test_unknown_file_rejected(self, server_multi_files):
        port = server_multi_files.port
        status, _, _ = _get(f"http://127.0.0.1:{port}/files/notexist.txt")
        assert status == 403

    def test_url_encoded_traversal_rejected(self, server_multi_files):
        port = server_multi_files.port
        status, _, _ = _get(
            f"http://127.0.0.1:{port}/files/%2e%2e/etc/passwd"
        )
        assert status == 403
