[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary,

    [string]$Report = (Join-Path $PWD "windows-desktop-true-host.json"),

    [string]$ExpectedSha256 = "91768fe2b4497339cd765ba916e247f19c20da6aa41fcae1d408eedcaf053bc3"
)

$ErrorActionPreference = "Stop"
$resolved = (Resolve-Path -LiteralPath $Binary).Path
$os = Get-CimInstance Win32_OperatingSystem
if ($os.ProductType -ne 1) {
    throw "The desktop probe must run on an interactive Windows client, not Windows Server."
}

$checks = [ordered]@{}
$checks.os = $os.Caption
$checks.version = $os.Version
$checks.architecture = $env:PROCESSOR_ARCHITECTURE
$checks.sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolved).Hash.ToLowerInvariant()
if ($checks.sha256 -ne $ExpectedSha256.ToLowerInvariant()) {
    throw "Unexpected probe SHA-256: $($checks.sha256)"
}
$checks.interactiveUser = [Environment]::UserInteractive
if (-not $checks.interactiveUser) {
    throw "No interactive Windows desktop is available."
}

$process = Start-Process -FilePath $resolved -PassThru
Start-Sleep -Seconds 2
$process.Refresh()
if ($process.HasExited) {
    throw "Desktop probe exited before interaction (exit code $($process.ExitCode))."
}

Write-Host ""
Write-Host "Complete the dialog/tray checks in the running probe." -ForegroundColor Cyan
Write-Host "Answer y only after observing the behavior on this Windows desktop."

$manual = [ordered]@{
    customButtons = "The first dialog shows 选择文件 / 选择文件夹 / 取消 with modern Windows styling"
    fileMultiSelect = "选择文件 opens the native picker and permits selecting two or more files"
    folderPicker = "选择文件夹 opens the native folder picker"
    initialDirectory = "Both native pickers start in the probe launch directory"
    parentAndAttention = "The dialog is visible above the background app or produces a clear attention signal"
    trayMenu = "A tray icon is visible and its 打开选择器 menu opens one picker workflow"
    gracefulExit = "The tray 退出 item closes the process without leaving the tray icon behind"
}

$allPassed = $true
foreach ($entry in $manual.GetEnumerator()) {
    $answer = Read-Host "$($entry.Value) [y/N]"
    $passed = $answer -match '^(?i:y|yes)$'
    $checks[$entry.Key] = $passed
    if (-not $passed) {
        $allPassed = $false
    }
}

Start-Sleep -Milliseconds 500
$process.Refresh()
if (-not $process.HasExited) {
    Write-Warning "Probe is still running; close it with the tray Exit item. Waiting 15 seconds."
    if (-not $process.WaitForExit(15000)) {
        $process.Kill($true)
        $process.WaitForExit()
        $checks.gracefulExit = $false
        $allPassed = $false
    }
}

$checks.exitCode = $process.ExitCode
$checks.passed = $allPassed -and $process.ExitCode -eq 0
$checks.timestampUtc = [DateTime]::UtcNow.ToString("o")
$checks | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Report -Encoding utf8NoBOM
Write-Host "True-host report: $Report"
if (-not $checks.passed) {
    exit 1
}
