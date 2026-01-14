import pytest
from src.logger import format_startup_message, format_download_log, format_shutdown_message

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

    # Should show RESTful URL format for directories
    assert f"http://{ip}:{port}/download/{dirname}.zip" in msg
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
