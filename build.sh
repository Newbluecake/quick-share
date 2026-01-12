#!/bin/bash
# Build script for Quick Share - Package into standalone executable

set -e

echo "Building Quick Share executable..."

# Install dependencies
pip install -r requirements-dev.txt

# Run tests first
echo "Running tests..."
pytest tests/ -v

# Build with PyInstaller
echo "Packaging with PyInstaller..."
pyinstaller --onefile \
            --name quick-share \
            --add-data "README.md:." \
            src/main.py

echo "Build complete! Executable at: dist/quick-share"
echo ""
echo "To test:"
echo "  ./dist/quick-share --help"
