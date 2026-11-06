# Writes the first assistant.toml during installation: the gateway key and voice settings.
# OpenRouter keys start with "sk-or-". Any other accepted key is Polza AI's (works from Russia
# without a VPN). A Kilo JWT ("eyJ…") is not a key for either gateway.
# The key comes from a temporary file (not the command line) and is never printed.
param(
    [Parameter(Mandatory = $true)][string]$Template,
    [string]$ConfigDir = (Join-Path $env:APPDATA "com.priler.jarvis"),
    [string]$KeysFile = "",
    # "sir" | "miss" | "" (keep); ASCII on the command line, the word is written here
    [string]$Address = ""
)

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)   # TOML must not start with a BOM
$paidModels = '["~deepseek/deepseek-v4-flash-latest"]'
$polzaModels = '["deepseek/deepseek-v4-flash"]'
$openRouterUrl = 'https://openrouter.ai/api/v1'

New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
$config = Join-Path $ConfigDir "assistant.toml"
if (-not (Test-Path $config)) {
    Copy-Item $Template $config
}
$text = [System.IO.File]::ReadAllText($config, $utf8)
# what happened, for bug reports ("Собрать логи"); never the keys themselves
$report = New-Object System.Collections.Generic.List[string]
$report.Add("configure.ps1 $(Get-Date -Format s): keys file '$KeysFile' exists=$([bool]($KeysFile -and (Test-Path $KeysFile)))")

# versions before Kilo had a Gemini block: drop it, together with the comment about its keys
$gemini = '(?m)^\[\[llm\.providers\]\][ \t]*\r?\n(?:[^\[\r\n][^\r\n]*\r?\n|\r?\n)*?name\s*=\s*"gemini"[^\r\n]*\r?\n(?:[^\[\r\n][^\r\n]*\r?\n|\r?\n)*'
if ($text -match $gemini) {
    $text = [regex]::Replace($text, $gemini, '')
    $text = [regex]::Replace($text, '(?ms)^# Нейросеть — Google Gemini\..*?(?=^\[)', '')
    $report.Add("gemini block removed")
}

# Kilo and the old "paid" block become OpenRouter. A keyless block is left alone.
# Only the previous default model list is replaced; a hand-edited list stays.
$text = [regex]::Replace($text, '(?ms)^\[\[llm\.providers\]\][^\r\n]*\r?\n.*?(?=^\[|\z)', {
    param($match)
    $block = $match.Value
    $keyless = $block -match '(?mi)^[ \t]*keyless[ \t]*=[ \t]*true\b'
    $isGateway = $block -match '(?mi)^[ \t]*name[ \t]*=[ \t]*"(?:openrouter|kilo|polza|paid)"[ \t]*(?:#[^\r\n]*)?\r?$'
    if ($isGateway -and -not $keyless) {
        $isPolza = $block -match '(?mi)^[ \t]*name[ \t]*=[ \t]*"polza"'
        $oldModels = '(?m)^([ \t]*models[ \t]*=[ \t]*)\[\s*(?:"anthropic/claude-haiku-5\.5"\s*,\s*)?"google/gemini-3\.5-flash-lite"\s*,\s*"google/gemini-3\.5-flash"\s*,\s*"deepseek/deepseek-v4-flash"\s*\]'
        $gatewayModels = if ($isPolza) { $polzaModels } else { $paidModels }
        $block = [regex]::Replace($block, $oldModels, {
            param($modelsMatch)
            $modelsMatch.Groups[1].Value + $gatewayModels
        })
        if (-not $isPolza) {
            $block = [regex]::Replace($block, '(?m)^([ \t]*name[ \t]*=[ \t]*)"(?:kilo|paid)"', '${1}"openrouter"')
            if ($block -match '(?m)^[ \t]*base_url[ \t]*=') {
                $block = [regex]::Replace($block, '(?m)^([ \t]*base_url[ \t]*=[ \t]*)"[^"]*"', '${1}"' + $openRouterUrl + '"')
            }
            $block = [regex]::Replace($block, '(?m)^([ \t]*keys[ \t]*=[ \t]*\[)([^\]]*)(\])', {
                param($keysMatch)
                $kept = @([regex]::Matches($keysMatch.Groups[2].Value, '"([^"]*)"') | ForEach-Object { $_.Groups[1].Value } | Where-Object { -not $_.StartsWith('eyJ') })
                $joined = ($kept | ForEach-Object { '"' + $_ + '"' }) -join ', '
                $keysMatch.Groups[1].Value + $joined + $keysMatch.Groups[3].Value
            })
        }
    }
    $block
})

if ($KeysFile -and (Test-Path $KeysFile)) {
    $raw = [System.IO.File]::ReadAllText($KeysFile, $utf8)
    Remove-Item $KeysFile -Force
    # a key copied from a page may be wrapped over lines
    $key = $raw -replace '\s', ''
    $isOpenRouter = $key.StartsWith("sk-or-")
    $isRetiredKilo = $key.StartsWith("eyJ")
    $gatewayName = if ($isOpenRouter) { 'openrouter' } elseif ($isRetiredKilo) { 'kilo' } else { 'polza' }
    $report.Add("key length: $($key.Length), gateway: $gatewayName, accepted: $($key -match '^[A-Za-z0-9_.\-]{20,}$')")
    if ($isRetiredKilo) {
        Write-Warning "Kilo keys are no longer used. Create a key at openrouter.ai (sk-or-…) or polza.ai."
        $report.Add("kilo key rejected")
    } elseif (($key -match '^[A-Za-z0-9_.\-]{20,}$') -and -not $isOpenRouter) {
        $pattern = '(?ms)(^\s*name\s*=\s*"polza".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + '["' + $key + '"]' }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            $report.Add("polza block found, key written")
        } else {
            # before the first provider block, so Polza is asked first and OpenRouter stays the fallback
            $block = "[[llm.providers]]`r`nname = `"polza`"`r`nenabled = true`r`nbase_url = `"https://polza.ai/api/v1`"`r`n" +
                "models = $polzaModels`r`nkeys = [`"$key`"]`r`n`r`n"
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
        # comments mention the name too, so the keys line has to belong to the openrouter block
        $pattern = '(?ms)(^\s*name\s*=\s*"openrouter".*?\n\s*keys\s*=\s*)\[[^\]]*\]'
        if ($text -match $pattern) {
            $evaluator = [System.Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + '["' + $key + '"]' }
            $text = ([regex]$pattern).Replace($text, $evaluator, 1)
            $report.Add("openrouter block found, key written")
        } else {
            $block = "`r`n[[llm.providers]]`r`nname = `"openrouter`"`r`nenabled = true`r`nbase_url = `"$openRouterUrl`"`r`n" +
                "models = $paidModels`r`nkeys = [`"$key`"]`r`n"
            $text = $text.TrimEnd() + "`r`n" + $block
            $report.Add("openrouter block added")
        }
        Write-Host "OpenRouter key saved"
    } else {
        Write-Warning "The key looks wrong and was not saved"
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

# Migrate only the TTS table; retain an explicit "none" and all unrelated settings.
$text = [regex]::Replace($text, '(?ms)^\[tts\][^\r\n]*\r?\n.*?(?=^\[|\z)', {
    param($match)
    $block = [regex]::Replace($match.Value, '(?m)^(\s*backend\s*=\s*)"sapi"', '$1"http"')
    [regex]::Replace($block, '(?m)^[ \t]*(?:sapi_voice|sapi_rate|http_fallback_sapi)[ \t]*=[^\r\n]*\r?\n?', '')
})

[System.IO.File]::WriteAllText($config, $text, $utf8)
$report.Add("written: $config")
[System.IO.File]::AppendAllText((Join-Path $ConfigDir "configure.log"), (($report -join "`r`n") + "`r`n"), $utf8)
Write-Host "Settings: $config"
