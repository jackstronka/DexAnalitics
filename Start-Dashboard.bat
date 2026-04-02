@echo off
title CLMM LP — dashboard (API + Vite)
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\Start-Dashboard.ps1"
echo.
if errorlevel 1 pause
exit /b %ERRORLEVEL%
