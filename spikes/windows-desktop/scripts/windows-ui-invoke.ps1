[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId
)

$ErrorActionPreference = "Stop"
$chooseFiles = -join ([char[]](0x9009, 0x62e9, 0x6587, 0x4ef6))
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$condition = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ProcessIdProperty,
    $ProcessId
)
$deadline = [DateTime]::UtcNow.AddSeconds(15)
$sourceButton = $null
do {
    $windows = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
        [System.Windows.Automation.TreeScope]::Children,
        $condition
    )
    foreach ($window in $windows) {
        foreach ($element in $window.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition
        )) {
            if ($element.Current.Name -eq $chooseFiles) {
                $sourceButton = $element
                break
            }
        }
        if ($null -ne $sourceButton) { break }
    }
    if ($null -eq $sourceButton) { Start-Sleep -Milliseconds 250 }
} while ($null -eq $sourceButton -and [DateTime]::UtcNow -lt $deadline)

if ($null -eq $sourceButton) { exit 1 }
$invoke = $sourceButton.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
$invoke.Invoke()
