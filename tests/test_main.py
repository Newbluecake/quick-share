import pytest
from unittest.mock import patch, MagicMock
from pathlib import Path
import sys
import os

from src.main import validate_file, main, detect_path_type, validate_path, handle_symlink

# T-013: Path type detection tests
def test_detect_path_type_file(tmp_path):
    """Test detection of file path type."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    result = detect_path_type(str(test_file))
    assert result == "file"

def test_detect_path_type_directory(tmp_path):
    """Test detection of directory path type."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    result = detect_path_type(str(test_dir))
    assert result == "directory"

def test_detect_path_type_invalid():
    """Test detection of non-existent path."""
    result = detect_path_type("/nonexistent/path/to/nowhere")
    assert result == "invalid"

def test_detect_path_type_symlink_to_file(tmp_path):
    """Test detection of symlink pointing to file."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    symlink = tmp_path / "link.txt"
    symlink.symlink_to(test_file)

    result = detect_path_type(str(symlink))
    assert result == "file"

def test_detect_path_type_symlink_to_directory(tmp_path):
    """Test detection of symlink pointing to directory."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    symlink = tmp_path / "link_dir"
    symlink.symlink_to(test_dir)

    result = detect_path_type(str(symlink))
    assert result == "directory"

# T-014: Unified validate_path tests
def test_validate_path_file_success(tmp_path):
    """Test unified validation for a valid file."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("hello world")

    is_valid, path_type, resolved_path = validate_path(str(test_file))
    assert is_valid is True
    assert path_type == "file"
    assert resolved_path == test_file.resolve()

def test_validate_path_directory_success(tmp_path):
    """Test unified validation for a valid directory."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    is_valid, path_type, resolved_path = validate_path(str(test_dir))
    assert is_valid is True
    assert path_type == "directory"
    assert resolved_path == test_dir.resolve()

def test_validate_path_invalid():
    """Test unified validation for non-existent path."""
    is_valid, path_type, resolved_path = validate_path("/nonexistent/path")
    assert is_valid is False
    assert path_type == "invalid"
    assert resolved_path is None

def test_validate_path_file_permission_error(tmp_path):
    """Test unified validation for file with permission denied."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")
    test_file.chmod(0o000)

    try:
        is_valid, path_type, resolved_path = validate_path(str(test_file))
        # If we can still detect it's a file (some systems allow this), path_type should be "file"
        # but is_valid should be False due to permission error
        assert is_valid is False
        assert path_type in ["file", "invalid"]
    finally:
        # Restore permissions for cleanup
        test_file.chmod(0o644)

def test_validate_path_directory_permission_error(tmp_path):
    """Test unified validation for directory with permission denied."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()
    test_dir.chmod(0o000)

    try:
        is_valid, path_type, resolved_path = validate_path(str(test_dir))
        # Similar to file case
        assert is_valid is False
        assert path_type in ["directory", "invalid"]
    finally:
        # Restore permissions for cleanup
        test_dir.chmod(0o755)

def test_validate_path_empty_file(tmp_path):
    """Test unified validation for empty file."""
    test_file = tmp_path / "empty.txt"
    test_file.touch()

    is_valid, path_type, resolved_path = validate_path(str(test_file))
    assert is_valid is True
    assert path_type == "file"
    assert resolved_path == test_file.resolve()

def test_validate_path_empty_directory(tmp_path):
    """Test unified validation for empty directory."""
    test_dir = tmp_path / "empty_dir"
    test_dir.mkdir()

    is_valid, path_type, resolved_path = validate_path(str(test_dir))
    assert is_valid is True
    assert path_type == "directory"
    assert resolved_path == test_dir.resolve()

# T-015: main() server dispatcher tests
@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.FileShareServer')
@patch('src.main.logger')
def test_main_dispatches_file_server(mock_logger, mock_file_server, mock_port, mock_ip, tmp_path):
    """Test main() dispatches to FileShareServer for files."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    # Mock server instance with thread that's not alive
    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_file_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file)]):
        main()

    # Verify FileShareServer was called
    mock_file_server.assert_called_once()
    call_args = mock_file_server.call_args
    assert str(test_file.resolve()) in str(call_args)
    server_instance.start.assert_called_once()

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.DirectoryShareServer')
@patch('src.main.logger')
def test_main_dispatches_directory_server(mock_logger, mock_dir_server, mock_port, mock_ip, tmp_path):
    """Test main() dispatches to DirectoryShareServer for directories."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    # Mock server instance with thread that's not alive
    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_dir_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_dir)]):
        main()

    # Verify DirectoryShareServer was called
    mock_dir_server.assert_called_once()
    call_args = mock_dir_server.call_args
    assert str(test_dir.resolve()) in str(call_args)
    server_instance.start.assert_called_once()

def test_main_invalid_path_exits():
    """Test main() exits with error for invalid path."""
    with patch('sys.argv', ['quick-share', '/nonexistent/path']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.DirectoryShareServer')
def test_main_directory_with_max_sessions(mock_dir_server, mock_port, mock_ip, tmp_path):
    """Test main() passes max_sessions to DirectoryShareServer."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_dir_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_dir), '--max-downloads', '5']):
        main()

    # Verify max_sessions parameter was passed
    call_kwargs = mock_dir_server.call_args.kwargs
    assert 'max_sessions' in call_kwargs
    assert call_kwargs['max_sessions'] == 5

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.FileShareServer')
def test_main_file_with_max_downloads(mock_file_server, mock_port, mock_ip, tmp_path):
    """Test main() uses existing max_downloads for files (backward compatibility)."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_file_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file), '--max-downloads', '3']):
        main()

    # FileShareServer doesn't support max_downloads yet, but should not fail
    mock_file_server.assert_called_once()
    server_instance.start.assert_called_once()

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.DirectoryShareServer')
@patch('src.main.logger')
def test_main_directory_keyboard_interrupt(mock_logger, mock_dir_server, mock_port, mock_ip, tmp_path):
    """Test main() handles KeyboardInterrupt for directory server."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    server_instance = MagicMock()
    server_instance.start.side_effect = KeyboardInterrupt()
    mock_dir_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_dir)]):
        try:
            main()
        except SystemExit as e:
            assert e.code == 0

    mock_dir_server.assert_called_once()
    server_instance.stop.assert_called_once()

# File validation tests
def test_validate_file_success(tmp_path):
    test_file = tmp_path / "test.txt"
    test_file.write_text("hello")

    path, size = validate_file(str(test_file))
    assert path.name == "test.txt"
    assert size == 5

def test_validate_file_not_found():
    with pytest.raises(FileNotFoundError):
        validate_file("/nonexistent.txt")

def test_validate_file_is_directory(tmp_path):
    with pytest.raises(ValueError, match="is a directory"):
        validate_file(str(tmp_path))

# Main flow tests
@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.FileShareServer')
@patch('src.main.logger')
def test_main_success(mock_logger, mock_server, mock_port, mock_ip, tmp_path):
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    # Mock server instance with thread that's not alive
    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file)]):
        main()

    mock_server.assert_called_once()
    server_instance.start.assert_called_once()

    # Check if startup message was formatted/logged
    mock_logger.format_startup_message.assert_called()

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.FileShareServer')
def test_main_keyboard_interrupt(mock_server, mock_port, mock_ip, tmp_path):
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    # Mock server start to raise KeyboardInterrupt
    server_instance = MagicMock()
    server_instance.start.side_effect = KeyboardInterrupt()
    mock_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file)]):
        # Should exit gracefully (we can catch SystemExit or just ensure no exception)
        try:
            main()
        except SystemExit as e:
            assert e.code == 0

    mock_server.assert_called_once()

@patch('src.main.get_local_ip')
def test_main_ip_error(mock_ip, tmp_path):
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    mock_ip.side_effect = RuntimeError("No IP found")

    with patch('sys.argv', ['quick-share', str(test_file)]):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

def test_main_no_args():
    with patch('sys.argv', ['quick-share']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 2

def test_main_invalid_argument():
    # Test invalid port argument causing validate_arguments to fail
    with patch('sys.argv', ['quick-share', 'test.txt', '-p', '99999']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.validate_file')
def test_main_permission_error(mock_validate):
    mock_validate.side_effect = PermissionError("Permission denied")
    # Need to pass argument validation so we need a valid file path in argv even if mocked
    with patch('sys.argv', ['quick-share', 'test.txt']):
         with pytest.raises(SystemExit) as e:
            main()
         assert e.value.code == 1

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port')
@patch('src.main.validate_path')
def test_main_port_error(mock_validate, mock_port, mock_ip, tmp_path):
    mock_validate.return_value = (True, "file", Path("test.txt"))
    mock_port.side_effect = RuntimeError("No ports available")

    with patch('sys.argv', ['quick-share', 'test.txt']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.parse_arguments')
def test_main_unexpected_error(mock_args):
    mock_args.side_effect = Exception("Boom")
    with patch('sys.argv', ['quick-share']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.validate_path')
def test_main_file_validation_error(mock_validate, mock_port, mock_ip):
    # This covers the invalid path detection
    mock_validate.return_value = (False, "invalid", None)
    with patch('sys.argv', ['quick-share', 'nonexistent.txt']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1


# T-001: Symlink handling tests
class TestSymlinkHandling:
    """Test symlink detection and handling"""

    def test_handle_symlink_to_file(self, tmp_path, capsys):
        """Test handling symlink to file with user confirmation"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello, World!")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input to confirm
        with patch('builtins.input', return_value='y'):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is True
        assert path_type == "file"
        assert resolved == real_file

        captured = capsys.readouterr()
        assert "检测到软链接" in captured.out
        assert "源路径:" in captured.out
        assert "目标路径:" in captured.out

    def test_handle_symlink_to_directory(self, tmp_path, capsys):
        """Test handling symlink to directory with user confirmation"""
        # Create a real directory
        real_dir = tmp_path / "real_dir"
        real_dir.mkdir()

        # Create a symlink
        symlink = tmp_path / "symlink_dir"
        symlink.symlink_to(real_dir)

        # Mock user input to confirm
        with patch('builtins.input', return_value='y'):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is True
        assert path_type == "directory"
        assert resolved == real_dir

    def test_handle_symlink_user_cancel(self, tmp_path, capsys):
        """Test user cancels symlink following"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input to cancel
        with patch('builtins.input', return_value='n'):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is False
        assert path_type == "symlink_cancelled"
        assert resolved is None

        captured = capsys.readouterr()
        assert "用户取消分享" in captured.out

    def test_handle_broken_symlink(self, tmp_path, capsys):
        """Test handling broken symlink"""
        # Create a symlink to non-existent target
        symlink = tmp_path / "broken_symlink"
        symlink.symlink_to("/nonexistent/path")

        # Call handle_symlink (no user input needed)
        is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is False
        assert path_type == "symlink_broken"
        assert resolved is None

        captured = capsys.readouterr()
        assert "错误：软链接目标不存在" in captured.out

    def test_validate_path_with_symlink(self, tmp_path):
        """Test validate_path detects and handles symlink"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input to confirm
        with patch('builtins.input', return_value='y'):
            is_valid, path_type, resolved = validate_path(str(symlink))

        assert is_valid is True
        assert path_type == "file"
        assert resolved == real_file

    def test_validate_path_normal_file_unaffected(self, tmp_path):
        """Test normal files are unaffected by symlink handling"""
        # Create a normal file
        normal_file = tmp_path / "normal_file.txt"
        normal_file.write_text("Hello")

        # No mock needed for input (should not prompt)
        is_valid, path_type, resolved = validate_path(str(normal_file))

        assert is_valid is True
        assert path_type == "file"
        assert resolved == normal_file

    def test_handle_symlink_invalid_input_loop(self, tmp_path):
        """Test invalid input prompts again"""
        # Create a real file
        real_file = tmp_path / "real_file.txt"
        real_file.write_text("Hello")

        # Create a symlink
        symlink = tmp_path / "symlink"
        symlink.symlink_to(real_file)

        # Mock user input: invalid -> invalid -> valid
        with patch('builtins.input', side_effect=['invalid', 'abc', 'y']):
            is_valid, path_type, resolved = handle_symlink(str(symlink))

        assert is_valid is True
        assert path_type == "file"

