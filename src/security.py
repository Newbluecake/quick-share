"""Security validation module."""

import urllib.parse
import os
from typing import List, Optional, Tuple


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


def validate_multi_share_path(
    request_path: str,
    shared_paths: List[Tuple[str, str]],
) -> Tuple[bool, str]:
    """
    Validate a download request path in a multi-share context.

    The virtual filesystem uses the ``/files/`` URL prefix:
      - ``/files/<top_name>``              – top-level shared file
      - ``/files/<top_dir>/<sub_path>``    – file inside a shared directory

    Security guarantees
    -------------------
    * URL decoding + path-traversal detection before any filesystem access.
    * For directory entries: ``os.path.realpath`` + ``os.path.commonpath`` ensure
      the resolved path stays inside the shared directory (symlink-safe).
    * A top-level file entry never accepts sub-path requests.

    Args:
        request_path: HTTP request path, e.g. ``/files/foo.txt`` or
            ``/files/dir1/sub/bar.txt``.
        shared_paths: List of ``(abs_path, path_type)`` tuples as produced
            by :func:`src.main.validate_multi_paths`.

    Returns:
        ``(is_valid, real_abs_path)`` where *real_abs_path* is the
        fully-resolved filesystem path when valid, or ``""`` otherwise.
    """
    FILES_PREFIX = "/files/"

    # 1. Must start with /files/
    if not request_path.startswith(FILES_PREFIX):
        return False, ""

    # 2. Strip query string / fragment
    clean = request_path.split("?")[0].split("#")[0]

    # 3. Extract the virtual path after /files/
    virtual_path = clean[len(FILES_PREFIX):]

    # 4. URL-decode the virtual path
    decoded = urllib.parse.unquote(virtual_path)

    # 5. Path traversal check on both raw and decoded forms
    if is_path_traversal_attack(virtual_path) or is_path_traversal_attack(decoded):
        return False, ""

    # Reject empty virtual path
    if not decoded:
        return False, ""

    # 6. Normalise to forward slashes and strip leading /
    decoded = decoded.replace("\\", "/").lstrip("/")

    # 7. Extract top-level name (first path component)
    parts = decoded.split("/")
    top_name = parts[0]
    if not top_name:
        return False, ""

    # 8. Find matching shared entry by basename
    matched_abs: Optional[str] = None
    matched_type: Optional[str] = None
    for abs_path, path_type in shared_paths:
        if os.path.basename(abs_path) == top_name:
            matched_abs = abs_path
            matched_type = path_type
            break

    if matched_abs is None:
        return False, ""

    # 9a. Shared file: no sub-path allowed
    if matched_type == "file":
        if len(parts) > 1:
            # Extra path components after a file entry – reject
            return False, ""
        if not os.path.isfile(matched_abs):
            return False, ""
        return True, matched_abs

    # 9b. Shared directory: validate sub-path within sandbox
    if matched_type == "directory":
        sub_path = "/".join(parts[1:])
        if not sub_path:
            # Requesting the directory itself is not a file download – reject
            return False, ""

        candidate = os.path.join(matched_abs, sub_path)
        try:
            real_path = os.path.realpath(candidate)
            real_shared = os.path.realpath(matched_abs)
        except Exception:
            return False, ""

        # Verify the resolved path is inside the shared directory
        try:
            common = os.path.commonpath([real_path, real_shared])
            if common != real_shared:
                return False, ""
        except ValueError:
            return False, ""

        if not os.path.isfile(real_path):
            return False, ""

        return True, real_path

    return False, ""
