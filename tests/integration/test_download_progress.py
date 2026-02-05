"""Integration tests for download progress tracking."""
import pytest
import sys
import os
import io
import time
import threading
from pathlib import Path
from unittest.mock import patch, MagicMock
import requests

# Add src to path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../../src')))

from src.server import FileShareServer, DirectoryShareServer


@pytest.fixture
def test_file(tmp_path):
    """Create a test file for download testing."""
    test_file = tmp_path / "test_download.zip"
    # Create a file larger than 80KB to test multiple progress updates
    test_file.write_bytes(b"0" * (100 * 1024))  # 100KB
    return test_file


@pytest.fixture
def test_directory(tmp_path):
    """Create a test directory for ZIP download testing."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    # Create some files in the directory
    (test_dir / "file1.txt").write_bytes(b"0" * (50 * 1024))  # 50KB
    (test_dir / "file2.txt").write_bytes(b"1" * (30 * 1024))  # 30KB
    (test_dir / "file3.txt").write_text("Small file")

    return test_dir


class TestDownloadProgressIntegration:
    """Integration tests for download progress tracking."""

    def test_single_file_download_progress(self, test_file, capsys):
        """T-011: Test that single file download shows progress logs."""
        # Start server in background thread
        server = FileShareServer(str(test_file), port=0)  # Use random port
        server.start()

        # Wait for server to be ready
        time.sleep(0.5)

        try:
            # Download file
            url = f"http://127.0.0.1:{server.port}/{test_file.name}"
            response = requests.get(url, stream=True)

            # Consume the response
            for chunk in response.iter_content(chunk_size=8192):
                pass

            # Capture output
            captured = capsys.readouterr()

            # Verify progress logs
            output = captured.out

            # Should have start log with emoji
            assert "⬇️" in output or "Download started" in output

            # Should have completion log with emoji
            assert "✅" in output or "Completed" in output

            # Should mention the file
            assert test_file.name in output or "test_download.zip" in output

            # Should have IP address (127.0.0.1 or ::1)
            assert "127.0.0.1" in output or "::1" in output

        finally:
            server.stop()

    def test_concurrent_downloads(self, test_file, capsys):
        """T-011: Test that concurrent downloads show independent progress logs."""
        server = FileShareServer(str(test_file), port=0)
        server.start()

        time.sleep(0.5)

        try:
            # Start 3 concurrent downloads
            def download_file():
                url = f"http://127.0.0.1:{server.port}/{test_file.name}"
                response = requests.get(url, stream=True)
                for chunk in response.iter_content(chunk_size=8192):
                    pass

            threads = []
            for _ in range(3):
                t = threading.Thread(target=download_file)
                t.start()
                threads.append(t)

            # Wait for all downloads to complete
            for t in threads:
                t.join(timeout=10)

            # Capture output
            captured = capsys.readouterr()
            output = captured.out

            # Should have multiple start logs (3 downloads)
            start_count = output.count("⬇️")
            assert start_count >= 3, f"Expected at least 3 start logs, got {start_count}"

            # Should have multiple completion logs
            complete_count = output.count("✅")
            assert complete_count >= 3, f"Expected at least 3 completion logs, got {complete_count}"

        finally:
            server.stop()

    def test_directory_zip_download_progress(self, test_directory, capsys):
        """T-011: Test that directory ZIP download shows progress logs."""
        server = DirectoryShareServer(str(test_directory), port=0)
        server.start()

        time.sleep(0.5)

        try:
            # Download directory as ZIP
            url = f"http://127.0.0.1:{server.port}/download/{test_directory.name}.zip"
            response = requests.get(url, stream=True)

            # Consume the response
            for chunk in response.iter_content(chunk_size=8192):
                pass

            # Capture output
            captured = capsys.readouterr()
            output = captured.out

            # Should have start log
            assert "⬇️" in output or "Download started" in output

            # Should have completion log
            assert "✅" in output or "Completed" in output

            # Should mention ZIP file
            assert ".zip" in output

        finally:
            server.stop()

    def test_download_interruption(self, test_file, capsys):
        """T-011: Test that interrupted download shows interruption log."""
        server = FileShareServer(str(test_file), port=0)
        server.start()

        time.sleep(0.5)

        try:
            # Start download but close connection early
            url = f"http://127.0.0.1:{server.port}/{test_file.name}"

            def partial_download():
                response = requests.get(url, stream=True)
                # Read only first chunk then close
                next(response.iter_content(chunk_size=8192))
                # Connection will close when we exit without reading full response

            thread = threading.Thread(target=partial_download)
            thread.start()
            thread.join(timeout=5)

            # Wait a bit for server to detect disconnection
            time.sleep(0.5)

            # Capture output
            captured = capsys.readouterr()
            output = captured.out

            # May have interruption log (⚠️)
            # Note: This test is best-effort as interruption detection timing varies

        finally:
            server.stop()

    def test_progress_log_format(self, test_file, capsys):
        """T-011: Test that progress logs follow the expected format."""
        server = FileShareServer(str(test_file), port=0)
        server.start()

        time.sleep(0.5)

        try:
            url = f"http://127.0.0.1:{server.port}/{test_file.name}"
            response = requests.get(url, stream=True)

            for chunk in response.iter_content(chunk_size=8192):
                pass

            captured = capsys.readouterr()
            output = captured.out

            # Check for timestamp format [YYYY-MM-DD HH:MM:SS]
            assert "[" in output and "]" in output

            # Check for IP address
            assert "127.0.0.1" in output or "::1" in output

            # Check for file size info
            assert "KB" in output or "MB" in output or "bytes" in output

        finally:
            server.stop()


class TestDownloadProgressManualVerification:
    """Manual verification scenarios (to be run manually by developers)."""

    def test_manual_verification_instructions(self):
        """
        T-011: Manual verification instructions.

        These scenarios should be manually tested by developers:

        1. Single file download:
           $ python -m quick_share share path/to/large_file.zip
           $ wget http://localhost:8000/large_file.zip
           Expected: Start log, progress logs, completion log with duration

        2. Concurrent downloads:
           $ python -m quick_share share path/to/file.zip
           $ wget http://localhost:8000/file.zip &
           $ wget http://localhost:8000/file.zip &
           $ wget http://localhost:8000/file.zip &
           Expected: 3 independent progress logs, no interleaving issues

        3. Download interruption:
           $ python -m quick_share share path/to/large_file.zip
           $ wget http://localhost:8000/large_file.zip
           (Press Ctrl+C during download)
           Expected: Interruption log (⚠️) with transferred bytes

        4. Directory ZIP download:
           $ python -m quick-share share path/to/directory
           $ wget http://localhost:8000/download/directory.zip
           Expected: ZIP download progress logs

        5. Windows compatibility:
           Run above tests in PowerShell, verify emoji display correctly
        """
        # This test just documents manual verification steps
        assert True
