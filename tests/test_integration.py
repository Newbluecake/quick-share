"""Integration tests for the complete quick-share application."""
import pytest
import sys
import os
from pathlib import Path
from unittest.mock import patch, MagicMock
import time
import threading

from src.main import main


def test_full_application_flow(tmp_path):
    """Test the complete application flow from CLI to server start."""
    # Create a test file
    test_file = tmp_path / "integration_test.txt"
    test_file.write_text("Integration test content")

    # Mock server start to avoid actually starting HTTP server
    with patch('src.main.FileShareServer') as mock_server_class:
        server_instance = MagicMock()
        server_instance.server_thread = MagicMock()
        server_instance.server_thread.is_alive.return_value = False
        mock_server_class.return_value = server_instance

        # Simulate command line arguments
        with patch('sys.argv', ['quick-share', str(test_file), '-p', '8080', '-t', '1m']):
            # Capture stdout
            from io import StringIO
            captured_output = StringIO()

            with patch('sys.stdout', captured_output):
                main()

            # Verify server was initialized correctly
            mock_server_class.assert_called_once()
            call_kwargs = mock_server_class.call_args[1]
            assert call_kwargs['file_path'] == str(test_file)
            assert call_kwargs['port'] == 8080
            assert call_kwargs['timeout_minutes'] == 1.0

            # Verify server.start() was called
            server_instance.start.assert_called_once()

            # Verify startup message was printed
            output = captured_output.getvalue()
            assert "Share started!" in output
            assert "integration_test.txt" in output
            assert "8080" in output


def test_application_with_defaults(tmp_path):
    """Test application with default port and timeout."""
    test_file = tmp_path / "default_test.txt"
    test_file.write_text("Default test")

    with patch('src.main.FileShareServer') as mock_server_class:
        server_instance = MagicMock()
        server_instance.server_thread = None
        mock_server_class.return_value = server_instance

        with patch('sys.argv', ['quick-share', str(test_file)]):
            from io import StringIO
            captured_output = StringIO()

            with patch('sys.stdout', captured_output):
                main()

            # Verify server was initialized
            mock_server_class.assert_called_once()
            call_kwargs = mock_server_class.call_args[1]

            # Port should be auto-detected (mocked find_available_port)
            assert 'port' in call_kwargs

            # Timeout should be 5m = 300s = 5 minutes
            assert call_kwargs['timeout_minutes'] == 5.0


def test_keyboard_interrupt_during_server(tmp_path):
    """Test that KeyboardInterrupt is handled gracefully during server operation."""
    test_file = tmp_path / "interrupt_test.txt"
    test_file.write_text("Interrupt test")

    with patch('src.main.FileShareServer') as mock_server_class:
        server_instance = MagicMock()

        # Simulate KeyboardInterrupt when start is called
        server_instance.start.side_effect = KeyboardInterrupt()
        mock_server_class.return_value = server_instance

        with patch('sys.argv', ['quick-share', str(test_file)]):
            with pytest.raises(SystemExit) as exc_info:
                main()

            # Should exit with code 0 (clean shutdown)
            assert exc_info.value.code == 0

            # Verify stop was called to clean up
            server_instance.stop.assert_called_once()


def test_real_file_validation_integration(tmp_path):
    """Test file validation with real filesystem operations."""
    # Test with non-existent file
    with patch('sys.argv', ['quick-share', '/nonexistent/file.txt']):
        with pytest.raises(SystemExit) as exc_info:
            main()
        assert exc_info.value.code == 1

    # Test with directory instead of file
    test_dir = tmp_path / "test_directory"
    test_dir.mkdir()

    with patch('sys.argv', ['quick-share', str(test_dir)]):
        with pytest.raises(SystemExit) as exc_info:
            main()
        assert exc_info.value.code == 1


def test_network_error_handling(tmp_path):
    """Test handling of network-related errors."""
    test_file = tmp_path / "network_test.txt"
    test_file.write_text("Network test")

    # Test when IP detection fails
    with patch('src.main.get_local_ip', side_effect=RuntimeError("No network")):
        with patch('sys.argv', ['quick-share', str(test_file)]):
            with pytest.raises(SystemExit) as exc_info:
                main()
            assert exc_info.value.code == 1

    # Test when port allocation fails
    with patch('src.main.find_available_port', side_effect=RuntimeError("No ports")):
        with patch('sys.argv', ['quick-share', str(test_file)]):
            with pytest.raises(SystemExit) as exc_info:
                main()
            assert exc_info.value.code == 1


def test_various_timeout_formats(tmp_path):
    """Test that various timeout formats are handled correctly."""
    test_file = tmp_path / "timeout_test.txt"
    test_file.write_text("Timeout test")

    test_cases = [
        ('30s', 0.5),   # 30 seconds = 0.5 minutes
        ('5m', 5.0),    # 5 minutes
        ('1h', 60.0),   # 1 hour = 60 minutes
    ]

    for timeout_str, expected_minutes in test_cases:
        with patch('src.main.FileShareServer') as mock_server_class:
            server_instance = MagicMock()
            server_instance.server_thread = None
            mock_server_class.return_value = server_instance

            with patch('sys.argv', ['quick-share', str(test_file), '-t', timeout_str]):
                from io import StringIO
                with patch('sys.stdout', StringIO()):
                    main()

                call_kwargs = mock_server_class.call_args[1]
                assert call_kwargs['timeout_minutes'] == expected_minutes
