@echo off
setlocal
cd /d "%~dp0"
where pwsh.exe >nul 2>nul
if not %errorlevel%==0 (
  echo PowerShell 7+ ^(pwsh.exe^) is required for the concurrent load test.
  echo Install it from https://learn.microsoft.com/powershell/ or run the server manually.
  echo.
  echo Load test ended. Press any key to close this window.
  pause >nul
  exit /b 1
)
if exist "%~dp0..\..\Cargo.toml" (
  pwsh.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-loadtest.ps1" -Root "%~dp0..\.." %*
) else (
  pwsh.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-loadtest.ps1" %*
)
echo.
echo Load test ended. Press any key to close this window.
pause >nul
