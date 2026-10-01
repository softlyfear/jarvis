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
$script:SkipModels = $false
$script:pipCalls = @()
$script:downloadCalls = 0
$script:failPackage = ''
$script:modelsOk = $true
function Step($text) {}
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
    Set-Content (Join-Path $here 'server.py') 'server-v1'
    Set-Content (Join-Path $here 'gpu-profile.json') '{"profile":"rocm"}'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq 2 -and $downloadCalls -eq 1) 'An old install must get runtime packages and models'
    Assert (($pipCalls[1] -join ' ') -eq 'f5-tts==1.1.22 --no-deps') 'F5 must not replace the installed torch build'
    Assert ((Get-Content $runtimeMarker -Raw).Trim() -eq (Runtime-Fingerprint)) 'Successful upgrade needs a marker'
    Update-VoiceRuntime
    Assert ($pipCalls.Count -eq 2 -and $downloadCalls -eq 1) 'An unchanged runtime must not download again'

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
    'OK: runtime migration, no-op, retries and missing models'
} finally {
    Remove-Item -LiteralPath $here -Recurse -Force
}
