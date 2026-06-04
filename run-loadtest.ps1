param(
    [string]$Root = $PSScriptRoot,
    [int]$Port = 8030,
    [int]$Students = 50,
    [int]$VideoClients = 80,
    [int]$Workers = 128,
    [int]$Queue = 2048,
    [switch]$UseExistingServer
)

$ErrorActionPreference = "Stop"

if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw "PowerShell 7+ is required for the concurrent load test. The server itself does not require PowerShell 7."
}

$Root = [System.IO.Path]::GetFullPath($Root)
$serverExe = Join-Path $Root "static-dock.exe"
$loadtestRoot = Join-Path $Root "loadtest"

if (-not (Test-Path -LiteralPath $loadtestRoot -PathType Container)) {
    throw "Missing loadtest folder: $loadtestRoot"
}

$videoUrls = @(
    Get-ChildItem -LiteralPath (Join-Path $loadtestRoot "videos") -Filter "*.mp4" -File -ErrorAction SilentlyContinue |
        Sort-Object Name |
        ForEach-Object { "/loadtest/videos/$($_.Name)" }
)

function Wait-Server {
    param(
        [string]$LogPath,
        [int]$FallbackPort
    )

    $actualPort = $FallbackPort
    for ($i = 0; $i -lt 80; $i++) {
        if (Test-Path -LiteralPath $LogPath) {
            $text = Get-Content -LiteralPath $LogPath -Raw -ErrorAction SilentlyContinue
            if ($text -match 'Port:\s+(\d+)') {
                $actualPort = [int]$Matches[1]
            }
        }

        try {
            $r = Invoke-WebRequest -Uri "http://127.0.0.1:$actualPort/loadtest/" -UseBasicParsing -TimeoutSec 1
            if ($r.StatusCode -eq 200) {
                return $actualPort
            }
        }
        catch {
            Start-Sleep -Milliseconds 200
        }
    }

    throw "Server did not become ready."
}

function Summarize-Results {
    param(
        [string]$Name,
        [object[]]$Results,
        [int]$ElapsedMs,
        [scriptblock]$IsGood
    )

    $bad = @($Results | Where-Object { -not (& $IsGood $_) })
    $bytes = 0L
    $times = New-Object System.Collections.Generic.List[double]
    foreach ($line in $Results) {
        if ($line -match '^[^:]+:\d+:(\d+):([0-9.]+)$') {
            $bytes += [int64]$Matches[1]
            $times.Add([double]$Matches[2])
        }
    }

    [pscustomobject]@{
        Test = $Name
        Requests = $Results.Count
        Errors = $bad.Count
        ElapsedMs = $ElapsedMs
        DownloadedMB = [math]::Round($bytes / 1MB, 2)
        AvgRequestSec = [math]::Round((($times | Measure-Object -Average).Average), 3)
        MaxRequestSec = [math]::Round((($times | Measure-Object -Maximum).Maximum), 3)
    }
}

$serverProcess = $null
$actualPort = $Port
$log = Join-Path $env:TEMP "static-dock-loadtest-$Port.log"
$err = Join-Path $env:TEMP "static-dock-loadtest-$Port.err.log"

try {
    if (-not $UseExistingServer) {
        if (-not (Test-Path -LiteralPath $serverExe -PathType Leaf)) {
            throw "Missing server executable: $serverExe"
        }

        Remove-Item -LiteralPath $log, $err -ErrorAction SilentlyContinue
        $serverProcess = Start-Process -FilePath $serverExe `
            -ArgumentList @("--root", $Root, "--port", [string]$Port, "--workers", [string]$Workers, "--queue", [string]$Queue) `
            -WindowStyle Hidden `
            -PassThru `
            -RedirectStandardOutput $log `
            -RedirectStandardError $err

        $actualPort = Wait-Server -LogPath $log -FallbackPort $Port
    }
    else {
        $actualPort = Wait-Server -LogPath $log -FallbackPort $Port
    }

    $base = "http://127.0.0.1:$actualPort"
    Write-Host ""
    Write-Host "Load test target: $base/loadtest/"
    Write-Host "Students: $Students"
    if ($videoUrls.Count -gt 0) {
        Write-Host "Video clients: $VideoClients"
        Write-Host "Video files: $($videoUrls.Count)"
    }
    else {
        Write-Host "Video clients: skipped, no MP4 files in loadtest/videos"
    }
    Write-Host ""

    $openTimer = [System.Diagnostics.Stopwatch]::StartNew()
    $openResults = 1..$Students | ForEach-Object -Parallel {
        $base = $using:base
        $videoUrls = $using:videoUrls
        $i = $_
        function Hit-Full($label, $url) {
            $out = & curl.exe -s -L -o NUL -w "$label`:%{http_code}`:%{size_download}`:%{time_total}" $url
            if ($LASTEXITCODE -ne 0) { return ("error:{0}:{1}" -f $label, $LASTEXITCODE) }
            return $out
        }
        function Hit-Range($label, $url) {
            $out = & curl.exe -s -L -o NUL -w "$label`:%{http_code}`:%{size_download}`:%{time_total}" -H "Range: bytes=0-262143" $url
            if ($LASTEXITCODE -ne 0) { return ("error:{0}:{1}" -f $label, $LASTEXITCODE) }
            return $out
        }
        function ImageName($n) {
            $dim = if ($n -le 6) { "1280x720" } else { "1920x1080" }
            return ("test_image_{0:D2}_{1}.png" -f $n, $dim)
        }

        $img1 = (($i * 7) % 12) + 1
        $img2 = (($i * 11) % 12) + 1

        $requests = @(
            Hit-Full "index" "$base/loadtest/"
            Hit-Full "manifest" "$base/loadtest/data/manifest.json"
            Hit-Full "records" "$base/loadtest/data/records.json"
            Hit-Full "image" "$base/loadtest/images/$(ImageName $img1)"
            Hit-Full "image" "$base/loadtest/images/$(ImageName $img2)"
        )
        if ($videoUrls.Count -gt 0) {
            $videoUrl = $videoUrls[(($i * 5) % $videoUrls.Count)]
            $requests += Hit-Range "video-range" "$base$videoUrl"
        }
        $requests
    } -ThrottleLimit $Students
    $openTimer.Stop()

    $flatOpen = @($openResults | ForEach-Object { $_ })
    Summarize-Results "student-open" $flatOpen $openTimer.ElapsedMilliseconds {
        param($line)
        $line -match '^(index|manifest|records|image):200:' -or $line -match '^video-range:206:'
    }

    if ($videoUrls.Count -gt 0 -and $VideoClients -gt 0) {
        $videoTimer = [System.Diagnostics.Stopwatch]::StartNew()
        $videoResults = 1..$VideoClients | ForEach-Object -Parallel {
            $base = $using:base
            $videoUrls = $using:videoUrls
            $videoUrl = $videoUrls[(($_ * 7) % $videoUrls.Count)]
            $out = & curl.exe -s -L -o NUL -w "video:%{http_code}:%{size_download}:%{time_total}" "$base$videoUrl"
            if ($LASTEXITCODE -ne 0) { "error:video:$LASTEXITCODE" } else { $out }
        } -ThrottleLimit $VideoClients
        $videoTimer.Stop()

        Summarize-Results "full-video" @($videoResults) $videoTimer.ElapsedMilliseconds {
            param($line)
            $line -match '^video:200:'
        }
    }
    else {
        [pscustomobject]@{
            Test = "full-video"
            Requests = 0
            Errors = 0
            ElapsedMs = 0
            DownloadedMB = 0
            AvgRequestSec = "skipped"
            MaxRequestSec = "add MP4 files to loadtest/videos"
        }
    }

    if ($serverProcess -and -not $serverProcess.HasExited) {
        $proc = Get-Process -Id $serverProcess.Id -ErrorAction SilentlyContinue
        if ($proc) {
            [pscustomobject]@{
                Test = "server-process"
                Requests = ""
                Errors = ""
                ElapsedMs = ""
                DownloadedMB = ""
                AvgRequestSec = "CPU $([math]::Round($proc.CPU, 2))s"
                MaxRequestSec = "Mem $([math]::Round($proc.WorkingSet64 / 1MB, 2))MB"
            }
        }
    }

    Write-Host ""
    Write-Host "Local loopback results mainly test the server process and disk path. Real classroom performance still depends on Wi-Fi/router bandwidth."
}
finally {
    if ($serverProcess -and -not $serverProcess.HasExited) {
        Stop-Process -Id $serverProcess.Id -Force -ErrorAction SilentlyContinue
    }
}
