@echo off
setlocal
cd /d "%~dp0"
where pwsh.exe >nul 2>nul
if %errorlevel%==0 (
  pwsh.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-loadtest.ps1" %*
) else (
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-loadtest.ps1" %*
)
echo.
echo Load test ended. Press any key to close this window.
pause >nul
