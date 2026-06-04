@echo off
setlocal
cd /d "%~dp0"
cargo build --release
if errorlevel 1 exit /b %errorlevel%
copy /y "target\release\static-dock.exe" "static-dock.exe" >nul
echo Built static-dock.exe
