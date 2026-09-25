# Writes the first assistant.toml during installation: the gateway key and voice settings.
# Kilo keys are JWTs ("eyJ…"); any other key is Polza AI's (works from Russia without a VPN).
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
    $isKilo = $key.StartsWith("eyJ")
    $report.Add("key length: $($key.Length), gateway: $(if ($isKilo) { 'kilo' } else { 'polza' }), accepted: $($key -match '^[A-Za-z0-9_.\-]{20,}$')")
    if (($key -match '^[A-Za-z0-9_.\-]{20,}$') -and -not $isKilo) {
        $pattern = '(?ms)(^\s*name\s*=\s*"polza".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + '["' + $key + '"]' }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            $report.Add("polza block found, key written")
        } else {
            # before the first provider block, so Polza is asked first and Kilo stays the fallback
            $block = "[[llm.providers]]`r`nname = `"polza`"`r`nenabled = true`r`nbase_url = `"https://polza.ai/api/v1`"`r`n" +
                "models = [`"google/gemini-3.5-flash-lite`", `"google/gemini-3.5-flash`", `"deepseek/deepseek-v4-flash`"]`r`nkeys = [`"$key`"]`r`n`r`n"
            $first = ([regex]'(?m)^\[\[llm\.providers\]\]').Match($text)
            if ($first.Success) {
                $text = $text.Insert($first.Index, $block)
            } else {
                $text = $text.TrimEnd() + "`r`n`r`n" + $block
            }
            $report.Add("polza block added")
        }
        Write-Host "Polza AI key saved"
    } elseif ($key -match '^[A-Za-z0-9_.\-]{20,}$') {
        # the keys line of the kilo provider block (a line of its own: comments mention the names too)
        $pattern = '(?ms)(^\s*name\s*=\s*"kilo".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + '["' + $key + '"]' }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            $report.Add("kilo block found, key written")
        } else {
            # a config from before Kilo: an array-of-tables block may go at the end of the file
            $block = "`r`n[[llm.providers]]`r`nname = `"kilo`"`r`nenabled = true`r`nbase_url = `"https://api.kilo.ai/api/gateway`"`r`n" +
                "models = [`"google/gemini-3.5-flash-lite`", `"google/gemini-3.5-flash`", `"deepseek/deepseek-v4-flash`"]`r`nkeys = [`"$key`"]`r`n"
            $text = $text.TrimEnd() + "`r`n" + $block
            $report.Add("kilo block added")
        }
        Write-Host "Kilo key saved"
    } else {
        Write-Warning "The key looks wrong and was not saved"
    }
}

# versions before Kilo had a Gemini block: Kilo is the only gateway now, the block and its keys go
$gemini = '(?m)^\[\[llm\.providers\]\][ \t]*\r?\n(?:[^\[\r\n][^\r\n]*\r?\n|\r?\n)*?name\s*=\s*"gemini"[^\r\n]*\r?\n(?:[^\[\r\n][^\r\n]*\r?\n|\r?\n)*'
if ($text -match $gemini) {
    $text = [regex]::Replace($text, $gemini, '')
    # and the comment about Gemini keys above it
    $text = [regex]::Replace($text, '(?ms)^# Нейросеть — Google Gemini\..*?(?=^\[)', '')
    $report.Add("gemini block removed")
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
