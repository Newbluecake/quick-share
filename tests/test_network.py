import unittest
from unittest.mock import patch, MagicMock
import socket
import sys
import os

# Ensure src is in path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..')))

from src.network import is_valid_lan_ip, get_local_ip

class TestNetwork(unittest.TestCase):
    def test_is_valid_lan_ip_private_ranges(self):
        """Test valid private LAN IP ranges (A, B, C classes)"""
        # Class A: 10.0.0.0 - 10.255.255.255
        self.assertTrue(is_valid_lan_ip('10.0.0.1'))
        self.assertTrue(is_valid_lan_ip('10.255.255.255'))

        # Class B: 172.16.0.0 - 172.31.255.255
        self.assertTrue(is_valid_lan_ip('172.16.0.1'))
        self.assertTrue(is_valid_lan_ip('172.31.255.255'))

        # Class C: 192.168.0.0 - 192.168.255.255
        self.assertTrue(is_valid_lan_ip('192.168.1.100'))
        self.assertTrue(is_valid_lan_ip('192.168.0.1'))

    def test_is_valid_lan_ip_invalid(self):
        """Test invalid IPs (Loopback, Link-local, Public)"""
        self.assertFalse(is_valid_lan_ip('127.0.0.1'))       # Loopback
        self.assertFalse(is_valid_lan_ip('169.254.1.1'))     # Link-local
        self.assertFalse(is_valid_lan_ip('8.8.8.8'))         # Public IP
        self.assertFalse(is_valid_lan_ip('invalid'))         # Not an IP
        self.assertFalse(is_valid_lan_ip('256.0.0.1'))       # Invalid octet

    @patch('socket.socket')
    def test_get_local_ip_success(self, mock_socket_cls):
        """Test successful local IP detection"""
        # Setup mock
        mock_socket = MagicMock()
        mock_socket_cls.return_value = mock_socket
        # Mock getsockname to return a valid local IP
        mock_socket.getsockname.return_value = ('192.168.1.105', 12345)

        # Execute
        ip = get_local_ip()

        # Verify
        self.assertEqual(ip, '192.168.1.105')
        # Verify socket calls
        mock_socket_cls.assert_called_with(socket.AF_INET, socket.SOCK_DGRAM)
        # We expect it to try to connect to a public DNS (doesn't send packets)
        mock_socket.connect.assert_called()
        mock_socket.close.assert_called()

    @patch('socket.socket')
    def test_get_local_ip_invalid_result(self, mock_socket_cls):
        """Test that get_local_ip rejects invalid IPs (e.g. loopback) even if socket returns them"""
        # Setup mock
        mock_socket = MagicMock()
        mock_socket_cls.return_value = mock_socket
        # Mock getsockname to return loopback
        mock_socket.getsockname.return_value = ('127.0.0.1', 12345)

        # Execute & Verify
        with self.assertRaises(RuntimeError) as cm:
            get_local_ip()

        self.assertIn("Found IP 127.0.0.1 is not a valid LAN address", str(cm.exception))

    @patch('socket.socket')
    def test_get_local_ip_failure(self, mock_socket_cls):
        """Test failure to detect local IP"""
        # Setup mock to raise exception on connect
        mock_socket = MagicMock()
        mock_socket_cls.return_value = mock_socket
        mock_socket.connect.side_effect = Exception("Network unreachable")

        # Execute & Verify
        with self.assertRaises(RuntimeError) as cm:
            get_local_ip()

        self.assertIn("Could not determine local IP", str(cm.exception))

if __name__ == '__main__':
    unittest.main()
