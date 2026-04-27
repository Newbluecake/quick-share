"""HTTP client for peer-to-peer communication between quick-share instances.

Pure standard library — uses urllib.request for HTTP calls and manually
constructs multipart bodies for file uploads.
"""

import json
import os
import uuid
from urllib import request, error as urllib_error


class PeerClient:
    """HTTP client for communicating with a remote quick-share peer."""

    def __init__(self, address: str, secret: str):
        self.host, port_str = address.rsplit(":", 1)
        self.port = int(port_str)
        self.secret = secret
        self._base = f"http://{self.host}:{self.port}"

    # -- public API ---------------------------------------------------

    def say_hello(self, my_host: str, my_port: int) -> dict:
        """Register with the peer, providing our own address."""
        body = json.dumps({"host": my_host, "port": my_port}).encode()
        return self._post("/api/peer/hello", body, timeout=10)

    def request_upload(self, my_host: str, my_port: int, timeout: int = 120) -> dict:
        """Ask the peer to select files and upload them to us.

        Returns {"status": "ok", "files": [...]} or {"status": "cancelled"}.
        """
        body = json.dumps({"reply_host": my_host, "reply_port": my_port}).encode()
        return self._post("/api/peer/request-upload", body, timeout=timeout)

    def request_download(
        self, files: list, my_host: str, my_port: int, timeout: int = 120
    ) -> dict:
        """Ask the peer to select a save path for files we want to send.

        Args:
            files: List of {"name": str, "size": int} dicts.
        Returns {"status": "ok", "path": "..."} or {"status": "cancelled"}.
        """
        body = json.dumps({
            "files": files,
            "sender_host": my_host,
            "sender_port": my_port,
        }).encode()
        return self._post("/api/peer/request-download", body, timeout=timeout)

    def send_files(
        self, file_paths: list, save_path: str = "", timeout: int = 300
    ) -> dict:
        """Upload files to the peer via POST /api/peer/receive.

        Returns {"status": "ok", "files": [...]}.
        """
        boundary = uuid.uuid4().hex
        body = _build_multipart_body(file_paths, boundary)

        url = f"{self._base}/api/peer/receive"
        if save_path:
            url += f"?save_path={request.quote(save_path)}"

        req = request.Request(
            url,
            data=body,
            headers={
                "Content-Type": f"multipart/form-data; boundary={boundary}",
                "X-Peer-Secret": self.secret,
            },
            method="POST",
        )
        return self._do_request(req, timeout)

    # -- internals ----------------------------------------------------

    def _post(self, path: str, body: bytes, timeout: int) -> dict:
        url = f"{self._base}{path}"
        req = request.Request(
            url,
            data=body,
            headers={
                "Content-Type": "application/json",
                "X-Peer-Secret": self.secret,
            },
            method="POST",
        )
        return self._do_request(req, timeout)

    def _do_request(self, req: request.Request, timeout: int) -> dict:
        try:
            with request.urlopen(req, timeout=timeout) as resp:
                raw = resp.read()
                return json.loads(raw.decode("utf-8")) if raw else {}
        except urllib_error.HTTPError as e:
            body = e.read().decode("utf-8", errors="replace")
            try:
                return json.loads(body)
            except json.JSONDecodeError:
                return {"status": "error", "http_status": e.code, "message": body}
        except (urllib_error.URLError, OSError, TimeoutError) as e:
            return {"status": "error", "message": str(e)}


def _build_multipart_body(file_paths: list, boundary: str) -> bytes:
    """Build a multipart/form-data body for file upload."""
    parts = []
    for path in file_paths:
        filename = os.path.basename(path)
        with open(path, "rb") as f:
            content = f.read()
        header = (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="files"; filename="{filename}"\r\n'
            "Content-Type: application/octet-stream\r\n"
            "\r\n"
        )
        parts.append(header.encode("utf-8") + content + b"\r\n")
    parts.append(f"--{boundary}--\r\n".encode("utf-8"))
    return b"".join(parts)
