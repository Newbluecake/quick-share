def format_startup_message(ip: str, port: int, filename: str, file_size: str, max_downloads: int, timeout: int) -> str:
    """
    Format the startup message with server details and download commands.
    """
    url = f"http://{ip}:{port}/{filename}"

    # Using triple quotes for cleaner multi-line string
    msg = [
        "Share started!",
        f"File: {filename} ({file_size})",
        f"URL: {url}",
        f"Max downloads: {max_downloads}",
        f"Timeout: {timeout} seconds",
        "",
        "Download commands:",
        f"  wget {url}",
        f"  curl -O {url}",
        "",
        "Scan QR code to download:"
    ]
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
