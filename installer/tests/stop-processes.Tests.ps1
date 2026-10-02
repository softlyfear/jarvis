$ErrorActionPreference = "Stop"
$root = "C:\Jarvis ' [test]"
$script:stopped = @()
$script:scans = 0
$script:mode = 'normal'
$script:mockProcesses = @(
    @{ Name = 'jarvis-app.exe'; ExecutablePath = "$root\jarvis-app.exe"; ProcessId = 1 },
    @{ Name = 'jarvis-gui.exe'; ExecutablePath = "$root\jarvis-gui.exe"; ProcessId = 2 },
    @{ Name = 'python.exe'; ExecutablePath = "$root\tools\voice-server\python\python.exe"; ProcessId = 3 },
    @{ Name = 'whisper-server.exe'; ExecutablePath = "$root\tools\voice-server\whispercpp\whisper-server.exe"; ProcessId = 4 },
    @{ Name = 'jarvis-app.exe'; ExecutablePath = "${root}-other\jarvis-app.exe"; ProcessId = 5 },
    @{ Name = 'other.exe'; ExecutablePath = "$root\other.exe"; ProcessId = 6 },
    @{ Name = 'python.exe'; ExecutablePath = 'C:\Python\python.exe'; ProcessId = 7 },
    @{ Name = 'system'; ExecutablePath = $null; ProcessId = 8 }
) | ForEach-Object { [pscustomobject]$_ }
function Get-CimInstance {
    param($ClassName)
    $script:scans++
    if ($script:mode -eq 'delayed' -and $script:scans -eq 4) {
        # The first kill finishes late, after its server child has already spawned.
        $script:mockProcesses = @([pscustomobject]@{
            Name = 'python.exe'; ExecutablePath = "$root\tools\voice-server\python\python.exe"; ProcessId = 9
        })
    }
    $script:mockProcesses
}
function Stop-Process {
    param($Id, [switch]$Force, $ErrorAction)
    $script:stopped += $Id
    if ($script:mode -eq 'refused') {
        Write-Error 'Access denied' -ErrorAction $ErrorAction
        return
    }
    if ($script:mode -eq 'delayed' -and $Id -ne 9) { return }
    $script:mockProcesses = @($script:mockProcesses | Where-Object { $_.ProcessId -ne $Id })
}
function Start-Sleep { param($Milliseconds) }
$stopScript = Join-Path $PSScriptRoot '..\stop-processes.ps1'
. $stopScript -Root $root
if (($script:stopped -join ',') -ne '2,1,3,4') { throw "Unexpected processes: $($script:stopped -join ',')" }
if ($script:scans -ne 4) { throw 'Must verify completion and wait for late children' }
if (($script:mockProcesses.ProcessId -join ',') -ne '5,6,7,8') { throw 'Unrelated processes must survive' }

$script:mode = 'delayed'
$script:scans = 0
$script:stopped = @()
$script:mockProcesses = @([pscustomobject]@{ Name = 'jarvis-app.exe'; ExecutablePath = "$root\jarvis-app.exe"; ProcessId = 1 })
. $stopScript -Root $root
if (($script:stopped -join ',') -ne '1,1,1,9') { throw 'Must stop a late-spawned server and wait for teardown' }
if ($script:mockProcesses.Count -ne 0 -or $script:scans -ne 7) { throw 'Must drain the processes before returning' }

$script:mode = 'refused'
$script:scans = 0
$script:mockProcesses = @([pscustomobject]@{ Name = 'jarvis-app.exe'; ExecutablePath = "$root\jarvis-app.exe"; ProcessId = 1 })
$failed = $false
try { . $stopScript -Root $root } catch { $failed = $_.Exception.Message -match 'jarvis-app.exe' }
if (-not $failed) { throw 'A surviving process must abort the update' }
"OK: scoped filtering, shutdown order, delayed teardown, late children and persistent failure"
