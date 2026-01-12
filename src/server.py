import socket
import os
import threading
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from socketserver import ThreadingMixIn
from typing import Optional
from security import validate_request_path

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
            file_size = os.path.getsize(file_path)

            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Disposition', f'attachment; filename="{allowed_filename}"')
            self.send_header('Content-Length', str(file_size))
            self.end_headers()

            # Stream file in chunks
            with open(file_path, 'rb') as f:
                while True:
                    chunk = f.read(8192)  # 8KB chunks
                    if not chunk:
                        break
                    self.wfile.write(chunk)

        except Exception as e:
            # Log error if needed, but for now just let the handler finish
            pass

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
