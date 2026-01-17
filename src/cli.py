import argparse
import re
from . import __version__

def parse_arguments(args=None):
    """
    Parse command line arguments.

    Args:
        args (list): List of arguments to parse. If None, uses sys.argv.

    Returns:
        argparse.Namespace: Parsed arguments.
    """
    parser = argparse.ArgumentParser(description="Quick Share - A simple file sharing CLI tool.")

    parser.add_argument(
        "--version",
        action="version",
        version=f"quick-share {__version__}"
    )

    parser.add_argument(
        "file_path",
        help="Path to the file to share"
    )

    parser.add_argument(
        "-p", "--port",
        type=int,
        help="Port to listen on (1024-65535)"
    )

    parser.add_argument(
        "-n", "--max-downloads",
        type=int,
        default=10,
        help="Maximum number of downloads allowed (default: 10)"
    )

    parser.add_argument(
        "-t", "--timeout",
        default="5m",
        help="Timeout duration (e.g., 30s, 5m, 1h) (default: 5m)"
    )

    parser.add_argument(
        "--legacy",
        action="store_true",
        help="Use legacy server-side rendered directory listing"
    )

    return parser.parse_args(args)

def validate_arguments(args):
    """
    Validate parsed arguments.

    Args:
        args (argparse.Namespace): Parsed arguments.

    Raises:
        ValueError: If arguments are invalid.
    """
    # Validate port
    if args.port is not None:
        if not (1024 <= args.port <= 65535):
            raise ValueError("Port must be between 1024 and 65535")

    # Validate max_downloads
    if args.max_downloads <= 0:
        raise ValueError("max_downloads must be a positive integer")

    # Validate timeout
    if args.timeout:
        # Check format <number><unit>
        if args.timeout[-1].isdigit():
             # Ends with digit implies missing unit
             raise ValueError("Timeout must be in format <number><unit> (e.g., 30s, 5m, 1h)")

        unit = args.timeout[-1]
        if unit not in ['s', 'm', 'h']:
            raise ValueError("Timeout unit must be 's', 'm', or 'h'")

        # Validate the number part
        number_part = args.timeout[:-1]
        if not number_part.isdigit():
             raise ValueError("Timeout must be in format <number><unit> (e.g., 30s, 5m, 1h)")
