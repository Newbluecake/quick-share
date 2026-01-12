"""Security validation module."""

import urllib.parse


def is_path_traversal_attack(path: str) -> bool:
    """
    Detect path traversal attacks.

    Checks for:
    - '..' in path
    - URL encoded '..'
    """
    # Check raw path for obvious traversal
    if ".." in path:
        return True

    # URL decode and check again
    decoded_path = urllib.parse.unquote(path)
    if ".." in decoded_path:
        return True

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
