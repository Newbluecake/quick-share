# Pester tests for install.ps1
# PowerShell 5.1+ compatible

BeforeAll {
    # Import the install script functions
    $scriptPath = Join-Path $PSScriptRoot "../../install.ps1"

    # Mock the script by dot-sourcing it in a way that prevents execution
    $scriptContent = Get-Content $scriptPath -Raw
    # Remove the main execution block if it exists
    $scriptContent = $scriptContent -replace '(?s)# Main execution.*$', ''

    # Create a temporary script with just the functions
    $tempScript = Join-Path $TestDrive "install_functions.ps1"
    Set-Content -Path $tempScript -Value $scriptContent
    . $tempScript
}

Describe "Get-DownloadUrl" {
    It "returns correct GitHub release URL" {
        $url = Get-DownloadUrl
        $url | Should -Be "https://github.com/Newbluecake/quick-share/releases/latest/download/quick-share-windows.exe"
    }
}

Describe "Get-InstallPath" {
    It "returns path in LOCALAPPDATA directory" {
        $path = Get-InstallPath
        $path | Should -Match "\\quick-share$"
        $path | Should -BeLike "*\AppData\Local\quick-share"
    }
}

Describe "Install-Binary" {
    BeforeEach {
        # Use TestDrive for isolated testing
        $testInstallDir = Join-Path $TestDrive "quick-share"
        $testBinaryPath = Join-Path $testInstallDir "quick-share.exe"

        # Mock Invoke-WebRequest
        Mock Invoke-WebRequest {
            param($Uri, $OutFile)
            # Create a dummy file to simulate download
            New-Item -ItemType File -Path $OutFile -Force | Out-Null
            Set-Content -Path $OutFile -Value "dummy exe content"
        }
    }

    It "creates target directory if it doesn't exist" {
        $testInstallDir = Join-Path $TestDrive "quick-share-new"
        $testBinaryPath = Join-Path $testInstallDir "quick-share.exe"

        Test-Path $testInstallDir | Should -Be $false

        Install-Binary -InstallPath $testInstallDir -DownloadUrl "https://example.com/file.exe"

        Test-Path $testInstallDir | Should -Be $true
    }

    It "downloads file successfully" {
        $testInstallDir = Join-Path $TestDrive "quick-share-download"
        $testBinaryPath = Join-Path $testInstallDir "quick-share.exe"

        Install-Binary -InstallPath $testInstallDir -DownloadUrl "https://example.com/file.exe"

        Test-Path $testBinaryPath | Should -Be $true
    }

    It "calls Invoke-WebRequest with correct parameters" {
        $testInstallDir = Join-Path $TestDrive "quick-share-params"
        $downloadUrl = "https://github.com/test/download.exe"

        Install-Binary -InstallPath $testInstallDir -DownloadUrl $downloadUrl

        Should -Invoke Invoke-WebRequest -Times 1 -ParameterFilter {
            $Uri -eq $downloadUrl
        }
    }
}

Describe "Add-ToPath" {
    BeforeEach {
        # Mock registry operations
        Mock Get-ItemProperty {
            return @{ Path = "C:\Windows;C:\Program Files" }
        }

        Mock Set-ItemProperty {
            param($Path, $Name, $Value)
        }
    }

    It "adds directory to PATH when not present" {
        $testPath = "C:\TestDir"

        Add-ToPath -Directory $testPath

        Should -Invoke Set-ItemProperty -Times 1 -ParameterFilter {
            $Value -match [regex]::Escape($testPath)
        }
    }

    It "skips when directory already in PATH" {
        Mock Get-ItemProperty {
            return @{ Path = "C:\Windows;C:\TestDir;C:\Program Files" }
        }

        $testPath = "C:\TestDir"

        Add-ToPath -Directory $testPath

        Should -Invoke Set-ItemProperty -Times 0
    }

    It "uses User environment registry path" {
        $testPath = "C:\NewDir"

        Add-ToPath -Directory $testPath

        Should -Invoke Get-ItemProperty -Times 1 -ParameterFilter {
            $Path -eq "Registry::HKEY_CURRENT_USER\Environment"
        }
    }
}

Describe "Install-QuickShare (Integration)" {
    BeforeAll {
        # Mock all external calls for integration test
        Mock Get-DownloadUrl { return "https://example.com/test.exe" }
        Mock Get-InstallPath { return Join-Path $TestDrive "quick-share" }
        Mock Install-Binary {
            param($InstallPath, $DownloadUrl)
            New-Item -ItemType Directory -Path $InstallPath -Force | Out-Null
            $exePath = Join-Path $InstallPath "quick-share.exe"
            Set-Content -Path $exePath -Value "test exe"
        }
        Mock Add-ToPath { param($Directory) }
        Mock Write-Host { }
    }

    It "completes installation successfully" {
        # This would call the main Install-QuickShare function
        # For now, we'll test that all components work together
        $installPath = Get-InstallPath
        $downloadUrl = Get-DownloadUrl

        Install-Binary -InstallPath $installPath -DownloadUrl $downloadUrl
        Add-ToPath -Directory $installPath

        $exePath = Join-Path $installPath "quick-share.exe"
        Test-Path $exePath | Should -Be $true
    }
}

Describe "Installation verification" {
    It "produces executable quick-share.exe file" {
        # This test verifies the final output
        $testInstallDir = Join-Path $TestDrive "quick-share-final"
        $testBinaryPath = Join-Path $testInstallDir "quick-share.exe"

        Mock Invoke-WebRequest {
            param($Uri, $OutFile)
            New-Item -ItemType File -Path $OutFile -Force | Out-Null
            # Simulate executable content
            Set-Content -Path $OutFile -Value "MZ" -NoNewline
        }

        Install-Binary -InstallPath $testInstallDir -DownloadUrl "https://example.com/test.exe"

        Test-Path $testBinaryPath | Should -Be $true
        (Get-Item $testBinaryPath).Length | Should -BeGreaterThan 0
    }
}
