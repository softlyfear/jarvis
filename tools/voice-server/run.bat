@echo off
chcp 65001 >nul
cd /d "%~dp0"
if exist python\python.exe (python\python.exe server.py %* & exit /b)
if exist .venv\Scripts\python.exe (.venv\Scripts\python.exe server.py %* & exit /b)
echo Сначала запустите setup.bat & pause & exit /b 1
