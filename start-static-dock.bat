@echo off
setlocal
cd /d "%~dp0"
set "ROOT=%~dp0resources"
if not exist "%ROOT%" set "ROOT=%~dp0"
if exist "%~dp0static-dock.exe" (
  "%~dp0static-dock.exe" --root "%ROOT%" --port 8787 --workers 128 --queue 2048 %*
) else (
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-static-dock-fallback.ps1" --root "%ROOT%" -Port 8787 %*
)
echo.
echo StaticDock ended. Press any key to close this window.
pause >nul
