$ErrorActionPreference = "Stop"
# Load only the runtime functions; never execute the installer or download packages.
$source = Join-Path $PSScriptRoot '../../tools/voice-server/install.ps1'
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $source), [ref]$null, [ref]$errors)
if ($errors.Count) { throw "install.ps1 does not parse" }
$names = @('Runtime-Fingerprint', 'Install-VoicePackages', 'Update-VoiceRuntime', 'Remove-RetiredVoice')
$ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -in $names }, $false) |
    ForEach-Object { Invoke-Expression $_.Extent.Text }

$script:here = Join-Path ([IO.Path]::GetTempPath()) ('jarvis-runtime-test-' + [guid]::NewGuid())
$script:runtimeMarker = Join-Path $here 'models/runtime-version.txt'
$script:f5Package = 'f5-tts==1.1.22'
$script:TtsEngine = ($ast.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'TtsEngine' }).DefaultValue.Value
if ($script:TtsEngine -ne 'f5') { throw 'Default engine must be F5' }
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
    Set-Content $engineFile 'nano'
    New-Item -ItemType Directory -Force (Join-Path $here 'models/nano') | Out-Null
    New-Item -ItemType Directory -Force (Join-Path $here 'models/f5') | Out-Null
    Set-Content (Join-Path $here 'models/nano/old.bin') 'obsolete'
    Set-Content (Join-Path $here 'models/f5/keep.bin') 'keep'
    Set-Content (Join-Path $here 'nano.py') 'obsolete'
    Set-Content (Join-Path $here 'assistant.toml') 'keep settings'
    Update-VoiceRuntime
    Assert ((Get-Content $engineFile -Raw).Trim() -eq 'f5') 'Old Nano selection must migrate to F5'
    Assert (-not (Test-Path (Join-Path $here 'models/nano')) -and -not (Test-Path (Join-Path $here 'nano.py'))) 'Old Nano files must be removed'
    Assert ((Get-Content (Join-Path $here 'models/f5/keep.bin') -Raw).Trim() -eq 'keep') 'F5 weights must stay'
    Assert ((Get-Content (Join-Path $here 'assistant.toml') -Raw).Trim() -eq 'keep settings') 'Settings must stay'
    Assert ($pipCalls.Count -eq 3 -and $downloadCalls -eq 1) 'An old install must get runtime packages and models'
    Assert (($pipCalls[2] -join ' ') -eq 'f5-tts==1.1.22 --no-deps') 'F5 must not replace the installed torch build'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -eq (Runtime-Fingerprint)) 'Successful upgrade needs a marker'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq 3 -and $downloadCalls -eq 1) 'An unchanged runtime must not download again'

    Set-Content (Join-Path $here 'server.py') 'server-v2'
    Set-Content (Join-Path $here 'nano.py') 'pending migration'
    $script:modelsOk = $false
    $failed = $false
    try { Update-VoiceRuntime } catch { $failed = $true }
    Assert $failed 'A model download failure must be reported'
    Assert (Test-Path (Join-Path $here 'nano.py')) 'Failed setup must not remove old files before retry'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -ne (Runtime-Fingerprint)) 'A failed download must stay retryable'
    $script:modelsOk = $true
    Update-VoiceRuntime
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -eq (Runtime-Fingerprint)) 'Retry must finish the upgrade'
    Assert (-not (Test-Path (Join-Path $here 'nano.py'))) 'Successful retry must clean old files'

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
    foreach ($kind in @('cpu', 'vulkan')) {
        Set-Content (Join-Path $here 'gpu-profile.json') ('{"profile":"' + $kind + '"}')
        $beforePip = $pipCalls.Count
        $beforeGpu = $gpuCalls.Count
        Update-VoiceRuntime
        Assert ($pipCalls.Count -eq $beforePip + 1 -and $gpuCalls.Count -eq $beforeGpu) 'Unsupported GPU must install recognition without another synthesizer'
        Assert ((Get-Content $engineFile -Raw).Trim() -eq 'f5') 'F5 stays the sole engine on every profile'
    }
    Set-Content (Join-Path $here 'gpu-profile.json') '{"profile":"cuda"}'
    $script:gpuFails = $true
    $before = $downloadCalls
    $failed = $false
    try { Update-VoiceRuntime } catch { $failed = $true }
    Assert ($failed -and $downloadCalls -eq $before) 'A supported but unavailable GPU must fail before downloading'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -ne (Runtime-Fingerprint)) 'Missing GPU must stay retryable'
    $script:gpuFails = $false
    Update-VoiceRuntime
    $beforePip = $pipCalls.Count
    Set-Content (Join-Path $here 'nano.py') 'leftover'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq $beforePip -and -not (Test-Path (Join-Path $here 'nano.py'))) 'No-op upgrade must still clean retired files'
    'OK: F5 default, Nano migration and cleanup, CUDA/ROCm, CPU/Vulkan recognition, retry and no-op'

} finally {
    Remove-Item -LiteralPath $here -Recurse -Force
}
