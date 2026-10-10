$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -ReferencedAssemblies @([System.Windows.Automation.AutomationElement].Assembly.Location, [System.Windows.Automation.ControlType].Assembly.Location) -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JarvisDialogVisibility {
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    public static void RegisterProviders() {
        // The .NET provider loader inspects the stack; a PowerShell dynamic frame has no ReflectedType.
        var assembly = typeof(System.Windows.Automation.AutomationElement).Assembly.GetName();
        assembly.Name = "UIAutomationClientsideProviders";
        try {
            System.Windows.Automation.ClientSettings.RegisterClientSideProviderAssembly(assembly);
        } catch (Exception error) {
            throw new InvalidOperationException(error.ToString(), error);
        }
    }
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [StructLayout(LayoutKind.Sequential)]
    public struct Keyboard { public ushort vk, scan; public uint flags, time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size = 32)]
    public struct InputUnion { [FieldOffset(0)] public Keyboard keyboard; [FieldOffset(0)] public UIntPtr alignment; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Input { public uint type; public InputUnion data; }
    [DllImport("user32.dll", SetLastError = true)]
    static extern uint SendInput(uint count, Input[] inputs, int size);
    public static void TypeText(IntPtr window, string text) {
        foreach (char c in text) {
            if (GetForegroundWindow() != window) throw new InvalidOperationException("Window lost focus; input cancelled");
            var inputs = new Input[2];
            inputs[0].type = inputs[1].type = 1;
            inputs[0].data.keyboard.scan = inputs[1].data.keyboard.scan = c;
            inputs[0].data.keyboard.flags = 4;
            inputs[1].data.keyboard.flags = 6;
            if (SendInput(2, inputs, Marshal.SizeOf(typeof(Input))) != 2)
                throw new InvalidOperationException("Windows rejected input; check elevation");
            System.Threading.Thread.Sleep(3);
        }
    }
}
'@
[JarvisDialogVisibility]::RegisterProviders()
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
function Get-TextControl {
    $pattern = $null
    if ($root.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern, [ref]$pattern)) { return $root }
    $textCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::IsTextPatternAvailableProperty, $true)
    $controls = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCondition)
    if ($controls.Count -gt 200) { throw 'Too many text controls' }
    foreach ($element in $controls) {
        if (Test-Available $element) { return $element }
    }
    return $null
}
function Get-ControlText($control) {
    if ($null -eq $control) { return '' }
    $pattern = $control.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
    $text = ''
    $ranges = $pattern.GetVisibleRanges()
    for ($index = $ranges.Count - 1; $index -ge 0 -and $text.Length -lt 8000; $index--) {
        # Read from the end without materializing the complete scrollback.
        $range = $ranges[$index].Clone()
        $range.MoveEndpointByRange([System.Windows.Automation.TextPatternRangeEndpoint]::Start, $range, [System.Windows.Automation.TextPatternRangeEndpoint]::End)
        $remaining = 8000 - $text.Length
        [void]$range.MoveEndpointByUnit([System.Windows.Automation.TextPatternRangeEndpoint]::Start, [System.Windows.Automation.TextUnit]::Character, -$remaining)
        if ($range.CompareEndpoints([System.Windows.Automation.TextPatternRangeEndpoint]::Start, $ranges[$index], [System.Windows.Automation.TextPatternRangeEndpoint]::Start) -lt 0) {
            $range.MoveEndpointByRange([System.Windows.Automation.TextPatternRangeEndpoint]::Start, $ranges[$index], [System.Windows.Automation.TextPatternRangeEndpoint]::Start)
        }
        $part = $range.GetText(-1)
        $text = if ($text) { $part + "`n" + $text } else { $part }
    }
    return $text.Substring([Math]::Max(0, $text.Length - 8000))
}
if ($env:JARVIS_TYPED_TEXT) {
    $control = Get-TextControl
    if ($null -eq $control) { throw 'Terminal does not expose readable text; input cancelled' }
    $before = Get-ControlText $control
    $control.SetFocus()
    Start-Sleep -Milliseconds 150
    [JarvisDialogVisibility]::TypeText([IntPtr][long]$env:JARVIS_WINDOW, $env:JARVIS_TYPED_TEXT)
    $verified = $false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 50
        $after = Get-ControlText $control
        $flat = $after.Replace("`r", '').Replace("`n", '').TrimEnd()
        if ($after -cne $before -and $flat.EndsWith($env:JARVIS_TYPED_TEXT, [StringComparison]::Ordinal)) { $verified = $true; break }
    } while ($watch.Elapsed.TotalMilliseconds -lt 1200)
    if (-not $verified) { throw 'Input not found at the end of terminal text; Enter cancelled' }
    '{"typed":true}'
    exit 0
}
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
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements[$index]
        $info = $element.Current
        if (Test-Available $element) { $buttons += @{ name = $info.Name; id = ($element.GetRuntimeId() -join '.') } }
    }
    $text = ''
    if ($env:JARVIS_READ_TEXT -eq '1') { $text = Get-ControlText (Get-TextControl) }
    @{ pid = $current.ProcessId; title = $current.Name; buttons = $buttons; text = $text } | ConvertTo-Json -Depth 4 -Compress
}
