param([Parameter(Mandatory = $true)][string]$Iscc)
$ErrorActionPreference = 'Stop'
# Compile the actual installer script with small fake applications and a delayed voice setup.
# No models, packages, user configuration, shortcuts or autostart registry entries are changed.
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('jarvis-lifecycle-' + [guid]::NewGuid())
$target = Join-Path $fixture 'installed'
$dist = Join-Path $fixture 'dist/Jarvis'
$utf8 = New-Object Text.UTF8Encoding($true)
function Write-TestFile($path, $text) {
    New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
    [IO.File]::WriteAllText($path, $text.Replace("`r`n", "`n").Replace("`n", "`r`n"), $utf8)
}
function Test-Apps {
    @(Get-CimInstance Win32_Process | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($target + '\', [StringComparison]::OrdinalIgnoreCase) -and
        $_.Name -in @('jarvis-app.exe', 'jarvis-gui.exe')
    })
}
try {
    New-Item -ItemType Directory -Force "$fixture/installer", "$fixture/resources/icons", $dist | Out-Null
    $source = [IO.File]::ReadAllText((Join-Path $repo 'installer/jarvis.iss'))
    # Isolate OS integration while preserving every lifecycle callback and all [Run] entries.
    $source = [regex]::Replace($source, '(?ms)^\[(Icons|INI|Registry)\]\r?\n.*?(?=^\[)', '')
    $source = $source.Replace('UsePreviousAppDir=yes', "UsePreviousAppDir=no`r`nUninstallable=no")
    Write-TestFile "$fixture/installer/jarvis.iss" $source
    Copy-Item (Join-Path $repo 'installer/stop-processes.ps1') "$fixture/installer/stop-processes.ps1"
    Copy-Item (Join-Path $repo 'resources/icons/icon.ico') "$fixture/resources/icons/icon.ico"
    Write-TestFile "$fixture/installer/configure.ps1" @'
param($Template, $KeysFile, $Address)
[IO.File]::WriteAllText((Join-Path (Split-Path $Template) 'configured.txt'), 'done')
'@
    Write-TestFile "$dist/assistant.example.toml" '[assistant]'
    Write-TestFile "$dist/tools/voice-server/install.ps1" @'
param([switch]$NoPause, [switch]$Installer, [switch]$RuntimeOnly)
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $RuntimeOnly) { throw 'Expected an update of the existing runtime' }
if (-not (Test-Path (Join-Path $root 'configured.txt'))) { throw 'Configuration must finish before runtime setup' }
[IO.File]::WriteAllText((Join-Path $root 'voice-started.txt'), 'started')
Write-Output '==> Test voice runtime'
Start-Sleep -Seconds 2
$premature = @(Get-CimInstance Win32_Process | Where-Object {
    $_.ExecutablePath -and $_.ExecutablePath.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase) -and
    $_.Name -in @('jarvis-app.exe', 'jarvis-gui.exe')
})
if ($premature.Count) { [IO.File]::WriteAllText((Join-Path $root 'premature.txt'), 'started during voice setup') }
[IO.File]::WriteAllText((Join-Path $root 'voice-finished.txt'), 'finished')
Write-Output '[server] download finished'
'@
    Write-TestFile "$fixture/stub.cs" @'
using System;
using System.IO;
using System.Diagnostics;
using System.Threading;
class InstallerTestApp {
    static void Main() {
        string root = AppDomain.CurrentDomain.BaseDirectory;
        string name = Path.GetFileNameWithoutExtension(Process.GetCurrentProcess().MainModule.FileName);
        bool hasRuntime = File.Exists(Path.Combine(root, "tools/voice-server/python/python.exe"));
        bool ready = File.Exists(Path.Combine(root, "configured.txt")) &&
            (!hasRuntime || File.Exists(Path.Combine(root, "voice-finished.txt")));
        File.WriteAllText(Path.Combine(root, name + "-started.txt"), ready ? "ready" : "premature");
        Thread.Sleep(300000);
    }
}
'@
    $compiler = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    & $compiler /nologo /target:winexe ("/out:$dist/jarvis-app.exe") "$fixture/stub.cs"
    if ($LASTEXITCODE) { throw 'Fake application compilation failed' }
    Copy-Item "$dist/jarvis-app.exe" "$dist/jarvis-gui.exe"
    & $Iscc /Qp /DAppVersion=0.0.1 "$fixture/installer/jarvis.iss"
    if ($LASTEXITCODE) { throw 'Fixture installer compilation failed' }
    foreach ($mode in @('without-runtime', 'existing-runtime')) {
        if ($mode -eq 'existing-runtime') {
            Write-TestFile "$target/tools/voice-server/python/python.exe" ''
            Remove-Item "$target/jarvis-app-started.txt", "$target/jarvis-gui-started.txt", "$target/configured.txt"
            if (@(Test-Apps).Count -ne 2) { throw 'Both old applications must be running before the update' }
        }
        $start = New-Object Diagnostics.ProcessStartInfo
        $start.FileName = "$fixture/dist/JarvisSetup.exe"
        $start.Arguments = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /TASKS="" "/DIR=' + $target + '" "/LOG=' + $fixture + '/' + $mode + '.log"'
        $start.UseShellExecute = $false
        $setup = [Diagnostics.Process]::Start($start)
        try {
            if (-not $setup.WaitForExit(60000)) { $setup.Kill(); $setup.WaitForExit(); throw 'Installer timed out' }
            if ($setup.ExitCode -ne 0) { throw ("Installer failed ($mode): " + $setup.ExitCode) }
        } finally { $setup.Dispose() }
        $watch = [Diagnostics.Stopwatch]::StartNew()
        while (@(Test-Apps).Count -ne 2 -and $watch.Elapsed.TotalSeconds -lt 5) { Start-Sleep -Milliseconds 100 }
        foreach ($app in @('jarvis-app', 'jarvis-gui')) {
            $marker = Join-Path $target ($app + '-started.txt')
            if (-not (Test-Path $marker) -or (Get-Content $marker -Raw) -ne 'ready') { throw "$app launched before setup finished ($mode)" }
        }
        if (Test-Path "$target/premature.txt") { throw 'Applications ran during voice setup' }
        if ($mode -eq 'existing-runtime' -and -not (Test-Path "$target/voice-finished.txt")) { throw 'Voice runtime setup did not finish' }
        Start-Sleep -Seconds 2
        if (@(Test-Apps).Count -ne 2) { throw "Applications stopped after the installer exited ($mode)" }
        Write-Host "OK installer lifecycle: $mode; applications launch after setup and stay alive"
    }
} catch {
    Get-ChildItem $fixture -Filter '*.log' -ErrorAction SilentlyContinue | ForEach-Object { Get-Content $_.FullName -Tail 80 }
    throw
} finally {
    Test-Apps | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
}
