import pytest
from unittest.mock import patch, MagicMock
from pathlib import Path
import sys
import os

# We need to add src to python path to import main
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from main import validate_file, main

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
@patch('main.get_local_ip', return_value='192.168.1.100')
@patch('main.find_available_port', return_value=8000)
@patch('main.FileShareServer')
@patch('main.logger')
def test_main_success(mock_logger, mock_server, mock_port, mock_ip, tmp_path):
    test_file = tmp_path / "test.txt"
    test_file.write_text("content")

    # Mock server instance
    server_instance = MagicMock()
    mock_server.return_value = server_instance

    with patch('sys.argv', ['quick-share', str(test_file)]):
        main()

    mock_server.assert_called_once()
    server_instance.start.assert_called_once()

    # Check if startup message was formatted/logged
    mock_logger.format_startup_message.assert_called()

@patch('main.get_local_ip', return_value='192.168.1.100')
@patch('main.find_available_port', return_value=8000)
@patch('main.FileShareServer')
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

@patch('main.get_local_ip')
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

@patch('main.validate_file')
def test_main_permission_error(mock_validate):
    mock_validate.side_effect = PermissionError("Permission denied")
    # Need to pass argument validation so we need a valid file path in argv even if mocked
    with patch('sys.argv', ['quick-share', 'test.txt']):
         with pytest.raises(SystemExit) as e:
            main()
         assert e.value.code == 1

@patch('main.get_local_ip', return_value='192.168.1.100')
@patch('main.find_available_port')
@patch('main.validate_file')
def test_main_port_error(mock_validate, mock_port, mock_ip, tmp_path):
    mock_validate.return_value = (Path("test.txt"), 100)
    mock_port.side_effect = RuntimeError("No ports available")

    with patch('sys.argv', ['quick-share', 'test.txt']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('main.parse_arguments')
def test_main_unexpected_error(mock_args):
    mock_args.side_effect = Exception("Boom")
    with patch('sys.argv', ['quick-share']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

@patch('main.get_local_ip', return_value='192.168.1.100')
@patch('main.find_available_port', return_value=8000)
@patch('main.validate_file')
def test_main_file_validation_error(mock_validate, mock_port, mock_ip):
    # This covers the specific FileNotFoundError/ValueError catch block in main
    mock_validate.side_effect = FileNotFoundError("File not found")
    with patch('sys.argv', ['quick-share', 'nonexistent.txt']):
        with pytest.raises(SystemExit) as e:
            main()
        assert e.value.code == 1

