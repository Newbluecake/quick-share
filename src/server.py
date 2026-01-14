import socket
import os
import threading
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from socketserver import ThreadingMixIn
from typing import Optional, Tuple

try:
    from .security import validate_request_path, validate_directory_path
    from .directory_handler import generate_directory_listing_html, stream_directory_as_zip
except ImportError:
    from security import validate_request_path, validate_directory_path
    from directory_handler import generate_directory_listing_html, stream_directory_as_zip

# Constants
CHUNK_SIZE = 8192  # 8KB chunks for file streaming


class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    """Handle requests in a separate thread."""
    pass

def is_port_available(port: int) -> bool:
    """Check if a port is available for binding."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        try:
            sock.bind(('', port))
            return True
        except OSError:
            return False

def find_available_port(start: int = 8000, end: int = 8099, custom_port: Optional[int] = None) -> int:
    """
    Find an available port within range or verify custom port.

    Args:
        start: Start of port range (inclusive)
        end: End of port range (inclusive)
        custom_port: Specific port to check

    Returns:
        Available port number

    Raises:
        RuntimeError: If no port is available
    """
    if custom_port is not None:
        if is_port_available(custom_port):
            return custom_port
        raise RuntimeError(f"Custom port {custom_port} is not available")

    for port in range(start, end + 1):
        if is_port_available(port):
            return port

    raise RuntimeError(f"No available ports found in range {start}-{end}")

class FileShareHandler(BaseHTTPRequestHandler):
    """Handler for serving a single file securely."""

    def do_GET(self):
        """Handle GET requests."""
        # Get server configuration
        file_path = self.server.file_path
        allowed_filename = self.server.allowed_filename

        # Validate path using security module
        is_valid, normalized_path = validate_request_path(self.path, allowed_filename)

        if not is_valid:
            self.send_error(403, "Access denied")
            return

        if not os.path.exists(file_path):
            self.send_error(404, "File not found")
            return

        try:
            self._stream_file(file_path, allowed_filename)
        except Exception as e:
            # Log error if needed, but for now just let the handler finish
            pass

    def _stream_file(self, file_path: str, filename: str):
        """Stream a file to the client in chunks."""
        file_size = os.path.getsize(file_path)

        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # Stream file in chunks
        with open(file_path, 'rb') as f:
            while True:
                chunk = f.read(CHUNK_SIZE)
                if not chunk:
                    break
                self.wfile.write(chunk)

    def log_message(self, format, *args):
        """Suppress default logging to stdout/stderr unless needed."""
        # We could implement custom logging here, but for now silence is golden for a library
        pass

class FileShareServer:
    """Managed HTTP server for file sharing."""

    def __init__(self, file_path: str, port: Optional[int] = None, timeout_minutes: int = 30):
        self.file_path = os.path.abspath(file_path)
        self.allowed_filename = os.path.basename(file_path)
        self.port = find_available_port(custom_port=port) if port else find_available_port()
        self.timeout_minutes = timeout_minutes
        self.httpd: Optional[HTTPServer] = None
        self.server_thread: Optional[threading.Thread] = None
        self.shutdown_timer: Optional[threading.Timer] = None

    def start(self):
        """Start the server in a background thread."""
        self.httpd = ThreadingHTTPServer(('', self.port), FileShareHandler)
        # Inject file info into server instance so handler can access it
        self.httpd.file_path = self.file_path
        self.httpd.allowed_filename = self.allowed_filename

        self.server_thread = threading.Thread(target=self.httpd.serve_forever)
        self.server_thread.daemon = True
        self.server_thread.start()

        # Schedule auto-shutdown
        self.shutdown_timer = threading.Timer(self.timeout_minutes * 60, self._shutdown_server)
        self.shutdown_timer.start()

    def stop(self):
        """Stop the server manually."""
        self._shutdown_server()

    def _shutdown_server(self):
        """Internal shutdown logic."""
        if self.shutdown_timer:
            self.shutdown_timer.cancel()

        if self.httpd:
            self.httpd.shutdown()
            self.httpd.server_close()


class DirectoryShareHandler(BaseHTTPRequestHandler):
    """Handler for serving a directory securely."""

    def do_GET(self):
        """Handle GET requests: directory listing, file download, or zip download."""
        directory_path = self.server.directory_path

        # Track session and enforce limit
        if hasattr(self.server, 'sessions'):
            # Session tracking is enabled
            allowed, session_id = self.server.track_session(self)
            if not allowed:
                self.send_error(403, "Session limit reached")
                return
            # Store session_id for cookie setting
            self.session_id = session_id
        else:
            self.session_id = None

        # Check if requesting zip download (before path validation)
        # For RESTful format (/download/{name}.zip), we need to validate root path
        if self._is_zip_download_request():
            # For RESTful zip URLs, validate the root directory instead
            if self.path.startswith('/download/') and self.path.endswith('.zip'):
                # Validate root directory
                is_valid, real_path = validate_directory_path('/', directory_path)
            else:
                # For query parameter format, validate the actual path
                is_valid, real_path = validate_directory_path(
                    self.path,
                    directory_path
                )

            if not is_valid:
                self.send_error(403, "Access denied")
                return

            self._serve_directory_zip(directory_path, real_path)
            return

        # Validate path for non-zip requests
        is_valid, real_path = validate_directory_path(
            self.path,
            directory_path
        )

        if not is_valid:
            self.send_error(403, "Access denied")
            return

        # Determine if path is a file or directory
        if os.path.isfile(real_path):
            self._serve_file(real_path)
        else:
            self._serve_directory_listing(directory_path, real_path)

    def _is_zip_download_request(self) -> bool:
        """Check if the request is for zip download."""
        # Support both query parameter format and RESTful path format
        return ('?download=zip' in self.path or
                '?action=zip' in self.path or
                self.path.startswith('/download/') and self.path.endswith('.zip'))

    def _serve_directory_listing(self, base_dir: str, current_dir: str):
        """Generate and return directory listing HTML."""
        html = generate_directory_listing_html(base_dir, current_dir)
        html_bytes = html.encode('utf-8')

        self.send_response(200)
        self._set_session_cookie_if_needed()
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(html_bytes)))
        self.end_headers()
        self.wfile.write(html_bytes)

    def _serve_file(self, file_path: str):
        """Stream a single file to the client."""
        filename = os.path.basename(file_path)

        try:
            self._stream_file_with_headers(file_path, filename)
        except OSError as e:
            self.send_error(500, "Internal server error")
        except Exception as e:
            self.send_error(500, "Internal server error")

    def _stream_file_with_headers(self, file_path: str, filename: str):
        """Send headers and stream file content."""
        file_size = os.path.getsize(file_path)

        self.send_response(200)
        self._set_session_cookie_if_needed()
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # Stream file in chunks
        with open(file_path, 'rb') as f:
            while True:
                chunk = f.read(CHUNK_SIZE)
                if not chunk:
                    break
                self.wfile.write(chunk)

    def _serve_directory_zip(self, base_dir: str, target_dir: str):
        """Stream directory as zip file."""
        dir_name = os.path.basename(base_dir)
        zip_filename = f"{dir_name}.zip"

        self.send_response(200)
        self._set_session_cookie_if_needed()
        self.send_header('Content-Type', 'application/zip')
        self.send_header('Content-Disposition', f'attachment; filename="{zip_filename}"')
        # Don't set Transfer-Encoding or Content-Length
        # Let the connection close naturally after streaming
        self.end_headers()

        # Stream zip to client
        try:
            stream_directory_as_zip(self.wfile, base_dir, target_dir)
        except (BrokenPipeError, ConnectionResetError):
            # Client disconnected - this is normal, ignore it
            pass

    def _set_session_cookie_if_needed(self):
        """Set session cookie header if we have a session_id."""
        if hasattr(self, 'session_id') and self.session_id:
            self.send_header('Set-Cookie', f'quick_share_session={self.session_id}; Path=/; HttpOnly')

    def log_message(self, format, *args):
        """Suppress default logging."""
        pass


class DirectoryShareServer:
    """Managed HTTP server for directory sharing with session support."""

    def __init__(
        self,
        directory_path: str,
        port: Optional[int] = None,
        timeout_minutes: int = 30,
        max_sessions: int = 10
    ):
        """
        Initialize DirectoryShareServer.

        Args:
            directory_path: Path to directory to share
            port: Port to bind to (None for auto-select)
            timeout_minutes: Minutes before auto-shutdown
            max_sessions: Maximum number of concurrent sessions
        """
        self.directory_path = os.path.abspath(directory_path)
        self.port = find_available_port(custom_port=port) if port else find_available_port()
        self.timeout_minutes = timeout_minutes
        self.max_sessions = max_sessions

        # Session management (to be implemented in later tasks)
        self.sessions = {}
        self.session_lock = threading.Lock()

        self.httpd: Optional[HTTPServer] = None
        self.server_thread: Optional[threading.Thread] = None
        self.shutdown_timer: Optional[threading.Timer] = None

    def track_session(self, request_handler) -> Tuple[bool, Optional[str]]:
        """
        Track session and return whether access is allowed.

        Args:
            request_handler: HTTP request handler instance

        Returns:
            Tuple of (allowed: bool, session_id: Optional[str])
        """
        import uuid
        import time

        with self.session_lock:
            # Get or create session ID from cookie
            cookie_header = request_handler.headers.get('Cookie', '')
            session_id = self._extract_session_id_from_cookie(cookie_header)

            # Check if this is an existing session
            if session_id and session_id in self.sessions:
                # Existing session - always allow
                return True, session_id

            # New session - check limit
            if len(self.sessions) >= self.max_sessions:
                return False, None

            # Create new session
            if not session_id:
                session_id = str(uuid.uuid4())

            self.sessions[session_id] = {
                'ip': request_handler.client_address[0],
                'created_at': time.time(),
                'user_agent': request_handler.headers.get('User-Agent', 'Unknown')
            }

            return True, session_id

    def _extract_session_id_from_cookie(self, cookie_header: str) -> Optional[str]:
        """Extract session ID from Cookie header."""
        if not cookie_header:
            return None

        for part in cookie_header.split(';'):
            part = part.strip()
            if part.startswith('quick_share_session='):
                return part.split('=', 1)[1]

        return None

    def start(self):
        """Start the server in a background thread."""
        self.httpd = ThreadingHTTPServer(('', self.port), DirectoryShareHandler)

        # Inject directory info and session management into server instance
        self.httpd.directory_path = self.directory_path
        self.httpd.sessions = self.sessions
        self.httpd.session_lock = self.session_lock
        self.httpd.max_sessions = self.max_sessions
        # Inject track_session method so handler can call it
        self.httpd.track_session = self.track_session
        self.httpd._extract_session_id_from_cookie = self._extract_session_id_from_cookie

        self.server_thread = threading.Thread(target=self.httpd.serve_forever)
        self.server_thread.daemon = True
        self.server_thread.start()

        # Schedule auto-shutdown
        self.shutdown_timer = threading.Timer(self.timeout_minutes * 60, self._shutdown_server)
        self.shutdown_timer.start()

    def stop(self):
        """Stop the server manually."""
        self._shutdown_server()

    def _shutdown_server(self):
        """Internal shutdown logic."""
        if self.shutdown_timer:
            self.shutdown_timer.cancel()

        if self.httpd:
            self.httpd.shutdown()
            self.httpd.server_close()
