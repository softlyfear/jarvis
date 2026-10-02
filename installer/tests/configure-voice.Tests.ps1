$ErrorActionPreference = "Stop"
function Assert($ok, $message) { if (-not $ok) { throw $message } }
$root = Join-Path ([IO.Path]::GetTempPath()) ('jarvis-config-voice-' + [guid]::NewGuid())
$template = (Resolve-Path (Join-Path $PSScriptRoot '../../crates/jarvis-core/assets/assistant.default.toml')).Path
$configure = (Resolve-Path (Join-Path $PSScriptRoot '../configure.ps1')).Path
try {
    New-Item -ItemType Directory $root | Out-Null
    foreach ($backend in @('sapi', 'http', 'none')) {
        $dir = Join-Path $root $backend
        New-Item -ItemType Directory $dir | Out-Null
        $config = Join-Path $dir 'assistant.toml'
        $text = "[assistant]`r`naddress = `"мисс`"`r`n[tts]`r`nbackend = `"$backend`"`r`nsapi_rate = 2`r`nsapi_voice = `"Irina`"`r`nhttp_fallback_sapi = true`r`nhttp_timeout_secs = 45`r`n[custom]`r`nbackend = `"sapi`"`r`nkeep = 123`r`n"
        [IO.File]::WriteAllText($config, $text, [Text.UTF8Encoding]::new($false))
        & $configure -Template $template -ConfigDir $dir
        $out = [IO.File]::ReadAllText($config)
        $expected = if ($backend -eq 'none') { 'none' } else { 'http' }
        Assert ($out -match ('(?s)\[tts\].*?backend = "' + $expected + '"')) 'The legacy voice backend must migrate'
        Assert ($out -notmatch '(?m)^(sapi_voice|sapi_rate|http_fallback_sapi)') 'Retired voice options must be removed'
        Assert ($out.Contains('address = "мисс"') -and $out.Contains('http_timeout_secs = 45')) 'Address and timeout must be retained'
        Assert ($out.Contains("[custom]`r`nbackend = `"sapi`"`r`nkeep = 123")) 'Other tables must be retained'
    }
    'OK: legacy SAPI migration, explicit none preserved, other settings retained'
} finally { Remove-Item -LiteralPath $root -Recurse -Force }
