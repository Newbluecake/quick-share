"""Tests for utility functions."""

import pytest
from src.utils import format_file_size, parse_duration


class TestFormatFileSize:
    """Tests for format_file_size function."""

    def test_format_bytes(self):
        """Test formatting bytes (less than 1 KB)."""
        assert format_file_size(0) == "0 B"
        assert format_file_size(1) == "1 B"
        assert format_file_size(500) == "500 B"
        assert format_file_size(1023) == "1023 B"

    def test_format_kilobytes(self):
        """Test formatting kilobytes."""
        assert format_file_size(1024) == "1.0 KB"
        assert format_file_size(1536) == "1.5 KB"
        assert format_file_size(2048) == "2.0 KB"
        assert format_file_size(10240) == "10.0 KB"

    def test_format_megabytes(self):
        """Test formatting megabytes."""
        assert format_file_size(1048576) == "1.0 MB"
        assert format_file_size(1258291) == "1.2 MB"
        assert format_file_size(5242880) == "5.0 MB"
        assert format_file_size(10485760) == "10.0 MB"

    def test_format_gigabytes(self):
        """Test formatting gigabytes."""
        assert format_file_size(1073741824) == "1.0 GB"
        assert format_file_size(2147483648) == "2.0 GB"
        assert format_file_size(5368709120) == "5.0 GB"

    def test_edge_cases(self):
        """Test edge cases and error handling."""
        # Negative numbers should raise ValueError
        with pytest.raises(ValueError):
            format_file_size(-1)

        with pytest.raises(ValueError):
            format_file_size(-1024)

        # Non-integer should raise TypeError
        with pytest.raises(TypeError):
            format_file_size("1024")

        with pytest.raises(TypeError):
            format_file_size(1024.5)

    def test_large_files(self):
        """Test very large file sizes."""
        # Terabyte range
        assert format_file_size(1099511627776) == "1.0 TB"
        # Very large GB value
        assert format_file_size(107374182400) == "100.0 GB"


class TestParseDuration:
    """Tests for parse_duration function."""

    def test_parse_seconds(self):
        """Test parsing seconds."""
        assert parse_duration("0s") == 0
        assert parse_duration("30s") == 30
        assert parse_duration("45s") == 45
        assert parse_duration("90s") == 90

    def test_parse_minutes(self):
        """Test parsing minutes."""
        assert parse_duration("1m") == 60
        assert parse_duration("5m") == 300
        assert parse_duration("10m") == 600
        assert parse_duration("30m") == 1800

    def test_parse_hours(self):
        """Test parsing hours."""
        assert parse_duration("1h") == 3600
        assert parse_duration("2h") == 7200
        assert parse_duration("24h") == 86400

    def test_parse_zero(self):
        """Test parsing zero duration."""
        assert parse_duration("0") == 0
        assert parse_duration("0s") == 0
        assert parse_duration("0m") == 0
        assert parse_duration("0h") == 0

    def test_edge_cases(self):
        """Test edge cases and error handling."""
        # Invalid format should raise ValueError
        with pytest.raises(ValueError):
            parse_duration("invalid")

        with pytest.raises(ValueError):
            parse_duration("10x")

        with pytest.raises(ValueError):
            parse_duration("abc")

        # Empty string should raise ValueError
        with pytest.raises(ValueError):
            parse_duration("")

        # Negative duration should raise ValueError
        with pytest.raises(ValueError):
            parse_duration("-5m")

        # Non-string should raise TypeError
        with pytest.raises(TypeError):
            parse_duration(30)

        with pytest.raises(TypeError):
            parse_duration(None)

    def test_whitespace_handling(self):
        """Test handling of whitespace."""
        assert parse_duration("5m") == 300
        assert parse_duration(" 5m ") == 300
        assert parse_duration("  10s  ") == 10

    def test_large_durations(self):
        """Test large duration values."""
        assert parse_duration("100h") == 360000
        assert parse_duration("1440m") == 86400  # 24 hours
        assert parse_duration("86400s") == 86400  # 24 hours
