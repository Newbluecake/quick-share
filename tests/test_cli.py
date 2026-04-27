import pytest
import argparse
from src.cli import parse_arguments, validate_arguments

def test_parse_arguments_defaults():
    """Test parsing arguments with default values (single path)."""
    args = parse_arguments(['test.txt'])
    assert args.file_paths == ['test.txt']
    assert args.port is None
    assert args.max_downloads == 10
    assert args.timeout == '5m'

def test_parse_arguments_custom():
    """Test parsing arguments with custom values (single path)."""
    args = parse_arguments(['test.txt', '-p', '9000', '-n', '3', '-t', '10m'])
    assert args.file_paths == ['test.txt']
    assert args.port == 9000
    assert args.max_downloads == 3
    assert args.timeout == '10m'

def test_parse_arguments_long_options():
    """Test parsing arguments with long option names."""
    args = parse_arguments(['test.txt', '--port', '8080', '--max-downloads', '5', '--timeout', '1h'])
    assert args.file_paths == ['test.txt']
    assert args.port == 8080
    assert args.max_downloads == 5
    assert args.timeout == '1h'

def test_parse_arguments_multi_paths():
    """Test parsing multiple paths (new nargs='+' behaviour)."""
    args = parse_arguments(['file1.txt', 'file2.pdf', 'mydir/'])
    assert args.file_paths == ['file1.txt', 'file2.pdf', 'mydir/']

def test_parse_arguments_multi_paths_with_flags():
    """Test multiple paths combined with optional flags."""
    args = parse_arguments(['a.txt', 'b.txt', '-p', '9100', '-n', '5'])
    assert args.file_paths == ['a.txt', 'b.txt']
    assert args.port == 9100
    assert args.max_downloads == 5

def test_parse_arguments_multi_paths_legacy():
    """Test multiple paths with --legacy flag."""
    args = parse_arguments(['a.txt', 'dir1/', '--legacy'])
    assert args.file_paths == ['a.txt', 'dir1/']
    assert args.legacy is True

def test_parse_arguments_help():
    """Test that help flag raises SystemExit."""
    with pytest.raises(SystemExit):
        parse_arguments(['--help'])

def test_parse_arguments_missing_file():
    """Test that missing file argument is now valid (for --upload mode)."""
    args = parse_arguments([])
    assert args.file_paths == []
    assert args.upload is None  # No upload flag either

def test_parse_arguments_upload_standalone():
    """Test parsing --upload without file paths."""
    args = parse_arguments(['--upload'])
    assert args.file_paths == []
    assert args.upload == '.'

def test_parse_arguments_upload_with_dir():
    """Test parsing --upload with save directory."""
    args = parse_arguments(['--upload', '/tmp/uploads'])
    assert args.file_paths == []
    assert args.upload == '/tmp/uploads'

def test_parse_arguments_upload_password():
    """Test parsing --upload-password."""
    args = parse_arguments(['--upload', '--upload-password', 'secret'])
    assert args.upload == '.'
    assert args.upload_password == 'secret'

def test_parse_arguments_share_and_upload():
    """Test parsing file paths combined with --upload."""
    args = parse_arguments(['./docs', '--upload', '/tmp/uploads'])
    assert args.file_paths == ['./docs']
    assert args.upload == '/tmp/uploads'

def test_validate_arguments_valid():
    """Test validation with valid arguments."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=8080,
        max_downloads=5,
        timeout='5m',
        upload=None,
        upload_password=None,
    )
    # Should not raise exception
    validate_arguments(args)

def test_validate_arguments_invalid_port_low():
    """Test validation fails with port number too low."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=1000,
        max_downloads=5,
        timeout='5m',
        upload=None,
        upload_password=None,
    )
    with pytest.raises(ValueError, match="Port must be between 1024 and 65535"):
        validate_arguments(args)

def test_validate_arguments_invalid_port_high():
    """Test validation fails with port number too high."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=70000,
        max_downloads=5,
        timeout='5m',
        upload=None,
        upload_password=None,
    )
    with pytest.raises(ValueError, match="Port must be between 1024 and 65535"):
        validate_arguments(args)

def test_validate_arguments_invalid_max_downloads():
    """Test validation fails with invalid max_downloads."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=8080,
        max_downloads=0,
        timeout='5m',
        upload=None,
        upload_password=None,
    )
    with pytest.raises(ValueError, match="max_downloads must be a positive integer"):
        validate_arguments(args)

def test_validate_arguments_invalid_timeout_format():
    """Test validation fails with invalid timeout format."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=8080,
        max_downloads=5,
        timeout='5', # Missing unit
        upload=None,
        upload_password=None,
    )
    with pytest.raises(ValueError, match="Timeout must be in format"):
        validate_arguments(args)

def test_validate_arguments_invalid_timeout_unit():
    """Test validation fails with invalid timeout unit."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=8080,
        max_downloads=5,
        timeout='5x',
        upload=None,
        upload_password=None,
    )
    with pytest.raises(ValueError, match="Timeout unit must be"):
        validate_arguments(args)

def test_validate_arguments_upload_valid():
    """Test validation with --upload and no file paths."""
    args = argparse.Namespace(
        file_paths=[],
        port=8080,
        max_downloads=5,
        timeout='5m',
        upload='.',
        upload_password=None,
    )
    # Should not raise exception
    validate_arguments(args)

def test_validate_arguments_upload_password_without_upload():
    """Test validation fails when --upload-password is used without --upload."""
    args = argparse.Namespace(
        file_paths=['test.txt'],
        port=8080,
        max_downloads=5,
        timeout='5m',
        upload=None,
        upload_password='secret',
    )
    with pytest.raises(ValueError, match="--upload-password requires --upload"):
        validate_arguments(args)

def test_validate_arguments_no_paths_no_upload():
    """Test validation fails when no file paths and no --upload."""
    args = argparse.Namespace(
        file_paths=[],
        port=8080,
        max_downloads=5,
        timeout='5m',
        upload=None,
        upload_password=None,
    )
    with pytest.raises(ValueError, match="Provide one or more"):
        validate_arguments(args)
