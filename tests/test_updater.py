"""Tests for the updater module."""

import sys
import json
from unittest.mock import patch, MagicMock
import pytest

from src.updater import Updater, UpdateError, UpdateCheckError, run_update
from src.cli import is_update_command, parse_update_arguments


class TestVersionComparison:
    """Test version comparison logic."""

    def test_compare_versions_greater(self):
        """Test v1 > v2."""
        updater = Updater()
        assert updater._compare_versions("1.0.1", "1.0.0") > 0
        assert updater._compare_versions("1.1.0", "1.0.9") > 0
        assert updater._compare_versions("2.0.0", "1.9.9") > 0

    def test_compare_versions_less(self):
        """Test v1 < v2."""
        updater = Updater()
        assert updater._compare_versions("1.0.0", "1.0.1") < 0
        assert updater._compare_versions("1.0.9", "1.1.0") < 0
        assert updater._compare_versions("1.9.9", "2.0.0") < 0

    def test_compare_versions_equal(self):
        """Test v1 == v2."""
        updater = Updater()
        assert updater._compare_versions("1.0.0", "1.0.0") == 0
        assert updater._compare_versions("2.1.3", "2.1.3") == 0

    def test_compare_versions_with_v_prefix(self):
        """Test version comparison with 'v' prefix."""
        updater = Updater()
        assert updater._compare_versions("v1.0.1", "1.0.0") > 0
        assert updater._compare_versions("1.0.1", "v1.0.0") > 0
        assert updater._compare_versions("v1.0.0", "v1.0.0") == 0

    def test_compare_versions_different_lengths(self):
        """Test version comparison with different segment counts."""
        updater = Updater()
        assert updater._compare_versions("1.0.0.1", "1.0.0") > 0
        assert updater._compare_versions("1.0", "1.0.0") == 0


class TestInstallMethodDetection:
    """Test installation method detection."""

    @patch('subprocess.run')
    def test_detect_pip_installation(self, mock_run):
        """Test detection of pip installation."""
        mock_run.return_value = MagicMock(
            returncode=0,
            stdout="Name: quick-share\nVersion: 1.0.0\n"
        )
        updater = Updater()
        # Should detect as pip since mock returns success
        assert updater.install_method in ('pip', 'source')

    def test_detect_not_frozen(self):
        """Test that non-frozen Python is not detected as exe."""
        updater = Updater()
        # Running in test environment, should not be exe
        assert updater.install_method != 'exe' or sys.platform != 'win32'


class TestUpdateCheck:
    """Test update checking functionality."""

    @patch('urllib.request.urlopen')
    def test_check_update_new_version_available(self, mock_urlopen):
        """Test when a new version is available."""
        mock_response = MagicMock()
        mock_response.read.return_value = json.dumps({
            'tag_name': 'v99.0.0',
            'body': '## What\'s new\n- Feature 1\n- Feature 2'
        }).encode('utf-8')
        mock_response.__enter__ = MagicMock(return_value=mock_response)
        mock_response.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_response

        updater = Updater()
        has_update, latest, changelog = updater.check_update()

        assert has_update is True
        assert latest == '99.0.0'
        assert 'Feature 1' in changelog

    @patch('urllib.request.urlopen')
    def test_check_update_already_latest(self, mock_urlopen):
        """Test when already on latest version."""
        mock_response = MagicMock()
        # Return same version as current
        mock_response.read.return_value = json.dumps({
            'tag_name': f'v{Updater().current_version}',
            'body': ''
        }).encode('utf-8')
        mock_response.__enter__ = MagicMock(return_value=mock_response)
        mock_response.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_response

        updater = Updater()
        has_update, latest, changelog = updater.check_update()

        assert has_update is False
        assert latest == updater.current_version

    @patch('urllib.request.urlopen')
    def test_check_update_network_error(self, mock_urlopen):
        """Test handling of network errors."""
        import urllib.error
        mock_urlopen.side_effect = urllib.error.URLError('Network error')

        updater = Updater()
        with pytest.raises(UpdateCheckError) as exc_info:
            updater.check_update()

        assert 'Network error' in str(exc_info.value)

    @patch('urllib.request.urlopen')
    def test_check_update_rate_limit(self, mock_urlopen):
        """Test handling of GitHub rate limit."""
        import urllib.error
        error = urllib.error.HTTPError(
            url='', code=403, msg='Forbidden', hdrs={}, fp=None
        )
        mock_urlopen.side_effect = error

        updater = Updater()
        with pytest.raises(UpdateCheckError) as exc_info:
            updater.check_update()

        assert 'rate limit' in str(exc_info.value).lower()


class TestUpdateCLI:
    """Test CLI argument parsing for update command."""

    def test_is_update_command_true(self):
        """Test is_update_command returns True for update."""
        assert is_update_command(['update']) is True
        assert is_update_command(['update', '--check']) is True
        assert is_update_command(['update', '-y']) is True

    def test_is_update_command_false(self):
        """Test is_update_command returns False for non-update."""
        assert is_update_command(['file.txt']) is False
        assert is_update_command([]) is False
        assert is_update_command(['--version']) is False
        assert is_update_command(['-h']) is False

    def test_parse_update_arguments_check(self):
        """Test parsing --check flag."""
        args = parse_update_arguments(['update', '--check'])
        assert args.check is True
        assert args.yes is False

    def test_parse_update_arguments_yes(self):
        """Test parsing -y/--yes flag."""
        args = parse_update_arguments(['update', '-y'])
        assert args.yes is True

        args = parse_update_arguments(['update', '--yes'])
        assert args.yes is True

    def test_parse_update_arguments_combined(self):
        """Test parsing multiple flags."""
        args = parse_update_arguments(['update', '--check', '-y'])
        assert args.check is True
        assert args.yes is True


class TestBackwardCompatibility:
    """Test that existing functionality is not broken."""

    def test_file_path_not_detected_as_update(self):
        """Ensure file paths are not detected as update command."""
        assert is_update_command(['document.pdf']) is False
        assert is_update_command(['/path/to/file']) is False
        assert is_update_command(['./local/file.txt']) is False

    def test_original_parse_arguments_still_works(self):
        """Test that original parse_arguments function still works."""
        from src.cli import parse_arguments

        args = parse_arguments(['test.txt'])
        assert args.file_path == 'test.txt'
        assert args.max_downloads == 10
        assert args.timeout == '5m'

    def test_original_parse_arguments_with_options(self):
        """Test original parse_arguments with options."""
        from src.cli import parse_arguments

        args = parse_arguments(['file.txt', '-n', '5', '-t', '10m', '-p', '9000'])
        assert args.file_path == 'file.txt'
        assert args.max_downloads == 5
        assert args.timeout == '10m'
        assert args.port == 9000


class TestRunUpdate:
    """Test the run_update entry point."""

    @patch.object(Updater, 'check_update')
    def test_run_update_check_only(self, mock_check):
        """Test run_update with --check flag."""
        mock_check.return_value = (False, '1.0.12', '')

        exit_code = run_update(['update', '--check'])
        assert exit_code == 0

    @patch.object(Updater, 'check_update')
    def test_run_update_check_error(self, mock_check):
        """Test run_update when check fails."""
        mock_check.side_effect = UpdateCheckError('Network error')

        exit_code = run_update(['update', '--check'])
        assert exit_code == 1
