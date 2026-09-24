# Writes the first assistant.toml during installation: Gemini keys and voice settings.
# Keys come from a temporary file (not the command line) and are never printed.
param(
    [Parameter(Mandatory = $true)][string]$Template,
    [string]$ConfigDir = (Join-Path $env:APPDATA "com.priler.jarvis"),
    [string]$KeysFile = "",
    [switch]$VoiceClone,
    # "sir" | "miss" | "" (keep); ASCII on the command line, the word is written here
    [string]$Address = ""
)

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)   # TOML must not start with a BOM

New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
$config = Join-Path $ConfigDir "assistant.toml"
if (-not (Test-Path $config)) {
    Copy-Item $Template $config
}
$text = [System.IO.File]::ReadAllText($config, $utf8)
# what happened, for bug reports ("Собрать логи"); never the keys themselves
$report = New-Object System.Collections.Generic.List[string]
$report.Add("configure.ps1 $(Get-Date -Format s): keys file '$KeysFile' exists=$([bool]($KeysFile -and (Test-Path $KeysFile)))")

if ($KeysFile -and (Test-Path $KeysFile)) {
    $raw = [System.IO.File]::ReadAllText($KeysFile, $utf8)
    Remove-Item $KeysFile -Force
    # "AIza..." and the newer "AQ.xxxx" keys (with a dot)
    $keys = @($raw -split "[,;\s]+" | Where-Object { $_ -match '^[A-Za-z0-9_.\-]{20,}$' } | Select-Object -Unique)
    $report.Add("keys in the file: $(@($raw -split '[,;\s]+' | Where-Object { $_ }).Count), accepted: $($keys.Count)")
    if ($keys.Count -gt 0) {
        $list = ($keys | ForEach-Object { '"' + $_ + '"' }) -join ", "
        # the keys line of the gemini provider block
        $pattern = '(?s)(name\s*=\s*"gemini".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + "[" + $list + "]" }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            Write-Host "Gemini keys saved: $($keys.Count)"
            $report.Add("gemini block found, keys written")
        } else {
            Write-Warning "Gemini block not found in $config"
            $report.Add("gemini block NOT found")
        }
    }
}

$word = @{ "sir" = "сэр"; "miss" = "мисс" }[$Address]
if ($word) {
    if ($text -match '(?m)^\s*address\s*=') {
        $text = ([regex]'(?m)^(\s*address\s*=\s*)"[^"\r\n]*"').Replace($text, '${1}"' + $word + '"', 1)
    } else {
        # a config from before the setting: the table goes first, before [llm]
        $text = "[assistant]`r`naddress = `"$word`"`r`n`r`n" + $text
    }
    $report.Add("address: $Address")
}

if ($VoiceClone) {
    $text = ([regex]'(?m)^(\s*backend\s*=\s*)"sapi"').Replace($text, '$1"http"', 1)
}

[System.IO.File]::WriteAllText($config, $text, $utf8)
$report.Add("written: $config")
[System.IO.File]::AppendAllText((Join-Path $ConfigDir "configure.log"), (($report -join "`r`n") + "`r`n"), $utf8)
Write-Host "Settings: $config"
