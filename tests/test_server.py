import unittest
from unittest.mock import MagicMock, patch, mock_open
import socket
import sys
import os
import threading
import time
from http.server import HTTPServer, BaseHTTPRequestHandler

# Adjust path to include src
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

import server
from server import find_available_port, is_port_available, FileShareHandler, FileShareServer

class TestPortUtils(unittest.TestCase):
    def test_is_port_available_true(self):
        with patch('socket.socket') as mock_sock:
            mock_sock.return_value.__enter__.return_value.bind.return_value = None
            self.assertTrue(is_port_available(8000))

    def test_is_port_available_false(self):
        with patch('socket.socket') as mock_sock:
            mock_sock.return_value.__enter__.return_value.bind.side_effect = OSError
            self.assertFalse(is_port_available(8000))

    def test_find_available_port_range(self):
        with patch('server.is_port_available', side_effect=[False, True]) as mock_is_avail:
            port = find_available_port(start=8000, end=8002)
            self.assertEqual(port, 8001)

    def test_find_available_port_custom_success(self):
        with patch('server.is_port_available', return_value=True):
            port = find_available_port(custom_port=9000)
            self.assertEqual(port, 9000)

    def test_find_available_port_custom_fail(self):
        with patch('server.is_port_available', return_value=False):
            with self.assertRaises(RuntimeError):
                find_available_port(custom_port=9000)

    def test_find_available_port_none_available(self):
        with patch('server.is_port_available', return_value=False):
            with self.assertRaises(RuntimeError):
                find_available_port(start=8000, end=8005)

class TestFileShareHandler(unittest.TestCase):
    def setUp(self):
        self.mock_server = MagicMock()
        self.mock_server.file_path = "/tmp/testfile.txt"
        self.mock_server.allowed_filename = "testfile.txt"
        self.mock_request = MagicMock()
        self.mock_client_address = ('127.0.0.1', 12345)

    def create_handler(self):
        # Patch BaseHTTPRequestHandler.__init__ to do nothing so we can set up the handler manually
        with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
            handler = FileShareHandler(self.mock_request, self.mock_client_address, self.mock_server)
            # Manually set attributes that __init__ would have set
            handler.request = self.mock_request
            handler.client_address = self.mock_client_address
            handler.server = self.mock_server
            handler.wfile = MagicMock()
            handler.rfile = MagicMock()
            handler.path = ""

            # Mock the helper methods from BaseHTTPRequestHandler
            handler.send_response = MagicMock()
            handler.send_header = MagicMock()
            handler.end_headers = MagicMock()
            handler.send_error = MagicMock()

            return handler

    @patch('server.validate_request_path')
    @patch('os.path.exists')
    @patch('os.path.getsize')
    def test_do_GET_success(self, mock_getsize, mock_exists, mock_validate):
        # Setup mocks
        mock_validate.return_value = (True, "/testfile.txt")
        mock_exists.return_value = True
        mock_getsize.return_value = 100

        handler = self.create_handler()
        handler.path = "/testfile.txt"

        # Mock file opening
        with patch('builtins.open', mock_open(read_data=b'hello world')) as m_open:
            handler.do_GET()

            # Checks
            mock_validate.assert_called_with("/testfile.txt", "testfile.txt")
            handler.send_response.assert_called_with(200)
            handler.send_header.assert_any_call('Content-Type', 'application/octet-stream')
            handler.send_header.assert_any_call('Content-Length', '100')
            # Check data written
            handler.wfile.write.assert_called()

    @patch('server.validate_request_path')
    def test_do_GET_invalid_path(self, mock_validate):
        mock_validate.return_value = (False, "")

        handler = self.create_handler()
        handler.path = "/badpath"

        handler.do_GET()

        handler.send_error.assert_called_with(403, "Access denied")

    @patch('server.validate_request_path')
    @patch('os.path.exists')
    def test_do_GET_file_not_found(self, mock_exists, mock_validate):
        mock_validate.return_value = (True, "/testfile.txt")
        mock_exists.return_value = False

        handler = self.create_handler()
        handler.path = "/testfile.txt"

        handler.do_GET()

        handler.send_error.assert_called_with(404, "File not found")

    @patch('server.validate_request_path')
    @patch('os.path.exists')
    @patch('os.path.getsize')
    def test_do_GET_exception(self, mock_getsize, mock_exists, mock_validate):
        # Setup mocks to proceed to file reading
        mock_validate.return_value = (True, "/testfile.txt")
        mock_exists.return_value = True
        mock_getsize.return_value = 100

        handler = self.create_handler()
        handler.path = "/testfile.txt"

        # Mock open to raise an exception
        with patch('builtins.open', side_effect=IOError("Disk error")):
            handler.do_GET()
            # Should just finish without crashing
            # We can verify it attempted to send headers at least
            handler.send_response.assert_called_with(200)

    def test_log_message(self):
        handler = self.create_handler()
        # Should not raise error
        handler.log_message("format %s", "args")

class TestFileShareServer(unittest.TestCase):
    @patch('server.find_available_port')
    @patch('server.ThreadingHTTPServer')
    def test_server_init(self, mock_http_server, mock_find_port):
        mock_find_port.return_value = 8080

        server_obj = FileShareServer("/tmp/test.txt", port=8080)

        self.assertEqual(server_obj.port, 8080)
        self.assertEqual(server_obj.file_path, "/tmp/test.txt")
        self.assertEqual(server_obj.allowed_filename, "test.txt")
        # In the new implementation, httpd is not created in __init__, but in start()
        self.assertIsNone(server_obj.httpd)

    @patch('server.find_available_port')
    @patch('server.ThreadingHTTPServer')
    def test_server_start(self, mock_http_server_cls, mock_find_port):
        mock_find_port.return_value = 8080
        mock_httpd = MagicMock()
        mock_http_server_cls.return_value = mock_httpd

        server_obj = FileShareServer("/tmp/test.txt")

        # Patch threading.Thread and threading.Timer to verify they are started
        with patch('threading.Thread') as mock_thread_cls, \
             patch('threading.Timer') as mock_timer_cls:

            mock_thread = MagicMock()
            mock_thread_cls.return_value = mock_thread
            mock_timer = MagicMock()
            mock_timer_cls.return_value = mock_timer

            server_obj.start()

            mock_http_server_cls.assert_called()
            mock_thread.start.assert_called()
            mock_timer.start.assert_called()

            # Verify file attributes were injected into httpd
            self.assertEqual(mock_httpd.file_path, "/tmp/test.txt")
            self.assertEqual(mock_httpd.allowed_filename, "test.txt")

    @patch('server.find_available_port')
    def test_server_shutdown_logic(self, mock_find_port):
        mock_find_port.return_value = 8080
        server_obj = FileShareServer("/tmp/test.txt")
        server_obj.httpd = MagicMock()
        server_obj.shutdown_timer = MagicMock()

        server_obj.stop()

        server_obj.shutdown_timer.cancel.assert_called()
        server_obj.httpd.shutdown.assert_called()
        server_obj.httpd.server_close.assert_called()

if __name__ == '__main__':
    unittest.main()
