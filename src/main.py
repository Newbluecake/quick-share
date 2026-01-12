import sys
import os
from pathlib import Path
from typing import Tuple

from .cli import parse_arguments, validate_arguments
from .network import get_local_ip
from .server import FileShareServer, find_available_port
from .utils import format_file_size, parse_duration
from . import logger

def validate_file(file_path: str) -> Tuple[Path, int]:
    """
    Validate that the file exists and is not a directory.

    Args:
        file_path: Path to the file.

    Returns:
        Tuple containing the Path object and file size in bytes.

    Raises:
        FileNotFoundError: If the file does not exist.
        ValueError: If the path points to a directory.
    """
    path = Path(file_path).resolve()

    if not path.exists():
        raise FileNotFoundError(f"File not found: {file_path}")

    if path.is_dir():
        raise ValueError(f"{file_path} is a directory")

    return path, path.stat().st_size

def main() -> None:
    """
    Main execution flow.
    """
    try:
        # Parse and validate arguments
        args = parse_arguments()
        try:
            validate_arguments(args)
        except ValueError as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        # Validate file
        try:
            file_path, file_size_bytes = validate_file(args.file_path)
        except (FileNotFoundError, ValueError) as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)
        except PermissionError:
            print(f"Error: Permission denied reading {args.file_path}", file=sys.stderr)
            sys.exit(1)

        # Get network info
        try:
            local_ip = get_local_ip()
        except RuntimeError as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        # Determine port
        try:
            port = find_available_port(custom_port=args.port)
        except RuntimeError as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        # Parse timeout and convert to minutes for server
        timeout_seconds = parse_duration(args.timeout)
        server_timeout_minutes = timeout_seconds / 60

        # Initialize server
        server = FileShareServer(
            file_path=str(file_path),
            port=port,
            timeout_minutes=server_timeout_minutes
        )

        # Print startup message
        msg = logger.format_startup_message(
            ip=local_ip,
            port=port,
            filename=file_path.name,
            file_size=format_file_size(file_size_bytes),
            max_downloads=args.max_downloads,
            timeout=timeout_seconds
        )
        print(msg)

        # Start server and wait for completion
        try:
            server.start()
            if server.server_thread:
                server.server_thread.join()
        except KeyboardInterrupt:
            print("\nStopping server...")
            server.stop()
            sys.exit(0)

    except Exception as e:
        print(f"Unexpected error: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
