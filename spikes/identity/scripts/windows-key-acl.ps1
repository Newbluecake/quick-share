param(
    [string]$Path = (Join-Path $PSScriptRoot "windows-identity-key.json")
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$acl = Get-Acl $Path
$access = $acl.Access | ForEach-Object {
    [PSCustomObject]@{
        identity = $_.IdentityReference.Value
        rights = $_.FileSystemRights.ToString()
        inherited = $_.IsInherited
        type = $_.AccessControlType.ToString()
    }
}
[PSCustomObject]@{
    path = $Path
    owner = $acl.Owner
    protected = $acl.AreAccessRulesProtected
    access = @($access)
} | ConvertTo-Json -Depth 5 -Compress
