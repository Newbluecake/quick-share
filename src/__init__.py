"""Quick Share - Fast file sharing via HTTP server."""

import os
import subprocess

__version__ = "1.8.0"


def _get_commit():
    # 1. Build-time injected _commit.py (used by PyInstaller releases)
    try:
        from ._commit import __commit__
        if __commit__ != "unknown":
            return __commit__
    except ImportError:
        pass

    # 2. Runtime git fallback (works in dev / editable installs)
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

    # 3. Read commit from pip's direct_url.json (git installs in site-packages)
    try:
        from pathlib import Path
        import json
        pkg_dir = Path(__file__).resolve().parent
        for parent in pkg_dir.parents:
            for info_dir in parent.glob("quick_share-*.dist-info"):
                url_file = info_dir / "direct_url.json"
                if url_file.is_file():
                    data = json.loads(url_file.read_text())
                    vcs_info = data.get("vcs_info", {})
                    if vcs_info.get("vcs") == "git":
                        return vcs_info["commit_id"][:7]
                    # local path install — try git in that directory
                    url = data.get("url", "")
                    if url.startswith("file://"):
                        src_path = url[7:]
                        result = subprocess.run(
                            ["git", "rev-parse", "--short", "HEAD"],
                            capture_output=True, text=True, timeout=5,
                            cwd=src_path,
                        )
                        if result.returncode == 0:
                            return result.stdout.strip()
    except Exception:
        pass

    return "unknown"


__commit__ = _get_commit()
__full_version__ = f"{__version__} ({__commit__})" if __commit__ != "unknown" else __version__
