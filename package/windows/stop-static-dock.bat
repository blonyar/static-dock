@echo off
setlocal
cd /d "%~dp0"

set "PIDFILE=%~dp0static-dock-server.pid"

if not exist "%PIDFILE%" (
    echo StaticDock server pid file was not found.
    echo If the server is still running, close static-dock.exe from Task Manager.
    pause
    exit /b 0
)

set /p PID=<"%PIDFILE%"
taskkill /PID %PID% /F >nul 2>nul
if errorlevel 1 (
    echo StaticDock server process was not running or could not be stopped.
) else (
    echo StaticDock server stopped. PID: %PID%
)

del "%PIDFILE%" >nul 2>nul
pause
