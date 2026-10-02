$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JarvisTestDialog {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string title);
}
'@
$helper = Join-Path $PSScriptRoot '../src/actions/dialog.ps1'
$script = Join-Path ([IO.Path]::GetTempPath()) ('jarvis-uia-' + [guid]::NewGuid() + '.ps1')
$env:JARVIS_TEST_TITLE = 'Jarvis UIA test ' + [guid]::NewGuid()
[IO.File]::WriteAllText($script, @'
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.MessageBox]::Show('Choose a button', $env:JARVIS_TEST_TITLE, [System.Windows.Forms.MessageBoxButtons]::YesNoCancel)
'@, (New-Object System.Text.UTF8Encoding($true)))
$child = $null
try {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = 'powershell'
    $start.Arguments = '-NoProfile -STA -ExecutionPolicy Bypass -File "' + $script + '"'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardError = $true
    $child = [Diagnostics.Process]::Start($start)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        $handle = [JarvisTestDialog]::FindWindow('#32770', $env:JARVIS_TEST_TITLE)
        if ($handle -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 50 }
    } while ($handle -eq [IntPtr]::Zero -and $watch.Elapsed.TotalSeconds -lt 10)
    if ($handle -eq [IntPtr]::Zero) {
        $running = -not $child.HasExited
        if ($running) { $child.Kill(); $child.WaitForExit() }
        throw ('Native dialog did not appear; was running=' + $running + '; stderr=' + $child.StandardError.ReadToEnd())
    }
    $env:JARVIS_WINDOW = $handle.ToInt64().ToString()
    $env:JARVIS_PID = $child.Id.ToString()
    Start-Sleep -Milliseconds 500
    $env:JARVIS_BUTTON_ID = ''
    $env:JARVIS_BUTTON_NAME = ''
    $inspection = [Diagnostics.Stopwatch]::StartNew()
    $raw = & powershell -NoProfile -NonInteractive -MTA -ExecutionPolicy Bypass -File $helper
    if ($LASTEXITCODE) { throw 'UIA inspection failed' }
    Write-Host ("UIA inspection: {0} ms" -f $inspection.ElapsedMilliseconds)
    Write-Host ($raw -join "`n")
    $data = ($raw -join "`n") | ConvertFrom-Json
    $buttons = @($data.buttons)
    foreach ($name in @('Yes', 'No', 'Cancel')) {
        if (@($buttons | Where-Object { ($_.name -replace '&', '') -eq $name }).Count -ne 1) {
            throw ('Native dialog must expose exactly one ' + $name + ' button')
        }
    }
    $button = @($buttons | Where-Object { ($_.name -replace '&', '') -eq 'No' })
    if ($button.Count -ne 1) { throw 'Expected exactly one No button' }
    $env:JARVIS_BUTTON_ID = $button[0].id
    $env:JARVIS_BUTTON_NAME = $button[0].name
    $output = & powershell -NoProfile -NonInteractive -MTA -ExecutionPolicy Bypass -File $helper
    if ($LASTEXITCODE) { throw 'UIA invocation failed' }
    if (-not $child.WaitForExit(5000)) { throw 'Dialog did not close after No' }
    Write-Host 'OK native UI Automation: read buttons and invoke No'
} finally {
    if ($child) {
        if (-not $child.HasExited) { $child.Kill(); $child.WaitForExit() }
        $child.Dispose()
    }
    Remove-Item -LiteralPath $script -ErrorAction SilentlyContinue
    foreach ($name in @('JARVIS_TEST_TITLE', 'JARVIS_WINDOW', 'JARVIS_PID', 'JARVIS_BUTTON_ID', 'JARVIS_BUTTON_NAME')) {
        Remove-Item "Env:$name" -ErrorAction SilentlyContinue
    }
}
