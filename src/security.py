"""Security validation module."""

import urllib.parse
import os
from typing import Tuple


def is_path_traversal_attack(path: str) -> bool:
    """
    Detect path traversal attacks (enhanced for directory sharing).

    Checks for:
    - '..' in path (raw and decoded)
    - Multi-level URL encoding (up to 2 levels)
    - Complex traversal patterns
    """
    # Check raw path for obvious traversal
    if ".." in path:
        return True

    # URL decode and check (up to 2 levels to catch multi-encoding)
    decoded = path
    for _ in range(2):
        try:
            decoded = urllib.parse.unquote(decoded)
            if ".." in decoded:
                return True
        except Exception:
            # If decoding fails, consider it suspicious
            break

    return False


def validate_request_path(request_path: str, allowed_basename: str) -> tuple[bool, str]:
    """
    Validate request path against allowed filename.

    Args:
        request_path: The raw HTTP request path
        allowed_basename: The specific filename allowed to be accessed

    Returns:
        tuple: (is_valid, normalized_path)
    """
    if not request_path:
        return False, ""

    # 1. Remove query string and fragment
    clean_path = request_path.split('?')[0].split('#')[0]

    # 2. Check for traversal attacks on the raw path before decoding
    # (Some attacks rely on double encoding or specific raw sequences)
    if is_path_traversal_attack(clean_path):
        return False, ""

    # 3. URL decode
    decoded_path = urllib.parse.unquote(clean_path)

    # 4. Check traversal on decoded path
    if is_path_traversal_attack(decoded_path):
        return False, ""

    # 5. Normalize and Extract filename
    # Remove leading slashes and replace backslashes
    normalized_path = decoded_path.replace("\\", "/")
    filename = normalized_path.lstrip("/")

    # 6. Verify exact filename match
    # This prevents directory traversal like "subdir/test.txt" if only "test.txt" is allowed
    # And prevents "/etc/passwd" because filename would be "etc/passwd" != "test.txt"
    if filename != allowed_basename:
        return False, ""

    return True, "/" + filename


def validate_directory_path(
    request_path: str,
    shared_directory: str
) -> Tuple[bool, str]:
    """
    Validate directory access request path.

    Security checks:
    1. URL decoding
    2. Path traversal detection (..)
    3. Path normalization
    4. Verify final path is within shared_directory
    5. Symlink real path detection

    Args:
        request_path: HTTP request path (e.g., /subdir/file.txt)
        shared_directory: Absolute path of shared directory

    Returns:
        (is_valid, normalized_real_path)
    """
    # 1. Clean path (remove query string and fragment)
    clean_path = request_path.split('?')[0].split('#')[0]

    # 2. URL decode
    decoded_path = urllib.parse.unquote(clean_path)

    # 3. Check for path traversal attacks
    if is_path_traversal_attack(decoded_path):
        return False, ""

    # 4. Build full path
    # Convert request path to relative path (remove leading /)
    relative_path = decoded_path.lstrip('/')
    if not relative_path:  # Root path
        full_path = shared_directory
    else:
        full_path = os.path.join(shared_directory, relative_path)

    # 5. Resolve real path (handles symlinks and normalizes ..)
    try:
        real_path = os.path.realpath(full_path)
        real_shared = os.path.realpath(shared_directory)
    except Exception:
        return False, ""

    # 6. Verify path is within sandbox using commonpath
    try:
        common = os.path.commonpath([real_path, real_shared])
        if common != real_shared:
            return False, ""
    except ValueError:
        # Different drives (Windows) or no common path
        return False, ""

    # 7. Verify path exists
    if not os.path.exists(real_path):
        return False, ""

    return True, real_path
