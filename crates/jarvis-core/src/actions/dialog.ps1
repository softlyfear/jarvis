$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JarvisDialogVisibility {
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
}
'@
function Test-Available($element) {
    $info = $element.Current
    if (-not $info.IsEnabled) { return $false }
    if ($info.NativeWindowHandle -ne 0) {
        return [JarvisDialogVisibility]::IsWindowVisible([IntPtr]$info.NativeWindowHandle)
    }
    return -not $info.IsOffscreen
}
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
    if ($button.Current.Name -cne $env:JARVIS_BUTTON_NAME -or -not (Test-Available $button)) { throw 'Button changed or is unavailable' }
    $invoke = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $invoke.Invoke()
    '{"invoked":true}'
} else {
    $buttons = @($elements | Where-Object { Test-Available $_ } | ForEach-Object {
        @{ name = $_.Current.Name; id = ($_.GetRuntimeId() -join '.') }
    })
    $observed = @($elements | ForEach-Object { @{ name = $_.Current.Name; enabled = $_.Current.IsEnabled; offscreen = $_.Current.IsOffscreen; native = $_.Current.NativeWindowHandle } })
    @{ observed = $observed; pid = $current.ProcessId; title = $current.Name; buttons = $buttons } | ConvertTo-Json -Depth 4 -Compress
}
