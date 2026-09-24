# Installs the Jarvis voice server: Python 3.12 (if missing), a virtual environment with
# PyTorch + faster-whisper + coqui-tts for the detected graphics card, and downloads the models.
#   NVIDIA          PyTorch CUDA; Whisper and the voice run on the card
#   AMD (RX 5000+)  PyTorch ROCm from AMD for the voice; Whisper runs on whisper.cpp (Vulkan)
#   other / none    PyTorch for the CPU; Whisper on whisper.cpp (Vulkan or CPU)
# Used by setup.bat and by the Jarvis installer. Safe to run again (repairs/updates).
param(
    [switch]$NoPause,
    [switch]$SkipModels,
    # force a profile instead of detecting: cuda | rocm | vulkan | cpu
    [string]$GpuProfile = "",
    # run by JarvisSetup.exe without a console: machine-readable pip progress for its progress page
    [switch]$Installer
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
$venv = Join-Path $here ".venv"
$marker = Join-Path $venv "jarvis-profile.txt"
$pythonVersion = "3.12.10"   # last 3.12 release with a Windows installer; AMD PyTorch needs 3.12
$pythonUrl = "https://www.python.org/ftp/python/$pythonVersion/python-$pythonVersion-amd64.exe"
$rocmIndex = "https://stable.repo.amd.com/rocm/whl-next/"

function Step($text) { Write-Host ""; Write-Host "==> $text" -ForegroundColor Cyan }

function Finish($code) {
    if (-not $NoPause) { Write-Host ""; Read-Host "Нажмите Enter, чтобы закрыть" | Out-Null }
    exit $code
}

function Find-Python312 {
    try {
        $p = & py -3.12 -c "import sys; print(sys.executable)" 2>$null
        if ($LASTEXITCODE -eq 0 -and $p -and (Test-Path $p)) { return $p.Trim() }
    } catch {}
    foreach ($candidate in @(
        (Join-Path $env:LOCALAPPDATA "Programs\Python\Python312\python.exe"),
        "C:\Program Files\Python312\python.exe"
    )) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

# pip output goes to the console, not into the function result.
# Long read timeout and retries: AMD's and PyTorch's servers are slow through a VPN.
function Pip([string[]]$PipArgs) {
    $common = @("--disable-pip-version-check", "--timeout", "60", "--retries", "10")
    if ($script:Installer) { $common += @("--progress-bar", "raw") }
    & $script:venvPython -m pip install @common @PipArgs | Out-Host
    return ($LASTEXITCODE -eq 0)
}

function Install-TorchCpu {
    Step "Установка PyTorch для процессора (~250 МБ)"
    if (-not (Pip @("torch==2.8.0", "torchaudio==2.8.0", "--index-url", "https://download.pytorch.org/whl/cpu"))) {
        throw "установка PyTorch"
    }
}

function Test-Rocm {
    & $script:venvPython -c "import torch, sys; ok = bool(torch.version.hip) and torch.cuda.is_available(); print('torch', torch.__version__, 'hip', torch.version.hip, 'gpu', torch.cuda.get_device_name(0) if ok else None); sys.exit(0 if ok else 1)" | Out-Host
    return ($LASTEXITCODE -eq 0)
}

try {
    Step "Поиск Python 3.12"
    $python = Find-Python312
    if (-not $python) {
        Step "Скачивание Python $pythonVersion"
        $installer = Join-Path $env:TEMP "python-$pythonVersion-amd64.exe"
        Invoke-WebRequest -Uri $pythonUrl -OutFile $installer -UseBasicParsing
        Step "Установка Python $pythonVersion (для текущего пользователя)"
        $proc = Start-Process -FilePath $installer -Wait -PassThru -ArgumentList @(
            "/quiet", "InstallAllUsers=0", "PrependPath=0", "Include_launcher=1",
            "InstallLauncherAllUsers=0", "Include_test=0", "Include_doc=0", "Shortcuts=0"
        )
        if ($proc.ExitCode -ne 0) { throw "установщик Python завершился с кодом $($proc.ExitCode)" }
        Remove-Item $installer -ErrorAction SilentlyContinue
        $python = Find-Python312
        if (-not $python) { throw "Python 3.12 установлен, но не найден" }
    }
    Write-Host "Python: $python"

    Step "Определение видеокарты"
    if ($GpuProfile) { $env:JARVIS_GPU_PROFILE = $GpuProfile }
    $json = & $python (Join-Path $here "gpu.py") --write
    if ($LASTEXITCODE -ne 0) { throw "не удалось определить видеокарту" }
    $gpu = $json | ConvertFrom-Json
    $gpuName = if ($gpu.gpu) { $gpu.gpu } else { "не найдена" }
    Write-Host "Видеокарта: $gpuName"
    switch ($gpu.profile) {
        "cuda"   { Write-Host "Режим: NVIDIA CUDA — распознавание и голос на видеокарте" }
        "rocm"   { Write-Host "Режим: AMD ROCm ($($gpu.gfx)) — голос на видеокарте, распознавание через Vulkan" }
        "vulkan" { Write-Host "Режим: Vulkan — распознавание на видеокарте, голос на процессоре (медленнее)" }
        default  { Write-Host "Режим: процессор — распознавание и голос на процессоре (медленно)" }
    }

    # a venv made for another card (or by the old Python 3.11 installer for NVIDIA) is rebuilt
    $venvPython = Join-Path $venv "Scripts\python.exe"
    $oldProfile = if (Test-Path $marker) { (Get-Content $marker -Raw).Trim() } elseif (Test-Path $venvPython) { "cuda" } else { "" }
    if ((Test-Path $venvPython) -and $oldProfile -ne $gpu.profile) {
        Step "Окружение было для другой видеокарты ($oldProfile), пересоздаю"
        Remove-Item -Recurse -Force $venv
    }
    if (-not (Test-Path $venvPython)) {
        Step "Создание окружения"
        & $python -m venv $venv
        if ($LASTEXITCODE -ne 0) { throw "не удалось создать окружение" }
    }

    Step "Обновление pip"
    & $venvPython -m pip install --upgrade pip --disable-pip-version-check -q
    if ($LASTEXITCODE -ne 0) { throw "pip upgrade" }

    switch ($gpu.profile) {
        "cuda" {
            # torch 2.8: from 2.9 torchaudio needs torchcodec + FFmpeg (server.py reads WAV itself, but CUDA stays on the tested pair)
            Step "Установка PyTorch с CUDA (~2.5 ГБ)"
            if (-not (Pip @("torch==2.8.0", "torchaudio==2.8.0", "--index-url", "https://download.pytorch.org/whl/cu126"))) { throw "установка PyTorch" }
        }
        "rocm" {
            Step "Установка PyTorch с ROCm для $($gpu.gfx) (~1.5 ГБ)"
            # dependencies from PyPI first, so the AMD index alone decides which torch is installed
            Pip @("filelock", "fsspec", "jinja2", "networkx", "sympy", "typing-extensions", "setuptools", "numpy", "pillow") | Out-Null
            $ok = Pip @("torch[device-$($gpu.gfx)]", "torchaudio", "--index-url", $rocmIndex)
            if ($ok) { $ok = Test-Rocm }
            if (-not $ok) {
                Write-Warning "PyTorch с ROCm не заработал. Частая причина — старый драйвер: обновите AMD Software: Adrenalin Edition и запустите setup.bat ещё раз. Пока голос Джарвиса будет на процессоре."
                & $venvPython -m pip uninstall -y torch torchaudio | Out-Null
                Install-TorchCpu
            }
        }
        default { Install-TorchCpu }
    }
    Set-Content -Path $marker -Value $gpu.profile -Encoding ascii

    Step "Установка Whisper и синтеза голоса"
    if (-not (Pip @("-r", (Join-Path $here "requirements.txt")))) { throw "установка зависимостей" }

    if ($gpu.profile -ne "cuda") {
        $exe = Join-Path $here "whispercpp\whisper-server.exe"
        if (-not (Test-Path $exe)) { Write-Warning "Не найден $exe — распознавание будет через faster-whisper на процессоре." }
    }

    if (-not $SkipModels) {
        Step "Скачивание моделей (~3 ГБ)"
        Push-Location $here
        & $venvPython -u server.py --download-only
        $code = $LASTEXITCODE
        Pop-Location
        if ($code -ne 0) { Write-Warning "Модели скачаются при первом запуске сервера." }
    }

    Step "Готово. Джарвис сам запускает голосовой сервер при старте."
    Finish 0
} catch {
    Write-Host ""
    Write-Host "ОШИБКА: $_" -ForegroundColor Red
    Write-Host "Проверьте интернет и запустите setup.bat ещё раз."
    Finish 1
}
