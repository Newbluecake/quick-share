# Quick Share - Windows Installation Script
# PowerShell 5.1+ compatible
# Downloads and installs quick-share to user's local application directory

param(
    [switch]$Help
)

# Show help message
if ($Help) {
    Write-Host @"
Quick Share Installation Script

This script will:
1. Download quick-share from GitHub Releases
2. Install to $env:LOCALAPPDATA\quick-share
3. Add the installation directory to your User PATH

Usage:
    .\install.ps1        # Run installation
    .\install.ps1 -Help  # Show this help message

"@
    exit 0
}

# Function: Get download URL for the latest release
function Get-DownloadUrl {
    return "https://github.com/Newbluecake/quick-share/releases/latest/download/quick-share-windows.exe"
}

# Function: Get installation path
function Get-InstallPath {
    return Join-Path $env:LOCALAPPDATA "quick-share"
}

# Function: Download and install binary
function Install-Binary {
    param(
        [Parameter(Mandatory=$true)]
        [string]$InstallPath,

        [Parameter(Mandatory=$true)]
        [string]$DownloadUrl
    )

    # Create installation directory if it doesn't exist
    if (-not (Test-Path $InstallPath)) {
        Write-Host "Creating installation directory: $InstallPath"
        New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null
    }

    # Download the binary
    $binaryPath = Join-Path $InstallPath "quick-share.exe"
    Write-Host "Downloading quick-share from: $DownloadUrl"

    try {
        Invoke-WebRequest -Uri $DownloadUrl -OutFile $binaryPath -UseBasicParsing
        Write-Host "Successfully downloaded to: $binaryPath"
    }
    catch {
        Write-Error "Failed to download quick-share: $_"
        throw
    }
}

# Function: Add directory to User PATH environment variable
function Add-ToPath {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Directory
    )

    # Get current User PATH from registry
    $registryPath = "Registry::HKEY_CURRENT_USER\Environment"
    $currentPath = (Get-ItemProperty -Path $registryPath -Name Path).Path

    # Check if directory is already in PATH
    $pathEntries = $currentPath -split ";"
    $directoryNormalized = $Directory.TrimEnd('\')

    $alreadyInPath = $false
    foreach ($entry in $pathEntries) {
        if ($entry.TrimEnd('\') -eq $directoryNormalized) {
            $alreadyInPath = $true
            break
        }
    }

    if ($alreadyInPath) {
        Write-Host "Directory already in PATH: $Directory"
        return
    }

    # Add to PATH
    Write-Host "Adding to User PATH: $Directory"
    $newPath = $currentPath.TrimEnd(';') + ';' + $Directory
    Set-ItemProperty -Path $registryPath -Name Path -Value $newPath

    Write-Host "Successfully added to PATH"
}

# Function: Main installation process
function Install-QuickShare {
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  Quick Share Installation for Windows  " -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""

    # Get installation parameters
    $installPath = Get-InstallPath
    $downloadUrl = Get-DownloadUrl

    # Download and install binary
    Install-Binary -InstallPath $installPath -DownloadUrl $downloadUrl

    # Add to PATH
    Add-ToPath -Directory $installPath

    # Success message
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "  Installation completed successfully!  " -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "Quick Share has been installed to:" -ForegroundColor Yellow
    Write-Host "  $installPath" -ForegroundColor White
    Write-Host ""
    Write-Host "IMPORTANT: Please restart your terminal or PowerShell session" -ForegroundColor Yellow
    Write-Host "to use the 'quick-share' command from anywhere." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "To test immediately (without restart), run:" -ForegroundColor Cyan
    Write-Host "  & '$installPath\quick-share.exe' --version" -ForegroundColor White
    Write-Host ""
}

# Main execution - only run if not being dot-sourced (for testing)
if ($MyInvocation.InvocationName -ne '.') {
    Install-QuickShare
}
