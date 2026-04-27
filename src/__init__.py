"""Quick Share - Fast file sharing via HTTP server."""

import os
import subprocess

__version__ = "1.6.0"


def _get_commit():
    try:
        from ._commit import __commit__
        if __commit__ != "unknown":
            return __commit__
    except ImportError:
        pass
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, timeout=5,
            cwd=os.path.dirname(__file__),
        )
        if result.returncode == 0:
            return result.stdout.strip()
    except Exception:
        pass
    return "unknown"


__commit__ = _get_commit()
__full_version__ = f"{__version__} ({__commit__})" if __commit__ != "unknown" else __version__
