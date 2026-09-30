$ErrorActionPreference = "Stop"
$root = "C:\Jarvis ' [test]"
$script:stopped = @()
function Get-CimInstance {
    param($ClassName)
    @(
        @{ Name = 'jarvis-app.exe'; ExecutablePath = "$root\jarvis-app.exe"; ProcessId = 1 },
        @{ Name = 'jarvis-gui.exe'; ExecutablePath = "$root\jarvis-gui.exe"; ProcessId = 2 },
        @{ Name = 'python.exe'; ExecutablePath = "$root\tools\voice-server\python\python.exe"; ProcessId = 3 },
        @{ Name = 'whisper-server.exe'; ExecutablePath = "$root\tools\voice-server\whispercpp\whisper-server.exe"; ProcessId = 4 },
        @{ Name = 'jarvis-app.exe'; ExecutablePath = "${root}-other\jarvis-app.exe"; ProcessId = 5 },
        @{ Name = 'other.exe'; ExecutablePath = "$root\other.exe"; ProcessId = 6 },
        @{ Name = 'python.exe'; ExecutablePath = 'C:\Python\python.exe'; ProcessId = 7 },
        @{ Name = 'system'; ExecutablePath = $null; ProcessId = 8 }
    ) | ForEach-Object { [pscustomobject]$_ }
}
function Stop-Process {
    param($Id, [switch]$Force, $ErrorAction)
    $script:stopped += $Id
}
. (Join-Path $PSScriptRoot '..\stop-processes.ps1') -Root $root
if (($script:stopped -join ',') -ne '1,2,3,4') { throw "Unexpected processes: $($script:stopped -join ',')" }
"OK: only the selected installation was stopped; quotes and brackets are literal"
