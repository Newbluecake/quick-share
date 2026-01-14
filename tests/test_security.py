"""Tests for security validation module."""

import pytest
import os
import tempfile
from pathlib import Path
from src.security import validate_request_path, is_path_traversal_attack, validate_directory_path


class TestPathTraversalDetection:
    """Test path traversal attack detection."""

    def test_clean_path(self):
        """Test that clean paths are not flagged."""
        assert is_path_traversal_attack("/test.txt") is False
        assert is_path_traversal_attack("test.txt") is False
        assert is_path_traversal_attack("/files/test.txt") is False

    def test_double_dot_attack(self):
        """Test detection of .. path traversal."""
        assert is_path_traversal_attack("../etc/passwd") is True
        assert is_path_traversal_attack("../../etc/passwd") is True
        assert is_path_traversal_attack("/test/../etc/passwd") is True

    def test_url_encoded_attack(self):
        """Test detection of URL-encoded path traversal."""
        # %2e = .
        assert is_path_traversal_attack("%2e%2e/etc/passwd") is True
        assert is_path_traversal_attack("/%2e%2e/etc/passwd") is True
        # %2f = /
        assert is_path_traversal_attack("%2e%2e%2fetc%2fpasswd") is True

    def test_mixed_encoding_attack(self):
        """Test detection of mixed encoding attacks."""
        assert is_path_traversal_attack("..%2fetc/passwd") is True
        assert is_path_traversal_attack("%2e./etc/passwd") is True

    def test_backslash_attack(self):
        """Test detection of backslash-based traversal (Windows)."""
        assert is_path_traversal_attack("..\\etc\\passwd") is True
        assert is_path_traversal_attack("..%5cetc%5cpasswd") is True  # %5c = \

    def test_multi_level_url_encoding(self):
        """Test detection of multi-level URL encoded path traversal."""
        # Double encoding: %252e = %2e = .
        assert is_path_traversal_attack("%252e%252e/etc/passwd") is True
        assert is_path_traversal_attack("%252e%252e%252fetc%252fpasswd") is True

    def test_complex_traversal_patterns(self):
        """Test detection of complex path traversal patterns."""
        assert is_path_traversal_attack("/../../../etc/passwd") is True
        assert is_path_traversal_attack("/subdir/../../../secret.txt") is True
        assert is_path_traversal_attack("/./subdir/../../file.txt") is True

    def test_allow_normal_paths_with_dots(self):
        """Test that normal paths with dots are allowed."""
        assert is_path_traversal_attack("/file.txt") is False
        assert is_path_traversal_attack("/subdir/file.txt") is False
        assert is_path_traversal_attack("/a/b/c/file.txt") is False
        assert is_path_traversal_attack("/file.tar.gz") is False


class TestValidateRequestPath:
    """Test request path validation."""

    def test_normal_path(self):
        """Test validation of normal request path."""
        is_valid, path = validate_request_path("/test.txt", "test.txt")
        assert is_valid is True
        assert path == "/test.txt"

    def test_with_query_string(self):
        """Test that query strings are removed."""
        is_valid, path = validate_request_path("/test.txt?download=1", "test.txt")
        assert is_valid is True
        assert path == "/test.txt"
        assert "?" not in path

    def test_with_fragment(self):
        """Test that URL fragments are removed."""
        is_valid, path = validate_request_path("/test.txt#section", "test.txt")
        assert is_valid is True
        assert path == "/test.txt"

    def test_path_traversal_double_dot(self):
        """Test rejection of .. path traversal."""
        is_valid, _ = validate_request_path("/../etc/passwd", "test.txt")
        assert is_valid is False

    def test_path_traversal_url_encoded(self):
        """Test rejection of URL-encoded path traversal."""
        is_valid, _ = validate_request_path("/%2e%2e/etc/passwd", "test.txt")
        assert is_valid is False

    def test_path_traversal_double_encoded(self):
        """Test rejection of double URL-encoded path traversal."""
        # %252e = %2e = .
        is_valid, _ = validate_request_path("/%252e%252e/etc/passwd", "test.txt")
        assert is_valid is False

    def test_absolute_path_sensitive(self):
        """Test rejection of absolute paths to sensitive locations."""
        is_valid, _ = validate_request_path("/etc/passwd", "test.txt")
        assert is_valid is False

    def test_wrong_filename(self):
        """Test rejection of wrong filename."""
        is_valid, _ = validate_request_path("/other.txt", "test.txt")
        assert is_valid is False

    def test_wrong_filename_with_query(self):
        """Test rejection of wrong filename even with query string."""
        is_valid, _ = validate_request_path("/other.txt?download=1", "test.txt")
        assert is_valid is False

    def test_case_sensitive_filename(self):
        """Test that filename matching is case-sensitive."""
        is_valid, _ = validate_request_path("/Test.txt", "test.txt")
        assert is_valid is False

    def test_url_encoded_valid_path(self):
        """Test that URL-encoded valid filenames work."""
        # %20 = space
        is_valid, path = validate_request_path("/my%20file.txt", "my file.txt")
        assert is_valid is True
        assert path == "/my file.txt"

    def test_nested_path_traversal(self):
        """Test detection of nested path traversal attempts."""
        is_valid, _ = validate_request_path("/test/../../etc/passwd", "test.txt")
        assert is_valid is False

    def test_backslash_traversal(self):
        """Test rejection of backslash-based traversal."""
        is_valid, _ = validate_request_path("/..\\etc\\passwd", "test.txt")
        assert is_valid is False

    def test_empty_path(self):
        """Test handling of empty path."""
        is_valid, _ = validate_request_path("", "test.txt")
        assert is_valid is False

    def test_root_path(self):
        """Test handling of root path only."""
        is_valid, _ = validate_request_path("/", "test.txt")
        assert is_valid is False

    def test_multiple_slashes(self):
        """Test handling of multiple slashes."""
        is_valid, path = validate_request_path("///test.txt", "test.txt")
        # Should normalize to /test.txt
        assert is_valid is True
        assert path == "/test.txt"


class TestValidateDirectoryPath:
    """Test directory path validation for sandbox security."""

    def test_should_allow_subdirectory_access(self):
        """Test allowing legitimate subdirectory access."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            # Create test directory structure
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()
            (shared_dir / "file.txt").write_text("content")
            subdir = shared_dir / "subdir"
            subdir.mkdir()
            (subdir / "nested.txt").write_text("nested")

            # Test root directory access
            is_valid, real_path = validate_directory_path("/", str(shared_dir))
            assert is_valid is True
            assert os.path.samefile(real_path, str(shared_dir))

            # Test file access
            is_valid, real_path = validate_directory_path("/file.txt", str(shared_dir))
            assert is_valid is True
            assert os.path.samefile(real_path, str(shared_dir / "file.txt"))

            # Test subdirectory access
            is_valid, real_path = validate_directory_path("/subdir/nested.txt", str(shared_dir))
            assert is_valid is True
            assert os.path.samefile(real_path, str(subdir / "nested.txt"))

    def test_should_reject_path_outside_sandbox(self):
        """Test rejecting paths outside sandbox."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()
            outside_file = Path(tmp_dir) / "outside.txt"
            outside_file.write_text("secret")

            # Try to traverse to parent directory
            is_valid, _ = validate_directory_path("/../outside.txt", str(shared_dir))
            assert is_valid is False

            is_valid, _ = validate_directory_path("/subdir/../../outside.txt", str(shared_dir))
            assert is_valid is False

    def test_should_reject_nonexistent_path(self):
        """Test rejecting non-existent paths."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()

            is_valid, _ = validate_directory_path("/nonexistent.txt", str(shared_dir))
            assert is_valid is False

    def test_should_handle_url_encoded_paths(self):
        """Test handling URL-encoded paths."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()
            (shared_dir / "my file.txt").write_text("content")

            # URL-encoded space (%20)
            is_valid, real_path = validate_directory_path("/my%20file.txt", str(shared_dir))
            assert is_valid is True
            assert os.path.samefile(real_path, str(shared_dir / "my file.txt"))

    def test_should_reject_encoded_traversal(self):
        """Test rejecting URL-encoded path traversal."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()

            # %2e%2e = ..
            is_valid, _ = validate_directory_path("/%2e%2e/etc/passwd", str(shared_dir))
            assert is_valid is False

    def test_should_handle_query_strings(self):
        """Test handling paths with query strings."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()
            (shared_dir / "file.txt").write_text("content")

            is_valid, real_path = validate_directory_path("/file.txt?download=1", str(shared_dir))
            assert is_valid is True
            assert os.path.samefile(real_path, str(shared_dir / "file.txt"))

    def test_symlink_escape_detection(self):
        """Test detection of symlink escaping sandbox."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()

            # Create file outside sandbox
            secret_dir = Path(tmp_dir) / "secret"
            secret_dir.mkdir()
            secret_file = secret_dir / "confidential.txt"
            secret_file.write_text("secret data")

            # Create symlink inside shared directory pointing outside
            symlink_path = shared_dir / "escape_link"
            symlink_path.symlink_to(secret_file)

            # Should reject access through symlink
            is_valid, _ = validate_directory_path("/escape_link", str(shared_dir))
            assert is_valid is False, "Should reject symlink pointing outside sandbox"

    def test_symlink_within_sandbox_allowed(self):
        """Test allowing symlinks within sandbox."""
        with tempfile.TemporaryDirectory() as tmp_dir:
            shared_dir = Path(tmp_dir) / "shared"
            shared_dir.mkdir()

            # Create file within sandbox
            real_file = shared_dir / "real.txt"
            real_file.write_text("real content")

            # Create symlink pointing to file within sandbox
            symlink_path = shared_dir / "link.txt"
            symlink_path.symlink_to(real_file)

            # Should allow access
            is_valid, real_path = validate_directory_path("/link.txt", str(shared_dir))
            assert is_valid is True
            # Verify it resolves to the real path
            assert os.path.samefile(real_path, str(real_file))
