# Installs the Jarvis voice server: a private Python 3.12 inside this folder (python\), with
# Whisper + F5-TTS ESpeech (Jarvis New). Files and caches stay in this folder.
# NVIDIA uses CUDA; supported AMD uses ROCm for F5 and Vulkan for recognition.
# Other GPUs and CPU support recognition and recorded replies, without synthesized speech.
# Used by setup.bat and by the Jarvis installer. Safe to run again (repairs/updates).
param(
    [switch]$NoPause,
    [switch]$SkipModels,
    # force a profile instead of detecting: cuda | rocm | vulkan | cpu
    [string]$GpuProfile = "",
    # run by JarvisSetup.exe without a console: machine-readable pip progress for its progress page
    [switch]$Installer,
    # update an existing server without replacing its working Python/PyTorch/GPU profile
    [switch]$RuntimeOnly,
    # Compatibility with setup.bat -TtsEngine f5; no other synthesizer is installed.
    [ValidateSet("f5")]
    [string]$TtsEngine = "f5"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is very slow with the progress bar
$env:PYTHONUTF8 = "1"                       # Python output decoded as UTF-8 like ours

# A click inside the console window starts QuickEdit selection, which pauses the script until
# Esc/Enter, so a download looks frozen. Switch QuickEdit off for this window.
try {
    Add-Type -Namespace Jarvis -Name ConsoleMode -MemberDefinition @"
[DllImport("kernel32.dll")] public static extern System.IntPtr GetStdHandle(int n);
[DllImport("kernel32.dll")] public static extern bool GetConsoleMode(System.IntPtr h, out uint m);
[DllImport("kernel32.dll")] public static extern bool SetConsoleMode(System.IntPtr h, uint m);
"@
    $stdin = [Jarvis.ConsoleMode]::GetStdHandle(-10)
    $mode = [uint32]0
    if ([Jarvis.ConsoleMode]::GetConsoleMode($stdin, [ref]$mode)) {
        # ENABLE_QUICK_EDIT_MODE off (0x40), ENABLE_EXTENDED_FLAGS on (0x80) so the change applies
        [Jarvis.ConsoleMode]::SetConsoleMode($stdin, [uint32](($mode -band 0xFFFFFFBF) -bor 0x80)) | Out-Null
    }
} catch {}

$here = $PSScriptRoot
$pyDir = Join-Path $here "python"
$py = Join-Path $pyDir "python.exe"
$marker = Join-Path $pyDir "jarvis-profile.txt"
$oldVenv = Join-Path $here ".venv"      # installs before the private Python
$pythonVersion = "3.12.10"               # AMD PyTorch needs 3.12; last 3.12 with Windows binaries
$pythonZip = "https://www.python.org/ftp/python/$pythonVersion/python-$pythonVersion-embed-amd64.zip"
$getPip = "https://bootstrap.pypa.io/get-pip.py"
$rocmIndex = "https://stable.repo.amd.com/rocm/whl-next/"
$f5Package = "f5-tts==1.1.22"
$runtimeMarker = Join-Path $here "models\runtime-version.txt"
$engineFile = Join-Path $here "tts-engine.txt"


function Step($text) { Write-Host ""; Write-Host "==> $text" -ForegroundColor Cyan }

function Finish($code) {
    if (-not $NoPause) { Write-Host ""; Read-Host "Нажмите Enter, чтобы закрыть" | Out-Null }
    exit $code
}

# pip output goes to the console, not into the function result.
# No cache (nothing left in %LOCALAPPDATA%\pip); long read timeout and retries for slow VPNs.
function Pip([string[]]$PipArgs) {
    $common = @("--disable-pip-version-check", "--no-cache-dir", "--no-warn-script-location", "--timeout", "60", "--retries", "10")
    if ($script:Installer) { $common += @("--progress-bar", "raw") }
    & $script:py -m pip install @common @PipArgs | Out-Host
    return ($LASTEXITCODE -eq 0)
}

function Test-VoiceGpu($profile) {
    $check = "import torch, torchaudio, sys; backend = 'rocm' if torch.version.hip else 'cuda'; ok = backend == sys.argv[1] and torch.cuda.is_available(); print('torch', torch.__version__, 'torchaudio', torchaudio.__version__, 'backend', backend, 'gpu', torch.cuda.get_device_name(0) if ok else None); x = torch.ones((4,4), device='cuda') if ok else None; y = x @ x if ok else None; torch.cuda.synchronize() if ok else None; ok = ok and y[0,0].item() == 4; sys.exit(0 if ok else 1)"
    & $script:py -c $check $profile | Out-Host
    return ($LASTEXITCODE -eq 0)
}

function Get-CudaWheelIndex($profile) {
    # A CPU torch build cannot report capabilities; names cover a fresh Blackwell install.
    if ($profile.gpu -match '(?i)\b(RTX\s*50\d{2}(?:Ti|D)?|Blackwell|B[12]\d{2}|GB\d{3})\b') { return "https://download.pytorch.org/whl/cu128" }
    try {
        $major = & $script:py -c "import torch; print(torch.cuda.get_device_capability(0)[0] if torch.cuda.is_available() else 0)" 2>$null
        if ($LASTEXITCODE -eq 0 -and [int]($major | Select-Object -Last 1) -ge 10) { return "https://download.pytorch.org/whl/cu128" }
    } catch {}
    # Keep older architectures supported by the cu126 build.
    return "https://download.pytorch.org/whl/cu126"
}

function Ensure-VoiceGpu($profile) {
    if (Test-VoiceGpu $profile.profile) { return }
    switch ($profile.profile) {
        "cuda" {
            $index = Get-CudaWheelIndex $profile
            Step "Установка PyTorch с CUDA (~2.5 ГБ): $index"
            if (-not (Pip @("--force-reinstall", "torch==2.8.0", "torchaudio==2.8.0", "--index-url", $index))) {
                throw "не удалось установить PyTorch с CUDA"
            }
        }
        "rocm" {
            Step "Установка PyTorch с ROCm для $($profile.gfx)"
            if (-not (Pip @("filelock", "fsspec", "jinja2", "networkx", "sympy", "typing-extensions", "setuptools", "numpy", "pillow"))) {
                throw "не удалось установить зависимости ROCm"
            }
            if (-not (Pip @("--force-reinstall", "torch[device-$($profile.gfx)]", "torchaudio", "--index-url", $script:rocmIndex))) {
                throw "не удалось установить PyTorch с ROCm"
            }
        }
    }
    if (-not (Test-VoiceGpu $profile.profile)) {
        throw "Видеокарта недоступна для синтеза Jarvis New. Обновите драйвер NVIDIA/AMD и повторите setup.bat. Озвучка на CPU отключена."
    }
}

function Runtime-Fingerprint {
    # Server changes may introduce new model files even when requirements stay the same.
    $files = @("requirements.txt", "requirements-tts.txt", "server.py", "gpu.py", "install.ps1", "gpu-profile.json")
    $hashes = @($files | ForEach-Object {
        $path = Join-Path $script:here $_
        if (Test-Path $path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
    })
    return ($script:TtsEngine + ":" + $script:f5Package + ':' + ($hashes -join ':'))
}

function Install-VoicePackages {
    Step "Установка распознавания речи GigaAM"
    if (-not (Pip @("-r", (Join-Path $script:here "requirements.txt")))) { throw "установка зависимостей распознавания речи" }
    $profileFile = Join-Path $script:here "gpu-profile.json"
    # Older installs may predate gpu-profile.json; detect and persist their hardware first.
    if (-not (Test-Path $profileFile)) {
        & $script:py (Join-Path $script:here "gpu.py") --write | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "не удалось определить видеокарту" }
    }
    $profile = Get-Content $profileFile -Raw | ConvertFrom-Json
    if ($profile.profile -notin @("cuda", "rocm")) {
        Step "Распознавание установлено. Для голоса Jarvis New нужна NVIDIA CUDA или поддерживаемая AMD ROCm; записанные отклики доступны."
        return
    }
    Ensure-VoiceGpu $profile
    Step "Установка голоса Jarvis New: F5 ESpeech RL-V2 на видеокарте"
    if (-not (Pip @("-r", (Join-Path $script:here "requirements-tts.txt")))) { throw "установка зависимостей F5-TTS" }
    # Preserve the chosen GPU torch build instead of resolving F5's full dependencies.
    if (-not (Pip @($script:f5Package, "--no-deps"))) { throw "установка F5-TTS" }

}

function Download-VoiceModels {
    Step "Скачивание моделей распознавания и Jarvis New ($script:TtsEngine)"
    Push-Location $script:here
    try {
        & $script:py -u server.py --download-only --tts-engine $script:TtsEngine | Out-Host
        return ($LASTEXITCODE -eq 0)
    } finally { Pop-Location }
}

function Remove-RetiredVoice {
    foreach ($name in @("nano.py", "nano-models.json", "requirements-nano.txt", "test_nano.py", "vendor\moss_nano", "models\nano", "models\gigaam-v3")) {
        $path = Join-Path $script:here $name
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
    }
}

function Update-VoiceRuntime {
    $fingerprint = Runtime-Fingerprint
    if ((Test-Path $script:runtimeMarker) -and
        (Get-Content $script:runtimeMarker -Raw).Trim() -eq $fingerprint) {
        Set-Content -LiteralPath $script:engineFile -Value $script:TtsEngine -Encoding ascii
        Remove-RetiredVoice
        Step "Голосовой сервер уже обновлён"
        return
    }
    Install-VoicePackages
    if (-not $script:SkipModels -and -not (Download-VoiceModels)) {
        throw "не удалось скачать модели голоса; обновление повторится при следующей установке"
    }
    Remove-RetiredVoice
    Set-Content -LiteralPath $script:engineFile -Value $script:TtsEngine -Encoding ascii
    # Never mark an incomplete model download as a successful upgrade.
    if (-not $script:SkipModels) {
        New-Item -ItemType Directory -Force (Split-Path $script:runtimeMarker) | Out-Null
        Set-Content -LiteralPath $script:runtimeMarker -Value $fingerprint -Encoding ascii
    }
}

# the embeddable Python from python.org: unpacked here, pip added with get-pip.py
function Install-Python {
    Step "Скачивание Python $pythonVersion (в папку Джарвиса)"
    $zip = Join-Path $here "python-embed.zip"
    Invoke-WebRequest -Uri $pythonZip -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $pyDir -Force
    Remove-Item $zip
    # python312._pth: enable site-packages ("import site") and put the server folder (..) on sys.path
    $pth = Get-ChildItem $pyDir -Filter "python*._pth" | Select-Object -First 1
    $lines = @(Get-Content $pth.FullName | ForEach-Object { if ($_ -eq "#import site") { "import site" } else { $_ } })
    if ($lines -notcontains "..") { $lines += ".." }
    Set-Content -Path $pth.FullName -Value $lines -Encoding ascii
    Step "Установка pip"
    $getPipFile = Join-Path $pyDir "get-pip.py"
    Invoke-WebRequest -Uri $getPip -OutFile $getPipFile -UseBasicParsing
    & $py $getPipFile --no-cache-dir --no-warn-script-location --disable-pip-version-check | Out-Host
    $code = $LASTEXITCODE
    Remove-Item $getPipFile
    if ($code -ne 0) { throw "не удалось установить pip" }
}

try {
    if ($RuntimeOnly) {
        if (-not (Test-Path $py)) { $py = Join-Path $oldVenv "Scripts\python.exe" }
        if (-not (Test-Path $py)) { Remove-RetiredVoice; Finish 0 }
        # The installer already stopped this installation's processes before replacing files.
        Update-VoiceRuntime
        Step "Готово. Голосовой сервер обновлён."
        Finish 0
    }
    # a running voice server (Python, whisper-server) locks the files that are replaced below
    $serverPrefix = $here.TrimEnd('\') + '\'
    Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($serverPrefix, [StringComparison]::OrdinalIgnoreCase) } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

    Step "Определение видеокарты"
    # gpu.py needs only the standard library: detect first, with the private Python if it is already there
    if ($GpuProfile) { $env:JARVIS_GPU_PROFILE = $GpuProfile }
    if (-not (Test-Path $py)) { Install-Python }
    $json = & $py (Join-Path $here "gpu.py") --write
    if ($LASTEXITCODE -ne 0) { throw "не удалось определить видеокарту" }
    $gpu = $json | ConvertFrom-Json
    $gpuName = if ($gpu.gpu) { $gpu.gpu } else { "не найдена" }
    Write-Host "Видеокарта: $gpuName"
    switch ($gpu.profile) {
        "cuda"   { Write-Host "Распознавание: NVIDIA CUDA" }
        "rocm"   { Write-Host "Распознавание: AMD Vulkan (F5 поддерживает ROCm $($gpu.gfx))" }
        "vulkan" { Write-Host "Распознавание: Vulkan" }
        default  { Write-Host "Распознавание: процессор" }
    }

    # packages installed for another card are dropped together with the Python
    $oldProfile = if (Test-Path $marker) { (Get-Content $marker -Raw).Trim() } else { "" }
    if ($oldProfile -and $oldProfile -ne $gpu.profile) {
        Step "Python был настроен под другую видеокарту ($oldProfile), пересоздаю"
        Remove-Item -Recurse -Force $pyDir
        Install-Python
    }
    if (Test-Path $oldVenv) {
        Step "Удаление старого окружения (.venv)"
        Remove-Item -Recurse -Force $oldVenv
    }

    Step "Обновление pip"
    if (-not (Pip @("--upgrade", "pip"))) { throw "pip upgrade" }

    Install-VoicePackages
    Set-Content -Path $marker -Value $gpu.profile -Encoding ascii

    if (-not $SkipModels) {
        if (Download-VoiceModels) {
            New-Item -ItemType Directory -Force (Split-Path $runtimeMarker) | Out-Null
            Set-Content -LiteralPath $runtimeMarker -Value (Runtime-Fingerprint) -Encoding ascii
        } else { throw "Не удалось скачать модели; повторите setup.bat после восстановления интернета." }
    }

    Remove-RetiredVoice
    Set-Content -LiteralPath $engineFile -Value $TtsEngine -Encoding ascii
    Step "Готово. Джарвис сам запускает голосовой сервер при старте."
    Finish 0
} catch {
    Write-Host ""
    Write-Host "ОШИБКА: $_" -ForegroundColor Red
    Write-Host "Проверьте интернет и запустите setup.bat ещё раз."
    Finish 1
}
