param([string]$Installer = (Join-Path $PSScriptRoot '../../tools/openclaw/install.ps1'))
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path $PSScriptRoot '../.tmp/openclaw-install-tests'
New-Item -ItemType Directory -Force -Path $testRoot | Out-Null
$testRoot = (Resolve-Path $testRoot).Path
$wrapper = Join-Path $testRoot 'mock-download.ps1'
@'
param([string]$ProfileDir, [string]$Installer, [string]$Mode)
function Invoke-WebRequest {
    param([string]$Uri, [string]$OutFile, [switch]$UseBasicParsing)
    if ($OutFile) { [IO.File]::WriteAllText($OutFile, 'verified-runtime-fixture'); return }
    $fixture = Join-Path $ProfileDir 'runtime/node-v24.16.0-win-x64.zip'
    $hash = (Get-FileHash $fixture -Algorithm SHA256).Hash
    if ($Mode -eq 'mismatch') { $hash = '0' * 64 }
    [pscustomobject]@{Content = "$hash  node-v24.16.0-win-x64.zip"}
}
function Expand-Archive {
    param([string]$Path, [string]$DestinationPath, [switch]$Force)
    New-Item -ItemType Directory -Force -Path (Join-Path $DestinationPath 'node-v24.16.0-win-x64') | Out-Null
    [IO.File]::WriteAllText((Join-Path $DestinationPath 'node-v24.16.0-win-x64/node.exe'), 'node-fixture')
}
& $Installer -ProfileDir $ProfileDir
exit $LASTEXITCODE
'@ | Set-Content -LiteralPath $wrapper -Encoding utf8
$shell = (Get-Process -Id $PID).Path
$cases = @('existing', 'download', 'mismatch')
if ($env:OS -eq 'Windows_NT') { $shell = (Get-Command powershell.exe).Source; $cases += @('npm-warning', 'npm-failure') }
foreach ($case in $cases) {
    $profile = Join-Path $testRoot ($case + '-' + [guid]::NewGuid().ToString('N'))
    $runtime = Join-Path $profile 'runtime'
    $package = Join-Path $runtime 'node_modules/openclaw'
    New-Item -ItemType Directory -Force -Path $package | Out-Null
    [IO.File]::WriteAllText((Join-Path $package 'package.json'), '{"version":"2026.9.8"}')
    [IO.File]::WriteAllText((Join-Path $package 'openclaw.mjs'), 'gateway-fixture')
    [IO.File]::WriteAllText((Join-Path $profile 'MEMORY.md'), 'user-memory')
    if ($case -in @('existing', 'npm-warning', 'npm-failure')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $runtime 'node-v24.16.0-win-x64') | Out-Null
        [IO.File]::WriteAllText((Join-Path $runtime 'node-v24.16.0-win-x64/node.exe'), 'existing-node')
    }
    if ($case -in @('npm-warning', 'npm-failure')) {
        [IO.File]::WriteAllText((Join-Path $package 'package.json'), '{"version":"old"}')
        $env:JARVIS_TEST_PACKAGE = Join-Path $package 'package.json'
        $exitCode = if ($case -eq 'npm-failure') { 51 } else { 0 }
        $cmd = "@echo off`r`necho npm WARN fixture 1>&2`r`necho {`"version`":`"2026.9.8`"}>`"%JARVIS_TEST_PACKAGE%`"`r`nexit /b $exitCode`r`n"
        [IO.File]::WriteAllText((Join-Path $runtime 'node-v24.16.0-win-x64/npm.cmd'), $cmd)
    }
    & $shell -NoProfile -NonInteractive -File $wrapper -ProfileDir $profile -Installer (Resolve-Path $Installer).Path -Mode $case
    $code = $LASTEXITCODE
    $expectedFailure = $case -in @('mismatch', 'npm-failure')
    if (($expectedFailure -and $code -eq 0) -or (-not $expectedFailure -and $code -ne 0)) { throw "Installer case $case failed: $code" }
    if ((Get-Content -Raw (Join-Path $profile 'MEMORY.md')) -ne 'user-memory') { throw 'Installer changed memory' }
    if ($case -eq 'mismatch' -and (Test-Path (Join-Path $runtime 'node-v24.16.0-win-x64/node.exe'))) { throw 'Unverified archive was extracted' }
    "PASS OpenClaw installer: $case"
}
