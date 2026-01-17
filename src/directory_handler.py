"""Directory sharing core handling logic."""

import os
import html
import zipfile
from pathlib import Path
from typing import Dict
from datetime import datetime

try:
    from .templates import generate_spa_html
except ImportError:
    from templates import generate_spa_html


def get_directory_info(directory_path: str) -> Dict:
    """
    Get directory statistics.

    Args:
        directory_path: Path to the directory

    Returns:
        Dictionary with total_files, total_dirs, and total_size
    """
    path = Path(directory_path)

    total_files = 0
    total_dirs = 0
    total_size = 0

    try:
        for item in path.rglob('*'):
            if item.is_file():
                total_files += 1
                try:
                    total_size += item.stat().st_size
                except (OSError, PermissionError):
                    # Skip files we can't read
                    pass
            elif item.is_dir():
                total_dirs += 1
    except (OSError, PermissionError):
        # Skip directories we can't read
        pass

    return {
        'total_files': total_files,
        'total_dirs': total_dirs,
        'total_size': total_size
    }


def get_directory_structure(base_dir: str, current_dir: str) -> Dict:
    """
    Get directory structure as a dictionary.

    Args:
        base_dir: Shared root directory
        current_dir: Current directory being listed

    Returns:
        Dictionary with path info and items list
    """
    # Calculate relative path
    relative_path = os.path.relpath(current_dir, base_dir)
    if relative_path == '.':
        relative_path = '/'
    else:
        relative_path = '/' + relative_path

    items = []
    try:
        for entry in os.scandir(current_dir):
            try:
                stat = entry.stat(follow_symlinks=False)
                items.append({
                    'name': entry.name,
                    'type': 'directory' if entry.is_dir() else 'file',
                    'size': stat.st_size if entry.is_file() else 0,
                    'modified': datetime.fromtimestamp(stat.st_mtime).isoformat()
                })
            except (OSError, PermissionError):
                continue
    except PermissionError:
        # Return empty list or specific error structure?
        # For now, let's return what we have (empty) and handle errors at caller if needed
        pass

    # Sort: directories first, then by name
    items.sort(key=lambda x: (x['type'] != 'directory', x['name'].lower()))

    return {
        'path': relative_path,
        'items': items
    }


def format_file_size(size_bytes: int) -> str:
    """
    Format bytes to human-readable format.

    Args:
        size_bytes: Size in bytes

    Returns:
        Formatted string (e.g., "1.5 MB")
    """
    for unit in ['B', 'KB', 'MB', 'GB', 'TB']:
        if size_bytes < 1024.0:
            return f"{size_bytes:.1f} {unit}"
        size_bytes /= 1024.0
    return f"{size_bytes:.1f} PB"


def generate_directory_listing_html(
    base_dir: str,
    current_dir: str
) -> str:
    """
    Generate directory listing HTML page.

    Args:
        base_dir: Shared root directory
        current_dir: Current directory being listed

    Returns:
        HTML string
    """
    # Calculate relative path (for display)
    relative_path = os.path.relpath(current_dir, base_dir)
    if relative_path == '.':
        relative_path = '/'
    else:
        relative_path = '/' + relative_path

    # List current directory contents
    items = []
    try:
        for entry in os.scandir(current_dir):
            try:
                stat = entry.stat(follow_symlinks=False)
                items.append({
                    'name': entry.name,
                    'is_dir': entry.is_dir(),
                    'size': stat.st_size if entry.is_file() else 0,
                    'modified': datetime.fromtimestamp(stat.st_mtime)
                })
            except (OSError, PermissionError):
                # Skip files we can't access
                continue
    except PermissionError:
        return generate_error_html("Permission denied")

    # Sort: directories first, then by name
    items.sort(key=lambda x: (not x['is_dir'], x['name'].lower()))

    # Generate HTML
    html_parts = [
        '<!DOCTYPE html>',
        '<html lang="en">',
        '<head>',
        '    <meta charset="UTF-8">',
        '    <meta name="viewport" content="width=device-width, initial-scale=1.0">',
        '    <title>Quick Share - Directory Listing</title>',
        '    <style>',
        '        body { font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }',
        '        .container { max-width: 1200px; margin: 0 auto; background: white; padding: 20px; border-radius: 8px; }',
        '        h1 { color: #333; border-bottom: 2px solid #007bff; padding-bottom: 10px; }',
        '        .btn { padding: 10px 20px; background: #007bff; color: white; text-decoration: none; border-radius: 4px; margin-right: 10px; }',
        '        table { width: 100%; border-collapse: collapse; margin-top: 20px; }',
        '        th, td { text-align: left; padding: 12px; border-bottom: 1px solid #ddd; }',
        '        .dir { color: #007bff; font-weight: bold; }',
        '    </style>',
        '</head>',
        '<body>',
        '    <div class="container">',
        f'        <h1>Quick Share - {html.escape(os.path.basename(base_dir))}</h1>',
        f'        <div>Current Path: {html.escape(relative_path)}</div>',
        '        <div>',
        f'            <a href="/?download=zip" class="btn">Download All as Zip</a>',
    ]

    # Add "Go Up" button if not at root
    if current_dir != base_dir:
        parent_relative = os.path.dirname(relative_path)
        if not parent_relative:
            parent_relative = '/'
        html_parts.append(f'            <a href="{parent_relative}" class="btn">Go Up</a>')

    html_parts.extend([
        '        </div>',
        '        <table>',
        '            <thead>',
        '                <tr><th>Name</th><th>Size</th><th>Modified</th></tr>',
        '            </thead>',
        '            <tbody>',
    ])

    # Add file/directory rows
    if not items:
        html_parts.append('                <tr><td colspan="3">No files or directories</td></tr>')
    else:
        for item in items:
            icon = '📁' if item['is_dir'] else '📄'
            css_class = 'dir' if item['is_dir'] else 'file'
            size_str = '-' if item['is_dir'] else format_file_size(item['size'])
            modified_str = item['modified'].strftime('%Y-%m-%d %H:%M')

            link_path = os.path.join(relative_path, item['name'])
            if item['is_dir']:
                link_path += '/'

            html_parts.append(
                f'                <tr>'
                f'<td class="{css_class}"><a href="{html.escape(link_path)}">{icon} {html.escape(item["name"])}</a></td>'
                f'<td>{size_str}</td>'
                f'<td>{modified_str}</td>'
                f'</tr>'
            )

    html_parts.extend([
        '            </tbody>',
        '        </table>',
        '    </div>',
        '</body>',
        '</html>',
    ])

    return '\n'.join(html_parts)


def generate_error_html(error_message: str) -> str:
    """Generate error page HTML."""
    return f"""
    <!DOCTYPE html>
    <html>
    <head>
        <title>Error - Quick Share</title>
    </head>
    <body>
        <h1>Error</h1>
        <p>{html.escape(error_message)}</p>
    </body>
    </html>
    """


def stream_directory_as_zip(
    output_stream,
    base_dir: str,
    target_dir: str
) -> None:
    """
    Stream directory as zip file.

    Args:
        output_stream: Output stream (HTTP response wfile)
        base_dir: Shared root directory
        target_dir: Target directory to zip (may be subdirectory)
    """
    # Use streaming zip writing
    with zipfile.ZipFile(output_stream, 'w', zipfile.ZIP_DEFLATED) as zipf:
        for root, dirs, files in os.walk(target_dir):
            for file in files:
                file_path = os.path.join(root, file)

                # Calculate relative path within zip
                arcname = os.path.relpath(file_path, base_dir)

                try:
                    zipf.write(file_path, arcname)
                except (OSError, PermissionError):
                    # Skip files we can't read
                    continue
