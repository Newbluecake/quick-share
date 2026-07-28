[CmdletBinding()]
param(
    [string]$Root = "$env:LOCALAPPDATA\Temp\qs-windows-desktop-spike",
    [string]$ExpectedArchiveSha256 = "f41511db45c862b7b7b2f1dd934c22555b8405c6b611574763df9a4ebeca57cf",
    [string]$ExpectedBinarySha256 = "91768fe2b4497339cd765ba916e247f19c20da6aa41fcae1d408eedcaf053bc3"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.Encoding]::GetEncoding(936)

$archive = Join-Path $Root "windows-desktop-true-host.zip"
$expanded = Join-Path $Root "expanded"
$binary = Join-Path $expanded "qs-windows-desktop-spike.exe"

$archiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
if ($archiveHash -ne $ExpectedArchiveSha256) {
    throw "archive SHA-256 mismatch: $archiveHash"
}

if (Test-Path -LiteralPath $expanded) {
    Remove-Item -LiteralPath $expanded -Recurse -Force
}
Expand-Archive -LiteralPath $archive -DestinationPath $expanded
$binaryHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash.ToLowerInvariant()
if ($binaryHash -ne $ExpectedBinarySha256) {
    throw "binary SHA-256 mismatch: $binaryHash"
}

$os = Get-CimInstance Win32_OperatingSystem
$activeConsoleSession = (quser.exe 2>&1 | Out-String).Trim()
$stdoutPath = Join-Path $expanded "probe.stdout.log"
$stderrPath = Join-Path $expanded "probe.stderr.log"
$process = Start-Process `
    -FilePath $binary `
    -WorkingDirectory $expanded `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath `
    -PassThru
Start-Sleep -Seconds 4
$process.Refresh()
$exitedEarly = $process.HasExited
if ($exitedEarly) {
    $process.WaitForExit()
    $process.Refresh()
}
$stdout = if (Test-Path -LiteralPath $stdoutPath) {
    [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($stdoutPath))
} else { "" }
$stderr = if (Test-Path -LiteralPath $stderrPath) {
    [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($stderrPath))
} else { "" }
$userInteractive = [Environment]::UserInteractive
$expectedNoUiRefusal = (
    -not $userInteractive -and
    $exitedEarly -and
    $stderr -like "*desktop initialization failed*"
)

$result = [ordered]@{
    timestampUtc = [DateTime]::UtcNow.ToString("o")
    osCaption = $os.Caption
    osVersion = $os.Version
    productType = $os.ProductType
    archiveSha256 = $archiveHash
    binarySha256 = $binaryHash
    sshUserInteractive = $userInteractive
    activeConsoleSession = $activeConsoleSession
    processId = $process.Id
    processSessionId = if ($exitedEarly) { $null } else { $process.SessionId }
    processExitedEarly = $exitedEarly
    processExitCode = if ($exitedEarly) { $process.ExitCode } else { $null }
    processResponding = if ($exitedEarly) { $false } else { $process.Responding }
    mainWindowTitle = if ($exitedEarly) { "" } else { $process.MainWindowTitle }
    stdout = $stdout
    stderr = $stderr
    expectedNoUiRefusal = $expectedNoUiRefusal
}

if (-not $exitedEarly) {
    Stop-Process -Id $process.Id -Force
    $process.WaitForExit()
}
Start-Sleep -Milliseconds 300
$result.cleanedUp = -not [bool](Get-Process -Id $process.Id -ErrorAction SilentlyContinue)
$result.basicPassed = (
    $result.productType -eq 1 -and
    $result.expectedNoUiRefusal -and
    $result.cleanedUp
)

$result | ConvertTo-Json -Depth 4 -Compress
if (-not $result.basicPassed) {
    exit 1
}
