import sys
import os
from pathlib import Path
from typing import List, Tuple, Optional

from .cli import parse_arguments, validate_arguments
from .network import get_local_ip, get_all_lan_ips
from .server import FileShareServer, DirectoryShareServer, MultiShareServer, find_available_port
from .utils import format_file_size, parse_duration
from . import logger


def detect_path_type(path: str) -> str:
    """
    Detect whether a path is a file, directory, or invalid.

    Args:
        path: Path to check.

    Returns:
        "file" if path is a file, "directory" if path is a directory,
        "invalid" if path does not exist.
    """
    if os.path.isfile(path):
        return "file"
    elif os.path.isdir(path):
        return "directory"
    else:
        return "invalid"


def handle_symlink(symlink_path: str) -> Tuple[bool, str, Optional[Path]]:
    """
    Handle symlink detection and user confirmation.

    Args:
        symlink_path: The symlink path to handle.

    Returns:
        Tuple containing:
        - is_valid: True if user confirms, False otherwise
        - path_type: "file", "directory", or "symlink_broken"/"symlink_cancelled"
        - resolved_path: Real target path if confirmed, None otherwise
    """
    # Resolve symlink to get real target
    real_path = Path(symlink_path).resolve()

    # Display symlink info
    print("⚠️  检测到软链接：")
    print(f"    源路径: {symlink_path}")
    print(f"    目标路径: {real_path}")

    # Check if target exists
    if not real_path.exists():
        print("❌ 错误：软链接目标不存在")
        return False, "symlink_broken", None

    # Ask user for confirmation
    while True:
        choice = input("是否跟随软链接并分享目标文件/目录？(y/n): ").strip().lower()
        if choice == 'y':
            # Recursively call validate_path to verify real path
            return validate_path(str(real_path))
        elif choice == 'n':
            print("❌ 用户取消分享")
            return False, "symlink_cancelled", None
        else:
            print("请输入 y 或 n")


def validate_path(path: str) -> Tuple[bool, str, Optional[Path]]:
    """
    Unified validation for both files and directories.

    Args:
        path: Path to validate (can be file or directory).

    Returns:
        Tuple containing:
        - is_valid: True if path is valid and accessible, False otherwise
        - path_type: "file", "directory", or "invalid"
        - resolved_path: Path object if valid, None otherwise
    """
    # Detect symlink first
    if os.path.islink(path):
        return handle_symlink(path)

    # Detect path type
    path_type = detect_path_type(path)

    if path_type == "invalid":
        return False, "invalid", None

    try:
        resolved_path = Path(path).resolve()

        if path_type == "file":
            # For files, verify it exists and we can read it
            if not resolved_path.exists():
                return False, "invalid", None
            # Try to access file stats to verify readability
            _ = resolved_path.stat()
            # Try to open file to verify it's readable
            try:
                with open(resolved_path, 'rb'):
                    pass
            except PermissionError:
                return False, "file", None
            return True, "file", resolved_path

        elif path_type == "directory":
            # For directories, verify it exists and is accessible
            if not resolved_path.exists():
                return False, "invalid", None
            # Try to access directory to verify accessibility
            _ = resolved_path.stat()
            # Try to list directory to ensure it's accessible
            try:
                list(resolved_path.iterdir())
            except PermissionError:
                return False, "directory", None
            return True, "directory", resolved_path

    except (PermissionError, OSError):
        return False, path_type, None

    return False, "invalid", None


def validate_multi_paths(
    paths: List[str],
) -> Tuple[bool, List[Tuple[str, str]], List[str]]:
    """
    Validate multiple paths and detect top-level name conflicts.

    Args:
        paths: List of path strings supplied from the CLI.

    Returns:
        Tuple of:
        - is_valid: True if all paths are valid and there are no name conflicts.
        - resolved_paths: List of (abs_path, path_type) tuples for valid paths.
        - errors: Human-readable error messages (empty when is_valid is True).
    """
    errors: List[str] = []
    resolved: List[Tuple[str, str]] = []

    for path in paths:
        is_valid, path_type, resolved_path = validate_path(path)

        if not is_valid:
            if path_type == "symlink_broken":
                errors.append(f"Broken symlink: {path}")
            elif path_type == "symlink_cancelled":
                errors.append(f"Symlink cancelled by user: {path}")
            else:
                errors.append(f"Invalid path (does not exist or is not accessible): {path}")
        else:
            resolved.append((str(resolved_path), path_type))

    if errors:
        return False, [], errors

    # Detect top-level name conflicts (basename duplicates)
    seen: dict = {}
    for abs_path, _ in resolved:
        name = os.path.basename(abs_path)
        if name in seen:
            errors.append(
                f"Name conflict: '{name}' appears in both "
                f"'{seen[name]}' and '{abs_path}'"
            )
        else:
            seen[name] = abs_path

    if errors:
        return False, [], errors

    return True, resolved, []


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

    Accepts one or more file/directory paths and serves them via
    MultiShareServer, which always presents a unified file-list page.
    """
    # Check for update command first (before normal argument parsing)
    from .cli import is_update_command
    if is_update_command():
        from .updater import run_update
        sys.exit(run_update())

    try:
        # Parse and validate arguments
        args = parse_arguments()
        try:
            validate_arguments(args)
        except ValueError as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        # Validate all paths and detect name conflicts
        try:
            is_valid, resolved_paths, errors = validate_multi_paths(args.file_paths)
            if not is_valid:
                for err in errors:
                    print(f"Error: {err}", file=sys.stderr)
                sys.exit(1)
        except PermissionError as e:
            print(f"Error: Permission denied: {e}", file=sys.stderr)
            sys.exit(1)

        # Get network info
        try:
            local_ip = get_local_ip()
        except RuntimeError as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        # Get all available LAN IPs (for multi-IP display)
        all_ips = get_all_lan_ips()

        # Determine port
        try:
            port = find_available_port(custom_port=args.port)
        except RuntimeError as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)

        # Parse timeout and convert to minutes for server
        timeout_seconds = parse_duration(args.timeout)
        server_timeout_minutes = timeout_seconds / 60

        # Build display label for startup message
        if len(resolved_paths) == 1:
            abs_path, path_type = resolved_paths[0]
            item_label = os.path.basename(abs_path)
            if path_type == "file":
                size_label = format_file_size(os.path.getsize(abs_path))
            else:
                size_label = "Directory"
        else:
            names = [os.path.basename(p) for p, _ in resolved_paths]
            item_label = ", ".join(names[:3])
            if len(names) > 3:
                item_label += f" ... (+{len(names) - 3} more)"
            size_label = f"{len(resolved_paths)} items"

        # Always use MultiShareServer (single or multiple paths)
        server = MultiShareServer(
            paths=resolved_paths,
            port=port,
            timeout_minutes=server_timeout_minutes,
            max_sessions=args.max_downloads,
            legacy_mode=args.legacy,
        )

        # Print startup message
        msg = logger.format_startup_message(
            ip=local_ip,
            port=port,
            filename=item_label,
            file_size=size_label,
            max_downloads=args.max_downloads,
            timeout=timeout_seconds,
            all_ips=all_ips,
        )
        print(msg)

        # Start server and wait for completion
        try:
            server.start()
            # Use timeout loop to allow Ctrl+C to work immediately
            while server.server_thread and server.server_thread.is_alive():
                server.server_thread.join(timeout=0.5)
        except KeyboardInterrupt:
            print("\nStopping server...")
            server.stop()
            sys.exit(0)

    except Exception as e:
        print(f"Unexpected error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
