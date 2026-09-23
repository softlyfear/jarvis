@echo off
chcp 65001 >nul
cd /d "%~dp0"
where py >/dev/null 2>/dev/null || (echo Не найден Python. Установите Python 3.11 с https://www.python.org/downloads/ и отметьте "Add to PATH". & pause & exit /b 1)
py -3.11 -m venv .venv || (echo Нужен именно Python 3.11. & pause & exit /b 1)
.venv\Scripts\python -m pip install --upgrade pip
rem PyTorch with CUDA for NVIDIA cards (RTX 3060 works)
rem torch 2.8: from 2.9 coqui-tts needs torchcodec + FFmpeg; transformers 5 breaks coqui-tts 0.27
.venv\Scripts\pip install torch==2.8.0 torchaudio==2.8.0 --index-url https://download.pytorch.org/whl/cu126 || (echo Ошибка установки PyTorch. & pause & exit /b 1)
.venv\Scripts\pip install -r requirements.txt || (echo Ошибка установки coqui-tts. & pause & exit /b 1)
echo.
echo Готово. Запуск сервера голоса: run.bat (первый запуск скачает модель ~2 ГБ)
pause
