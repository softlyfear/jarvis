$ErrorActionPreference = "Stop"
$source = Join-Path $PSScriptRoot '../../tools/voice-server/install.ps1'
$ast = [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $source), [ref]$null, [ref]$null)
$ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -in @('Ensure-VoiceGpu', 'Get-CudaWheelIndex') }, $false) |
    ForEach-Object { Invoke-Expression $_.Extent.Text }
function Assert($ok, $message) { if (-not $ok) { throw $message } }
function Step($text) {}
$script:rocmIndex = 'https://stable.repo.amd.com/rocm/whl-next/'
$script:pipCalls = @()
$script:gpuChecks = 0
$script:readyAfter = 0
function Test-VoiceGpu($profile) {
    $script:gpuChecks++
    return ($script:gpuChecks -gt $script:readyAfter)
}
function Pip([string[]]$PipArgs) { $script:pipCalls += ,$PipArgs; return $true }
foreach ($kind in @('cuda', 'rocm')) {
    $profile = [pscustomobject]@{ profile = $kind; gfx = 'gfx1100' }
    $script:pipCalls = @(); $script:gpuChecks = 0; $script:readyAfter = 0
    Ensure-VoiceGpu $profile
    Assert ($pipCalls.Count -eq 0) 'A working GPU build must stay untouched'
    $script:gpuChecks = 0; $script:readyAfter = 1
    Ensure-VoiceGpu $profile
    $argsText = ($pipCalls | ForEach-Object { $_ -join ' ' }) -join ' '
    Assert ($argsText -match '--force-reinstall' -and $argsText -notmatch '/whl/cpu') 'An old CPU build must be replaced by a GPU build'
    Assert ($argsText -match $(if ($kind -eq 'cuda') { 'whl/cu126' } else { 'device-gfx1100' })) 'Repair must use the detected GPU profile'
    $script:pipCalls = @(); $script:gpuChecks = 0; $script:readyAfter = 10
    $failed = $false
    try { Ensure-VoiceGpu $profile } catch { $failed = $_.ToString() -match 'CPU' }
    Assert $failed 'A GPU unavailable after repair must stop with an explicit reason'
    Assert ((($pipCalls | ForEach-Object { $_ -join ' ' }) -join ' ') -notmatch '/whl/cpu') 'A GPU failure must never install CPU torch'
}
$script:pipCalls = @(); $script:gpuChecks = 0; $script:readyAfter = 1
Ensure-VoiceGpu ([pscustomobject]@{ profile='cuda'; gpu='NVIDIA GeForce RTX 5050 Laptop GPU' })
Assert ((($pipCalls | ForEach-Object { $_ -join ' ' }) -join ' ') -match '/whl/cu128') 'Blackwell requires a wheel with sm120 kernels'
foreach ($name in @('RTX 5090', 'RTX 5090D', 'RTX 5070Ti', 'NVIDIA RTX PRO 6000 Blackwell', 'NVIDIA B200')) {
    Assert ((Get-CudaWheelIndex ([pscustomobject]@{gpu=$name})) -match '/whl/cu128') 'Fresh Blackwell installs must choose cu128 without torch'
}
'OK: working GPU preserved, old CPU torch repaired, Blackwell cu128, old NVIDIA cu126, GPU failure explicit'
