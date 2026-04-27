#!/usr/bin/env python3
"""Setup configuration for Quick Share."""

import os
import subprocess
from setuptools import setup, find_packages
from pathlib import Path

# Read README for long description
readme_file = Path(__file__).parent / "README.md"
long_description = readme_file.read_text(encoding="utf-8") if readme_file.exists() else ""


def get_commit():
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, timeout=5,
            cwd=os.path.dirname(__file__),
        )
        if result.returncode == 0:
            return result.stdout.strip()
    except Exception:
        pass
    return "unknown"


# Generate _commit.py before build
_commit_file = Path(__file__).parent / "src" / "_commit.py"
_original_commit = _commit_file.read_text() if _commit_file.exists() else None
_commit_file.write_text(f'__commit__ = "{get_commit()}"\n')

try:
    setup(
        name="quick-share",
        version="1.8.0",
        description="Fast file sharing via HTTP server with automatic LAN IP detection",
        long_description=long_description,
        long_description_content_type="text/markdown",
        author="Quick Share Contributors",
        url="https://github.com/Newbluecake/quick-share",
        packages=find_packages(exclude=["tests", "tests.*"]),
        entry_points={
            "console_scripts": [
                "quick-share=src.main:main",
            ],
        },
        python_requires=">=3.8",
        classifiers=[
            "Development Status :: 4 - Beta",
            "Intended Audience :: Developers",
            "Topic :: Internet :: WWW/HTTP :: HTTP Servers",
            "License :: OSI Approved :: MIT License",
            "Programming Language :: Python :: 3",
            "Programming Language :: Python :: 3.8",
            "Programming Language :: Python :: 3.9",
            "Programming Language :: Python :: 3.10",
            "Programming Language :: Python :: 3.11",
        ],
    )
finally:
    if _original_commit is not None:
        _commit_file.write_text(_original_commit)
