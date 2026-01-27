"""Network utilities for Quick Share.

Provides LAN IP detection and validation for multi-interface environments.
"""

import socket
import ipaddress
import re
from typing import List, Tuple

# Interface name patterns to filter out (virtual/container networks)
VIRTUAL_INTERFACE_PATTERNS = [
    r'^docker\d*$',      # Docker default bridge
    r'^br-',             # Docker custom bridges
    r'^veth',            # Container virtual ethernet
    r'^virbr',           # libvirt virtual bridge
    r'^vbox',            # VirtualBox
    r'^vmnet',           # VMware
    r'^vEthernet',       # Hyper-V (Windows)
    r'^wsl',             # WSL network
    r'^lo$',             # Loopback
    r'^dummy',           # Dummy interfaces
    r'^tap',             # TAP devices
    r'^tun',             # TUN devices
]


def is_virtual_interface(interface_name: str) -> bool:
    """Check if an interface is a virtual/container network interface.

    Args:
        interface_name: Network interface name (e.g., 'eth0', 'docker0')

    Returns:
        True if the interface is a virtual network interface, False otherwise
    """
    for pattern in VIRTUAL_INTERFACE_PATTERNS:
        if re.match(pattern, interface_name, re.IGNORECASE):
            return True
    return False


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


def get_all_lan_ips() -> List[Tuple[str, str]]:
    """Get all valid LAN IP addresses with their interface names.

    Scans all network interfaces and returns IPs that are:
    - Valid private LAN addresses
    - Not on virtual/container network interfaces (Docker, VM, WSL, etc.)

    Returns:
        List of tuples (interface_name, ip_address), sorted with primary IP first

    Example:
        [('eth0', '192.168.1.100'), ('wlan0', '192.168.1.101')]
    """
    import platform

    lan_ips = []
    primary_ip = None

    # Get the primary IP (the one used for internet routing)
    try:
        primary_ip = get_local_ip()
    except RuntimeError:
        pass

    system = platform.system()

    if system == 'Linux' or system == 'Darwin':
        lan_ips = _get_lan_ips_unix()
    elif system == 'Windows':
        lan_ips = _get_lan_ips_windows()
    else:
        # Fallback: just return primary IP if available
        if primary_ip:
            lan_ips = [('default', primary_ip)]

    # Sort: primary IP first, then alphabetically by interface name
    def sort_key(item):
        iface, ip = item
        if ip == primary_ip:
            return (0, iface)  # Primary IP comes first
        return (1, iface)

    lan_ips.sort(key=sort_key)

    # If no IPs found but we have primary, use it
    if not lan_ips and primary_ip:
        lan_ips = [('default', primary_ip)]

    return lan_ips


def _get_lan_ips_unix() -> List[Tuple[str, str]]:
    """Get LAN IPs on Unix-like systems (Linux/macOS)."""
    import subprocess

    lan_ips = []

    try:
        # Use 'ip addr' on Linux, 'ifconfig' on macOS
        import platform
        if platform.system() == 'Linux':
            result = subprocess.run(
                ['ip', '-4', 'addr', 'show'],
                capture_output=True, text=True, timeout=5
            )
            if result.returncode == 0:
                lan_ips = _parse_ip_addr_output(result.stdout)
        else:
            # macOS
            result = subprocess.run(
                ['ifconfig'],
                capture_output=True, text=True, timeout=5
            )
            if result.returncode == 0:
                lan_ips = _parse_ifconfig_output(result.stdout)
    except (subprocess.TimeoutExpired, FileNotFoundError, Exception):
        pass

    return lan_ips


def _get_lan_ips_windows() -> List[Tuple[str, str]]:
    """Get LAN IPs on Windows."""
    import subprocess

    lan_ips = []

    try:
        result = subprocess.run(
            ['ipconfig'],
            capture_output=True, text=True, timeout=5
        )
        if result.returncode == 0:
            lan_ips = _parse_ipconfig_output(result.stdout)
    except (subprocess.TimeoutExpired, FileNotFoundError, Exception):
        pass

    return lan_ips


def _parse_ip_addr_output(output: str) -> List[Tuple[str, str]]:
    """Parse 'ip addr' output on Linux."""
    lan_ips = []
    current_iface = None

    for line in output.split('\n'):
        # Match interface line: "2: eth0: <BROADCAST..."
        iface_match = re.match(r'^\d+:\s+(\S+):', line)
        if iface_match:
            current_iface = iface_match.group(1).rstrip(':')
            # Remove @xxx suffix (e.g., eth0@if123)
            if '@' in current_iface:
                current_iface = current_iface.split('@')[0]
            continue

        # Match inet line: "    inet 192.168.1.100/24 ..."
        inet_match = re.match(r'^\s+inet\s+(\d+\.\d+\.\d+\.\d+)', line)
        if inet_match and current_iface:
            ip = inet_match.group(1)
            if is_valid_lan_ip(ip) and not is_virtual_interface(current_iface):
                lan_ips.append((current_iface, ip))

    return lan_ips


def _parse_ifconfig_output(output: str) -> List[Tuple[str, str]]:
    """Parse 'ifconfig' output on macOS."""
    lan_ips = []
    current_iface = None

    for line in output.split('\n'):
        # Match interface line: "en0: flags=..."
        iface_match = re.match(r'^(\S+):\s+flags=', line)
        if iface_match:
            current_iface = iface_match.group(1)
            continue

        # Match inet line: "\tinet 192.168.1.100 netmask..."
        inet_match = re.match(r'^\s+inet\s+(\d+\.\d+\.\d+\.\d+)', line)
        if inet_match and current_iface:
            ip = inet_match.group(1)
            if is_valid_lan_ip(ip) and not is_virtual_interface(current_iface):
                lan_ips.append((current_iface, ip))

    return lan_ips


def _parse_ipconfig_output(output: str) -> List[Tuple[str, str]]:
    """Parse 'ipconfig' output on Windows."""
    lan_ips = []
    current_iface = None

    for line in output.split('\n'):
        line = line.strip()

        # Match adapter line: "Ethernet adapter Local Area Connection:"
        # or "Wireless LAN adapter Wi-Fi:"
        adapter_match = re.match(r'^(.+adapter\s+.+):$', line, re.IGNORECASE)
        if adapter_match:
            current_iface = adapter_match.group(1)
            # Simplify interface name
            current_iface = re.sub(r'.*adapter\s+', '', current_iface, flags=re.IGNORECASE)
            continue

        # Match IPv4 line: "IPv4 Address. . . . . . . . . . . : 192.168.1.100"
        ipv4_match = re.match(r'^IPv4.*:\s*(\d+\.\d+\.\d+\.\d+)', line, re.IGNORECASE)
        if ipv4_match and current_iface:
            ip = ipv4_match.group(1)
            if is_valid_lan_ip(ip) and not is_virtual_interface(current_iface):
                lan_ips.append((current_iface, ip))

    return lan_ips
