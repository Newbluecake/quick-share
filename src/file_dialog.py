"""Cross-platform system file dialog for quick-share.

Uses native tools (zenity, kdialog, osascript, PowerShell) and falls back
to terminal input when no GUI is available.
"""

import os
import shlex
import subprocess
import sys
from typing import List, Optional


def _detect_tool() -> str:
    """Detect the best available dialog tool for the current platform.

    Returns one of: "zenity", "kdialog", "osascript", "powershell", "stdio"
    """
    if sys.platform == "darwin":
        return "osascript"

    if sys.platform == "win32":
        return "powershell"

    # Linux / other Unix: probe for GUI tools
    for tool in ("zenity", "kdialog"):
        try:
            subprocess.run(
                [tool, "--version"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=5,
            )
            return tool
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            continue

    return "stdio"


def open_file_dialog(multiple: bool = True) -> List[str]:
    """Open a native file selection dialog.

    Args:
        multiple: Allow selecting multiple files.

    Returns:
        List of selected file paths (empty if cancelled).
    """
    tool = _detect_tool()

    if tool == "zenity":
        return _zenity_file_dialog(multiple)
    elif tool == "kdialog":
        return _kdialog_file_dialog(multiple)
    elif tool == "osascript":
        return _osascript_file_dialog(multiple)
    elif tool == "powershell":
        return _powershell_file_dialog(multiple)
    else:
        return _stdio_file_dialog(multiple)


def open_directory_dialog() -> Optional[str]:
    """Open a native directory selection dialog.

    Returns:
        Selected directory path, or None if cancelled.
    """
    tool = _detect_tool()

    if tool == "zenity":
        return _zenity_directory_dialog()
    elif tool == "kdialog":
        return _kdialog_directory_dialog()
    elif tool == "osascript":
        return _osascript_directory_dialog()
    elif tool == "powershell":
        return _powershell_directory_dialog()
    else:
        return _stdio_directory_dialog()


def open_save_dialog(default_name: str = "") -> Optional[str]:
    """Open a native save-path selection dialog.

    Args:
        default_name: Suggested filename.

    Returns:
        Selected path, or None if cancelled.
    """
    tool = _detect_tool()

    if tool == "zenity":
        return _zenity_save_dialog(default_name)
    elif tool == "kdialog":
        return _kdialog_save_dialog(default_name)
    elif tool == "osascript":
        return _osascript_save_dialog(default_name)
    elif tool == "powershell":
        return _powershell_save_dialog(default_name)
    else:
        return _stdio_save_dialog(default_name)


# -- zenity ----------------------------------------------------------

def _zenity_file_dialog(multiple: bool) -> List[str]:
    args = ["zenity", "--file-selection"]
    if multiple:
        args.append("--multiple")
        args.append("--separator=:")
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, timeout=300
        )
        if result.returncode != 0:
            return []
        output = result.stdout.strip()
        if not output:
            return []
        if multiple:
            return [p.strip() for p in output.split(":") if p.strip()]
        return [output]
    except (subprocess.TimeoutExpired, OSError):
        return []


def _zenity_directory_dialog() -> Optional[str]:
    try:
        result = subprocess.run(
            ["zenity", "--file-selection", "--directory"],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() or None
    except (subprocess.TimeoutExpired, OSError):
        return None


def _zenity_save_dialog(default_name: str) -> Optional[str]:
    args = ["zenity", "--file-selection", "--save", "--confirm-overwrite"]
    if default_name:
        args.extend(["--filename", default_name])
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() or None
    except (subprocess.TimeoutExpired, OSError):
        return None


# -- kdialog ---------------------------------------------------------

def _kdialog_file_dialog(multiple: bool) -> List[str]:
    args = ["kdialog", "--getopenfilename"]
    if multiple:
        args.append("--multiple")
        args.append("--separate-output")
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return []
        output = result.stdout.strip()
        if not output:
            return []
        return [p.strip() for p in output.split("\n") if p.strip()]
    except (subprocess.TimeoutExpired, OSError):
        return []


def _kdialog_directory_dialog() -> Optional[str]:
    try:
        result = subprocess.run(
            ["kdialog", "--getexistingdirectory"],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() or None
    except (subprocess.TimeoutExpired, OSError):
        return None


def _kdialog_save_dialog(default_name: str) -> Optional[str]:
    args = ["kdialog", "--getsavefilename"]
    if default_name:
        args.append(default_name)
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() or None
    except (subprocess.TimeoutExpired, OSError):
        return None


# -- osascript (macOS) -----------------------------------------------

def _osascript_file_dialog(multiple: bool) -> List[str]:
    script = """
    set fileList to choose file with prompt "Select files to share" multiple selections allowed {0}
    set output to ""
    repeat with f in fileList
        set output to output & (POSIX path of f) & linefeed
    end repeat
    return text 1 thru -2 of output
    """.format("true" if multiple else "false")
    try:
        result = subprocess.run(
            ["osascript", "-e", script],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return []
        return [p for p in result.stdout.strip().split("\n") if p]
    except (subprocess.TimeoutExpired, OSError):
        return []


def _osascript_directory_dialog() -> Optional[str]:
    script = """
    set theFolder to choose folder with prompt "Select a folder"
    return POSIX path of theFolder
    """
    try:
        result = subprocess.run(
            ["osascript", "-e", script],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() or None
    except (subprocess.TimeoutExpired, OSError):
        return None


def _osascript_save_dialog(default_name: str) -> Optional[str]:
    script = 'choose folder with prompt "Select folder to save files"'
    if default_name:
        script = (
            f'set defaultName to "{default_name}"\n'
            'choose folder with prompt "Select folder to save files"'
        )
    try:
        result = subprocess.run(
            ["osascript", "-e", script],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        folder = result.stdout.strip()
        if not folder:
            return None
        if default_name:
            return os.path.join(folder, default_name)
        return folder
    except (subprocess.TimeoutExpired, OSError):
        return None


# -- PowerShell (Windows) --------------------------------------------

def _powershell_file_dialog(multiple: bool) -> List[str]:
    multiselect = "$true" if multiple else "$false"
    script = f"""
Add-Type -AssemblyName System.Windows.Forms
$fd = New-Object System.Windows.Forms.OpenFileDialog
$fd.Multiselect = {multiselect}
$fd.Title = "Select files to share"
if ($fd.ShowDialog() -eq 'OK') {{ $fd.FileNames -join '|' }}
"""
    try:
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command", script],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return []
        output = result.stdout.strip()
        if not output:
            return []
        return [p.strip() for p in output.split("|") if p.strip()]
    except (subprocess.TimeoutExpired, OSError):
        return []


def _powershell_directory_dialog() -> Optional[str]:
    script = """
Add-Type -AssemblyName System.Windows.Forms
$fb = New-Object System.Windows.Forms.FolderBrowserDialog
$fb.Description = "Select a folder to share"
if ($fb.ShowDialog() -eq 'OK') { $fb.SelectedPath }
"""
    try:
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command", script],
            capture_output=True, text=True, timeout=300,
        )
        if result.returncode != 0:
            return None
        return result.stdout.strip() or None
    except (subprocess.TimeoutExpired, OSError):
        return None


def _powershell_save_dialog(default_name: str) -> Optional[str]:
    folder = _powershell_directory_dialog()
    if folder is None:
        return None
    if default_name:
        return os.path.join(folder, default_name)
    return folder


# -- stdio fallback --------------------------------------------------

def _stdio_file_dialog(multiple: bool) -> List[str]:
    print()
    print("(No GUI dialog available — enter file paths manually)")
    if multiple:
        print("Enter file paths, one per line. Empty line to finish.")
    else:
        print("Enter file path:")
    paths = []
    while True:
        try:
            line = input().strip()
        except (EOFError, KeyboardInterrupt):
            break
        if not line:
            break
        if os.path.exists(line):
            paths.append(line)
        else:
            print(f"  Path not found: {line}")
        if not multiple:
            break
    return paths


def _stdio_directory_dialog() -> Optional[str]:
    print()
    print("(No GUI dialog available — enter directory path manually)")
    try:
        path = input("Directory path: ").strip()
    except (EOFError, KeyboardInterrupt):
        return None
    if path and os.path.isdir(path):
        return path
    if path:
        print(f"  Not a directory: {path}")
    return None


def _stdio_save_dialog(default_name: str) -> Optional[str]:
    print()
    print("(No GUI dialog available — enter save path manually)")
    prompt = f"Save to directory"
    if default_name:
        prompt += f" (default filename: {default_name})"
    try:
        path = input(f"{prompt}: ").strip()
    except (EOFError, KeyboardInterrupt):
        return None
    if not path:
        return None
    if os.path.isdir(path) and default_name:
        return os.path.join(path, default_name)
    return path
