@echo off
setlocal EnableExtensions
cd /d "%~dp0"

set "GUI=%~dp0StaticDockGui.exe"
if not exist "%GUI%" (
    echo [错误] 找不到原生 GUI 启动器:
    echo "%GUI%"
    echo.
    echo 如果你是源码目录，请先重新构建 StaticDockGui.exe。
    pause
    exit /b 1
)

start "StaticDock GUI" "%GUI%"
exit /b 0
