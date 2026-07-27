param(
    [string]$Binary = (Join-Path $PSScriptRoot "qs-resume-spike.exe")
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$name = -join @(
    [char]0x0051, [char]0x0053, [char]0x002D,
    [char]0x6062, [char]0x590D, [char]0x0020,
    [char]0x6D4B, [char]0x8BD5
)
$root = Join-Path $env:TEMP $name
& (Join-Path $PSScriptRoot "windows-kill-resume.ps1") -Binary $Binary -Root $root
