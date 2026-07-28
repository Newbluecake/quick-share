[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId,

    [Parameter(Mandatory = $true)]
    [string]$Report
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$condition = New-Object System.Windows.Automation.PropertyCondition(
    [System.Windows.Automation.AutomationElement]::ProcessIdProperty,
    $ProcessId
)
$elements = [System.Windows.Automation.AutomationElement]::RootElement.FindAll(
    [System.Windows.Automation.TreeScope]::Descendants,
    $condition
)
$items = @()
foreach ($element in $elements) {
    $name = $element.Current.Name
    if (-not [string]::IsNullOrWhiteSpace($name)) {
        $items += [pscustomobject]@{
            name = $name
            controlType = $element.Current.ControlType.ProgrammaticName
            automationId = $element.Current.AutomationId
            className = $element.Current.ClassName
            helpText = $element.Current.HelpText
        }
    }
}
[ordered]@{
    timestampUtc = [DateTime]::UtcNow.ToString("o")
    processId = $ProcessId
    inspectorSessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
    elements = $items
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Report -Encoding utf8
