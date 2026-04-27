import pytest
from unittest.mock import patch, MagicMock
from pathlib import Path
import sys
import os

from src.main import validate_file, main, detect_path_type, validate_path, handle_symlink, validate_multi_paths

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

# T-015 (updated): main() server dispatcher tests – now uses MultiShareServer
@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.MultiShareServer')
@patch('src.main.logger')
def test_main_dispatches_file_server(mock_logger, mock_multi_server, mock_port, mock_ip, tmp_path):
    """Test main() uses MultiShareServer for a single file."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_multi_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file)]):
        main()

    mock_multi_server.assert_called_once()
    call_kwargs = mock_multi_server.call_args.kwargs
    assert 'paths' in call_kwargs
    assert str(test_file.resolve()) in str(call_kwargs['paths'])
    server_instance.start.assert_called_once()

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.MultiShareServer')
@patch('src.main.logger')
def test_main_dispatches_directory_server(mock_logger, mock_multi_server, mock_port, mock_ip, tmp_path):
    """Test main() uses MultiShareServer for a single directory."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_multi_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_dir)]):
        main()

    mock_multi_server.assert_called_once()
    call_kwargs = mock_multi_server.call_args.kwargs
    assert 'paths' in call_kwargs
    assert str(test_dir.resolve()) in str(call_kwargs['paths'])
    server_instance.start.assert_called_once()

def test_main_invalid_path_exits():
    """Test main() exits with error for invalid path."""
    with patch('sys.argv', ['quick-share', '/nonexistent/path']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.MultiShareServer')
def test_main_directory_with_max_sessions(mock_multi_server, mock_port, mock_ip, tmp_path):
    """Test main() passes max_sessions to MultiShareServer."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_multi_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_dir), '--max-downloads', '5']):
        main()

    call_kwargs = mock_multi_server.call_args.kwargs
    assert 'max_sessions' in call_kwargs
    assert call_kwargs['max_sessions'] == 5

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.MultiShareServer')
def test_main_file_with_max_downloads(mock_multi_server, mock_port, mock_ip, tmp_path):
    """Test main() passes max_sessions to MultiShareServer for files too."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    server_instance = MagicMock()
    server_instance.server_thread = MagicMock()
    server_instance.server_thread.is_alive.return_value = False
    mock_multi_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file), '--max-downloads', '3']):
        main()

    mock_multi_server.assert_called_once()
    call_kwargs = mock_multi_server.call_args.kwargs
    assert call_kwargs['max_sessions'] == 3
    server_instance.start.assert_called_once()

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port', return_value=8000)
@patch('src.main.MultiShareServer')
@patch('src.main.logger')
def test_main_directory_keyboard_interrupt(mock_logger, mock_multi_server, mock_port, mock_ip, tmp_path):
    """Test main() handles KeyboardInterrupt."""
    test_dir = tmp_path / "test_dir"
    test_dir.mkdir()

    server_instance = MagicMock()
    server_instance.start.side_effect = KeyboardInterrupt()
    mock_multi_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_dir)]):
        try:
            main()
        except SystemExit as e:
            assert e.code == 0

    mock_multi_server.assert_called_once()
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
@patch('src.main.MultiShareServer')
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
@patch('src.main.MultiShareServer')
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
        assert e.value.code == 1

def test_main_invalid_argument():
    # Test invalid port argument causing validate_arguments to fail
    with patch('sys.argv', ['quick-share', 'test.txt', '-p', '99999']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.validate_multi_paths')
def test_main_permission_error(mock_validate):
    """Test main() handles errors from validate_multi_paths."""
    mock_validate.side_effect = PermissionError("Permission denied")
    with patch('sys.argv', ['quick-share', 'test.txt']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('src.main.get_local_ip', return_value='192.168.1.100')
@patch('src.main.find_available_port')
@patch('src.main.validate_multi_paths')
def test_main_port_error(mock_validate, mock_port, mock_ip, tmp_path):
    """Test main() exits when no port is available."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("x")
    mock_validate.return_value = (True, [(str(test_file), "file")], [])
    mock_port.side_effect = RuntimeError("No ports available")

    with patch('sys.argv', ['quick-share', str(test_file)]):
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
@patch('src.main.validate_multi_paths')
def test_main_file_validation_error(mock_validate, mock_port, mock_ip):
    """Test main() exits when path validation fails."""
    mock_validate.return_value = (False, [], ["Invalid path: nonexistent.txt"])
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


# T-002: validate_multi_paths tests
class TestValidateMultiPaths:
    """Tests for validate_multi_paths() – multi-path validation & conflict detection."""

    def test_single_valid_file(self, tmp_path):
        """Single valid file returns True with one resolved entry."""
        f = tmp_path / "file.txt"
        f.write_text("hello")
        ok, resolved, errors = validate_multi_paths([str(f)])
        assert ok is True
        assert len(resolved) == 1
        assert resolved[0][1] == "file"
        assert errors == []

    def test_single_valid_directory(self, tmp_path):
        """Single valid directory returns True with one resolved entry."""
        d = tmp_path / "mydir"
        d.mkdir()
        ok, resolved, errors = validate_multi_paths([str(d)])
        assert ok is True
        assert len(resolved) == 1
        assert resolved[0][1] == "directory"
        assert errors == []

    def test_multiple_valid_paths(self, tmp_path):
        """Multiple valid paths all resolve correctly."""
        f1 = tmp_path / "a.txt"
        f1.write_text("a")
        f2 = tmp_path / "b.txt"
        f2.write_text("b")
        d = tmp_path / "d"
        d.mkdir()
        ok, resolved, errors = validate_multi_paths([str(f1), str(f2), str(d)])
        assert ok is True
        assert len(resolved) == 3
        assert errors == []

    def test_invalid_path_returns_error(self):
        """Non-existent path causes validation to fail with a descriptive error."""
        ok, resolved, errors = validate_multi_paths(["/nonexistent_path_xyz"])
        assert ok is False
        assert resolved == []
        assert len(errors) == 1
        assert "nonexistent_path_xyz" in errors[0]

    def test_mixed_valid_and_invalid(self, tmp_path):
        """A mix of valid and invalid paths fails overall."""
        f = tmp_path / "ok.txt"
        f.write_text("ok")
        ok, resolved, errors = validate_multi_paths([str(f), "/no_such_path"])
        assert ok is False
        assert len(errors) >= 1

    def test_name_conflict_files(self, tmp_path):
        """Files with identical basenames from different directories → conflict error."""
        dir_a = tmp_path / "a"
        dir_a.mkdir()
        dir_b = tmp_path / "b"
        dir_b.mkdir()
        f_a = dir_a / "dup.txt"
        f_a.write_text("a")
        f_b = dir_b / "dup.txt"
        f_b.write_text("b")
        ok, resolved, errors = validate_multi_paths([str(f_a), str(f_b)])
        assert ok is False
        assert len(errors) == 1
        assert "dup.txt" in errors[0]
        assert "conflict" in errors[0].lower()

    def test_name_conflict_directories(self, tmp_path):
        """Directories with identical basenames → conflict error."""
        parent_a = tmp_path / "x"
        parent_a.mkdir()
        parent_b = tmp_path / "y"
        parent_b.mkdir()
        d_a = parent_a / "shared"
        d_a.mkdir()
        d_b = parent_b / "shared"
        d_b.mkdir()
        ok, resolved, errors = validate_multi_paths([str(d_a), str(d_b)])
        assert ok is False
        assert "shared" in errors[0]

    def test_no_conflict_different_names(self, tmp_path):
        """Files with different names from different directories → no conflict."""
        dir_a = tmp_path / "a"
        dir_a.mkdir()
        dir_b = tmp_path / "b"
        dir_b.mkdir()
        f_a = dir_a / "foo.txt"
        f_a.write_text("foo")
        f_b = dir_b / "bar.txt"
        f_b.write_text("bar")
        ok, resolved, errors = validate_multi_paths([str(f_a), str(f_b)])
        assert ok is True
        assert errors == []

    def test_empty_list_returns_valid(self):
        """Empty path list returns True with no items (edge case)."""
        ok, resolved, errors = validate_multi_paths([])
        assert ok is True
        assert resolved == []
        assert errors == []
