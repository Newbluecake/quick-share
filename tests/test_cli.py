import pytest
import argparse
from src.cli import parse_arguments, validate_arguments

def test_parse_arguments_defaults():
    """Test parsing arguments with default values."""
    args = parse_arguments(['test.txt'])
    assert args.file_path == 'test.txt'
    assert args.port is None
    assert args.max_downloads == 10
    assert args.timeout == '5m'

def test_parse_arguments_custom():
    """Test parsing arguments with custom values."""
    args = parse_arguments(['test.txt', '-p', '9000', '-n', '3', '-t', '10m'])
    assert args.file_path == 'test.txt'
    assert args.port == 9000
    assert args.max_downloads == 3
    assert args.timeout == '10m'

def test_parse_arguments_long_options():
    """Test parsing arguments with long option names."""
    args = parse_arguments(['test.txt', '--port', '8080', '--max-downloads', '5', '--timeout', '1h'])
    assert args.file_path == 'test.txt'
    assert args.port == 8080
    assert args.max_downloads == 5
    assert args.timeout == '1h'

def test_parse_arguments_help():
    """Test that help flag raises SystemExit."""
    with pytest.raises(SystemExit):
        parse_arguments(['--help'])

def test_parse_arguments_missing_file():
    """Test that missing required file argument raises SystemExit."""
    with pytest.raises(SystemExit):
        parse_arguments([])

def test_validate_arguments_valid():
    """Test validation with valid arguments."""
    args = argparse.Namespace(
        file_path='test.txt',
        port=8080,
        max_downloads=5,
        timeout='5m'
    )
    # Should not raise exception
    validate_arguments(args)

def test_validate_arguments_invalid_port_low():
    """Test validation fails with port number too low."""
    args = argparse.Namespace(
        file_path='test.txt',
        port=1000,
        max_downloads=5,
        timeout='5m'
    )
    with pytest.raises(ValueError, match="Port must be between 1024 and 65535"):
        validate_arguments(args)

def test_validate_arguments_invalid_port_high():
    """Test validation fails with port number too high."""
    args = argparse.Namespace(
        file_path='test.txt',
        port=70000,
        max_downloads=5,
        timeout='5m'
    )
    with pytest.raises(ValueError, match="Port must be between 1024 and 65535"):
        validate_arguments(args)

def test_validate_arguments_invalid_max_downloads():
    """Test validation fails with invalid max_downloads."""
    args = argparse.Namespace(
        file_path='test.txt',
        port=8080,
        max_downloads=0,
        timeout='5m'
    )
    with pytest.raises(ValueError, match="max_downloads must be a positive integer"):
        validate_arguments(args)

def test_validate_arguments_invalid_timeout_format():
    """Test validation fails with invalid timeout format."""
    args = argparse.Namespace(
        file_path='test.txt',
        port=8080,
        max_downloads=5,
        timeout='5' # Missing unit
    )
    with pytest.raises(ValueError, match="Timeout must be in format"):
        validate_arguments(args)

def test_validate_arguments_invalid_timeout_unit():
    """Test validation fails with invalid timeout unit."""
    args = argparse.Namespace(
        file_path='test.txt',
        port=8080,
        max_downloads=5,
        timeout='5x'
    )
    with pytest.raises(ValueError, match="Timeout unit must be"):
        validate_arguments(args)
