BeforeAll {
    $scriptPath = Join-Path $PSScriptRoot "../../install.ps1"
    . $scriptPath
}

Describe "Rust release mapping" {
    It "maps AMD64 to the MSVC asset" {
        Get-ReleaseTarget -Architecture "AMD64" | Should -Be "x86_64-pc-windows-msvc"
        Get-AssetName -Target "x86_64-pc-windows-msvc" | Should -Be "quick-share-x86_64-pc-windows-msvc.exe"
    }

    It "rejects unsupported architectures" {
        { Get-ReleaseTarget -Architecture "ARM32" } | Should -Throw
    }

    It "uses only the fixed repository and validates versions" {
        Get-ReleaseDownloadRoot -RequestedVersion "latest" | Should -Be `
            "https://github.com/Newbluecake/quick-share/releases/latest/download"
        Get-ReleaseDownloadRoot -RequestedVersion "2.0.0" | Should -Be `
            "https://github.com/Newbluecake/quick-share/releases/download/v2.0.0"
        { Get-ReleaseDownloadRoot -RequestedVersion "../../evil" } | Should -Throw
    }
}

Describe "Checksum verification" {
    It "accepts one exact checksum and rejects duplicates or corruption" {
        $candidate = Join-Path $TestDrive "candidate.exe"
        $manifest = Join-Path $TestDrive "SHA256SUMS"
        Set-Content -LiteralPath $candidate -Value "payload" -NoNewline
        $digest = (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant()
        Set-Content -LiteralPath $manifest -Value "$digest  quick-share-target.exe"
        { Test-ReleaseChecksum -Candidate $candidate -Manifest $manifest -AssetName "quick-share-target.exe" } | Should -Not -Throw

        Set-Content -LiteralPath $manifest -Value @(
            "$digest  quick-share-target.exe",
            "$digest  quick-share-target.exe"
        )
        { Test-ReleaseChecksum -Candidate $candidate -Manifest $manifest -AssetName "quick-share-target.exe" } | Should -Throw
    }
}

Describe "Fixed-origin downloads" {
    BeforeEach {
        Mock Invoke-WebRequest { return $null }
    }

    It "rejects foreign initial origins before downloading" {
        { Receive-FixedReleaseFile -Uri "https://evil.example/file" -OutFile (Join-Path $TestDrive "x") } | Should -Throw
        Should -Invoke Invoke-WebRequest -Times 0
    }

    It "passes only a fixed GitHub release URL to Invoke-WebRequest" {
        $url = "https://github.com/Newbluecake/quick-share/releases/latest/download/SHA256SUMS"
        Receive-FixedReleaseFile -Uri $url -OutFile (Join-Path $TestDrive "sums")
        Should -Invoke Invoke-WebRequest -Times 1 -ParameterFilter { $Uri -ceq $url }
    }
}

Describe "No-clobber shortcuts and rollback" {
    It "does not overwrite existing sc or rc files" {
        $directory = Join-Path $TestDrive "bin"
        New-Item -ItemType Directory -Path $directory | Out-Null
        Set-Content -LiteralPath (Join-Path $directory "quick-share.exe") -Value "new"
        Set-Content -LiteralPath (Join-Path $directory "sc.exe") -Value "existing-sc"
        Set-Content -LiteralPath (Join-Path $directory "rc.exe") -Value "existing-rc"
        Add-QuickShareAliases -DestinationDirectory $directory
        Get-Content -LiteralPath (Join-Path $directory "sc.exe") | Should -Be "existing-sc"
        Get-Content -LiteralPath (Join-Path $directory "rc.exe") | Should -Be "existing-rc"
    }

    It "preserves the old executable when the candidate cannot start" {
        $directory = Join-Path $TestDrive "installed"
        New-Item -ItemType Directory -Path $directory | Out-Null
        $current = Join-Path $directory "quick-share.exe"
        $candidate = Join-Path $TestDrive "bad.exe"
        Set-Content -LiteralPath $current -Value "old-binary"
        Set-Content -LiteralPath $candidate -Value "not-an-executable"
        { Install-VerifiedBinary -Candidate $candidate -DestinationDirectory $directory } | Should -Throw
        Get-Content -LiteralPath $current | Should -Be "old-binary"
        @(Get-ChildItem -LiteralPath $directory -Filter ".quick-share.new.*").Count | Should -Be 0
    }
}

Describe "Uninstall ownership" {
    BeforeEach {
        Mock Remove-QuickSharePrivateFirewallRule { }
        Mock Remove-FromUserPath { }
    }

    It "does not delete unmarked pre-existing sc or rc files" {
        $directory = Join-Path $TestDrive "uninstall"
        New-Item -ItemType Directory -Path $directory | Out-Null
        Set-Content -LiteralPath (Join-Path $directory "quick-share.exe") -Value "managed-main"
        Set-Content -LiteralPath (Join-Path $directory "sc.exe") -Value "existing-sc"
        Set-Content -LiteralPath (Join-Path $directory "rc.exe") -Value "existing-rc"
        Uninstall-QuickShare -DestinationDirectory $directory
        Test-Path -LiteralPath (Join-Path $directory "quick-share.exe") | Should -Be $false
        Get-Content -LiteralPath (Join-Path $directory "sc.exe") | Should -Be "existing-sc"
        Get-Content -LiteralPath (Join-Path $directory "rc.exe") | Should -Be "existing-rc"
    }
}

Describe "Firewall is explicit and minimal" {
    BeforeEach {
        Mock Get-NetFirewallRule { return $null }
        Mock New-NetFirewallRule { return $null }
    }

    It "adds only a program-scoped Private TCP inbound rule" {
        Add-QuickSharePrivateFirewallRule -Program "C:\QuickShare\quick-share.exe"
        Should -Invoke New-NetFirewallRule -Times 1 -ParameterFilter {
            $DisplayName -ceq "Quick Share (Private inbound)" -and
            $Direction -ceq "Inbound" -and $Action -ceq "Allow" -and
            $Profile -ceq "Private" -and $Protocol -ceq "TCP" -and
            $Program -ceq "C:\QuickShare\quick-share.exe"
        }
    }

    It "refuses to alter an existing same-named rule" {
        Mock Get-NetFirewallRule { return @{ DisplayName = "Quick Share (Private inbound)" } }
        { Add-QuickSharePrivateFirewallRule -Program "C:\QuickShare\quick-share.exe" } | Should -Throw
        Should -Invoke New-NetFirewallRule -Times 0
    }
}
