param([Parameter(Mandatory=$true)][string]$ProfileDir)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$nodeVersion = '24.16.0'
$openclawVersion = '2026.9.8'
$runtime = Join-Path $ProfileDir 'runtime'
$nodeDir = Join-Path $runtime "node-v$nodeVersion-win-x64"
$node = Join-Path $nodeDir 'node.exe'
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
$log = Join-Path $ProfileDir 'setup.log'
try {
    if (-not (Test-Path $node)) {
        'Загрузка среды OpenClaw...' | Out-File $log -Encoding utf8
        $archive = "node-v$nodeVersion-win-x64.zip"
        $base = "https://nodejs.org/dist/v$nodeVersion"
        $zip = Join-Path $runtime $archive
        Invoke-WebRequest "$base/$archive" -OutFile $zip -UseBasicParsing
        $sums = (Invoke-WebRequest "$base/SHASUMS256.txt" -UseBasicParsing).Content
        $line = ($sums -split "`n" | Where-Object { $_ -match ('\s+' + [regex]::Escape($archive) + '\s*$') } | Select-Object -First 1)
        if (-not $line -or (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne ($line -split '\s+')[0]) { throw 'Проверка загруженной среды не прошла.' }
        Expand-Archive -Path $zip -DestinationPath $runtime -Force
        Remove-Item -LiteralPath $zip
    }
    $env:PATH = "$nodeDir;$env:PATH"
    $env:npm_config_cache = Join-Path $ProfileDir 'cache'
    $package = Join-Path $runtime 'node_modules/openclaw/package.json'
    $installed = if (Test-Path $package) { (Get-Content -Raw $package | ConvertFrom-Json).version } else { '' }
    if ($installed -ne $openclawVersion) {
        'Установка OpenClaw...' | Out-File $log -Append -Encoding utf8
        $ErrorActionPreference = 'Continue'
        & (Join-Path $nodeDir 'npm.cmd') install --prefix $runtime --omit=dev --no-audit --no-fund "openclaw@$openclawVersion" 2>&1 | Out-File $log -Append -Encoding utf8
        $npmExitCode = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if ($npmExitCode -ne 0) { throw 'Не удалось установить OpenClaw. Проверьте интернет и setup.log.' }
    }
    if ((Get-Content -Raw $package | ConvertFrom-Json).version -ne $openclawVersion) { throw 'Версия OpenClaw не совпала с ожидаемой.' }
    if (-not (Test-Path (Join-Path $runtime 'node_modules/openclaw/openclaw.mjs'))) { throw 'Не найден Gateway OpenClaw.' }
} catch {
    'Установка OpenClaw не завершена.' | Out-File $log -Append -Encoding utf8
    exit 1
}
