import unittest
from unittest.mock import MagicMock, patch
import json
import sys
import os
from http.server import BaseHTTPRequestHandler

# Adjust path to include src
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from server import DirectoryShareHandler

class TestApiRouting(unittest.TestCase):
    def setUp(self):
        self.mock_server = MagicMock()
        self.mock_server.directory_path = "/tmp/test"

        # Patch BaseHTTPRequestHandler so we can instantiate the handler
        with patch.object(BaseHTTPRequestHandler, '__init__', return_value=None):
            self.handler = DirectoryShareHandler(MagicMock(), ('127.0.0.1', 12345), self.mock_server)
            self.handler.server = self.mock_server
            self.handler.request = MagicMock()
            self.handler.client_address = ('127.0.0.1', 12345)
            self.handler.wfile = MagicMock()
            self.handler.rfile = MagicMock()
            self.handler.headers = {}

            # Mock helper methods
            self.handler.send_response = MagicMock()
            self.handler.send_header = MagicMock()
            self.handler.end_headers = MagicMock()
            self.handler.send_error = MagicMock()

            # Inject session methods if needed (mimicking DirectoryShareServer setup)
            self.handler.session_id = None
            if not hasattr(self.handler, '_set_session_cookie_if_needed'):
                 self.handler._set_session_cookie_if_needed = MagicMock()

        # Mock track_session to avoid ValueError
        self.mock_server.track_session.return_value = (True, "mock-session-id")

    def test_api_request_routing(self):
        """Test that /api/ requests are intercepted and routed."""
        # Test unknown endpoint
        self.handler.path = "/api/nonexistent"
        self.handler.do_GET()

        # Verify 404 response
        self.handler.send_response.assert_called_with(404)
        self.handler.send_header.assert_any_call('Content-Type', 'application/json')

        # Verify JSON body
        written_data = b''.join(call.args[0] for call in self.handler.wfile.write.call_args_list)
        response = json.loads(written_data.decode('utf-8'))
        self.assertEqual(response['error'], "API Endpoint Not Found")
        self.assertEqual(response['status'], 404)

    @patch('server.validate_directory_path')
    @patch('server.get_directory_structure')
    @patch('os.path.exists')
    @patch('os.path.isdir')
    def test_tree_api_success(self, mock_isdir, mock_exists, mock_get_structure, mock_validate):
        """Test that /api/tree returns directory structure."""
        self.handler.path = "/api/tree?path=/subdir"

        # Mock successful validation and data retrieval
        mock_validate.return_value = (True, "/tmp/test/subdir")
        mock_exists.return_value = True
        mock_isdir.return_value = True

        expected_data = {
            'path': '/subdir',
            'items': [{'name': 'file.txt', 'type': 'file'}]
        }
        mock_get_structure.return_value = expected_data

        self.handler.do_GET()

        # Verify response
        self.handler.send_response.assert_called_with(200)

        # Verify JSON
        written_data = b''.join(call.args[0] for call in self.handler.wfile.write.call_args_list)
        response = json.loads(written_data.decode('utf-8'))
        self.assertEqual(response, expected_data)

    def test_tree_api_placeholder(self):
        """Deprecated: Covered by test_tree_api_success"""
        pass

    def test_content_api_success(self):
        """Test that /api/content returns file content."""
        self.handler.path = "/api/content?path=/test.txt"

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test/test.txt")), \
             patch('os.path.exists', return_value=True), \
             patch('os.path.isfile', return_value=True), \
             patch('os.path.getsize', return_value=100), \
             patch('mimetypes.guess_type', return_value=('text/plain', None)), \
             patch('builtins.open', unittest.mock.mock_open(read_data=b'hello world')):

            self.handler.do_GET()

            # Verify response
            self.handler.send_response.assert_called_with(200)

            # Verify JSON
            written_data = b''.join(call.args[0] for call in self.handler.wfile.write.call_args_list)
            response = json.loads(written_data.decode('utf-8'))
            self.assertEqual(response['content'], "hello world")
            self.assertEqual(response['type'], "text/plain")

    def test_content_api_too_large(self):
        """Test that /api/content rejects large files."""
        self.handler.path = "/api/content?path=/large.txt"

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test/large.txt")), \
             patch('os.path.exists', return_value=True), \
             patch('os.path.isfile', return_value=True), \
             patch('os.path.getsize', return_value=2 * 1024 * 1024):  # 2MB

            self.handler.do_GET()

            # Verify 413 response
            self.handler.send_response.assert_called_with(413)

    def test_content_api_binary(self):
        """Test that /api/content handles binary files gracefully."""
        self.handler.path = "/api/content?path=/binary.bin"

        # Create a mock open that raises UnicodeDecodeError when read().decode() is attempted
        # Or simpler: verify implementation catches decoding error.
        # Implementation strategy: read bytes, try decode('utf-8').

        # We'll mock open to return bytes that are invalid utf-8
        invalid_utf8 = b'\x80\x81'

        with patch('server.validate_directory_path', return_value=(True, "/tmp/test/binary.bin")), \
             patch('os.path.exists', return_value=True), \
             patch('os.path.isfile', return_value=True), \
             patch('os.path.getsize', return_value=100), \
             patch('builtins.open', unittest.mock.mock_open(read_data=invalid_utf8)):

            self.handler.do_GET()

            # Verify 415 response (Unsupported Media Type) or generic error
            self.handler.send_response.assert_called_with(415)

    def test_content_api_placeholder(self):
        """Deprecated: Covered by test_content_api_success"""
        pass

    def test_unknown_api_endpoint(self):
        """Deprecated: Covered by test_api_request_routing"""
        pass

if __name__ == '__main__':
    unittest.main()
