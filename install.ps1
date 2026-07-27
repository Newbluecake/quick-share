# Quick Share Rust single-binary installer. PowerShell 5.1+ compatible.
[CmdletBinding()]
param(
    [string]$Version = "latest",
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "QuickShare\bin"),
    [switch]$NoAliases,
    [switch]$AddPrivateFirewallRule,
    [switch]$Uninstall,
    [switch]$Help
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$script:Utf8 = [System.Text.UTF8Encoding]::new($false)
$global:OutputEncoding = $script:Utf8
try {
    [Console]::InputEncoding = $script:Utf8
    [Console]::OutputEncoding = $script:Utf8
}
catch {
    # Redirected/non-console hosts still use $OutputEncoding above.
}
$script:Repository = "Newbluecake/quick-share"
$script:ReleaseRoot = "https://github.com/$script:Repository/releases"
$script:FirewallRuleName = "Quick Share (Private inbound)"
$script:ReleasePublicKeyDerBase64 = "MCowBQYDK2VwAyEAcXGzr1dl2fQcyFJBD044/DrWlgc5rYjQTozO3yzr8Q0="

function Write-Info([string]$Message) { Write-Host "[INFO] $Message" }
function Write-Warn([string]$Message) { Write-Host "[WARN] $Message" -ForegroundColor Yellow }

function Get-ReleaseTarget {
    param([string]$Architecture = $env:PROCESSOR_ARCHITECTURE)
    switch ($Architecture.ToUpperInvariant()) {
        "AMD64" { return "x86_64-pc-windows-msvc" }
        "X86_64" { return "x86_64-pc-windows-msvc" }
        default { throw "No Quick Share Windows release is published for $Architecture" }
    }
}

function Get-AssetName {
    param([Parameter(Mandatory=$true)][string]$Target)
    return "quick-share-$Target.exe"
}

function Get-ReleaseDownloadRoot {
    param([Parameter(Mandatory=$true)][string]$RequestedVersion)
    if ($RequestedVersion -eq "latest") {
        return "$script:ReleaseRoot/latest/download"
    }
    if ($RequestedVersion -notmatch '^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$') {
        throw "Invalid release version: $RequestedVersion"
    }
    return "$script:ReleaseRoot/download/v$($RequestedVersion.TrimStart('v'))"
}

function Get-EffectiveUri {
    param($Response, [string]$Fallback)
    if ($null -eq $Response -or $null -eq $Response.BaseResponse) { return $Fallback }
    $responseUri = $Response.BaseResponse.PSObject.Properties["ResponseUri"]
    if ($null -ne $responseUri -and $null -ne $responseUri.Value) {
        return $responseUri.Value.AbsoluteUri
    }
    $requestMessage = $Response.BaseResponse.PSObject.Properties["RequestMessage"]
    if ($null -ne $requestMessage -and $null -ne $requestMessage.Value -and
        $null -ne $requestMessage.Value.RequestUri) {
        return $requestMessage.Value.RequestUri.AbsoluteUri
    }
    return $Fallback
}

function Receive-FixedReleaseFile {
    param(
        [Parameter(Mandatory=$true)][string]$Uri,
        [Parameter(Mandatory=$true)][string]$OutFile
    )
    if (-not $Uri.StartsWith("https://github.com/Newbluecake/quick-share/releases/", [StringComparison]::Ordinal)) {
        throw "Refusing download outside the fixed Quick Share GitHub repository"
    }
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $response = Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing -PassThru
    $effective = Get-EffectiveUri -Response $response -Fallback $Uri
    if (-not ($effective.StartsWith("https://github.com/", [StringComparison]::Ordinal) -or
              $effective.StartsWith("https://release-assets.githubusercontent.com/", [StringComparison]::Ordinal))) {
        Remove-Item -LiteralPath $OutFile -Force -ErrorAction SilentlyContinue
        throw "Release redirect left the allowed GitHub asset origins"
    }
}

function Get-ExpectedChecksum {
    param(
        [Parameter(Mandatory=$true)][string]$Manifest,
        [Parameter(Mandatory=$true)][string]$AssetName
    )
    $foundChecksums = @()
    foreach ($line in Get-Content -LiteralPath $Manifest) {
        if ($line -match '^([0-9A-Fa-f]{64})\s+\*?([^/\\]+)$' -and $Matches[2] -ceq $AssetName) {
            $foundChecksums += $Matches[1].ToLowerInvariant()
        }
    }
    if ($foundChecksums.Count -ne 1) {
        throw "Checksum manifest must contain exactly one entry for $AssetName"
    }
    return $foundChecksums[0]
}

function Test-ReleaseChecksum {
    param(
        [Parameter(Mandatory=$true)][string]$Candidate,
        [Parameter(Mandatory=$true)][string]$Manifest,
        [Parameter(Mandatory=$true)][string]$AssetName
    )
    $expected = Get-ExpectedChecksum -Manifest $Manifest -AssetName $AssetName
    $actual = (Get-FileHash -LiteralPath $Candidate -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $expected) { throw "SHA-256 verification failed for $AssetName" }
}

function Test-ReleaseSignatureIfAvailable {
    param(
        [Parameter(Mandatory=$true)][string]$Manifest,
        [Parameter(Mandatory=$true)][string]$Signature,
        [Parameter(Mandatory=$true)][string]$Candidate,
        [Parameter(Mandatory=$true)][string]$TemporaryDirectory
    )
    $openssl = Get-Command openssl -ErrorAction SilentlyContinue
    if ($null -ne $openssl) {
        $algorithms = & $openssl.Source list -public-key-algorithms 2>$null
        if ($algorithms -match 'ED25519') {
            $publicKey = Join-Path $TemporaryDirectory "release-signing-key.der"
            [IO.File]::WriteAllBytes($publicKey, [Convert]::FromBase64String($script:ReleasePublicKeyDerBase64))
            & $openssl.Source pkeyutl -verify -rawin -pubin -keyform DER `
                -inkey $publicKey -in $Manifest -sigfile $Signature *> $null
            if ($LASTEXITCODE -ne 0) { throw "Ed25519 release signature verification failed" }
            Write-Info "Verified pinned Ed25519 release signature"
            return
        }
    }
    $gh = Get-Command gh -ErrorAction SilentlyContinue
    if ($null -ne $gh) {
        & $gh.Source auth status *> $null
        if ($LASTEXITCODE -eq 0) {
            & $gh.Source attestation verify $Candidate --repo $script:Repository `
                --signer-workflow "$script:Repository/.github/workflows/release.yml" *> $null
            if ($LASTEXITCODE -ne 0) { throw "GitHub build provenance verification failed" }
            Write-Info "Verified GitHub build provenance"
            return
        }
    }
    Write-Warn "Ed25519/GitHub attestation verification is unavailable; SHA-256 was still verified"
}

function Install-VerifiedBinary {
    param(
        [Parameter(Mandatory=$true)][string]$Candidate,
        [Parameter(Mandatory=$true)][string]$DestinationDirectory
    )
    New-Item -ItemType Directory -Path $DestinationDirectory -Force | Out-Null
    $current = Join-Path $DestinationDirectory "quick-share.exe"
    $nonce = [Guid]::NewGuid().ToString("N")
    $staged = Join-Path $DestinationDirectory ".quick-share.new.$nonce.exe"
    $backup = Join-Path $DestinationDirectory ".quick-share.old.$nonce.exe"
    Copy-Item -LiteralPath $Candidate -Destination $staged
    try {
        & $staged --version *> $null
        if ($LASTEXITCODE -ne 0) { throw "non-zero version status" }
    }
    catch {
        Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue
        throw "Downloaded executable failed its startup check"
    }
    $hadCurrent = Test-Path -LiteralPath $current
    if ($hadCurrent) { Move-Item -LiteralPath $current -Destination $backup }
    try {
        Move-Item -LiteralPath $staged -Destination $current
        & $current --version *> $null
        if ($LASTEXITCODE -ne 0) { throw "New executable failed after installation" }
        Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    }
    catch {
        Remove-Item -LiteralPath $current -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $backup) {
            Move-Item -LiteralPath $backup -Destination $current
        }
        Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue
        throw "Installation failed; previous executable was restored: $_"
    }
}

function Test-CommandConflict {
    param([string]$Name, [string]$Destination)
    if (Test-Path -LiteralPath $Destination) { return $true }
    $existing = Get-Command $Name -ErrorAction SilentlyContinue
    return $null -ne $existing -and $existing.Source -cne $Destination
}

function Add-QuickShareAliases {
    param([Parameter(Mandatory=$true)][string]$DestinationDirectory)
    $source = Join-Path $DestinationDirectory "quick-share.exe"
    foreach ($name in @("sc", "rc")) {
        $destination = Join-Path $DestinationDirectory "$name.exe"
        if (Test-CommandConflict -Name $name -Destination $destination) {
            Write-Warn "Not overwriting existing command or path: $name"
            continue
        }
        if (Test-Path -LiteralPath $destination) {
            Write-Warn "Not overwriting existing shortcut: $destination"
            continue
        }
        Copy-Item -LiteralPath $source -Destination $destination
        New-Item -ItemType File -Path (Join-Path $DestinationDirectory ".quick-share-managed-$name") -Force | Out-Null
        Write-Info "Created shortcut: $destination"
    }
}

function Add-ToUserPath {
    param([Parameter(Mandatory=$true)][string]$Directory)
    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @($current -split ';' | Where-Object { $_ })
    if ($entries.TrimEnd('\') -contains $Directory.TrimEnd('\')) { return }
    $newValue = (@($entries) + $Directory) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $newValue, "User")
    Write-Info "Added installation directory to the user PATH"
}

function Add-QuickSharePrivateFirewallRule {
    param([Parameter(Mandatory=$true)][string]$Program)
    if ($null -ne (Get-NetFirewallRule -DisplayName $script:FirewallRuleName -ErrorAction SilentlyContinue)) {
        throw "A firewall rule named '$script:FirewallRuleName' already exists; refusing to alter it"
    }
    New-NetFirewallRule -DisplayName $script:FirewallRuleName -Direction Inbound -Action Allow `
        -Profile Private -Program $Program -Protocol TCP | Out-Null
    Write-Info "Added an explicit program-scoped Private-profile TCP firewall rule"
}

function Remove-QuickSharePrivateFirewallRule {
    param([Parameter(Mandatory=$true)][string]$ExpectedProgram)
    $rule = Get-NetFirewallRule -DisplayName $script:FirewallRuleName -ErrorAction SilentlyContinue
    if ($null -eq $rule) { return }
    $application = $rule | Get-NetFirewallApplicationFilter
    if ($application.Program -cne $ExpectedProgram) {
        throw "Refusing to remove a same-named firewall rule for another program"
    }
    $rule | Remove-NetFirewallRule
}

function Remove-FromUserPath {
    param([Parameter(Mandatory=$true)][string]$Directory)
    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @($current -split ';' | Where-Object {
        $_ -and $_.TrimEnd('\') -cne $Directory.TrimEnd('\')
    })
    [Environment]::SetEnvironmentVariable("Path", ($entries -join ';'), "User")
}

function Uninstall-QuickShare {
    param([Parameter(Mandatory=$true)][string]$DestinationDirectory)
    $program = Join-Path $DestinationDirectory "quick-share.exe"
    Remove-QuickSharePrivateFirewallRule -ExpectedProgram $program
    Remove-Item -LiteralPath (Join-Path $DestinationDirectory "quick-share.exe") -Force -ErrorAction SilentlyContinue
    foreach ($name in @("sc", "rc")) {
        $marker = Join-Path $DestinationDirectory ".quick-share-managed-$name"
        if (Test-Path -LiteralPath $marker) {
            Remove-Item -LiteralPath (Join-Path $DestinationDirectory "$name.exe") -Force -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
        }
    }
    Remove-FromUserPath -Directory $DestinationDirectory
    Write-Info "Uninstalled Quick Share files and its own firewall rule"
}

function Show-Usage {
    @"
Usage: install.ps1 [-Version VERSION] [-InstallDir PATH] [-NoAliases]
                   [-AddPrivateFirewallRule] [-Uninstall]

Downloads one Rust executable, verifies SHA-256 and a pinned Ed25519 signature
(or GitHub provenance when available). Existing sc/rc commands are not overwritten.
The firewall is changed only with -AddPrivateFirewallRule and only for the
installed program on the Private profile.
"@ | Write-Host
}

function Install-QuickShare {
    if ($Help) { Show-Usage; return }
    $fullInstallDir = [IO.Path]::GetFullPath($InstallDir)
    if ($Uninstall) { Uninstall-QuickShare -DestinationDirectory $fullInstallDir; return }
    $target = Get-ReleaseTarget
    $asset = Get-AssetName -Target $target
    $root = Get-ReleaseDownloadRoot -RequestedVersion $Version
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ("quick-share-install-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $temporary | Out-Null
    try {
        $candidate = Join-Path $temporary $asset
        $checksums = Join-Path $temporary "SHA256SUMS"
        $signature = Join-Path $temporary "SHA256SUMS.sig"
        Write-Info "Downloading $asset"
        Receive-FixedReleaseFile -Uri "$root/$asset" -OutFile $candidate
        Receive-FixedReleaseFile -Uri "$root/SHA256SUMS" -OutFile $checksums
        Receive-FixedReleaseFile -Uri "$root/SHA256SUMS.sig" -OutFile $signature
        Test-ReleaseSignatureIfAvailable -Manifest $checksums -Signature $signature `
            -Candidate $candidate -TemporaryDirectory $temporary
        Test-ReleaseChecksum -Candidate $candidate -Manifest $checksums -AssetName $asset
        Install-VerifiedBinary -Candidate $candidate -DestinationDirectory $fullInstallDir
        if (-not $NoAliases) { Add-QuickShareAliases -DestinationDirectory $fullInstallDir }
        Add-ToUserPath -Directory $fullInstallDir
        if ($AddPrivateFirewallRule) {
            Add-QuickSharePrivateFirewallRule -Program (Join-Path $fullInstallDir "quick-share.exe")
        }
        Write-Info "Installed $(& (Join-Path $fullInstallDir 'quick-share.exe') --version)"
        Write-Warn "Open a new terminal for the user PATH change to take effect"
    }
    finally {
        Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if ($Help) { Show-Usage }
elseif ($MyInvocation.InvocationName -ne '.') { Install-QuickShare }
