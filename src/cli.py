import argparse
import sys
from . import __version__


def is_update_command(args=None):
    """
    Check if the command is an update command.

    Args:
        args: List of arguments. If None, uses sys.argv[1:].

    Returns:
        True if the first argument is 'update', False otherwise.
    """
    if args is None:
        args = sys.argv[1:]

    if not args:
        return False

    # Check if first argument is 'update' (not a flag)
    return args[0] == 'update'


def parse_update_arguments(args=None):
    """
    Parse update subcommand arguments.

    Args:
        args: List of arguments. If None, uses sys.argv[1:].

    Returns:
        argparse.Namespace with update command options.
    """
    if args is None:
        args = sys.argv[1:]

    # Skip 'update' if present
    if args and args[0] == 'update':
        args = args[1:]

    parser = argparse.ArgumentParser(
        prog='quick-share update',
        description='Check and update quick-share to the latest version'
    )

    parser.add_argument(
        '--check',
        action='store_true',
        help='Only check for updates, do not install'
    )

    parser.add_argument(
        '-y', '--yes',
        action='store_true',
        help='Skip confirmation prompt'
    )

    return parser.parse_args(args)


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
        "file_paths",
        nargs='*',
        help="One or more files or directories to share"
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

    parser.add_argument(
        "--upload",
        nargs='?',
        const='.',
        default=None,
        help="Enable upload mode. Optionally specify save directory (default: current directory)"
    )

    parser.add_argument(
        "--upload-password",
        type=str,
        default=None,
        help="Password required for uploading"
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
    # Either file paths or --upload must be provided
    if not args.file_paths and args.upload is None:
        raise ValueError(
            "Provide one or more files/directories to share, "
            "or use --upload to start an upload server"
        )

    # --upload-password requires --upload
    if args.upload_password is not None and args.upload is None:
        raise ValueError(
            "--upload-password requires --upload to be enabled"
        )

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


def is_config_command(args=None):
    """Check if the command is a config command."""
    if args is None:
        args = sys.argv[1:]
    return len(args) >= 1 and args[0] == "config"


def handle_config_command(args=None):
    """Parse and execute the config subcommand.

    Returns exit code (0 for success, 1 for error).
    """
    if args is None:
        args = sys.argv[1:]
    config_args = args[1:] if args and args[0] == "config" else args

    parser = argparse.ArgumentParser(
        prog="quick-share config",
        description="Configure quick-share settings",
    )
    parser.add_argument(
        "--peer",
        type=str,
        metavar="HOST:PORT",
        help="Remote quick-share instance address",
    )
    parser.add_argument(
        "--secret",
        type=str,
        metavar="KEY",
        help="Shared secret key for peer authentication",
    )
    parser.add_argument(
        "--show",
        action="store_true",
        help="Show current configuration",
    )

    try:
        parsed = parser.parse_args(config_args)
    except SystemExit:
        return 1

    from .config import load_config, set_peer_config

    if parsed.show:
        config = load_config()
        if not config:
            print("No configuration found.")
        else:
            peer = config.get("peer", {})
            if peer:
                address = peer.get("address", "(not set)")
                secret = peer.get("secret", "")
                masked = secret[:4] + "****" if len(secret) > 4 else "****"
                print(f"Peer address: {address}")
                print(f"Peer secret:  {masked}")
            else:
                print("Peer not configured.")
                print("Use: quick-share config --peer HOST:PORT --secret KEY")
        return 0

    if parsed.peer or parsed.secret:
        # Validate address format
        if parsed.peer:
            if ":" not in parsed.peer:
                print("Error: --peer must be in HOST:PORT format", file=sys.stderr)
                return 1
            host, port_str = parsed.peer.rsplit(":", 1)
            try:
                port = int(port_str)
                if not (1 <= port <= 65535):
                    raise ValueError
            except ValueError:
                print("Error: port must be between 1 and 65535", file=sys.stderr)
                return 1

        if not parsed.secret:
            print("Error: --secret is required when configuring a peer", file=sys.stderr)
            return 1

        set_peer_config(parsed.peer, parsed.secret)
        print(f"Peer configured: {parsed.peer}")
        return 0

    parser.print_help()
    return 0
