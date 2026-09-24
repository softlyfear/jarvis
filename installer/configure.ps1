# Writes the first assistant.toml during installation: Gemini keys and voice settings.
# Keys come from a temporary file (not the command line) and are never printed.
param(
    [Parameter(Mandatory = $true)][string]$Template,
    [string]$ConfigDir = (Join-Path $env:APPDATA "com.priler.jarvis"),
    [string]$KeysFile = "",
    [switch]$VoiceClone
)

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)   # TOML must not start with a BOM

New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
$config = Join-Path $ConfigDir "assistant.toml"
if (-not (Test-Path $config)) {
    Copy-Item $Template $config
}
$text = [System.IO.File]::ReadAllText($config, $utf8)

if ($KeysFile -and (Test-Path $KeysFile)) {
    $raw = [System.IO.File]::ReadAllText($KeysFile, $utf8)
    Remove-Item $KeysFile -Force
    $keys = @($raw -split "[,;\s]+" | Where-Object { $_ -match '^[A-Za-z0-9_\-]{20,}$' } | Select-Object -Unique)
    if ($keys.Count -gt 0) {
        $list = ($keys | ForEach-Object { '"' + $_ + '"' }) -join ", "
        # the keys line of the gemini provider block
        $pattern = '(?s)(name\s*=\s*"gemini".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + "[" + $list + "]" }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            Write-Host "Gemini keys saved: $($keys.Count)"
        } else {
            Write-Warning "Gemini block not found in $config"
        }
    }
}

if ($VoiceClone) {
    $text = ([regex]'(?m)^(\s*backend\s*=\s*)"sapi"').Replace($text, '$1"http"', 1)
}

[System.IO.File]::WriteAllText($config, $text, $utf8)
Write-Host "Settings: $config"
