# Installs the Jarvis voice server: Python 3.11 (if missing), a virtual environment with
# PyTorch CUDA + faster-whisper + coqui-tts, and downloads the models.
# Used by setup.bat and by the Jarvis installer. Safe to run again (repairs/updates).
param(
    [switch]$NoPause,
    [switch]$SkipModels
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is very slow with the progress bar

$here = $PSScriptRoot
$venv = Join-Path $here ".venv"
$pythonVersion = "3.11.9"   # last 3.11 release with a Windows installer
$pythonUrl = "https://www.python.org/ftp/python/$pythonVersion/python-$pythonVersion-amd64.exe"

function Step($text) { Write-Host ""; Write-Host "==> $text" -ForegroundColor Cyan }

function Finish($code) {
    if (-not $NoPause) { Write-Host ""; Read-Host "Нажмите Enter, чтобы закрыть" | Out-Null }
    exit $code
}

function Find-Python311 {
    try {
        $p = & py -3.11 -c "import sys; print(sys.executable)" 2>$null
        if ($LASTEXITCODE -eq 0 -and $p -and (Test-Path $p)) { return $p.Trim() }
    } catch {}
    foreach ($candidate in @(
        (Join-Path $env:LOCALAPPDATA "Programs\Python\Python311\python.exe"),
        "C:\Program Files\Python311\python.exe"
    )) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

try {
    Step "Поиск Python 3.11"
    $python = Find-Python311
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
        $python = Find-Python311
        if (-not $python) { throw "Python 3.11 установлен, но не найден" }
    }
    Write-Host "Python: $python"

    $venvPython = Join-Path $venv "Scripts\python.exe"
    if (-not (Test-Path $venvPython)) {
        Step "Создание окружения"
        & $python -m venv $venv
        if ($LASTEXITCODE -ne 0) { throw "не удалось создать окружение" }
    }

    Step "Обновление pip"
    & $venvPython -m pip install --upgrade pip --disable-pip-version-check -q
    if ($LASTEXITCODE -ne 0) { throw "pip upgrade" }

    # torch 2.8: from 2.9 coqui-tts needs torchcodec + FFmpeg; transformers 5 breaks coqui-tts 0.27
    Step "Установка PyTorch с CUDA (~2.5 ГБ)"
    & $venvPython -m pip install --disable-pip-version-check "torch==2.8.0" "torchaudio==2.8.0" --index-url https://download.pytorch.org/whl/cu126
    if ($LASTEXITCODE -ne 0) { throw "установка PyTorch" }

    Step "Установка Whisper и синтеза голоса"
    & $venvPython -m pip install --disable-pip-version-check -r (Join-Path $here "requirements.txt")
    if ($LASTEXITCODE -ne 0) { throw "установка зависимостей" }

    if (-not $SkipModels) {
        Step "Скачивание моделей (~3.5 ГБ)"
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
