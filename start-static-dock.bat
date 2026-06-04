@echo off
setlocal
cd /d "%~dp0"
if exist "%~dp0static-dock.exe" (
  "%~dp0static-dock.exe" --workers 128 --queue 2048 %*
) else (
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-static-dock-fallback.ps1" %*
)
echo.
echo StaticDock ended. Press any key to close this window.
pause >nul
