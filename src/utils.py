"""Utility functions for file operations and parsing."""

from typing import Dict

# Constants
SIZE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"]
TIME_MULTIPLIERS: Dict[str, int] = {
    "s": 1,
    "m": 60,
    "h": 3600,
    "d": 86400,
}


def format_file_size(size_bytes: int) -> str:
    """
    Convert bytes to a human-readable string (B, KB, MB, GB, etc.).

    Args:
        size_bytes (int): The size in bytes.

    Returns:
        str: Human-readable file size string.

    Raises:
        TypeError: If size_bytes is not an integer.
        ValueError: If size_bytes is negative.
    """
    if not isinstance(size_bytes, int):
        raise TypeError("Size must be an integer")

    if size_bytes < 0:
        raise ValueError("Size cannot be negative")

    if size_bytes < 1024:
        return f"{size_bytes} {SIZE_UNITS[0]}"

    unit_index = 0
    size = float(size_bytes)

    # Iterate while size is large enough to be converted to next unit
    # and we still have units left to convert to.
    while size >= 1024 and unit_index < len(SIZE_UNITS) - 1:
        size /= 1024
        unit_index += 1

    return f"{size:.1f} {SIZE_UNITS[unit_index]}"


def parse_duration(duration_str: str) -> int:
    """
    Parse a duration string to seconds.
    Supported formats: "30s", "5m", "1h", "0".

    Args:
        duration_str (str): The duration string to parse.

    Returns:
        int: Duration in seconds.

    Raises:
        TypeError: If duration_str is not a string.
        ValueError: If format is invalid or value is negative.
    """
    if not isinstance(duration_str, str):
        raise TypeError("Duration must be a string")

    # Clean up whitespace
    clean_str = duration_str.strip()

    if not clean_str:
        raise ValueError("Duration string cannot be empty")

    if clean_str == "0":
        return 0

    if clean_str.startswith("-"):
        raise ValueError("Duration cannot be negative")

    # Extract unit and value
    unit = clean_str[-1].lower()
    value_str = clean_str[:-1]

    # Validate value part is numeric
    if not value_str.isdigit():
        raise ValueError("Invalid duration format: value must be an integer")

    # Validate unit exists
    if unit not in TIME_MULTIPLIERS:
        raise ValueError(f"Unknown or missing time unit: {unit}")

    value = int(value_str)

    return value * TIME_MULTIPLIERS[unit]
