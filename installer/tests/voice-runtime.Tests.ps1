$ErrorActionPreference = "Stop"
# Load only the runtime functions; never execute the installer or download packages.
$source = Join-Path $PSScriptRoot '../../tools/voice-server/install.ps1'
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $source), [ref]$null, [ref]$errors)
if ($errors.Count) { throw "install.ps1 does not parse" }
$names = @('Runtime-Fingerprint', 'Install-VoicePackages', 'Update-VoiceRuntime')
$ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -in $names }, $false) |
    ForEach-Object { Invoke-Expression $_.Extent.Text }

$script:here = Join-Path ([IO.Path]::GetTempPath()) ('jarvis-runtime-test-' + [guid]::NewGuid())
$script:runtimeMarker = Join-Path $here 'models/runtime-version.txt'
$script:f5Package = 'f5-tts==1.1.22'
$script:TtsEngine = 'f5'
$script:engineFile = Join-Path $here 'tts-engine.txt'
$script:SkipModels = $false
$script:pipCalls = @()
$script:downloadCalls = 0
$script:failPackage = ''
$script:modelsOk = $true
function Step($text) {}
$script:gpuCalls = @()
$script:gpuFails = $false
function Ensure-VoiceGpu($profile) {
    $script:gpuCalls += $profile.profile
    if ($script:gpuFails) { throw "GPU unavailable" }
}
function Pip([string[]]$PipArgs) {
    $script:pipCalls += ,$PipArgs
    return ($PipArgs[0] -ne $script:failPackage)
}
function Download-VoiceModels {
    $script:downloadCalls++
    return $script:modelsOk
}
function Assert($condition, $message) { if (-not $condition) { throw $message } }
try {
    New-Item -ItemType Directory $here | Out-Null
    Set-Content (Join-Path $here 'requirements.txt') 'deps-v1'
    Set-Content (Join-Path $here 'requirements-tts.txt') 'tts-deps-v1'
    Set-Content (Join-Path $here 'server.py') 'server-v1'
    Set-Content (Join-Path $here 'gpu-profile.json') '{"profile":"rocm"}'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq 3 -and $downloadCalls -eq 1) 'An old install must get runtime packages and models'
    Assert (($pipCalls[2] -join ' ') -eq 'f5-tts==1.1.22 --no-deps') 'F5 must not replace the installed torch build'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -eq (Runtime-Fingerprint)) 'Successful upgrade needs a marker'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq 3 -and $downloadCalls -eq 1) 'An unchanged runtime must not download again'

    Set-Content (Join-Path $here 'server.py') 'server-v2'
    $script:modelsOk = $false
    $failed = $false
    try { Update-VoiceRuntime } catch { $failed = $true }
    Assert $failed 'A model download failure must be reported'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -ne (Runtime-Fingerprint)) 'A failed download must stay retryable'
    $script:modelsOk = $true
    Update-VoiceRuntime
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -eq (Runtime-Fingerprint)) 'Retry must finish the upgrade'

    Set-Content (Join-Path $here 'requirements.txt') 'deps-v2'
    $script:failPackage = $f5Package
    $before = $downloadCalls
    $failed = $false
    try { Update-VoiceRuntime } catch { $failed = $true }
    Assert ($failed -and $downloadCalls -eq $before) 'A failed package install must not download models'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -ne (Runtime-Fingerprint)) 'A failed package install must stay retryable'

    $script:failPackage = ''
    $script:SkipModels = $true
    Remove-Item $runtimeMarker
    Update-VoiceRuntime
    Assert ($downloadCalls -eq $before -and -not (Test-Path $runtimeMarker)) 'SkipModels must not mark missing models as installed'
    $script:SkipModels = $false
    Set-Content (Join-Path $here 'gpu-profile.json') '{"profile":"cpu"}'
    $script:TtsEngine = 'nano'
    $beforePip = $pipCalls.Count
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq $beforePip + 2) 'A CPU installation must install STT and portable Nano dependencies'
    Assert ($pipCalls[-1][1] -like '*requirements-nano.txt') 'Nano must not install F5 dependencies'
    Assert ((Get-Content $engineFile -Raw).Trim() -eq 'nano') 'RuntimeOnly must select Nano'
    $beforePip = $pipCalls.Count
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq $beforePip) 'Unchanged Nano must skip packages'
    Set-Content (Join-Path $here 'nano-models.json') 'new-pinned-models'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq $beforePip + 2) 'Changed Nano model revisions require an upgrade'
    Set-Content (Join-Path $here 'gpu-profile.json') '{"profile":"rocm"}'
    $beforeGpu = $gpuCalls.Count
    Update-VoiceRuntime
    Assert ($gpuCalls.Count -eq $beforeGpu) 'Nano on AMD must not install or require ROCm'
    Set-Content (Join-Path $here 'gpu-profile.json') '{"profile":"cuda"}'
    $script:gpuFails = $true
    Update-VoiceRuntime
    Assert ((Get-Content $engineFile -Raw).Trim() -eq 'nano') 'Unavailable NVIDIA driver must not disable Nano'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -eq (Runtime-Fingerprint)) 'Portable Nano upgrade must finish with STT CPU fallback'

    'OK: GPU runtime migration, portable Nano CPU, no-op, retries and missing models'
} finally {
    Remove-Item -LiteralPath $here -Recurse -Force
}
