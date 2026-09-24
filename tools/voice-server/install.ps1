# Installs the Jarvis voice server: a private Python 3.12 inside this folder (python\), with
# PyTorch + faster-whisper + coqui-tts for the detected graphics card, and downloads the models.
# Everything stays in this folder: no system Python, no pip cache, no files in the user profile.
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
$pyDir = Join-Path $here "python"
$py = Join-Path $pyDir "python.exe"
$marker = Join-Path $pyDir "jarvis-profile.txt"
$oldVenv = Join-Path $here ".venv"      # installs before the private Python
$pythonVersion = "3.12.10"               # AMD PyTorch needs 3.12; last 3.12 with Windows binaries
$pythonZip = "https://www.python.org/ftp/python/$pythonVersion/python-$pythonVersion-embed-amd64.zip"
$getPip = "https://bootstrap.pypa.io/get-pip.py"
$rocmIndex = "https://stable.repo.amd.com/rocm/whl-next/"

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

function Install-TorchCpu {
    Step "Установка PyTorch для процессора (~250 МБ)"
    if (-not (Pip @("torch==2.8.0", "torchaudio==2.8.0", "--index-url", "https://download.pytorch.org/whl/cpu"))) {
        throw "установка PyTorch"
    }
}

function Test-Rocm {
    & $script:py -c "import torch, sys; ok = bool(torch.version.hip) and torch.cuda.is_available(); print('torch', torch.__version__, 'hip', torch.version.hip, 'gpu', torch.cuda.get_device_name(0) if ok else None); sys.exit(0 if ok else 1)" | Out-Host
    return ($LASTEXITCODE -eq 0)
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
    # a running voice server (Python, whisper-server) locks the files that are replaced below
    Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$here\*" } |
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
        "cuda"   { Write-Host "Режим: NVIDIA CUDA — распознавание и голос на видеокарте" }
        "rocm"   { Write-Host "Режим: AMD ROCm ($($gpu.gfx)) — голос на видеокарте, распознавание через Vulkan" }
        "vulkan" { Write-Host "Режим: Vulkan — распознавание на видеокарте, голос на процессоре (медленнее)" }
        default  { Write-Host "Режим: процессор — распознавание и голос на процессоре (медленно)" }
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

    switch ($gpu.profile) {
        "cuda" {
            # torch 2.8 for CUDA: the tested pair; server.py also works with newer torch (reads WAV itself)
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
                & $py -m pip uninstall -y torch torchaudio | Out-Null
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
        & $py -u server.py --download-only
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
