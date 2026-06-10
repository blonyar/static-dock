param(
    [string]$Dist = "dist/static-dock-windows-x64",
    [string]$FullDist = "dist/static-dock-windows-x64-with-loadtest"
)

$ErrorActionPreference = "Stop"

function Assert-Exists {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Missing required path: $Path"
    }
    Write-Host "[ok] $Path"
}

function Assert-Missing {
    param([string]$Path)
    if (Test-Path -LiteralPath $Path) {
        throw "Unexpected path exists: $Path"
    }
    Write-Host "[ok] absent $Path"
}

function Assert-NoForbiddenFiles {
    param([string]$Path)
    $forbidden = @(
        '*.env', '*.pem', '*.key', '*.pfx', '*.log', '*.pid',
        '.git', '.github', '.gitignore', '.gitattributes'
    )
    foreach ($pattern in $forbidden) {
        $matches = @(Get-ChildItem -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue -Filter $pattern)
        if ($matches.Count -gt 0) {
            throw "Forbidden release file found in $Path matching ${pattern}: $($matches[0].FullName)"
        }
    }
    Write-Host "[ok] no forbidden files in $Path"
}

Assert-Missing "static-dock.exe"
Assert-Missing "StaticDockGui.exe"
Assert-Exists "build/windows/static-dock.exe"
Assert-Exists "build/windows/StaticDockGui.exe"

Assert-Exists $Dist
Assert-Exists $FullDist
Assert-Exists "$Dist/static-dock.exe"
Assert-Exists "$Dist/StaticDockGui.exe"
Assert-Exists "$Dist/start-static-dock-gui.bat"
Assert-Exists "$Dist/start-static-dock.bat"
Assert-Exists "$Dist/stop-static-dock.bat"
Assert-Exists "$Dist/resources/index.html"
Assert-Exists "$Dist/resources/data/manifest.json"
Assert-Exists "$Dist/使用说明.txt"
Assert-Missing "$Dist/loadtest"
Assert-Missing "$Dist/run-loadtest.bat"
Assert-Missing "$Dist/static-dock-gui-error.log"
Assert-Missing "$Dist/static-dock-server.pid"
Assert-Exists "$Dist/assets/showcase.png"
Assert-Exists "$Dist/assets/icon.png"
Assert-NoForbiddenFiles $Dist

Assert-Exists "$FullDist/loadtest"
Assert-Exists "$FullDist/run-loadtest.bat"
Assert-Exists "$FullDist/run-loadtest.ps1"
Assert-NoForbiddenFiles $FullDist
Assert-Exists "dist/static-dock-windows-x64.zip"
Assert-Exists "dist/static-dock-windows-x64-with-loadtest.zip"

$help = & "$Dist/static-dock.exe" --help
if ($LASTEXITCODE -ne 0) {
    throw "static-dock.exe --help failed with exit code $LASTEXITCODE"
}
$helpText = ($help | Out-String)
if ($helpText -notmatch "static-dock\.exe" -or $helpText -notmatch "--root") {
    throw "static-dock.exe --help output did not look right"
}
Write-Host "[ok] static-dock.exe --help"

Write-Host "Release package check passed."
