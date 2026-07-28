[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId,

    [Parameter(Mandatory = $true)]
    [string]$Report
)

$ErrorActionPreference = "Stop"
$chooseFiles = -join ([char[]](0x9009, 0x62e9, 0x6587, 0x4ef6))
$chooseFolder = -join ([char[]](0x9009, 0x62e9, 0x6587, 0x4ef6, 0x5939))
$cancel = -join ([char[]](0x53d6, 0x6d88))
$pickerName = -join ([char[]](0x9009, 0x62e9, 0x4e00, 0x4e2a, 0x6216, 0x591a, 0x4e2a, 0x6587, 0x4ef6))
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

function Get-ProcessElements([int]$Id) {
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty,
        $Id
    )
    $windows = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
        [System.Windows.Automation.TreeScope]::Children,
        $condition
    )
    $result = New-Object System.Collections.ArrayList
    foreach ($window in $windows) {
        [void]$result.Add($window)
        foreach ($element in $window.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition
        )) {
            [void]$result.Add($element)
        }
    }
    return $result
}

function Find-ElementByName([int]$Id, [string]$Name) {
    foreach ($element in (Get-ProcessElements $Id)) {
        if ($element.Current.Name -eq $Name) {
            return $element
        }
    }
    return $null
}

function Close-AutomationWindow($Element) {
    if ($null -eq $Element) { return $false }
    try {
        $pattern = $Element.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern)
        $pattern.Close()
        return $true
    }
    catch {
        return $false
    }
}

$deadline = [DateTime]::UtcNow.AddSeconds(15)
do {
    $elements = Get-ProcessElements $ProcessId
    $sourceButton = $null
    foreach ($element in $elements) {
        if ($element.Current.Name -eq $chooseFiles) {
            $sourceButton = $element
            break
        }
    }
    if ($null -eq $sourceButton) { Start-Sleep -Milliseconds 250 }
} while ($null -eq $sourceButton -and [DateTime]::UtcNow -lt $deadline)

$initialNames = @()
foreach ($element in (Get-ProcessElements $ProcessId)) {
    if ($element.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button) {
        $initialNames += $element.Current.Name
    }
}

$pickerShown = $false
$pickerTitle = ""
$pickerClosed = $false
if ($null -ne $sourceButton) {
    $invokeScript = Join-Path $PSScriptRoot "windows-ui-invoke.ps1"
    $invoker = Start-Process -FilePath "powershell.exe" -ArgumentList @(
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", "`"$invokeScript`"",
        "-ProcessId", $ProcessId
    ) -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 250
        $picker = Find-ElementByName $ProcessId $pickerName
    } while ($null -eq $picker -and [DateTime]::UtcNow -lt $deadline)
    if ($null -ne $picker) {
        $pickerShown = $true
        $pickerTitle = $picker.Current.Name
        $pickerClosed = Close-AutomationWindow $picker
    }
    if (-not $invoker.WaitForExit(10000)) {
        $invoker.Kill()
        $invoker.WaitForExit()
    }
}

Start-Sleep -Milliseconds 750
$summary = Find-ElementByName $ProcessId "Quick Share probe result"
$summaryClosed = Close-AutomationWindow $summary

$result = [ordered]@{
    timestampUtc = [DateTime]::UtcNow.ToString("o")
    processId = $ProcessId
    automationSessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
    initialButtons = $initialNames
    hasChooseFiles = $initialNames -contains $chooseFiles
    hasChooseFolder = $initialNames -contains $chooseFolder
    hasCancel = $initialNames -contains $cancel
    pickerShown = $pickerShown
    pickerTitle = $pickerTitle
    pickerClosed = $pickerClosed
    summaryClosed = $summaryClosed
}
$result.passed = (
    $result.hasChooseFiles -and
    $result.hasChooseFolder -and
    $result.hasCancel -and
    $result.pickerShown -and
    $result.pickerClosed
)
$result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Report -Encoding utf8
if (-not $result.passed) { exit 1 }
