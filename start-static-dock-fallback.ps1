param(
    [int]$Port = 8000
)

$ErrorActionPreference = "Stop"

$root = $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($root)) {
    $root = (Get-Location).Path
}
$root = [System.IO.Path]::GetFullPath($root)
$rootWithSlash = $root.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar

function Test-PortAvailable {
    param([int]$CandidatePort)

    $listener = Get-NetTCPConnection -LocalPort $CandidatePort -State Listen -ErrorAction SilentlyContinue
    return $null -eq $listener
}

function Get-FreePort {
    param([int]$PreferredPort)

    $candidates = @($PreferredPort, 8000, 8080, 8010, 8020, 3000, 5173, 9000) | Select-Object -Unique
    foreach ($candidate in $candidates) {
        if (Test-PortAvailable -CandidatePort $candidate) {
            return $candidate
        }
    }

    throw "No free port found. Close another local server and try again."
}

function Get-LanAddresses {
    $addresses = Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Where-Object {
            $_.IPAddress -notlike "127.*" -and
            $_.IPAddress -notlike "169.254.*"
        } |
        Sort-Object @{
            Expression = {
                if ($_.InterfaceAlias -match "VMware|Virtual|Loopback") { 1 } else { 0 }
            }
        }, InterfaceMetric, InterfaceAlias

    $seen = @{}
    foreach ($address in $addresses) {
        if (-not $seen.ContainsKey($address.IPAddress)) {
            $seen[$address.IPAddress] = $true
            [pscustomobject]@{
                IPAddress = $address.IPAddress
                InterfaceAlias = $address.InterfaceAlias
            }
        }
    }
}

function Get-ContentType {
    param([string]$Path)

    switch ([System.IO.Path]::GetExtension($Path).ToLowerInvariant()) {
        ".html" { "text/html; charset=utf-8"; break }
        ".htm"  { "text/html; charset=utf-8"; break }
        ".css"  { "text/css; charset=utf-8"; break }
        ".js"   { "application/javascript; charset=utf-8"; break }
        ".json" { "application/json; charset=utf-8"; break }
        ".txt"  { "text/plain; charset=utf-8"; break }
        ".xml"  { "application/xml; charset=utf-8"; break }
        ".png"  { "image/png"; break }
        ".jpg"  { "image/jpeg"; break }
        ".jpeg" { "image/jpeg"; break }
        ".gif"  { "image/gif"; break }
        ".webp" { "image/webp"; break }
        ".svg"  { "image/svg+xml"; break }
        ".ico"  { "image/x-icon"; break }
        ".mp4"  { "video/mp4"; break }
        ".webm" { "video/webm"; break }
        ".mov"  { "video/quicktime"; break }
        ".m4v"  { "video/x-m4v"; break }
        ".mp3"  { "audio/mpeg"; break }
        ".wav"  { "audio/wav"; break }
        ".m3u8" { "application/vnd.apple.mpegurl"; break }
        ".ts"   { "video/mp2t"; break }
        default { "application/octet-stream" }
    }
}

function Write-Ascii {
    param(
        [System.IO.Stream]$Stream,
        [string]$Text
    )

    $bytes = [System.Text.Encoding]::ASCII.GetBytes($Text)
    $Stream.Write($bytes, 0, $bytes.Length)
}

function Send-Headers {
    param(
        [System.IO.Stream]$Stream,
        [int]$StatusCode,
        [string]$Reason,
        [hashtable]$Headers
    )

    $common = [ordered]@{
        "Date" = [DateTime]::UtcNow.ToString("R")
        "Server" = "StaticDockFallback/1.0"
        "Connection" = "close"
        "Access-Control-Allow-Origin" = "*"
        "Access-Control-Allow-Methods" = "GET, HEAD, OPTIONS"
        "Access-Control-Allow-Headers" = "Origin, X-Requested-With, Content-Type, Accept, Range"
        "Accept-Ranges" = "bytes"
    }

    Write-Ascii $Stream "HTTP/1.1 $StatusCode $Reason`r`n"
    foreach ($key in $common.Keys) {
        Write-Ascii $Stream "$key`: $($common[$key])`r`n"
    }
    foreach ($key in $Headers.Keys) {
        Write-Ascii $Stream "$key`: $($Headers[$key])`r`n"
    }
    Write-Ascii $Stream "`r`n"
}

function Send-Text {
    param(
        [System.IO.Stream]$Stream,
        [int]$StatusCode,
        [string]$Reason,
        [string]$Text,
        [string]$ContentType = "text/plain; charset=utf-8",
        [bool]$HeadOnly = $false
    )

    $body = [System.Text.Encoding]::UTF8.GetBytes($Text)
    Send-Headers $Stream $StatusCode $Reason @{
        "Content-Type" = $ContentType
        "Content-Length" = $body.Length
    }
    if (-not $HeadOnly) {
        $Stream.Write($body, 0, $body.Length)
    }
}

function Read-HttpRequest {
    param([System.IO.Stream]$Stream)

    $Stream.ReadTimeout = 10000
    $buffer = New-Object byte[] 8192
    $memory = [System.IO.MemoryStream]::new()

    while ($true) {
        $read = $Stream.Read($buffer, 0, $buffer.Length)
        if ($read -le 0) {
            break
        }

        $memory.Write($buffer, 0, $read)
        if ($memory.Length -gt 65536) {
            throw "Request header is too large."
        }

        $text = [System.Text.Encoding]::ASCII.GetString($memory.ToArray())
        if ($text.Contains("`r`n`r`n")) {
            break
        }
    }

    if ($memory.Length -eq 0) {
        return $null
    }

    $headerText = [System.Text.Encoding]::ASCII.GetString($memory.ToArray())
    $headerText = $headerText.Substring(0, $headerText.IndexOf("`r`n`r`n"))
    $lines = $headerText -split "`r`n"
    $requestLine = $lines[0] -split " "
    if ($requestLine.Length -lt 2) {
        throw "Bad request line."
    }

    $headers = [System.Collections.Generic.Dictionary[string,string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    for ($i = 1; $i -lt $lines.Length; $i++) {
        $line = $lines[$i]
        $colon = $line.IndexOf(":")
        if ($colon -gt 0) {
            $name = $line.Substring(0, $colon).Trim()
            $value = $line.Substring($colon + 1).Trim()
            $headers[$name] = $value
        }
    }

    [pscustomobject]@{
        Method = $requestLine[0].ToUpperInvariant()
        Target = $requestLine[1]
        Headers = $headers
    }
}

function Resolve-RequestPath {
    param([string]$Target)

    $pathPart = ($Target -split "\?", 2)[0]
    $pathPart = $pathPart -replace '/', '\'
    $pathPart = $pathPart.TrimStart('\')
    $decoded = [System.Uri]::UnescapeDataString($pathPart)
    $fullPath = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($root, $decoded))

    if (
        -not $fullPath.Equals($root, [System.StringComparison]::OrdinalIgnoreCase) -and
        -not $fullPath.StartsWith($rootWithSlash, [System.StringComparison]::OrdinalIgnoreCase)
    ) {
        return $null
    }

    return $fullPath
}

function New-DirectoryListing {
    param(
        [string]$DirectoryPath,
        [string]$Target
    )

    $basePath = ($Target -split "\?", 2)[0]
    if (-not $basePath.EndsWith("/")) {
        $basePath += "/"
    }

    $items = Get-ChildItem -LiteralPath $DirectoryPath -Force | Sort-Object @{ Expression = { -not $_.PSIsContainer } }, Name
    $rows = foreach ($item in $items) {
        $name = [System.Net.WebUtility]::HtmlEncode($item.Name)
        $hrefName = [System.Uri]::EscapeDataString($item.Name).Replace("%2F", "/")
        if ($item.PSIsContainer) {
            $name += "/"
            $hrefName += "/"
        }
        "<li><a href=`"$basePath$hrefName`">$name</a></li>"
    }

    "<!doctype html><html><head><meta charset=`"utf-8`"><title>StaticDock resources</title></head><body><h1>StaticDock resources</h1><ul>$($rows -join '')</ul></body></html>"
}

function Send-File {
    param(
        [System.IO.Stream]$Stream,
        [string]$Path,
        [System.Collections.Generic.Dictionary[string,string]]$RequestHeaders,
        [bool]$HeadOnly = $false
    )

    $fileInfo = [System.IO.FileInfo]::new($Path)
    $fileLength = [long]$fileInfo.Length
    $start = [long]0
    $end = [long]($fileLength - 1)
    $partial = $false

    if ($RequestHeaders.ContainsKey("Range") -and $fileLength -gt 0) {
        $rangeValue = ($RequestHeaders["Range"] -split ",", 2)[0].Trim()
        $match = [regex]::Match($rangeValue, "^bytes=(\d*)-(\d*)$")
        if ($match.Success) {
            $first = $match.Groups[1].Value
            $last = $match.Groups[2].Value

            if ($first -eq "" -and $last -ne "") {
                $suffixLength = [long]$last
                if ($suffixLength -gt 0) {
                    $start = [Math]::Max(0, $fileLength - $suffixLength)
                    $partial = $true
                }
            }
            elseif ($first -ne "") {
                $start = [long]$first
                if ($last -ne "") {
                    $end = [long]$last
                }
                $partial = $true
            }

            if ($end -ge $fileLength) {
                $end = $fileLength - 1
            }

            if ($start -ge $fileLength -or $start -gt $end) {
                Send-Headers $Stream 416 "Range Not Satisfiable" @{
                    "Content-Range" = "bytes */$fileLength"
                    "Content-Length" = 0
                }
                return
            }
        }
    }

    $contentLength = if ($fileLength -eq 0) { [long]0 } else { [long]($end - $start + 1) }
    $headers = @{
        "Content-Type" = Get-ContentType $Path
        "Content-Length" = $contentLength
        "Last-Modified" = $fileInfo.LastWriteTimeUtc.ToString("R")
        "Cache-Control" = "no-cache, no-store, must-revalidate"
    }

    if ($partial) {
        $headers["Content-Range"] = "bytes $start-$end/$fileLength"
        Send-Headers $Stream 206 "Partial Content" $headers
    }
    else {
        Send-Headers $Stream 200 "OK" $headers
    }

    if ($HeadOnly -or $contentLength -eq 0) {
        return
    }

    $fileStream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
    try {
        $fileStream.Seek($start, [System.IO.SeekOrigin]::Begin) | Out-Null
        $buffer = New-Object byte[] 65536
        $remaining = $contentLength
        while ($remaining -gt 0) {
            $toRead = [int][Math]::Min($buffer.Length, $remaining)
            $read = $fileStream.Read($buffer, 0, $toRead)
            if ($read -le 0) {
                break
            }
            $Stream.Write($buffer, 0, $read)
            $remaining -= $read
        }
    }
    finally {
        $fileStream.Dispose()
    }
}

function Handle-Client {
    param([System.Net.Sockets.TcpClient]$Client)

    try {
        $stream = $Client.GetStream()
        $request = Read-HttpRequest $stream
        if ($null -eq $request) {
            return
        }

        $headOnly = $request.Method -eq "HEAD"
        if ($request.Method -eq "OPTIONS") {
            Send-Headers $stream 204 "No Content" @{ "Content-Length" = 0 }
            return
        }

        if ($request.Method -ne "GET" -and $request.Method -ne "HEAD") {
            Send-Text $stream 405 "Method Not Allowed" "Method not allowed." "text/plain; charset=utf-8" $headOnly
            return
        }

        $path = Resolve-RequestPath $request.Target
        if ($null -eq $path) {
            Send-Text $stream 403 "Forbidden" "Forbidden." "text/plain; charset=utf-8" $headOnly
            return
        }

        if ([System.IO.Directory]::Exists($path)) {
            $index = [System.IO.Path]::Combine($path, "index.html")
            if ([System.IO.File]::Exists($index)) {
                Send-File $stream $index $request.Headers $headOnly
                return
            }

            $html = New-DirectoryListing $path $request.Target
            Send-Text $stream 200 "OK" $html "text/html; charset=utf-8" $headOnly
            return
        }

        if (-not [System.IO.File]::Exists($path)) {
            Send-Text $stream 404 "Not Found" "Not found." "text/plain; charset=utf-8" $headOnly
            return
        }

        Send-File $stream $path $request.Headers $headOnly
    }
    catch {
        try {
            if ($Client.Connected) {
                Send-Text $Client.GetStream() 500 "Internal Server Error" "Server error." "text/plain; charset=utf-8" $false
            }
        }
        catch {
        }
        Write-Host "Request failed: $($_.Exception.Message)"
    }
    finally {
        $Client.Close()
    }
}

$selectedPort = Get-FreePort -PreferredPort $Port
$lanAddresses = @(Get-LanAddresses)

Write-Host ""
Write-Host "Resource root: $root"
Write-Host "Port: $selectedPort"
Write-Host ""
Write-Host "PC local:          http://127.0.0.1:$selectedPort/"
if ($lanAddresses.Count -gt 0) {
    Write-Host "Phone/LAN URLs:"
    foreach ($address in $lanAddresses) {
        Write-Host ("  http://{0}:{1}/  ({2})" -f $address.IPAddress, $selectedPort, $address.InterfaceAlias)
    }
}
Write-Host "Android emulator:  http://10.0.2.2:$selectedPort/"
Write-Host ""
Write-Host "Example JSON:      http://127.0.0.1:$selectedPort/loadtest/data/manifest.json"
Write-Host "Example image:     http://127.0.0.1:$selectedPort/loadtest/images/test_image_01_1280x720.png"
Write-Host ""
Write-Host "No Node.js, Python, npm packages, or manual dependency install required."
Write-Host "Keep this window open while using the server. Press Ctrl+C to stop."
Write-Host ""

$tcpListener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Any, $selectedPort)
$tcpListener.Start()

try {
    while ($true) {
        $client = $tcpListener.AcceptTcpClient()
        Handle-Client $client
    }
}
finally {
    $tcpListener.Stop()
}
