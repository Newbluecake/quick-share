"""Network utilities for Quick Share.

Provides LAN IP detection and validation for multi-interface environments.
"""

import socket
import ipaddress


def is_valid_lan_ip(ip: str) -> bool:
    """Check if IP is a valid LAN address.

    Args:
        ip: IP address string to validate

    Returns:
        True if IP is a valid private LAN address, False otherwise

    Rejects:
        - Loopback addresses (127.0.0.0/8)
        - Link-local addresses (169.254.0.0/16)
        - Public IP addresses
        - Invalid IP formats
    """
    try:
        ip_obj = ipaddress.ip_address(ip)

        # Reject loopback addresses
        if ip_obj.is_loopback:
            return False

        # Reject link-local addresses (169.254.x.x)
        if ip_obj.is_link_local:
            return False

        # Accept only private addresses
        if ip_obj.is_private:
            return True

        return False

    except ValueError:
        # Invalid IP format
        return False


def get_local_ip() -> str:
    """Get the local LAN IP address.

    Uses the socket trick of connecting to a public DNS server (doesn't actually
    send packets) to determine which local IP would be used for internet traffic.
    This works across platforms and handles multi-interface environments.

    Returns:
        Local LAN IP address as string

    Raises:
        RuntimeError: If no valid LAN IP address could be determined
    """
    try:
        # Create a UDP socket (doesn't send any packets)
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            # Connect to a public DNS server (doesn't send packets, just determines routing)
            # Using Google's DNS at 8.8.8.8:80
            s.connect(('8.8.8.8', 80))

            # Get the local IP that would be used for this connection
            local_ip = s.getsockname()[0]

            if not is_valid_lan_ip(local_ip):
                raise RuntimeError(f"Found IP {local_ip} is not a valid LAN address")

            return local_ip
        finally:
            s.close()

    except Exception as e:
        raise RuntimeError(f"Could not determine local IP address: {e}")
