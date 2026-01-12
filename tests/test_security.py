"""Tests for security validation module."""

import pytest
from src.security import validate_request_path, is_path_traversal_attack


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
