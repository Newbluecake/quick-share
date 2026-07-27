$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

$profiles = Get-NetConnectionProfile | ForEach-Object {
    [PSCustomObject]@{
        interface = $_.InterfaceAlias
        category = $_.NetworkCategory.ToString()
        ipv4_connectivity = $_.IPv4Connectivity.ToString()
        ipv6_connectivity = $_.IPv6Connectivity.ToString()
    }
}
$firewall = Get-NetFirewallProfile | ForEach-Object {
    [PSCustomObject]@{
        name = $_.Name.ToString()
        enabled = $_.Enabled
        default_inbound = $_.DefaultInboundAction.ToString()
        default_outbound = $_.DefaultOutboundAction.ToString()
        notify_on_listen = $_.NotifyOnListen
    }
}
$quickShareRules = Get-NetFirewallRule -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -like "*Quick Share*" } |
    Select-Object DisplayName, Enabled, Direction, Action, Profile

[PSCustomObject]@{
    os = (Get-CimInstance Win32_OperatingSystem).Caption
    profiles = @($profiles)
    firewall = @($firewall)
    existing_quick_share_rules = @($quickShareRules)
} | ConvertTo-Json -Depth 5 -Compress
