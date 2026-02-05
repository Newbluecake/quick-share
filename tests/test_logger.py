import pytest
from src.logger import (
    format_startup_message,
    format_download_log,
    format_shutdown_message,
    get_timestamp,
    format_download_start,
    format_download_progress,
    format_download_complete,
    format_download_interrupted,
    format_download_error
)

def test_format_startup_message():
    ip = "192.168.1.100"
    port = 8000
    filename = "test.txt"
    file_size = "1.2 MB"
    max_downloads = 10
    timeout = 300

    msg = format_startup_message(ip, port, filename, file_size, max_downloads, timeout)

    assert f"{ip}:{port}" in msg
    assert filename in msg
    assert file_size in msg
    assert "curl" in msg
    assert "wget" in msg
    assert str(max_downloads) in msg
    assert str(timeout) in msg

def test_format_startup_message_directory():
    """Test that directory shares show the RESTful download URL format."""
    ip = "192.168.1.100"
    port = 8000
    dirname = "my-docs"
    file_size = "Directory"  # Special marker for directories
    max_downloads = 10
    timeout = 300

    msg = format_startup_message(ip, port, dirname, file_size, max_downloads, timeout)

    # Should show Browse URL
    assert f"Browse: http://{ip}:{port}/" in msg
    # Should show RESTful URL format for directories
    assert f"Zip URL: http://{ip}:{port}/download/{dirname}.zip" in msg
    assert "curl" in msg
    assert "wget" in msg
    # Should not contain QR code text
    assert "QR code" not in msg
    assert "Scan" not in msg.lower() or "scan" not in msg.lower()

def test_format_startup_message_no_qr_code():
    """Test that QR code references are removed from output."""
    ip = "192.168.1.100"
    port = 8000
    filename = "test.txt"
    file_size = "1.2 MB"
    max_downloads = 10
    timeout = 300

    msg = format_startup_message(ip, port, filename, file_size, max_downloads, timeout)

    # Should not contain QR code text
    assert "QR code" not in msg
    assert "Scan QR" not in msg

def test_format_download_log():
    timestamp = "2026-01-12 14:30:25"
    client_ip = "192.168.1.101"
    method = "GET"
    path = "/test.txt"
    status_code = 200
    current_count = 1
    max_count = 10

    log = format_download_log(timestamp, client_ip, method, path, status_code, current_count, max_count)

    assert f"[{timestamp}]" in log
    assert client_ip in log
    assert method in log
    assert path in log
    assert str(status_code) in log
    assert f"{current_count}/{max_count}" in log

def test_format_shutdown_message():
    total_downloads = 5
    max_downloads = 10

    msg = format_shutdown_message(total_downloads, max_downloads)

    assert str(total_downloads) in msg
    assert str(max_downloads) in msg
    assert "Shutdown" in msg or "Stopping" in msg


def test_get_timestamp():
    """Test timestamp generation."""
    timestamp = get_timestamp()

    # Should match format YYYY-MM-DD HH:MM:SS (no brackets in the timestamp itself)
    assert len(timestamp) == 19  # 2025-02-05 10:30:45
    assert '-' in timestamp  # Date separators
    assert ':' in timestamp  # Time separators
    assert ' ' in timestamp  # Space between date and time


def test_format_download_start():
    """Test download start log format."""
    timestamp = "2025-02-05 10:30:45"
    client_ip = "192.168.1.100"
    filename = "test.zip"
    file_size = "2.5MB"

    log = format_download_start(timestamp, client_ip, filename, file_size)

    assert f"[{timestamp}]" in log
    assert "⬇️" in log
    assert client_ip in log
    assert filename in log
    assert file_size in log


def test_format_download_progress():
    """Test download progress log format."""
    timestamp = "2025-02-05 10:30:46"
    client_ip = "192.168.1.100"
    bytes_transferred = 1200000  # 1.2MB
    total_bytes = 2500000  # 2.5MB
    percentage = 48.0

    log = format_download_progress(timestamp, client_ip, bytes_transferred, total_bytes, percentage)

    assert f"[{timestamp}]" in log
    assert "⬇️" in log
    assert client_ip in log
    assert "48%" in log or "48" in log  # Percentage should be present
    assert "/" in log  # Should show "transferred / total"


def test_format_download_complete():
    """Test download completion log format."""
    timestamp = "2025-02-05 10:30:47"
    client_ip = "192.168.1.100"
    filename = "test.zip"
    total_bytes = 2500000  # 2.5MB
    duration_sec = 2.3

    log = format_download_complete(timestamp, client_ip, filename, total_bytes, duration_sec)

    assert f"[{timestamp}]" in log
    assert "✅" in log
    assert client_ip in log
    assert "Completed" in log
    assert filename in log
    assert "2.3s" in log or "2.30s" in log


def test_format_download_interrupted():
    """Test download interruption log format."""
    timestamp = "2025-02-05 10:30:46"
    client_ip = "192.168.1.100"
    filename = "test.zip"
    bytes_transferred = 1200000  # 1.2MB
    total_bytes = 2500000  # 2.5MB

    log = format_download_interrupted(timestamp, client_ip, filename, bytes_transferred, total_bytes)

    assert f"[{timestamp}]" in log
    assert "⚠️" in log
    assert client_ip in log
    assert "Interrupted" in log
    assert filename in log
    assert "/" in log  # Should show "transferred / total"


def test_format_download_error():
    """Test download error log format."""
    timestamp = "2025-02-05 10:30:45"
    client_ip = "192.168.1.100"
    filename = "test.zip"
    error_message = "File not found"

    log = format_download_error(timestamp, client_ip, filename, error_message)

    assert f"[{timestamp}]" in log
    assert "❌" in log
    assert client_ip in log
    assert "Error" in log
    assert filename in log
    assert error_message in log

