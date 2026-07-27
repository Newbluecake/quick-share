$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$root = Join-Path $env:TEMP "qs-symlink-probe"
if (Test-Path $root) { Remove-Item -Recurse -Force $root }
New-Item -ItemType Directory -Path $root | Out-Null
$target = Join-Path $root "target.txt"
$link = Join-Path $root "link.txt"
Set-Content -Encoding UTF8 -NoNewline -Path $target -Value "quick-share-symlink"

$result = [ordered]@{
    os = "windows"
    elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    symlink_created = $false
    link_type = $null
    content_matches = $false
    error = $null
}
try {
    $item = New-Item -ItemType SymbolicLink -Path $link -Target $target -ErrorAction Stop
    $result.symlink_created = $true
    $result.link_type = $item.LinkType
    $result.content_matches = ((Get-Content -Raw -Encoding UTF8 $link) -eq "quick-share-symlink")
}
catch {
    $result.error = $_.Exception.Message
}
finally {
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
[PSCustomObject]$result | ConvertTo-Json -Compress
