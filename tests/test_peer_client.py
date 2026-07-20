"""Tests for route callbacks and streaming peer uploads."""

import json
from unittest.mock import MagicMock, patch

from src.peer_client import (
    PEER_UPLOAD_CHUNK_SIZE,
    PeerClient,
    _MultipartBody,
)


def test_multipart_body_streams_large_file_in_bounded_chunks(tmp_path):
    file_path = tmp_path / "linux-program"
    data = b"x" * (PEER_UPLOAD_CHUNK_SIZE * 2 + 123)
    file_path.write_bytes(data)

    body = _MultipartBody([str(file_path)], "test-boundary")
    chunks = list(body)

    assert sum(map(len, chunks)) == body.content_length
    assert b"".join(chunks).count(data) == 1
    assert chunks[1] == data[:PEER_UPLOAD_CHUNK_SIZE]
    assert chunks[2] == data[PEER_UPLOAD_CHUNK_SIZE:PEER_UPLOAD_CHUNK_SIZE * 2]
    assert chunks[3] == data[PEER_UPLOAD_CHUNK_SIZE * 2:]


def test_send_files_sets_content_length_and_passes_stream(tmp_path):
    file_path = tmp_path / "program"
    file_path.write_bytes(b"binary-data")

    response = MagicMock()
    response.read.return_value = json.dumps({"status": "ok"}).encode()
    response.__enter__.return_value = response

    with patch("src.peer_client.request.urlopen", return_value=response) as urlopen:
        result = PeerClient("192.168.32.38:8000", "secret").send_files(
            [str(file_path)]
        )

    assert result == {"status": "ok"}
    req = urlopen.call_args.args[0]
    assert isinstance(req.data, _MultipartBody)
    assert int(req.get_header("Content-length")) == req.data.content_length
