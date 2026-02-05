"""Quick Share updater module - Check and update to latest version."""

import sys
import subprocess
import shutil
import json
import urllib.request
import urllib.error
from pathlib import Path
from typing import Tuple, Optional

from . import __version__


class UpdateError(Exception):
    """Base exception for update errors."""
    pass


class UpdateCheckError(UpdateError):
    """Error checking for updates."""
    pass


class Updater:
    """Update manager for quick-share."""

    GITHUB_API = "https://api.github.com/repos/Newbluecake/quick-share/releases/latest"
    GITHUB_REPO = "https://github.com/Newbluecake/quick-share"

    def __init__(self):
        self.current_version = __version__
        self.install_method = self._detect_install_method()
        self._previous_version: Optional[str] = None

    def _detect_install_method(self) -> str:
        """
        Detect installation method: pip, exe, or source.

        Returns:
            'pip' if installed via pip
            'exe' if running as Windows executable
            'source' if running from git repository
        """
        # 1. Check for exe (Windows frozen executable)
        if sys.platform == 'win32' and getattr(sys, 'frozen', False):
            return 'exe'

        # 2. Check for pip installation
        try:
            result = subprocess.run(
                [sys.executable, '-m', 'pip', 'show', 'quick-share'],
                capture_output=True,
                text=True,
                timeout=10
            )
            if result.returncode == 0 and 'quick-share' in result.stdout:
                return 'pip'
        except (subprocess.TimeoutExpired, FileNotFoundError):
            pass

        # 3. Check for source installation (git repo)
        try:
            script_dir = Path(__file__).parent.parent
            if (script_dir / '.git').exists():
                return 'source'
        except Exception:
            pass

        # 4. Default to pip (most common)
        return 'pip'

    def _compare_versions(self, v1: str, v2: str) -> int:
        """
        Compare two version strings.

        Args:
            v1: First version string (e.g., "1.0.12")
            v2: Second version string (e.g., "1.0.10")

        Returns:
            1 if v1 > v2, -1 if v1 < v2, 0 if equal
        """
        def parse_version(v: str) -> Tuple[int, ...]:
            # Remove 'v' prefix if present
            v = v.lstrip('v')
            # Split by '.' and convert to integers
            parts = []
            for part in v.split('.'):
                # Handle versions like "1.0.0-beta"
                num_part = ''.join(c for c in part if c.isdigit())
                parts.append(int(num_part) if num_part else 0)
            return tuple(parts)

        v1_parts = parse_version(v1)
        v2_parts = parse_version(v2)

        # Pad shorter version with zeros
        max_len = max(len(v1_parts), len(v2_parts))
        v1_parts = v1_parts + (0,) * (max_len - len(v1_parts))
        v2_parts = v2_parts + (0,) * (max_len - len(v2_parts))

        if v1_parts > v2_parts:
            return 1
        elif v1_parts < v2_parts:
            return -1
        return 0

    def check_update(self) -> Tuple[bool, str, str]:
        """
        Check for available updates.

        Returns:
            Tuple of (has_update, latest_version, changelog)

        Raises:
            UpdateCheckError: If unable to check for updates
        """
        try:
            req = urllib.request.Request(
                self.GITHUB_API,
                headers={
                    'User-Agent': f'quick-share/{self.current_version}',
                    'Accept': 'application/vnd.github.v3+json'
                }
            )

            with urllib.request.urlopen(req, timeout=10) as resp:
                data = json.loads(resp.read().decode('utf-8'))

            latest = data.get('tag_name', '').lstrip('v')
            changelog = data.get('body', '')

            if not latest:
                raise UpdateCheckError("Invalid response from GitHub API")

            has_update = self._compare_versions(latest, self.current_version) > 0
            return has_update, latest, changelog

        except urllib.error.HTTPError as e:
            if e.code == 403:
                raise UpdateCheckError(
                    "GitHub API rate limit exceeded. Please try again later."
                )
            elif e.code == 404:
                raise UpdateCheckError("No releases found.")
            raise UpdateCheckError(f"HTTP error: {e.code}")

        except urllib.error.URLError as e:
            raise UpdateCheckError(
                f"Network error: {e.reason}. Please check your internet connection."
            )

        except json.JSONDecodeError:
            raise UpdateCheckError("Invalid response from GitHub API")

        except Exception as e:
            raise UpdateCheckError(f"Failed to check for updates: {e}")

    def _update_pip(self) -> bool:
        """
        Update via pip.

        Returns:
            True if successful, False otherwise
        """
        self._previous_version = self.current_version

        print("Updating via pip...")
        cmd = [
            sys.executable, '-m', 'pip', 'install', '--upgrade',
            f'git+{self.GITHUB_REPO}.git'
        ]

        try:
            result = subprocess.run(cmd, timeout=300)
            return result.returncode == 0
        except subprocess.TimeoutExpired:
            print("Error: Update timed out")
            return False
        except Exception as e:
            print(f"Error: {e}")
            return False

    def _update_exe(self, latest_version: str) -> bool:
        """
        Update Windows executable.

        Args:
            latest_version: Version to download

        Returns:
            True if successful, False otherwise
        """
        download_url = (
            f"{self.GITHUB_REPO}/releases/download/"
            f"v{latest_version}/quick-share.exe"
        )

        current_exe = Path(sys.executable)
        backup_exe = current_exe.with_suffix('.exe.bak')
        temp_exe = current_exe.parent / 'quick-share.exe.new'

        try:
            # Backup current exe
            print("Backing up current version...")
            shutil.copy2(current_exe, backup_exe)
            self._previous_version = self.current_version

            # Download new version
            print(f"Downloading v{latest_version}...")
            urllib.request.urlretrieve(download_url, temp_exe)

            # Create replacement batch script
            self._create_replace_script(temp_exe, current_exe, backup_exe)

            print("\n✅ Update downloaded successfully!")
            print("   Please restart quick-share to complete the update.")
            return True

        except Exception as e:
            print(f"Error downloading update: {e}")
            # Clean up
            if temp_exe.exists():
                temp_exe.unlink()
            return False

    def _create_replace_script(
        self,
        source: Path,
        target: Path,
        backup: Optional[Path] = None
    ) -> None:
        """
        Create a batch script to replace exe after process exits (Windows).
        """
        script_path = target.parent / 'update_quick_share.bat'
        script_content = f'''@echo off
timeout /t 2 /nobreak > nul
move /y "{source}" "{target}"
if %errorlevel% neq 0 (
    echo Update failed. Restoring backup...
    {"move /y " + f'"{backup}" "{target}"' if backup else ""}
)
del "%~f0"
'''
        script_path.write_text(script_content)
        # Start the script
        subprocess.Popen(
            ['cmd', '/c', str(script_path)],
            creationflags=subprocess.CREATE_NO_WINDOW
            if hasattr(subprocess, 'CREATE_NO_WINDOW') else 0
        )

    def do_update(self, skip_confirm: bool = False) -> bool:
        """
        Perform the update.

        Args:
            skip_confirm: If True, skip user confirmation

        Returns:
            True if successful, False otherwise
        """
        try:
            has_update, latest, changelog = self.check_update()
        except UpdateCheckError as e:
            print(f"❌ {e}")
            return False

        if not has_update:
            print(f"✅ Already up to date ({self.current_version})")
            return True

        # Display update info
        print("\nQuick Share Update")
        print("━" * 40)
        print(f"Current version: {self.current_version}")
        print(f"Latest version:  {latest}")

        if changelog:
            print("\nWhat's new:")
            # Truncate long changelog
            lines = changelog.strip().split('\n')[:10]
            for line in lines:
                print(f"  {line}")
            if len(changelog.strip().split('\n')) > 10:
                print("  ...")

        # Confirm
        if not skip_confirm:
            try:
                confirm = input("\nDo you want to update? [Y/n]: ").strip().lower()
                if confirm and confirm not in ('y', 'yes', ''):
                    print("Update cancelled.")
                    return False
            except (EOFError, KeyboardInterrupt):
                print("\nUpdate cancelled.")
                return False

        print()

        # Perform update based on install method
        success = False
        if self.install_method == 'pip':
            success = self._update_pip()
        elif self.install_method == 'exe':
            success = self._update_exe(latest)
        else:
            print("Source installation detected.")
            print("Please update manually using: git pull && pip install -e .")
            return False

        if success:
            print(f"\n✅ Successfully updated to {latest}!")
            return True
        else:
            print("\n❌ Update failed.")
            if self._previous_version:
                self.rollback()
            return False

    def rollback(self) -> bool:
        """
        Rollback to previous version.

        Returns:
            True if successful, False otherwise
        """
        print("\nRolling back to previous version...")

        if self.install_method == 'pip' and self._previous_version:
            try:
                cmd = [
                    sys.executable, '-m', 'pip', 'install',
                    f'git+{self.GITHUB_REPO}.git@v{self._previous_version}'
                ]
                result = subprocess.run(cmd, capture_output=True, timeout=300)
                if result.returncode == 0:
                    print(f"✅ Rollback successful. Restored to {self._previous_version}")
                    return True
            except Exception as e:
                print(f"Rollback error: {e}")

        elif self.install_method == 'exe':
            backup = Path(sys.executable).with_suffix('.exe.bak')
            if backup.exists():
                self._create_replace_script(backup, Path(sys.executable))
                print("✅ Rollback scheduled. Please restart to complete.")
                return True

        print("❌ Rollback failed.")
        return False


def run_update(args: list = None) -> int:
    """
    Entry point for update command.

    Args:
        args: Command line arguments (default: sys.argv[1:])

    Returns:
        Exit code (0 for success, 1 for error)
    """
    from .cli import parse_update_arguments

    try:
        parsed = parse_update_arguments(args)
    except SystemExit:
        return 1

    updater = Updater()

    if parsed.check:
        # Check only mode
        try:
            has_update, latest, changelog = updater.check_update()
        except UpdateCheckError as e:
            print(f"❌ {e}")
            return 1

        print("\nQuick Share Update Check")
        print("━" * 40)
        print(f"Current version: {updater.current_version}")
        print(f"Latest version:  {latest}")

        if has_update:
            if changelog:
                print(f"\nWhat's new in {latest}:")
                lines = changelog.strip().split('\n')[:10]
                for line in lines:
                    print(f"  {line}")

            print(f"\nRun 'quick-share update' to install the update.")
        else:
            print("\n✅ You are running the latest version!")

        return 0

    # Perform update
    success = updater.do_update(skip_confirm=parsed.yes)
    return 0 if success else 1
