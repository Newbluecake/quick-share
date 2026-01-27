import urllib.parse
from typing import List, Tuple, Optional


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
