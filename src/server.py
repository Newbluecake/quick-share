import socket
import os
import threading
import time
import json
from http.server import HTTPServer, BaseHTTPRequestHandler
from socketserver import ThreadingMixIn
from typing import Optional, Tuple

try:
    from .security import validate_request_path, validate_directory_path
    from .directory_handler import generate_directory_listing_html, stream_directory_as_zip, get_directory_structure, generate_spa_html
except ImportError:
    from security import validate_request_path, validate_directory_path
    from directory_handler import generate_directory_listing_html, stream_directory_as_zip, get_directory_structure, generate_spa_html

# Constants
CHUNK_SIZE = 8192  # 8KB chunks for file streaming


class DownloadProgressTracker:
    """Track download progress for a single client connection.

    This class is thread-safe as each connection gets its own instance.
    No shared state means no locks needed.
    """

    def __init__(self, client_ip: str, filename: str, file_size: int):
        """
        Initialize progress tracker.

        Args:
            client_ip: Client IP address
            filename: Download filename
            file_size: Total file size in bytes
        """
        self.client_ip = client_ip
        self.filename = filename
        self.file_size = file_size
        self.bytes_transferred = 0
        self.start_time = time.time()
        self.is_complete = False

    def update(self, chunk_size: int) -> bool:
        """
        Update progress after each chunk.

        Args:
            chunk_size: Size of transferred chunk

        Returns:
            True if should log progress (every N chunks to avoid spam)
        """
        self.bytes_transferred += chunk_size
        # Log every 10 chunks (~80KB) to avoid excessive output
        return self.bytes_transferred % (CHUNK_SIZE * 10) == 0 or self.bytes_transferred == self.file_size

    def complete(self):
        """Mark download as complete."""
        self.is_complete = True

    def get_progress_percentage(self) -> float:
        """Calculate progress percentage."""
        if self.file_size == 0:
            return 0.0
        return (self.bytes_transferred / self.file_size) * 100


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
        """Stream a file to the client in chunks with progress tracking."""
        try:
            from .logger import (
                format_download_start,
                format_download_progress,
                format_download_complete,
                format_download_interrupted,
                format_download_error,
                get_timestamp
            )
            from .directory_handler import format_file_size
        except ImportError:
            from logger import (
                format_download_start,
                format_download_progress,
                format_download_complete,
                format_download_interrupted,
                format_download_error,
                get_timestamp
            )
            from directory_handler import format_file_size

        import datetime

        file_size = os.path.getsize(file_path)
        client_ip = self.client_address[0]

        self.send_response(200)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # Initialize progress tracker
        tracker = DownloadProgressTracker(client_ip, filename, file_size)

        # Log download start
        timestamp = get_timestamp()
        print(format_download_start(timestamp, client_ip, filename, format_file_size(file_size)))

        # Stream file in chunks
        try:
            with open(file_path, 'rb') as f:
                while True:
                    chunk = f.read(CHUNK_SIZE)
                    if not chunk:
                        break

                    self.wfile.write(chunk)

                    # Update progress
                    if tracker.update(len(chunk)):
                        percentage = tracker.get_progress_percentage()
                        timestamp = get_timestamp()
                        print(format_download_progress(
                            timestamp,
                            client_ip,
                            tracker.bytes_transferred,
                            file_size,
                            percentage
                        ))

            # Log completion
            tracker.complete()
            duration = time.time() - tracker.start_time
            timestamp = get_timestamp()
            print(format_download_complete(timestamp, client_ip, filename, file_size, duration))

        except (BrokenPipeError, ConnectionResetError):
            # Client disconnected
            timestamp = get_timestamp()
            print(format_download_interrupted(
                timestamp,
                client_ip,
                filename,
                tracker.bytes_transferred,
                file_size
            ))
        except Exception as e:
            # Other errors
            timestamp = get_timestamp()
            print(format_download_error(timestamp, client_ip, filename, str(e)))

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

        # Handle API requests
        if self.path.startswith('/api/'):
            self._handle_api_request()
            return

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

    def _handle_api_request(self):
        """Handle JSON API requests."""
        from urllib.parse import urlparse, parse_qs

        parsed_path = urlparse(self.path)
        query_params = parse_qs(parsed_path.query)

        # Simple routing for now
        if parsed_path.path == '/api/tree':
            # Get path from query params, default to root
            request_path = query_params.get('path', ['/'])[0]

            # Validate path
            is_valid, real_path = validate_directory_path(
                request_path,
                self.server.directory_path
            )

            if not is_valid:
                self._send_json_error(403, "Access denied")
                return

            if not os.path.exists(real_path):
                self._send_json_error(404, "Path not found")
                return

            if not os.path.isdir(real_path):
                self._send_json_error(400, "Path is not a directory")
                return

            try:
                data = get_directory_structure(self.server.directory_path, real_path)
                self._send_json_response(data)
            except Exception as e:
                self._send_json_error(500, str(e))

        elif parsed_path.path == '/api/content':
            # Get path from query params
            request_path = query_params.get('path', [''])[0]
            if not request_path:
                self._send_json_error(400, "Missing path parameter")
                return

            # Validate path
            is_valid, real_path = validate_directory_path(
                request_path,
                self.server.directory_path
            )

            if not is_valid:
                self._send_json_error(403, "Access denied")
                return

            if not os.path.exists(real_path):
                self._send_json_error(404, "File not found")
                return

            if not os.path.isfile(real_path):
                self._send_json_error(400, "Path is not a file")
                return

            # Check size limit (1MB)
            try:
                file_size = os.path.getsize(real_path)
                if file_size > 1024 * 1024:  # 1MB
                    self._send_json_error(413, "File too large for preview (max 1MB)")
                    return
            except OSError:
                self._send_json_error(500, "Error reading file info")
                return

            # Read content
            try:
                with open(real_path, 'rb') as f:
                    content_bytes = f.read()

                # Try decode as utf-8
                try:
                    content_str = content_bytes.decode('utf-8')
                    import mimetypes
                    mime_type, _ = mimetypes.guess_type(real_path)

                    data = {
                        'path': request_path,
                        'content': content_str,
                        'size': file_size,
                        'encoding': 'utf-8',
                        'type': mime_type or 'text/plain'
                    }
                    self._send_json_response(data)
                except UnicodeDecodeError:
                    self._send_json_error(415, "Binary file not supported for preview")
                    return

            except Exception as e:
                import traceback
                traceback.print_exc()
                self._send_json_error(500, str(e))
        else:
            self._send_json_error(404, "API Endpoint Not Found")

    def _send_json_response(self, data: dict, status: int = 200):
        """Send a JSON response."""
        response_body = json.dumps(data).encode('utf-8')

        self.send_response(status)
        self._set_session_cookie_if_needed()
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(response_body)))
        self.end_headers()
        self.wfile.write(response_body)

    def _send_json_error(self, status: int, message: str):
        """Send a JSON error response."""
        data = {
            'error': message,
            'status': status
        }
        self._send_json_response(data, status)

    def _is_zip_download_request(self) -> bool:
        """Check if the request is for zip download."""
        # Support both query parameter format and RESTful path format
        return ('?download=zip' in self.path or
                '?action=zip' in self.path or
                self.path.startswith('/download/') and self.path.endswith('.zip'))

    def _serve_directory_listing(self, base_dir: str, current_dir: str):
        """Generate and return directory listing HTML."""
        # Check for legacy view toggle (server config or query param)
        use_legacy = getattr(self.server, 'legacy_mode', False) or '?legacy=1' in self.path

        if use_legacy:
            html = generate_directory_listing_html(base_dir, current_dir)
        else:
            # Serve SPA
            html = generate_spa_html(os.path.basename(base_dir))

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
        """Send headers and stream file content with progress tracking."""
        try:
            from .logger import (
                format_download_start,
                format_download_progress,
                format_download_complete,
                format_download_interrupted,
                format_download_error,
                get_timestamp
            )
            from .directory_handler import format_file_size
        except ImportError:
            from logger import (
                format_download_start,
                format_download_progress,
                format_download_complete,
                format_download_interrupted,
                format_download_error,
                get_timestamp
            )
            from directory_handler import format_file_size

        file_size = os.path.getsize(file_path)
        client_ip = self.client_address[0]

        self.send_response(200)
        self._set_session_cookie_if_needed()
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
        self.send_header('Content-Length', str(file_size))
        self.end_headers()

        # Initialize progress tracker
        tracker = DownloadProgressTracker(client_ip, filename, file_size)

        # Log download start
        timestamp = get_timestamp()
        print(format_download_start(timestamp, client_ip, filename, format_file_size(file_size)))

        # Stream file in chunks
        try:
            with open(file_path, 'rb') as f:
                while True:
                    chunk = f.read(CHUNK_SIZE)
                    if not chunk:
                        break

                    self.wfile.write(chunk)

                    # Update progress
                    if tracker.update(len(chunk)):
                        percentage = tracker.get_progress_percentage()
                        timestamp = get_timestamp()
                        print(format_download_progress(
                            timestamp,
                            client_ip,
                            tracker.bytes_transferred,
                            file_size,
                            percentage
                        ))

            # Log completion
            tracker.complete()
            duration = time.time() - tracker.start_time
            timestamp = get_timestamp()
            print(format_download_complete(timestamp, client_ip, filename, file_size, duration))

        except (BrokenPipeError, ConnectionResetError):
            # Client disconnected
            timestamp = get_timestamp()
            print(format_download_interrupted(
                timestamp,
                client_ip,
                filename,
                tracker.bytes_transferred,
                file_size
            ))
        except Exception as e:
            # Other errors
            timestamp = get_timestamp()
            print(format_download_error(timestamp, client_ip, filename, str(e)))

    def _serve_directory_zip(self, base_dir: str, target_dir: str):
        """Stream directory as zip file with progress tracking."""
        dir_name = os.path.basename(base_dir)
        zip_filename = f"{dir_name}.zip"

        self.send_response(200)
        self._set_session_cookie_if_needed()
        self.send_header('Content-Type', 'application/zip')
        self.send_header('Content-Disposition', f'attachment; filename="{zip_filename}"')
        # Don't set Transfer-Encoding or Content-Length
        # Let the connection close naturally after streaming
        self.end_headers()

        # Stream zip to client with progress tracking
        try:
            stream_directory_as_zip(self.wfile, base_dir, target_dir, progress_callback=True)
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
        max_sessions: int = 10,
        legacy_mode: bool = False
    ):
        """
        Initialize DirectoryShareServer.

        Args:
            directory_path: Path to directory to share
            port: Port to bind to (None for auto-select)
            timeout_minutes: Minutes before auto-shutdown
            max_sessions: Maximum number of concurrent sessions
            legacy_mode: If True, use legacy server-side rendering by default
        """
        self.directory_path = os.path.abspath(directory_path)
        self.port = find_available_port(custom_port=port) if port else find_available_port()
        self.timeout_minutes = timeout_minutes
        self.max_sessions = max_sessions
        self.legacy_mode = legacy_mode

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
        self.httpd.legacy_mode = self.legacy_mode
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
