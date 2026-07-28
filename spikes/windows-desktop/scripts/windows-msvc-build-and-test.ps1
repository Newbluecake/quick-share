[CmdletBinding()]
param([string]$Report = (Join-Path $PWD "windows-desktop-true-host.json"))

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $hostLine = (& rustc -Vv | Select-String '^host:').Line
    if ($hostLine -notmatch 'pc-windows-msvc') {
        throw "The native gate requires an MSVC Rust host; found $hostLine"
    }

    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed" }

    cargo clippy --locked --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed" }

    cargo build --locked --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $binary = Join-Path $root "target\release\qs-windows-desktop-spike.exe"
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash.ToLowerInvariant()
    & (Join-Path $PSScriptRoot "windows-true-host.ps1") `
        -Binary $binary `
        -Report $Report `
        -ExpectedSha256 $hash
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
