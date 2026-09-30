# Stop Jarvis processes from this installation without interpreting the path as PowerShell or a wildcard.
param([Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$Root)

$ErrorActionPreference = "Stop"
$prefix = $Root.TrimEnd('\') + '\'
$voicePrefix = $prefix + 'tools\voice-server\'
Get-CimInstance Win32_Process | Where-Object {
    $path = $_.ExecutablePath
    if (-not $path) { return $false }
    $app = $path.Equals($prefix + 'jarvis-app.exe', [StringComparison]::OrdinalIgnoreCase) -or
        $path.Equals($prefix + 'jarvis-gui.exe', [StringComparison]::OrdinalIgnoreCase)
    $voice = $path.StartsWith($voicePrefix, [StringComparison]::OrdinalIgnoreCase) -and
        ($_.Name -in @('python.exe', 'pythonw.exe', 'whisper-server.exe'))
    $app -or $voice
} | ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop
}
