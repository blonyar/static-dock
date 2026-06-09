param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v\d+\.\d+\.\d+$')]
    [string]$Version,

    [string]$Title = "StaticDock $Version",
    [switch]$SkipPush,
    [switch]$Draft,
    [switch]$Prerelease
)

$ErrorActionPreference = "Stop"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
Set-Location $RepoRoot

function Run-Step {
    param([string]$Name, [scriptblock]$Body)
    Write-Host "`n==> $Name" -ForegroundColor Cyan
    & $Body
}

function Assert-CleanOrOnlyVersionWork {
    $status = git status --porcelain
    if ($LASTEXITCODE -ne 0) { throw "git status failed" }
    if (-not $status) { return }
    Write-Host "Current working tree has changes:" -ForegroundColor Yellow
    $status | ForEach-Object { Write-Host "  $_" }
    throw "Release requires a clean working tree. Commit or stash changes first."
}

function Set-CargoVersion {
    param([string]$Version)
    $plain = $Version.TrimStart('v')
    $cargo = "Cargo.toml"
    $text = Get-Content -LiteralPath $cargo -Raw
    $updated = $text -replace '(?m)^version\s*=\s*"[^"]+"', "version = `"$plain`""
    if ($updated -eq $text) { throw "Could not update Cargo.toml version" }
    Set-Content -LiteralPath $cargo -Value $updated -NoNewline -Encoding UTF8
}

function Write-ReleaseNotes {
    param([string]$Version)
    $path = Join-Path "dist" "release-notes-$Version.md"
    $body = @"
# StaticDock $Version

## Highlights

- 更准确的局域网地址识别：过滤 198.18.x.x / 198.19.x.x 等虚拟或测试网段，优先显示真实 LAN 地址。
- GUI 增加最近使用目录下拉记忆，常用资源目录可以快速切换。
- 保留原生 C# WinForms GUI、默认 resources/、默认端口 8787、资源管理台和 Windows 免依赖体验。
- 新增一键 release 脚本，后续版本构建、测试、提交、打 tag、创建 GitHub Release 更稳定。

## Packages

- static-dock-windows-x64.zip：普通用户推荐下载，体积更小。
- static-dock-windows-x64-with-loadtest.zip：包含 loadtest 压测素材和脚本。

## Quick start

解压后双击：

```text
start-static-dock-gui.bat
```

默认访问：

```text
http://127.0.0.1:8787/
```
"@
    Set-Content -LiteralPath $path -Value $body -Encoding UTF8
    return $path
}

Assert-CleanOrOnlyVersionWork

Run-Step "Update Cargo.toml version to $Version" { Set-CargoVersion $Version }
Run-Step "Update Cargo.lock" { cargo check }
Run-Step "Build Windows packages" { & "$RepoRoot/build-windows.bat" }
Run-Step "Run release package checks" { & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "$RepoRoot/scripts/release/test-release.ps1" }
$notes = $null
Run-Step "Write release notes" { $script:notes = Write-ReleaseNotes $Version; Write-Host $script:notes }

Run-Step "Commit and tag" {
    git add Cargo.toml Cargo.lock src/main.rs gui/StaticDockGui.cs resources/index.html build-windows.bat README.md .gitignore docs scripts package tools
    git commit -m "Release $Version"
    git tag -a $Version -m "Release $Version"
}

if (-not $SkipPush) {
    Run-Step "Push commit and tag" {
        git push origin HEAD
        git push origin $Version
    }

    Run-Step "Create GitHub Release" {
        $args = @('release','create',$Version,'dist/static-dock-windows-x64.zip','dist/static-dock-windows-x64-with-loadtest.zip','--title',$Title,'--notes-file',$notes)
        if ($Draft) { $args += '--draft' }
        if ($Prerelease) { $args += '--prerelease' }
        gh @args
    }
}

Write-Host "`nRelease $Version completed." -ForegroundColor Green
