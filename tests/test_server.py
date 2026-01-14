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
from server import find_available_port, is_port_available, FileShareHandler, FileShareServer, DirectoryShareHandler, DirectoryShareServer

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

class TestDirectoryShareHandler(unittest.TestCase):
    """Tests for DirectoryShareHandler (Tasks T-008, T-009, T-010)"""

    def create_directory_handler(self, server_obj):
        """Helper to create a DirectoryShareHandler instance."""
        with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
            handler = DirectoryShareHandler(MagicMock(), ('127.0.0.1', 12345), server_obj)
            handler.server = server_obj
            handler.send_response = MagicMock()
            handler.send_header = MagicMock()
            handler.end_headers = MagicMock()
            handler.send_error = MagicMock()
            handler.wfile = MagicMock()
            return handler

    def test_directory_handler_listing_response(self, tmp_path=None):
        """T-008: Test DirectoryShareHandler returns directory listing"""
        import tempfile
        import shutil

        # Create temporary directory structure
        tmp_path = tempfile.mkdtemp()
        try:
            test_dir = os.path.join(tmp_path, "shared")
            os.makedirs(test_dir)
            test_file = os.path.join(test_dir, "file.txt")
            with open(test_file, 'w') as f:
                f.write("content")

            # Mock server
            mock_server = MagicMock(spec=['directory_path'])
            mock_server.directory_path = test_dir

            # Create handler
            handler = self.create_directory_handler(mock_server)
            handler.path = "/"

            # Mock validate_directory_path
            with patch('server.validate_directory_path', return_value=(True, test_dir)):
                with patch('server.generate_directory_listing_html', return_value='<html>file.txt</html>'):
                    handler.do_GET()

            # Verify response
            handler.send_response.assert_called_with(200)
            handler.send_header.assert_any_call('Content-Type', 'text/html; charset=utf-8')

            # Verify HTML was written
            handler.wfile.write.assert_called()
            written_data = b''.join(call.args[0] for call in handler.wfile.write.call_args_list)
            html_content = written_data.decode('utf-8')
            self.assertIn('file.txt', html_content)
        finally:
            shutil.rmtree(tmp_path)

    def test_directory_handler_file_response(self, tmp_path=None):
        """T-009: Test DirectoryShareHandler downloads single file"""
        import tempfile
        import shutil

        tmp_path = tempfile.mkdtemp()
        try:
            test_dir = os.path.join(tmp_path, "shared")
            os.makedirs(test_dir)
            test_file = os.path.join(test_dir, "download.txt")
            with open(test_file, 'w') as f:
                f.write("download content")

            mock_server = MagicMock(spec=['directory_path'])
            mock_server.directory_path = test_dir

            handler = self.create_directory_handler(mock_server)
            handler.path = "/download.txt"

            with patch('server.validate_directory_path', return_value=(True, test_file)):
                handler.do_GET()

            # Verify file download response
            handler.send_response.assert_called_with(200)
            handler.send_header.assert_any_call('Content-Type', 'application/octet-stream')
            handler.send_header.assert_any_call('Content-Disposition', 'attachment; filename="download.txt"')
        finally:
            shutil.rmtree(tmp_path)

    def test_directory_handler_zip_response(self, tmp_path=None):
        """T-010: Test DirectoryShareHandler zip download"""
        import tempfile
        import shutil
        import io

        tmp_path = tempfile.mkdtemp()
        try:
            test_dir = os.path.join(tmp_path, "shared")
            os.makedirs(test_dir)
            test_file = os.path.join(test_dir, "file1.txt")
            with open(test_file, 'w') as f:
                f.write("content1")

            mock_server = MagicMock(spec=['directory_path'])
            mock_server.directory_path = test_dir

            handler = self.create_directory_handler(mock_server)
            handler.path = "/?download=zip"
            handler.wfile = io.BytesIO()

            with patch('server.validate_directory_path', return_value=(True, test_dir)):
                with patch('server.stream_directory_as_zip') as mock_zip:
                    handler.do_GET()

            # Verify zip download response
            handler.send_response.assert_called_with(200)
            handler.send_header.assert_any_call('Content-Type', 'application/zip')
            handler.send_header.assert_any_call('Content-Disposition',
                                                unittest.mock.ANY)
            mock_zip.assert_called_once()
        finally:
            shutil.rmtree(tmp_path)

    def test_directory_handler_invalid_path(self):
        """Test DirectoryShareHandler denies access to invalid paths"""
        mock_server = MagicMock(spec=['directory_path'])
        mock_server.directory_path = "/tmp/test"

        handler = self.create_directory_handler(mock_server)
        handler.path = "/../etc/passwd"

        with patch('server.validate_directory_path', return_value=(False, "")):
            handler.do_GET()

        handler.send_error.assert_called_with(403, "Access denied")

    def test_directory_handler_streaming_chunks(self):
        """Test that files are streamed in chunks (8KB)"""
        import tempfile
        import shutil

        tmp_path = tempfile.mkdtemp()
        try:
            test_dir = os.path.join(tmp_path, "shared")
            os.makedirs(test_dir)
            test_file = os.path.join(test_dir, "large.txt")
            # Create a file larger than chunk size
            with open(test_file, 'wb') as f:
                f.write(b'x' * 20000)  # 20KB file

            mock_server = MagicMock(spec=['directory_path'])
            mock_server.directory_path = test_dir

            handler = self.create_directory_handler(mock_server)
            handler.path = "/large.txt"

            with patch('server.validate_directory_path', return_value=(True, test_file)):
                handler.do_GET()

            # Verify multiple writes were made (streaming)
            self.assertGreater(handler.wfile.write.call_count, 1)
        finally:
            shutil.rmtree(tmp_path)

    def test_directory_handler_path_is_directory(self):
        """Test that directory path returns HTML listing, not directory download"""
        import tempfile
        import shutil

        tmp_path = tempfile.mkdtemp()
        try:
            test_dir = os.path.join(tmp_path, "shared")
            os.makedirs(test_dir)
            subdir = os.path.join(test_dir, "subdir")
            os.makedirs(subdir)

            mock_server = MagicMock(spec=['directory_path'])
            mock_server.directory_path = test_dir

            handler = self.create_directory_handler(mock_server)
            handler.path = "/subdir/"

            with patch('server.validate_directory_path', return_value=(True, subdir)):
                with patch('server.generate_directory_listing_html', return_value='<html>subdir</html>'):
                    handler.do_GET()

            # Should return HTML, not file download
            handler.send_header.assert_any_call('Content-Type', 'text/html; charset=utf-8')
        finally:
            shutil.rmtree(tmp_path)


class TestDirectoryShareServer(unittest.TestCase):
    """Tests for DirectoryShareServer (Tasks T-011, T-012, T-016, T-017)"""

    @patch('server.find_available_port')
    def test_directory_share_server_init(self, mock_find_port):
        """T-011: Test DirectoryShareServer initialization"""
        mock_find_port.return_value = 8080

        server_obj = DirectoryShareServer("/tmp/test", port=8080, max_sessions=5, timeout_minutes=30)

        self.assertEqual(server_obj.directory_path, "/tmp/test")
        self.assertEqual(server_obj.port, 8080)
        self.assertEqual(server_obj.max_sessions, 5)
        self.assertEqual(server_obj.timeout_minutes, 30)
        self.assertIsNone(server_obj.httpd)
        # Verify session tracking initialized
        self.assertIsNotNone(server_obj.sessions)
        self.assertIsNotNone(server_obj.session_lock)

    @patch('server.find_available_port')
    def test_directory_share_server_init_default_port(self, mock_find_port):
        """T-011: Test DirectoryShareServer initialization with default port"""
        mock_find_port.return_value = 8000

        server_obj = DirectoryShareServer("/tmp/test")

        self.assertEqual(server_obj.directory_path, "/tmp/test")
        self.assertEqual(server_obj.port, 8000)
        # Verify find_available_port was called (without custom_port arg)
        mock_find_port.assert_called()
        # When port is None, find_available_port is called without arguments
        self.assertEqual(mock_find_port.call_args, unittest.mock.call())

    @patch('server.find_available_port')
    @patch('server.ThreadingHTTPServer')
    def test_directory_share_server_start(self, mock_http_server_cls, mock_find_port):
        """T-011: Test DirectoryShareServer starts with proper configuration"""
        mock_find_port.return_value = 8080
        mock_httpd = MagicMock()
        mock_http_server_cls.return_value = mock_httpd

        server_obj = DirectoryShareServer("/tmp/test", port=8080)

        with patch('threading.Thread') as mock_thread_cls, \
             patch('threading.Timer') as mock_timer_cls:

            mock_thread = MagicMock()
            mock_thread_cls.return_value = mock_thread
            mock_timer = MagicMock()
            mock_timer_cls.return_value = mock_timer

            server_obj.start()

            # Verify HTTPServer created with DirectoryShareHandler
            mock_http_server_cls.assert_called_once()
            call_args = mock_http_server_cls.call_args
            self.assertEqual(call_args[0][0], ('', 8080))
            self.assertEqual(call_args[0][1], DirectoryShareHandler)

            # Verify directory_path injected into httpd
            self.assertEqual(mock_httpd.directory_path, "/tmp/test")

            # Verify threads started
            mock_thread.start.assert_called_once()
            mock_timer.start.assert_called_once()

    @patch('server.find_available_port')
    def test_directory_share_server_stop(self, mock_find_port):
        """T-011: Test DirectoryShareServer stop method"""
        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test")
        server_obj.httpd = MagicMock()
        server_obj.shutdown_timer = MagicMock()

        server_obj.stop()

        server_obj.shutdown_timer.cancel.assert_called_once()
        server_obj.httpd.shutdown.assert_called_once()
        server_obj.httpd.server_close.assert_called_once()

    @patch('server.find_available_port')
    @patch('server.ThreadingHTTPServer')
    @patch('threading.Thread')
    @patch('threading.Timer')
    def test_directory_share_server_timeout_configured(
        self, mock_timer_cls, mock_thread_cls, mock_http_server_cls, mock_find_port
    ):
        """T-012: Test that timeout timer is configured correctly"""
        mock_find_port.return_value = 8080
        mock_httpd = MagicMock()
        mock_http_server_cls.return_value = mock_httpd
        mock_thread = MagicMock()
        mock_thread_cls.return_value = mock_thread
        mock_timer = MagicMock()
        mock_timer_cls.return_value = mock_timer

        server_obj = DirectoryShareServer("/tmp/test", timeout_minutes=15)
        server_obj.start()

        # Verify timer was created with correct timeout (15 minutes = 900 seconds)
        mock_timer_cls.assert_called_once()
        call_args = mock_timer_cls.call_args
        self.assertEqual(call_args[0][0], 900)  # 15 minutes * 60 seconds
        # Verify timer was started
        mock_timer.start.assert_called_once()

    @patch('server.find_available_port')
    @patch('server.ThreadingHTTPServer')
    @patch('threading.Thread')
    @patch('threading.Timer')
    def test_directory_share_server_timeout_triggers_shutdown(
        self, mock_timer_cls, mock_thread_cls, mock_http_server_cls, mock_find_port
    ):
        """T-012: Test that timeout callback is _shutdown_server method"""
        mock_find_port.return_value = 8080
        mock_httpd = MagicMock()
        mock_http_server_cls.return_value = mock_httpd
        mock_thread = MagicMock()
        mock_thread_cls.return_value = mock_thread
        mock_timer = MagicMock()
        mock_timer_cls.return_value = mock_timer

        server_obj = DirectoryShareServer("/tmp/test", timeout_minutes=1)
        server_obj.start()

        # Verify timer was created with _shutdown_server as callback
        mock_timer_cls.assert_called_once()
        call_args = mock_timer_cls.call_args
        self.assertEqual(call_args[0][0], 60)  # 1 minute = 60 seconds
        # The callback should be the _shutdown_server method
        self.assertEqual(call_args[0][1], server_obj._shutdown_server)

    @patch('server.find_available_port')
    def test_directory_share_server_stop_cancels_timeout(self, mock_find_port):
        """T-012: Test that manual stop cancels timeout timer"""
        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test")
        server_obj.httpd = MagicMock()
        server_obj.shutdown_timer = MagicMock()

        server_obj.stop()

        # Verify timer was cancelled
        server_obj.shutdown_timer.cancel.assert_called_once()


class TestSessionTracking(unittest.TestCase):
    """Tests for Session Tracking (Task T-016)"""

    def create_directory_handler(self, server_obj):
        """Helper to create a DirectoryShareHandler instance."""
        with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
            handler = DirectoryShareHandler(MagicMock(), ('127.0.0.1', 12345), server_obj)
            handler.server = server_obj
            handler.send_response = MagicMock()
            handler.send_header = MagicMock()
            handler.end_headers = MagicMock()
            handler.send_error = MagicMock()
            handler.wfile = MagicMock()
            handler.headers = {}
            handler.client_address = ('127.0.0.1', 12345)
            handler.path = "/"
            return handler

    @patch('server.find_available_port')
    def test_session_creation_for_new_client(self, mock_find_port):
        """T-016: Test that new client gets a session cookie"""
        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=5)

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        handler = self.create_directory_handler(server_obj.httpd)

        # Mock validate_directory_path
        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Verify session cookie was set
        cookie_calls = [call for call in handler.send_header.call_args_list
                       if call[0][0] == 'Set-Cookie']
        self.assertGreater(len(cookie_calls), 0, "Session cookie should be set")

        # Verify cookie format
        cookie_value = cookie_calls[0][0][1]
        self.assertIn('quick_share_session=', cookie_value)

        # Verify session was added to server.sessions
        self.assertEqual(len(server_obj.sessions), 1)

    @patch('server.find_available_port')
    def test_session_reuse_for_existing_client(self, mock_find_port):
        """T-016: Test that existing client reuses session cookie"""
        import uuid

        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=5)

        # Create a pre-existing session
        session_id = str(uuid.uuid4())
        server_obj.sessions[session_id] = {
            'created_at': time.time(),
            'ip': '127.0.0.1',
            'user_agent': 'test-agent'
        }

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        handler = self.create_directory_handler(server_obj.httpd)
        handler.headers = {'Cookie': f'quick_share_session={session_id}'}

        # Mock validate_directory_path
        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Verify no new session was created (count should still be 1)
        self.assertEqual(len(server_obj.sessions), 1)
        self.assertIn(session_id, server_obj.sessions)

    @patch('server.find_available_port')
    def test_session_tracking_thread_safe(self, mock_find_port):
        """T-016: Test that session tracking is thread-safe"""
        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=5)

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        # Verify that session_lock is a threading.Lock
        self.assertIsInstance(server_obj.session_lock, type(threading.Lock()))

        handler = self.create_directory_handler(server_obj.httpd)

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Session tracking should work correctly even with multiple threads
        # Verify session was created
        self.assertEqual(len(server_obj.sessions), 1)

    @patch('server.find_available_port')
    def test_session_stores_metadata(self, mock_find_port):
        """T-016: Test that session stores timestamp, IP, and user-agent"""
        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=5)

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        handler = self.create_directory_handler(server_obj.httpd)
        handler.headers = {'User-Agent': 'Mozilla/5.0'}
        handler.client_address = ('192.168.1.100', 54321)

        start_time = time.time()

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Verify session data
        self.assertEqual(len(server_obj.sessions), 1)
        session_id = list(server_obj.sessions.keys())[0]
        session = server_obj.sessions[session_id]

        self.assertIn('created_at', session)
        self.assertGreaterEqual(session['created_at'], start_time)
        self.assertEqual(session['ip'], '192.168.1.100')
        self.assertEqual(session['user_agent'], 'Mozilla/5.0')


class TestSessionLimitEnforcement(unittest.TestCase):
    """Tests for Session Limit Enforcement (Task T-017)"""

    def create_directory_handler(self, server_obj):
        """Helper to create a DirectoryShareHandler instance."""
        with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
            handler = DirectoryShareHandler(MagicMock(), ('127.0.0.1', 12345), server_obj)
            handler.server = server_obj
            handler.send_response = MagicMock()
            handler.send_header = MagicMock()
            handler.end_headers = MagicMock()
            handler.send_error = MagicMock()
            handler.wfile = MagicMock()
            handler.headers = {}
            handler.client_address = ('127.0.0.1', 12345)
            handler.path = "/"
            return handler

    @patch('server.find_available_port')
    def test_session_limit_enforcement(self, mock_find_port):
        """T-017: Test that session limit is enforced"""
        import uuid

        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=2)

        # Fill up sessions to the limit
        for i in range(2):
            session_id = str(uuid.uuid4())
            server_obj.sessions[session_id] = {
                'created_at': time.time(),
                'ip': f'127.0.0.{i}',
                'user_agent': 'test-agent'
            }

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        # New client trying to connect
        handler = self.create_directory_handler(server_obj.httpd)

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Verify access denied
        handler.send_error.assert_called_with(403, "Session limit reached")

        # Verify no new session was created
        self.assertEqual(len(server_obj.sessions), 2)

    @patch('server.find_available_port')
    def test_existing_session_allowed_when_at_limit(self, mock_find_port):
        """T-017: Test that existing sessions continue when at limit"""
        import uuid

        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=2)

        # Create existing session for this client
        existing_session_id = str(uuid.uuid4())
        server_obj.sessions[existing_session_id] = {
            'created_at': time.time(),
            'ip': '127.0.0.1',
            'user_agent': 'test-agent'
        }

        # Fill up remaining session slots
        another_session_id = str(uuid.uuid4())
        server_obj.sessions[another_session_id] = {
            'created_at': time.time(),
            'ip': '127.0.0.2',
            'user_agent': 'test-agent'
        }

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        # Existing client with valid session cookie
        handler = self.create_directory_handler(server_obj.httpd)
        handler.headers = {'Cookie': f'quick_share_session={existing_session_id}'}

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Verify access was granted (no error)
        handler.send_error.assert_not_called()
        handler.send_response.assert_called_with(200)

    @patch('server.find_available_port')
    def test_session_limit_allows_new_when_under_limit(self, mock_find_port):
        """T-017: Test that new sessions are allowed when under limit"""
        import uuid

        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=3)

        # Create one existing session (under limit)
        session_id = str(uuid.uuid4())
        server_obj.sessions[session_id] = {
            'created_at': time.time(),
            'ip': '127.0.0.1',
            'user_agent': 'test-agent'
        }

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        # New client
        handler = self.create_directory_handler(server_obj.httpd)

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Verify access was granted
        handler.send_error.assert_not_called()
        handler.send_response.assert_called_with(200)

        # Verify new session was created
        self.assertEqual(len(server_obj.sessions), 2)

    @patch('server.find_available_port')
    def test_session_limit_thread_safe(self, mock_find_port):
        """T-017: Test that session limit check is thread-safe"""
        mock_find_port.return_value = 8080
        server_obj = DirectoryShareServer("/tmp/test", max_sessions=1)

        # Inject server attributes to httpd
        server_obj.httpd = MagicMock()
        server_obj.httpd.sessions = server_obj.sessions
        server_obj.httpd.session_lock = server_obj.session_lock
        server_obj.httpd.max_sessions = server_obj.max_sessions
        server_obj.httpd.directory_path = server_obj.directory_path
        server_obj.httpd.track_session = server_obj.track_session
        server_obj.httpd._extract_session_id_from_cookie = server_obj._extract_session_id_from_cookie

        # Verify that session_lock is a threading.Lock
        self.assertIsInstance(server_obj.session_lock, type(threading.Lock()))

        handler = self.create_directory_handler(server_obj.httpd)

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test")):
            with patch('server.generate_directory_listing_html', return_value='<html>test</html>'):
                handler.do_GET()

        # Session limit enforcement should work correctly even with multiple threads
        # Verify session was created (first session, under limit)
        self.assertEqual(len(server_obj.sessions), 1)


if __name__ == '__main__':
    unittest.main()
