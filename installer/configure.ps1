# Writes the first assistant.toml during installation: the Kilo key and voice settings.
# The key comes from a temporary file (not the command line) and is never printed.
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
    # one key per Kilo account; copied from the profile page it may be wrapped over lines
    $key = $raw -replace '\s', ''
    $report.Add("key length: $($key.Length), accepted: $($key -match '^[A-Za-z0-9_.\-]{20,}$')")
    if ($key -match '^[A-Za-z0-9_.\-]{20,}$') {
        # the keys line of the kilo provider block
        $pattern = '(?s)(name\s*=\s*"kilo".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + '["' + $key + '"]' }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            $report.Add("kilo block found, key written")
        } else {
            # a config from before Kilo: an array-of-tables block may go at the end of the file
            $block = "`r`n[[llm.providers]]`r`nname = `"kilo`"`r`nenabled = true`r`nbase_url = `"https://api.kilo.ai/api/gateway`"`r`n" +
                "models = [`"google/gemini-3.5-flash-lite`", `"deepseek/deepseek-v4-flash`", `"google/gemini-3.5-flash`"]`r`nkeys = [`"$key`"]`r`n"
            $text = $text.TrimEnd() + "`r`n" + $block
            # the old Gemini block would be asked first (and hangs without a VPN): switched off, keys kept
            $text = ([regex]'(?s)(name\s*=\s*"gemini".*?\n\s*enabled\s*=\s*)true').Replace($text, '${1}false', 1)
            $report.Add("kilo block added, gemini switched off")
        }
        Write-Host "Kilo key saved"
    } else {
        Write-Warning "The Kilo key looks wrong and was not saved"
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
