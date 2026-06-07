@echo off
setlocal
cd /d "%~dp0"
set "ROOT=%~dp0resources"
if not exist "%ROOT%" set "ROOT=%~dp0"
if exist "%~dp0static-dock.exe" (
  "%~dp0static-dock.exe" --root "%ROOT%" --port 8787 --workers 128 --queue 2048 %*
) else (
  echo [error] static-dock.exe was not found in this package.
  pause
  exit /b 1
)
echo.
echo StaticDock ended. Press any key to close this window.
pause >nul
