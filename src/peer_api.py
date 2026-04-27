"""Peer API handlers for quick-share inter-instance communication.

Processes /api/peer/* HTTP requests — validates shared secrets, opens
system file dialogs, and orchestrates file transfers between peers.
"""

import json
import os
from http.server import BaseHTTPRequestHandler

try:
    from .file_dialog import open_file_dialog, open_directory_dialog, open_save_dialog
    from .peer_client import PeerClient
    from .logger import (
        format_peer_upload_request,
        format_peer_download_request,
        format_peer_transfer_complete,
        format_peer_auth_failed,
        get_timestamp,
    )
except ImportError:
    from file_dialog import open_file_dialog, open_directory_dialog, open_save_dialog
    from peer_client import PeerClient
    from logger import (
        format_peer_upload_request,
        format_peer_download_request,
        format_peer_transfer_complete,
        format_peer_auth_failed,
        get_timestamp,
    )


def handle_peer_api(handler, server_config: dict) -> bool:
    """Route /api/peer/* requests. Returns True if the request was handled."""
    path = handler.path.split("?")[0]

    if not path.startswith("/api/peer/"):
        return False

    if not _verify_peer_secret(handler, server_config):
        return True

    if handler.command == "POST":
        if path == "/api/peer/hello":
            return _handle_peer_hello(handler)
        elif path == "/api/peer/request-upload":
            return _handle_peer_request_upload(handler, server_config)
        elif path == "/api/peer/request-download":
            return _handle_peer_request_download(handler, server_config)
        elif path == "/api/peer/receive":
            return _handle_peer_receive(handler, server_config)

    _send_json_error(handler, 404, "Peer API endpoint not found")
    return True


# -- auth ------------------------------------------------------------


def _verify_peer_secret(handler, server_config: dict) -> bool:
    """Check X-Peer-Secret header against configured secret.

    Returns True if valid (or peer not configured — we only verify if
    a secret is configured). Sends 403 and returns False on mismatch.
    """
    configured_secret = server_config.get("peer_secret")
    if not configured_secret:
        _send_json_error(handler, 403, "Peer secret not configured")
        return False
    provided = handler.headers.get("X-Peer-Secret", "")
    if provided != configured_secret:
        peer_addr = handler.client_address[0]
        print(format_peer_auth_failed(peer_addr))
        _send_json_error(handler, 403, "Invalid peer secret")
        return False
    return True


# -- handlers --------------------------------------------------------


def _handle_peer_hello(handler) -> bool:
    """Process peer registration. Returns peer info."""
    body = _read_body(handler)
    try:
        data = json.loads(body) if body else {}
    except json.JSONDecodeError:
        data = {}
    peer_host = data.get("host", handler.client_address[0])
    peer_port = data.get("port", "unknown")
    try:
        from . import __full_version__
    except ImportError:
        __full_version__ = "unknown"
    _send_json(handler, {"status": "ok", "version": __full_version__,
                          "peer": {"host": peer_host, "port": peer_port}})
    return True


def _handle_peer_request_upload(handler, server_config: dict) -> bool:
    """Handle peer request: open file dialog, upload selected files back.

    The request body contains {"reply_host": "...", "reply_port": ...}
    so we know where to send the files.
    """
    body = _read_body(handler)
    try:
        data = json.loads(body) if body else {}
    except json.JSONDecodeError:
        data = {}
    reply_host = data.get("reply_host", handler.client_address[0])
    reply_port = data.get("reply_port")

    peer_addr = f"{reply_host}:{reply_port}" if reply_port else reply_host
    print(format_peer_upload_request(peer_addr))

    # Open file selection dialog first; fall back to directory dialog
    print("Select file(s) to share (cancel file dialog to share a directory)...")
    paths = open_file_dialog(multiple=True)
    if not paths:
        print("Select a directory to share...")
        dir_path = open_directory_dialog()
        if dir_path:
            paths = [dir_path]

    if not paths:
        _send_json(handler, {"status": "cancelled"})
        return True

    # Send files to the requesting peer
    if reply_port:
        peer_secret = server_config.get("peer_secret", "")
        client = PeerClient(f"{reply_host}:{reply_port}", peer_secret)
        result = client.send_files(paths)
    else:
        result = {"status": "error",
                   "message": "No reply_port in request"}

    # Report results
    if result.get("status") == "ok":
        for p in paths:
            size = os.path.getsize(p) if os.path.isfile(p) else 0
            print(format_peer_transfer_complete(
                "sent", os.path.basename(p), size))

    _send_json(handler, result)
    return True


def _handle_peer_request_download(handler, server_config: dict) -> bool:
    """Handle peer request: open save dialog, confirm path, peer sends files.

    The request body contains {"files": [...], "sender_host": "...",
    "sender_port": ...}.
    """
    body = _read_body(handler)
    try:
        data = json.loads(body) if body else {}
    except json.JSONDecodeError:
        data = {}
    files_info = data.get("files", [])
    peer_addr = data.get("sender_host", handler.client_address[0])
    peer_port = data.get("sender_port")

    addr_str = f"{peer_addr}:{peer_port}" if peer_port else peer_addr
    file_count = len(files_info)
    print(format_peer_download_request(addr_str, file_count))

    # Default filename from first file if available
    default_name = files_info[0]["name"] if files_info else ""

    # Open save dialog
    save_path = open_save_dialog(default_name)
    if not save_path:
        _send_json(handler, {"status": "cancelled"})
        return True

    _send_json(handler, {
        "status": "ok",
        "path": save_path,
        "ready": True,
    })
    return True


def _handle_peer_receive(handler, server_config: dict) -> bool:
    """Receive files from a peer via multipart POST.

    The save_path query param hints where to save (from download flow);
    otherwise saves to cwd.
    """
    from urllib.parse import urlparse, parse_qs
    parsed = urlparse(handler.path)
    params = parse_qs(parsed.query)
    save_paths = params.get("save_path", [])
    save_dir = save_paths[0] if save_paths else os.getcwd()

    content_type = handler.headers.get("Content-Type", "")
    content_length = int(handler.headers.get("Content-Length", "0") or "0")

    if "multipart/form-data" not in content_type:
        _send_json_error(handler, 400, "Expected multipart/form-data")
        return True

    try:
        from .upload_handler import parse_multipart_request, save_uploaded_file
    except ImportError:
        from upload_handler import parse_multipart_request, save_uploaded_file

    try:
        uploaded, _ = parse_multipart_request(
            content_type, content_length, handler.rfile,
            include_form_fields=True,
        )
    except ValueError as exc:
        _send_json_error(handler, 400, str(exc))
        return True

    if not uploaded:
        _send_json_error(handler, 400, "No files received")
        return True

    # Ensure save dir exists
    if os.path.isfile(save_dir):
        # save_dir is a file path with filename — use its parent
        save_dir = os.path.dirname(save_dir)
    os.makedirs(save_dir, exist_ok=True)

    saved = []
    for uf in uploaded:
        try:
            final_path = save_uploaded_file(uf, save_dir)
            saved.append(os.path.basename(final_path))
            size = uf.size
            print(format_peer_transfer_complete("received", uf.filename, size))
        except (ValueError, OSError) as exc:
            _send_json_error(handler, 500, str(exc))
            return True

    _send_json(handler, {"status": "ok", "files": saved, "count": len(saved)})
    return True


# -- helpers ---------------------------------------------------------


def _read_body(handler) -> bytes:
    """Read the HTTP request body."""
    content_length = int(handler.headers.get("Content-Length", "0") or "0")
    if content_length <= 0:
        return b""
    return handler.rfile.read(content_length)


def _send_json(handler, data: dict, status: int = 200):
    """Send a JSON response."""
    body = json.dumps(data).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


def _send_json_error(handler, status: int, message: str):
    """Send a JSON error response."""
    _send_json(handler, {"error": message, "status": status}, status)
