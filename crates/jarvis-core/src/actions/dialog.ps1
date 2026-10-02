$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr][long]$env:JARVIS_WINDOW)
if ($null -eq $root) { throw 'Window no longer exists' }
$current = $root.Current
if ($env:JARVIS_PID -and $current.ProcessId -ne [int]$env:JARVIS_PID) { throw 'Window owner changed' }
$condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
$elements = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
if ($elements.Count -gt 200) { throw 'Too many controls in this window' }
if ($env:JARVIS_BUTTON_ID) {
    $found = @($elements | Where-Object { ($_.GetRuntimeId() -join '.') -eq $env:JARVIS_BUTTON_ID })
    if ($found.Count -ne 1) { throw 'Button no longer exists or is ambiguous' }
    $button = $found[0]
    if ($button.Current.Name -cne $env:JARVIS_BUTTON_NAME -or -not $button.Current.IsEnabled -or $button.Current.IsOffscreen) { throw 'Button changed or is unavailable' }
    $invoke = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $invoke.Invoke()
    '{"invoked":true}'
} else {
    $buttons = @($elements | Where-Object { $_.Current.IsEnabled -and -not $_.Current.IsOffscreen } | ForEach-Object {
        @{ name = $_.Current.Name; id = ($_.GetRuntimeId() -join '.') }
    })
    @{ pid = $current.ProcessId; title = $current.Name; buttons = $buttons } | ConvertTo-Json -Depth 4 -Compress
}
