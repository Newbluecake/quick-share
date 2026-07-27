param(
    [string]$Binary = (Join-Path $PSScriptRoot "..\..\..\target\x86_64-pc-windows-gnu\release\qs-resume-spike.exe"),
    [string]$Root = (Join-Path $env:TEMP "qs-resume-kill-spike")
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

if (Test-Path $Root) {
    Remove-Item -Recurse -Force $Root
}
New-Item -ItemType Directory -Force $Root | Out-Null

$firstOut = Join-Path $Root "first.stdout.log"
$firstErr = Join-Path $Root "first.stderr.log"
$secondOut = Join-Path $Root "second.stdout.log"
$secondErr = Join-Path $Root "second.stderr.log"
$statePath = Join-Path $Root "state.json"
$finalPath = Join-Path $Root "final.bin"

$quotedRoot = '"' + $Root.Replace('"', '\"') + '"'
$arguments = @(
    "worker", "--root", $quotedRoot,
    "--size-mib", "256",
    "--chunk-mib", "4",
    "--delay-ms", "100"
)
$process = Start-Process -FilePath $Binary -ArgumentList $arguments `
    -RedirectStandardOutput $firstOut -RedirectStandardError $firstErr -PassThru

$completed = 0
for ($attempt = 0; $attempt -lt 200; $attempt++) {
    Start-Sleep -Milliseconds 50
    if (Test-Path $statePath) {
        try {
            $state = Get-Content -Raw -Encoding UTF8 $statePath | ConvertFrom-Json
            $completed = @($state.completed | Where-Object { $null -ne $_ }).Count
            if ($completed -ge 5) { break }
        }
        catch {
            # The process may be between temporary journal write and rename.
        }
    }
}

if ($completed -lt 5) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    throw "Worker did not commit five chunks in time"
}
Stop-Process -Id $process.Id -Force
$process.WaitForExit()

if (Test-Path $finalPath) {
    throw "Final file became visible before completion"
}

$state = Get-Content -Raw -Encoding UTF8 $statePath | ConvertFrom-Json
$recordedBeforeRestart = @($state.completed | Where-Object { $null -ne $_ }).Count

$arguments[8] = "0" # --delay-ms
$second = Start-Process -FilePath $Binary -ArgumentList $arguments `
    -RedirectStandardOutput $secondOut -RedirectStandardError $secondErr -PassThru -Wait
if ($second.ExitCode -ne 0) {
    throw "Resume process failed with exit code $($second.ExitCode): $(Get-Content -Raw $secondErr)"
}
if (-not (Test-Path $finalPath)) {
    throw "Final file missing after resume"
}
$finalLength = (Get-Item $finalPath).Length
if ($finalLength -ne 268435456) {
    throw "Final length $finalLength did not match 256 MiB"
}

[PSCustomObject]@{
    os = "windows"
    killed_process_id = $process.Id
    recorded_chunks_before_restart = $recordedBeforeRestart
    final_length = $finalLength
    resume_exit_code = $second.ExitCode
    result = "PASS"
} | ConvertTo-Json -Compress

Remove-Item -Recurse -Force $Root
