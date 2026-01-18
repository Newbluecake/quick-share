def format_startup_message(ip: str, port: int, filename: str, file_size: str, max_downloads: int, timeout: int) -> str:
    """
    Format the startup message with server details and download commands.
    """
    # Check if this is a directory share (file_size will be "Directory")
    is_directory = (file_size == "Directory")

    if is_directory:
        base_url = f"http://{ip}:{port}/"
        download_url = f"http://{ip}:{port}/download/{filename}.zip"

        msg = [
            "Share started!",
            f"File: {filename} ({file_size})",
            f"Browse: {base_url}",
            f"Zip URL: {download_url}",
            f"Max downloads: {max_downloads}",
            f"Timeout: {timeout} seconds",
            "",
            "Download commands:",
            f"  wget {download_url}",
            f"  curl -O {download_url}"
        ]
    else:
        url = f"http://{ip}:{port}/{filename}"

        msg = [
            "Share started!",
            f"File: {filename} ({file_size})",
            f"URL: {url}",
            f"Max downloads: {max_downloads}",
            f"Timeout: {timeout} seconds",
            "",
            "Download commands:",
            f"  wget {url}",
            f"  curl -O {url}"
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
