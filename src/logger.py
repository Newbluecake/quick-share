import urllib.parse
import datetime
from typing import List, Tuple, Optional


def get_timestamp() -> str:
    """
    Get formatted timestamp for logging.

    Returns:
        Timestamp string in format [YYYY-MM-DD HH:MM:SS]

    Example:
        >>> get_timestamp()
        '[2025-02-05 10:30:45]'
    """
    return datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')


def format_startup_message(
    ip: str,
    port: int,
    filename: str,
    file_size: str,
    max_downloads: int,
    timeout: int,
    all_ips: Optional[List[Tuple[str, str]]] = None
) -> str:
    """
    Format the startup message with server details and download commands.

    Args:
        ip: Primary IP address
        port: Server port
        filename: Name of the file/directory being shared
        file_size: Human-readable file size or "Directory"
        max_downloads: Maximum number of downloads allowed
        timeout: Timeout in seconds
        all_ips: Optional list of (interface_name, ip) tuples for multi-IP display
    """
    # Check if this is a directory share (file_size will be "Directory")
    is_directory = (file_size == "Directory")

    # URL encode the filename for use in URLs (handles Chinese and special characters)
    encoded_filename = urllib.parse.quote(filename, safe='')

    msg = [
        "Share started!",
        f"File: {filename} ({file_size})",
    ]

    # Build URL(s) section
    if all_ips and len(all_ips) > 1:
        # Multiple IPs: show all with interface names
        msg.append("")
        msg.append("Available URLs:")
        for iface, iface_ip in all_ips:
            if is_directory:
                url = f"http://{iface_ip}:{port}/"
            else:
                url = f"http://{iface_ip}:{port}/{encoded_filename}"
            msg.append(f"  {iface:12} {url}")
    else:
        # Single IP: simple format
        if is_directory:
            base_url = f"http://{ip}:{port}/"
            msg.append(f"Browse: {base_url}")
        else:
            url = f"http://{ip}:{port}/{encoded_filename}"
            msg.append(f"URL: {url}")

    # Add zip URL for directories
    if is_directory:
        download_url = f"http://{ip}:{port}/download/{encoded_filename}.zip"
        msg.append(f"Zip URL: {download_url}")

    msg.extend([
        f"Max downloads: {max_downloads}",
        f"Timeout: {timeout} seconds",
        "",
        "Download commands:",
    ])

    # Generate download commands using primary IP
    if is_directory:
        download_url = f"http://{ip}:{port}/download/{encoded_filename}.zip"
        msg.append(f"  wget '{download_url}'")
        msg.append(f"  curl -O '{download_url}'")
    else:
        url = f"http://{ip}:{port}/{encoded_filename}"
        msg.append(f"  wget '{url}'")
        msg.append(f"  curl -O '{url}'")

    return "\n".join(msg)

def format_download_log(timestamp: str, client_ip: str, method: str, path: str, status_code: int, current_count: int, max_count: int) -> str:
    """
    Format a download access log entry.
    """
    return f"[{timestamp}] {client_ip} - \"{method} {path}\" {status_code} - Download {current_count}/{max_count}"

def format_shutdown_message(total_downloads: int, max_downloads: int) -> str:
    """
    Format the shutdown message.
    """
    return f"Shutdown initiated. Total downloads: {total_downloads}/{max_downloads}. Server stopping..."


def format_download_start(
    timestamp: str,
    client_ip: str,
    filename: str,
    file_size: str
) -> str:
    """
    Format download start log entry.

    Args:
        timestamp: Formatted timestamp [YYYY-MM-DD HH:MM:SS]
        client_ip: Client IP address
        filename: Download filename
        file_size: Human-readable file size (e.g., "2.5MB")

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:45] ⬇️  192.168.1.100 - file.zip (2.5MB)
    """
    return f"[{timestamp}] ⬇️  {client_ip} - {filename} ({file_size})"


def format_download_progress(
    timestamp: str,
    client_ip: str,
    bytes_transferred: int,
    total_bytes: int,
    percentage: float
) -> str:
    """
    Format download progress log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        bytes_transferred: Transferred bytes
        total_bytes: Total file size in bytes
        percentage: Progress percentage (0-100)

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:46] ⬇️  192.168.1.100 - 1.2MB / 2.5MB (48%)
    """
    # Import here to avoid circular dependency
    try:
        from .directory_handler import format_file_size
    except ImportError:
        from directory_handler import format_file_size

    transferred_str = format_file_size(bytes_transferred)
    total_str = format_file_size(total_bytes)

    return f"[{timestamp}] ⬇️  {client_ip} - {transferred_str} / {total_str} ({percentage:.0f}%)"


def format_download_complete(
    timestamp: str,
    client_ip: str,
    filename: str,
    total_bytes: int,
    duration_sec: float
) -> str:
    """
    Format download completion log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        filename: Download filename
        total_bytes: Total transferred bytes
        duration_sec: Transfer duration in seconds

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:47] ✅ 192.168.1.100 - Completed: file.zip (2.5MB in 2.3s)
    """
    # Import here to avoid circular dependency
    try:
        from .directory_handler import format_file_size
    except ImportError:
        from directory_handler import format_file_size

    size_str = format_file_size(total_bytes)

    return f"[{timestamp}] ✅ {client_ip} - Completed: {filename} ({size_str} in {duration_sec:.1f}s)"


def format_download_interrupted(
    timestamp: str,
    client_ip: str,
    filename: str,
    bytes_transferred: int,
    total_bytes: int
) -> str:
    """
    Format download interruption log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        filename: Download filename
        bytes_transferred: Transferred bytes before interruption
        total_bytes: Total file size

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:46] ⚠️  192.168.1.100 - Interrupted: file.zip (1.2MB / 2.5MB transferred)
    """
    # Import here to avoid circular dependency
    try:
        from .directory_handler import format_file_size
    except ImportError:
        from directory_handler import format_file_size

    transferred_str = format_file_size(bytes_transferred)
    total_str = format_file_size(total_bytes)

    return f"[{timestamp}] ⚠️  {client_ip} - Interrupted: {filename} ({transferred_str} / {total_str} transferred)"


def format_download_error(
    timestamp: str,
    client_ip: str,
    filename: str,
    error_message: str
) -> str:
    """
    Format download error log entry.

    Args:
        timestamp: Formatted timestamp
        client_ip: Client IP address
        filename: Download filename
        error_message: Error description

    Returns:
        Formatted log string

    Example:
        [2025-02-05 10:30:45] ❌ 192.168.1.100 - Error: file.zip - File not found
    """
    return f"[{timestamp}] ❌ {client_ip} - Error: {filename} - {error_message}"

