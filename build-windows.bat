@echo off
setlocal EnableExtensions
cd /d "%~dp0"

set "BUILD=build\windows"
set "SERVER_EXE=%BUILD%\static-dock.exe"
set "GUI_EXE=%BUILD%\StaticDockGui.exe"
set "DIST=dist\static-dock-windows-x64"
set "DIST_FULL=dist\static-dock-windows-x64-with-loadtest"
set "ZIP=dist\static-dock-windows-x64.zip"
set "ZIP_FULL=dist\static-dock-windows-x64-with-loadtest.zip"
set "CSC=C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
set "ICON=assets\icon.ico"

echo ========================================
echo  Building StaticDock Windows packages
echo ========================================
echo.

rem Remove legacy root-level build outputs from older builds.
del /q "static-dock.exe" "StaticDockGui.exe" 2>nul

echo [1/6] Building Rust server...
if not exist "%BUILD%" mkdir "%BUILD%"
cargo build --release
if errorlevel 1 exit /b %errorlevel%
copy /y "target\release\static-dock.exe" "%SERVER_EXE%" >nul

echo [2/6] Building native Windows GUI...
if not exist "%CSC%" (
    echo [error] C# compiler was not found:
    echo %CSC%
    exit /b 1
)
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -Command "& '%CSC%' /target:winexe /win32icon:'%ICON%' /out:'%GUI_EXE%' /reference:System.Windows.Forms.dll /reference:System.Drawing.dll gui\StaticDockGui.cs"
if errorlevel 1 exit /b %errorlevel%

echo [3/6] Refreshing standard dist folder...
if exist "%DIST%" rmdir /s /q "%DIST%"
mkdir "%DIST%"

copy /y "%SERVER_EXE%" "%DIST%\static-dock.exe" >nul
copy /y "%GUI_EXE%" "%DIST%\StaticDockGui.exe" >nul
copy /y "package\windows\start-static-dock-gui.bat" "%DIST%\start-static-dock-gui.bat" >nul
copy /y "package\windows\start-static-dock.bat" "%DIST%\start-static-dock.bat" >nul
copy /y "package\windows\stop-static-dock.bat" "%DIST%\stop-static-dock.bat" >nul
copy /y "README.md" "%DIST%\README.md" >nul
copy /y "LICENSE" "%DIST%\LICENSE" >nul
xcopy /e /i /y "assets" "%DIST%\assets" >nul
xcopy /e /i /y "resources" "%DIST%\resources" >nul
call :write_readme "%DIST%\使用说明.txt" "standard"
call :clean_runtime "%DIST%"

echo [4/6] Refreshing full dist folder with loadtest...
if exist "%DIST_FULL%" rmdir /s /q "%DIST_FULL%"
xcopy /e /i /y "%DIST%" "%DIST_FULL%" >nul
xcopy /e /i /y "loadtest" "%DIST_FULL%\loadtest" >nul
copy /y "tools\loadtest\run-loadtest.bat" "%DIST_FULL%\run-loadtest.bat" >nul
copy /y "tools\loadtest\run-loadtest.ps1" "%DIST_FULL%\run-loadtest.ps1" >nul
call :write_readme "%DIST_FULL%\使用说明.txt" "full"
call :clean_runtime "%DIST_FULL%"

echo [5/6] Creating zip packages...
if exist "%ZIP%" del /q "%ZIP%"
if exist "%ZIP_FULL%" del /q "%ZIP_FULL%"
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -Command "Compress-Archive -LiteralPath '%DIST%' -DestinationPath '%ZIP%' -Force; Compress-Archive -LiteralPath '%DIST_FULL%' -DestinationPath '%ZIP_FULL%' -Force"
if errorlevel 1 exit /b %errorlevel%

echo [6/6] Done.
echo.
echo Build folder:    "%BUILD%"
echo Standard folder: "%DIST%"
echo Standard zip:    "%ZIP%"
echo Full folder:     "%DIST_FULL%"
echo Full zip:        "%ZIP_FULL%"
exit /b 0

:write_readme
set "OUT=%~1"
set "MODE=%~2"
> "%OUT%" echo # StaticDock Windows x64 使用说明
>> "%OUT%" echo.
>> "%OUT%" echo 普通用户只需要双击：
>> "%OUT%" echo start-static-dock-gui.bat
>> "%OUT%" echo.
>> "%OUT%" echo 默认挂载 resources\，默认端口 8787。
>> "%OUT%" echo 打开后是资源管理台，不是裸目录列表。
>> "%OUT%" echo.
>> "%OUT%" echo 资源放置位置：
>> "%OUT%" echo resources\images\   图片、封面、头像、轮播图
>> "%OUT%" echo resources\videos\   MP4/WebM 视频
>> "%OUT%" echo resources\data\     JSON mock 数据
>> "%OUT%" echo resources\docs\     文档或其他静态文件
>> "%OUT%" echo.
>> "%OUT%" echo GUI 支持中文 / English、最近使用目录、设置记忆、自动重启、恢复默认、打开文件夹、检测端口、手机访问二维码。
if "%MODE%"=="full" (
  >> "%OUT%" echo.
  >> "%OUT%" echo 本包包含 loadtest\ 压测素材和 run-loadtest.bat。
  >> "%OUT%" echo 压测脚本需要 PowerShell 7+ ^(pwsh.exe^)。
) else (
  >> "%OUT%" echo.
  >> "%OUT%" echo 本包是不含压测素材的普通包，体积更小。
)
exit /b 0

:clean_runtime
del /q "%~1\*.log" "%~1\*.pid" "%~1\static-dock-urls.txt" 2>nul
del /q "%~1\StaticDockGui.cs" "%~1\start-static-dock-gui.ps1" 2>nul
exit /b 0
