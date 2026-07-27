param([Parameter(Mandatory=$true)][string]$Binary)
$ErrorActionPreference = "Stop"
$utf8 = [System.Text.UTF8Encoding]::new($false)
$global:OutputEncoding = $utf8
try { [Console]::OutputEncoding = $utf8 } catch { }
& $Binary --version
if ($LASTEXITCODE -ne 0) { throw "--version smoke failed" }
& $Binary --help *> $null
if ($LASTEXITCODE -ne 0) { throw "--help smoke failed" }

$root = Join-Path ([IO.Path]::GetTempPath()) ("quick-share-smoke-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $root | Out-Null
$receiver = $null
$originalLocalAppData = $env:LOCALAPPDATA
$originalAppData = $env:APPDATA
try {
    $receiverHome = Join-Path $root "receiver-home"
    $senderHome = Join-Path $root "sender-home"
    $output = Join-Path $root "output"
    New-Item -ItemType Directory -Path $receiverHome, $senderHome, $output | Out-Null
    $payload = Join-Path $root "payload.txt"
    Set-Content -LiteralPath $payload -Value "release-loopback-windows" -NoNewline
    $receiverLog = Join-Path $root "receiver.log"
    $receiverError = Join-Path $root "receiver.err"

    $env:LOCALAPPDATA = $receiverHome
    $env:APPDATA = $receiverHome
    $receiver = Start-Process -FilePath $Binary -ArgumentList @(
        "receive", "--bind", "127.0.0.1", "--port", "49327", "--once", "--yes", "--output", $output
    ) -RedirectStandardOutput $receiverLog -RedirectStandardError $receiverError -PassThru
    Start-Sleep -Seconds 2

    $env:LOCALAPPDATA = $senderHome
    $env:APPDATA = $senderHome
    & $Binary send --peer "127.0.0.1:49327" --yes $payload
    if ($LASTEXITCODE -ne 0) { throw "loopback sender failed" }
    if (-not $receiver.WaitForExit(15000)) {
        $receiver.Kill()
        throw "loopback receiver did not stop"
    }
    if ($receiver.ExitCode -ne 0) {
        Get-Content -LiteralPath $receiverError -ErrorAction SilentlyContinue | Write-Error
        throw "loopback receiver failed"
    }
    $received = Join-Path $output "payload.txt"
    if (-not (Test-Path -LiteralPath $received)) { throw "loopback payload is missing" }
    if ((Get-Content -LiteralPath $received -Raw) -cne "release-loopback-windows") {
        throw "loopback payload mismatch"
    }
}
finally {
    $env:LOCALAPPDATA = $originalLocalAppData
    $env:APPDATA = $originalAppData
    if ($null -ne $receiver -and -not $receiver.HasExited) { $receiver.Kill() }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
