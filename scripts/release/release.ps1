param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v\d+\.\d+\.\d+$')]
    [string]$Version,

    [string]$Title = "StaticDock $Version",
    [string]$NotesFile,
    [switch]$SkipPush,
    [switch]$CreateReleaseLocally,
    [switch]$Draft,
    [switch]$Prerelease
)

$ErrorActionPreference = "Stop"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
Set-Location $RepoRoot

if (($Draft -or $Prerelease) -and -not $CreateReleaseLocally) {
    throw "-Draft and -Prerelease only apply with -CreateReleaseLocally. Without it, the tag-triggered GitHub Actions workflow creates the Release using its configured defaults."
}

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

function Get-ChangelogSection {
    param([string]$Version)
    $changelog = "CHANGELOG.md"
    if (-not (Test-Path -LiteralPath $changelog)) { return $null }

    $text = Get-Content -LiteralPath $changelog -Raw
    $plain = [regex]::Escape($Version.TrimStart('v'))
    $tag = [regex]::Escape($Version)
    $pattern = "(?ms)^##\s+(?:$tag|$plain)\s*\r?\n(?<body>.*?)(?=^##\s+|\z)"
    $match = [regex]::Match($text, $pattern)
    if (-not $match.Success) { return $null }

    $body = $match.Groups['body'].Value.Trim()
    if (-not $body) { return $null }
    return "# StaticDock $Version`r`n`r`n$body`r`n"
}

function Write-ReleaseNotes {
    param(
        [string]$Version,
        [string]$NotesFile
    )
    if (-not (Test-Path -LiteralPath "dist")) {
        New-Item -ItemType Directory -Path "dist" | Out-Null
    }

    $path = Join-Path "dist" "release-notes-$Version.md"

    if ($NotesFile) {
        if (-not (Test-Path -LiteralPath $NotesFile)) { throw "Notes file not found: $NotesFile" }
        Copy-Item -LiteralPath $NotesFile -Destination $path -Force
        return $path
    }

    $changelogNotes = Get-ChangelogSection $Version
    if ($changelogNotes) {
        Set-Content -LiteralPath $path -Value $changelogNotes -Encoding UTF8
        return $path
    }

    $body = @"
# StaticDock $Version

See CHANGELOG.md for release details.
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
Run-Step "Write release notes" { $script:notes = Write-ReleaseNotes $Version $NotesFile; Write-Host $script:notes }

Run-Step "Commit and tag" {
    git add Cargo.toml Cargo.lock src/main.rs gui/StaticDockGui.cs resources/index.html build-windows.bat README.md CHANGELOG.md .gitignore .github docs scripts package tools
    git commit -m "Release $Version"
    git tag -a $Version -m "Release $Version"
}

if (-not $SkipPush) {
    Run-Step "Push commit and tag" {
        git push origin HEAD
        git push origin $Version
    }

    if ($CreateReleaseLocally) {
        Run-Step "Create GitHub Release" {
            $args = @('release','create',$Version,'dist/static-dock-windows-x64.zip','dist/static-dock-windows-x64-with-loadtest.zip','--title',$Title,'--notes-file',$notes)
            if ($Draft) { $args += '--draft' }
            if ($Prerelease) { $args += '--prerelease' }
            gh @args
        }
    } else {
        Write-Host "Tag pushed. GitHub Actions will create the Release. Use -CreateReleaseLocally to override." -ForegroundColor Yellow
    }
}

Write-Host "`nRelease $Version completed." -ForegroundColor Green
