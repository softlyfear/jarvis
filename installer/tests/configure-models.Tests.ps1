$ErrorActionPreference = "Stop"
function Assert($ok, $message) { if (-not $ok) { throw $message } }
$root = Join-Path ([IO.Path]::GetTempPath()) ('jarvis-config-models-' + [guid]::NewGuid())
$template = (Resolve-Path (Join-Path $PSScriptRoot '../../crates/jarvis-core/assets/assistant.default.toml')).Path
$configure = (Resolve-Path (Join-Path $PSScriptRoot '../configure.ps1')).Path
$utf8 = New-Object Text.UTF8Encoding($false)
$oldModels = '["google/gemini-3.5-flash-lite", "google/gemini-3.5-flash", "deepseek/deepseek-v4-flash"]'
$models = '["anthropic/claude-haiku-5.5", "google/gemini-3.5-flash-lite", "google/gemini-3.5-flash", "deepseek/deepseek-v4-flash"]'
try {
    New-Item -ItemType Directory $root | Out-Null
    foreach ($gateway in @('kilo', 'polza', 'paid', 'custom', 'keyless')) {
        $dir = Join-Path $root $gateway
        New-Item -ItemType Directory $dir | Out-Null
        $config = Join-Path $dir 'assistant.toml'
        $name = if ($gateway -eq 'keyless') { 'kilo' } else { $gateway }
        $extra = if ($gateway -eq 'keyless') { "keyless = true`r`n" } else { '' }
        $input = "[llm]`r`nfree_only = true`r`n[[llm.providers]]`r`nname = `"$name`"`r`nmodels = $oldModels # keep comment`r`nkeys = [`"first`", `"second`"]`r`n$extra[custom]`r`nmodels = $oldModels`r`n"
        [IO.File]::WriteAllText($config, $input, $utf8)
        & $configure -Template $template -ConfigDir $dir
        $out = [IO.File]::ReadAllText($config)
        if ($gateway -in @('custom', 'keyless')) {
            Assert ($out -eq $input) 'Custom and keyless providers must stay unchanged'
        } else {
            Assert ($out.Contains("models = $models # keep comment")) 'Haiku must precede the old fallback models'
            Assert ($out.Contains('keys = ["first", "second"]')) 'Rotation keys must stay unchanged'
            Assert ($out.Contains('free_only = true')) 'Free-only must stay unchanged'
            Assert ($out.Contains("[custom]`r`nmodels = $oldModels")) 'Other tables must stay unchanged'
        }
        & $configure -Template $template -ConfigDir $dir
        Assert ([IO.File]::ReadAllText($config) -eq $out) 'Migration must be idempotent'
    }
    foreach ($case in @(
        @{ Name = 'fresh-kilo'; Key = 'eyJFAKE0123456789abcdefghijkl'; Gateway = 'kilo' },
        @{ Name = 'fresh-polza'; Key = 'sk-polza-FAKE0123456789abcdef'; Gateway = 'polza' }
    )) {
        $dir = Join-Path $root $case.Name
        $keys = Join-Path $root 'keys.txt'
        [IO.File]::WriteAllText($keys, $case.Key, $utf8)
        & $configure -Template $template -ConfigDir $dir -KeysFile $keys
        $out = [IO.File]::ReadAllText((Join-Path $dir 'assistant.toml'))
        $pattern = '(?s)name = "' + $case.Gateway + '".*?models = ' + [regex]::Escape($models)
        Assert ($out -match $pattern) 'New gateway blocks must use Haiku first'
        Assert (-not (Test-Path $keys)) 'The temporary key file must be removed'
        Assert (-not ([IO.File]::ReadAllText((Join-Path $dir 'configure.log')).Contains($case.Key))) 'The key must not appear in logs'
    }
    $dir = Join-Path $root 'custom-models'
    New-Item -ItemType Directory $dir | Out-Null
    $config = Join-Path $dir 'assistant.toml'
    $custom = "[[llm.providers]]`r`nname = `"polza`"`r`nmodels = [`"google/gemini-3.5-flash`"]`r`nkeys = []`r`n"
    [IO.File]::WriteAllText($config, $custom, $utf8)
    & $configure -Template $template -ConfigDir $dir
    Assert ([IO.File]::ReadAllText($config) -eq $custom) 'Hand-edited gateway models must stay unchanged'
    'OK: Haiku defaults, upgrade, custom models, free-only, keys and idempotence'
} finally { Remove-Item -LiteralPath $root -Recurse -Force }
