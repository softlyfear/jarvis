$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
using System.Text;
public static class JarvisDialogVisibility {
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    public delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder value, int size);
    public static string[] Children(IntPtr parent) {
        var result = new List<string>();
        EnumChildWindows(parent, delegate(IntPtr window, IntPtr parameter) {
            var name = new StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            result.Add(window.ToInt64() + ":" + name + ":visible=" + IsWindowVisible(window));
            return result.Count < 30;
        }, IntPtr.Zero);
        return result.ToArray();
    }
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
    $found = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements[$index]
        if (($element.GetRuntimeId() -join '.') -eq $env:JARVIS_BUTTON_ID) { $found += $element }
    }
    if ($found.Count -ne 1) { throw 'Button no longer exists or is ambiguous' }
    $button = $found[0]
    if ($button.Current.Name -cne $env:JARVIS_BUTTON_NAME -or -not (Test-Available $button)) { throw 'Button changed or is unavailable' }
    $invoke = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $invoke.Invoke()
    '{"invoked":true}'
} else {
    $buttons = @()
    $observed = @()
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements[$index]
        $info = $element.Current
        $observed += @{ name = $info.Name; enabled = $info.IsEnabled; offscreen = $info.IsOffscreen; native = $info.NativeWindowHandle }
        if (Test-Available $element) { $buttons += @{ name = $info.Name; id = ($element.GetRuntimeId() -join '.') } }
    }
    $diagnostic = $null
    if ($env:JARVIS_UIA_DIAGNOSTICS -and $elements.Count -eq 0) {
        $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        $tree = @()
        for ($index = 0; $index -lt [Math]::Min(30, $all.Count); $index++) {
            $info = $all[$index].Current
            $tree += @{ name = $info.Name; type = $info.ControlType.ProgrammaticName; class = $info.ClassName }
        }
        $diagnostic = @{
            tree = $tree
            children = [JarvisDialogVisibility]::Children([IntPtr][long]$env:JARVIS_WINDOW)
            class = $current.ClassName
            framework = $current.FrameworkId
            type = $current.ControlType.ProgrammaticName
            offscreen = $current.IsOffscreen
            session = [Diagnostics.Process]::GetCurrentProcess().SessionId
            assemblies = @([AppDomain]::CurrentDomain.GetAssemblies() | Where-Object { $_.GetName().Name -like "UIAutomation*" } | ForEach-Object { $_.FullName })
        }
    }
    @{ diagnostic = $diagnostic; observed = $observed; pid = $current.ProcessId; title = $current.Name; buttons = $buttons } | ConvertTo-Json -Depth 5 -Compress
}
