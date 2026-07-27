param(
    [string]$Binary = (Join-Path $PSScriptRoot "..\target\x86_64-pc-windows-gnu\release\qs-clipboard-spike.exe")
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$text = -join @(
    [char]0x5FEB, [char]0x901F, [char]0x5206, [char]0x4EAB,
    [char]0x002D, [char]0x6D4B, [char]0x8BD5,
    [char]0x002D, [char]0xD83D, [char]0xDE80
)
& $Binary --mode roundtrip --text $text
if ($LASTEXITCODE -ne 0) {
    throw "Clipboard probe failed with exit code $LASTEXITCODE"
}
