"""Tests for directory handler module."""

import sys
import os
import tempfile
from pathlib import Path
import zipfile
import io

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

from directory_handler import (
    get_directory_info,
    format_file_size,
    generate_directory_listing_html,
    stream_directory_as_zip,
    get_multi_share_root_structure,
    stream_multi_paths_as_zip,
)


class TestDirectoryInfo:
    """Test directory information functions."""

    def test_get_directory_info_basic(self):
        """Test basic directory statistics."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            test_dir = Path(tmp_dir) / "test"
            test_dir.mkdir()

            # Create files
            (test_dir / "file1.txt").write_text("a" * 100)
            (test_dir / "file2.txt").write_text("b" * 200)

            # Create subdirectory and file
            subdir = test_dir / "subdir"
            subdir.mkdir()
            (subdir / "file3.txt").write_text("c" * 150)

            info = get_directory_info(str(test_dir))

            assert info['total_files'] == 3
            assert info['total_dirs'] == 1
            assert info['total_size'] == 450

    def test_get_directory_info_empty(self):
        """Test empty directory statistics."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            empty_dir = Path(tmp_dir) / "empty"
            empty_dir.mkdir()

            info = get_directory_info(str(empty_dir))

            assert info['total_files'] == 0
            assert info['total_dirs'] == 0
            assert info['total_size'] == 0

    def test_format_file_size(self):
        """Test file size formatting."""
        assert format_file_size(500) == "500.0 B"
        assert format_file_size(1500) == "1.5 KB"
        assert format_file_size(1024 * 1024) == "1.0 MB"
        assert format_file_size(1536 * 1024) == "1.5 MB"
        assert format_file_size(1024 * 1024 * 1024) == "1.0 GB"


class TestHTMLListingGeneration:
    """Test HTML directory listing generation."""

    def test_generate_directory_listing_basic(self):
        """Test basic directory listing HTML generation."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            base_dir = Path(tmp_dir) / "shared"
            base_dir.mkdir()

            # Create files and directories
            (base_dir / "file1.txt").write_text("content")
            subdir = base_dir / "subdir"
            subdir.mkdir()

            html = generate_directory_listing_html(str(base_dir), str(base_dir))

            # Verify HTML contains key elements
            assert "Quick Share" in html
            assert "file1.txt" in html
            assert "subdir" in html
            assert "Download All as Zip" in html
            assert "<!DOCTYPE html>" in html

    def test_generate_directory_listing_subdirectory(self):
        """Test subdirectory listing HTML."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            base_dir = Path(tmp_dir) / "shared"
            base_dir.mkdir()
            subdir = base_dir / "subdir"
            subdir.mkdir()
            (subdir / "nested.txt").write_text("nested")

            html = generate_directory_listing_html(str(base_dir), str(subdir))

            # Should show subdirectory path
            assert "/subdir" in html
            assert "nested.txt" in html
            # Should have "Go Up" button
            assert "Go Up" in html or "up" in html.lower()

    def test_generate_directory_listing_empty(self):
        """Test empty directory listing."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            empty_dir = Path(tmp_dir) / "empty"
            empty_dir.mkdir()

            html = generate_directory_listing_html(str(empty_dir), str(empty_dir))

            assert "No files" in html or "empty" in html.lower()

    def test_generate_directory_listing_special_chars(self):
        """Test special characters in filenames."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            base_dir = Path(tmp_dir) / "shared"
            base_dir.mkdir()

            # Create files with special characters
            (base_dir / "file with spaces.txt").write_text("content")
            (base_dir / "中文文件.txt").write_text("中文")

            html = generate_directory_listing_html(str(base_dir), str(base_dir))

            # HTML should escape properly
            assert "file with spaces.txt" in html
            assert "中文文件.txt" in html


class TestZipGeneration:
    """Test streaming zip generation."""

    def test_stream_directory_as_zip_basic(self):
        """Test basic zip generation."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            base_dir = Path(tmp_dir) / "shared"
            base_dir.mkdir()

            # Create files
            (base_dir / "file1.txt").write_text("content1")
            subdir = base_dir / "subdir"
            subdir.mkdir()
            (subdir / "file2.txt").write_text("content2")

            # Stream zip to memory
            output = io.BytesIO()
            stream_directory_as_zip(output, str(base_dir), str(base_dir))

            # Verify zip contents
            output.seek(0)
            with zipfile.ZipFile(output, 'r') as zf:
                names = zf.namelist()
                assert 'file1.txt' in names
                assert 'subdir/file2.txt' in names

                # Verify content
                assert zf.read('file1.txt') == b'content1'
                assert zf.read('subdir/file2.txt') == b'content2'

    def test_stream_directory_as_zip_preserves_structure(self):
        """Test zip preserves directory structure."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            base_dir = Path(tmp_dir) / "shared"
            base_dir.mkdir()

            # Create deep directory structure
            deep = base_dir / "a" / "b" / "c"
            deep.mkdir(parents=True)
            (deep / "deep.txt").write_text("deep content")

            output = io.BytesIO()
            stream_directory_as_zip(output, str(base_dir), str(base_dir))

            output.seek(0)
            with zipfile.ZipFile(output, 'r') as zf:
                assert 'a/b/c/deep.txt' in zf.namelist()

    def test_stream_directory_as_zip_empty(self):
        """Test empty directory zip."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            empty_dir = Path(tmp_dir) / "empty"
            empty_dir.mkdir()

            output = io.BytesIO()
            stream_directory_as_zip(output, str(empty_dir), str(empty_dir))

            output.seek(0)
            with zipfile.ZipFile(output, 'r') as zf:
                # Empty zip is still valid
                assert len(zf.namelist()) == 0


class TestGetMultiShareRootStructure:
    """Tests for get_multi_share_root_structure() – T-004."""

    def test_single_file(self, tmp_path):
        f = tmp_path / "hello.txt"
        f.write_text("hi")
        data = get_multi_share_root_structure([(str(f), "file")])
        assert data["path"] == "/"
        assert len(data["items"]) == 1
        item = data["items"][0]
        assert item["name"] == "hello.txt"
        assert item["type"] == "file"
        assert item["size"] == 2

    def test_single_directory(self, tmp_path):
        d = tmp_path / "mydir"
        d.mkdir()
        data = get_multi_share_root_structure([(str(d), "directory")])
        assert len(data["items"]) == 1
        item = data["items"][0]
        assert item["name"] == "mydir"
        assert item["type"] == "directory"
        assert item["size"] == 0

    def test_mixed_paths(self, tmp_path):
        f = tmp_path / "a.txt"
        f.write_text("x")
        d = tmp_path / "mydir"
        d.mkdir()
        data = get_multi_share_root_structure([(str(f), "file"), (str(d), "directory")])
        assert len(data["items"]) == 2
        types = [i["type"] for i in data["items"]]
        # Directories sorted first
        assert types[0] == "directory"

    def test_empty_list(self):
        data = get_multi_share_root_structure([])
        assert data["path"] == "/"
        assert data["items"] == []

    def test_sorting(self, tmp_path):
        """Directories come before files; within each group, alphabetical."""
        f1 = tmp_path / "z.txt"
        f1.write_text("z")
        f2 = tmp_path / "a.txt"
        f2.write_text("a")
        d = tmp_path / "mydir"
        d.mkdir()
        data = get_multi_share_root_structure([
            (str(f1), "file"), (str(f2), "file"), (str(d), "directory")
        ])
        names = [i["name"] for i in data["items"]]
        assert names[0] == "mydir"
        assert names[1] == "a.txt"
        assert names[2] == "z.txt"


class TestStreamMultiPathsAsZip:
    """Tests for stream_multi_paths_as_zip() – T-004."""

    def test_single_file(self, tmp_path):
        f = tmp_path / "test.txt"
        f.write_text("content")
        buf = io.BytesIO()
        stream_multi_paths_as_zip(buf, [(str(f), "file")])
        buf.seek(0)
        with zipfile.ZipFile(buf) as zf:
            assert "test.txt" in zf.namelist()
            assert zf.read("test.txt") == b"content"

    def test_directory(self, tmp_path):
        d = tmp_path / "mydir"
        d.mkdir()
        (d / "sub.txt").write_text("sub")
        (d / "inner").mkdir()
        (d / "inner" / "deep.txt").write_text("deep")
        buf = io.BytesIO()
        stream_multi_paths_as_zip(buf, [(str(d), "directory")])
        buf.seek(0)
        with zipfile.ZipFile(buf) as zf:
            names = zf.namelist()
            assert "mydir/sub.txt" in names
            assert "mydir/inner/deep.txt" in names

    def test_multiple_paths(self, tmp_path):
        f = tmp_path / "file.txt"
        f.write_text("hello")
        d = tmp_path / "folder"
        d.mkdir()
        (d / "child.txt").write_text("child")
        buf = io.BytesIO()
        stream_multi_paths_as_zip(buf, [(str(f), "file"), (str(d), "directory")])
        buf.seek(0)
        with zipfile.ZipFile(buf) as zf:
            names = zf.namelist()
            assert "file.txt" in names
            assert "folder/child.txt" in names

    def test_empty_paths(self):
        buf = io.BytesIO()
        stream_multi_paths_as_zip(buf, [])
        buf.seek(0)
        with zipfile.ZipFile(buf) as zf:
            assert zf.namelist() == []
