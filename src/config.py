"""Configuration file management for quick-share.

Stores peer connection settings in ~/.quick-share/config.json.
"""

import json
import os
import tempfile
from pathlib import Path
from typing import Optional


def get_config_path() -> Path:
    """Return path to config file, ensuring parent directory exists."""
    config_dir = Path.home() / ".quick-share"
    config_dir.mkdir(parents=True, exist_ok=True)
    return config_dir / "config.json"


def load_config() -> dict:
    """Read config from disk. Returns empty dict if file doesn't exist."""
    path = get_config_path()
    if not path.exists():
        return {}
    try:
        with open(path, "r") as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return {}


def save_config(config: dict):
    """Write config to disk atomically via temp file."""
    path = get_config_path()
    tmp_path = path.with_suffix(".tmp")
    with open(tmp_path, "w") as f:
        json.dump(config, f, indent=2)
    os.replace(tmp_path, path)


def get_peer_config() -> Optional[dict]:
    """Return the 'peer' section of config, or None if not configured."""
    config = load_config()
    peer = config.get("peer")
    if peer and peer.get("address") and peer.get("secret"):
        return peer
    return None


def set_peer_config(address: str, secret: str):
    """Write peer address and secret to config."""
    config = load_config()
    config["peer"] = {"address": address, "secret": secret}
    save_config(config)
