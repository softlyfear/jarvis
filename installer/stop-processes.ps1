# Stop and drain this installation before replacing files or restarting the assistant.
param([Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$Root)

$ErrorActionPreference = "Stop"
$prefix = $Root.TrimEnd('\') + '\'
$voicePrefix = $prefix + 'tools\voice-server\'
function Get-JarvisProcesses {
    Get-CimInstance Win32_Process | Where-Object {
        $path = $_.ExecutablePath
        if (-not $path) { return $false }
        $app = $path.Equals($prefix + 'jarvis-app.exe', [StringComparison]::OrdinalIgnoreCase) -or
            $path.Equals($prefix + 'jarvis-gui.exe', [StringComparison]::OrdinalIgnoreCase)
        $voice = $path.StartsWith($voicePrefix, [StringComparison]::OrdinalIgnoreCase) -and
            ($_.Name -in @('python.exe', 'pythonw.exe', 'whisper-server.exe'))
        $app -or $voice
    }
}

# Stop-Process may return before teardown. A child can also appear after the first snapshot.
# Close windows first, then the assistant, then its servers, and rescan until quiescent.
$emptyScans = 0
for ($attempt = 0; $attempt -lt 60; $attempt++) {
    $processes = @(Get-JarvisProcesses)
    if (-not $processes.Count) {
        $emptyScans++
        if ($emptyScans -ge 3) { return }
    } else {
        $emptyScans = 0
        $processes | Sort-Object @{ Expression = {
            if ($_.Name -eq 'jarvis-gui.exe') { 0 }
            elseif ($_.Name -eq 'jarvis-app.exe') { 1 }
            else { 2 }
        } }, ProcessId | ForEach-Object {
            # A process that exited since the snapshot is harmless. Persistent failures
            # remain visible in the next scan and eventually abort the installation.
            Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
        }
    }
    Start-Sleep -Milliseconds 250
}
$remaining = @(Get-JarvisProcesses)
throw ("Не удалось завершить процессы Джарвиса: " + (($remaining | ForEach-Object { "$($_.Name) ($($_.ProcessId))" }) -join ', '))
