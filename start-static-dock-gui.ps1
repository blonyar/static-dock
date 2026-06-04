param(
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serverExe = Join-Path $scriptDir "static-dock.exe"
$serverProcess = $null
$lastUrls = @()
$activePort = $null
$activeRoot = $null

function Normalize-RootPath {
    param([string]$Path)

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if ($fullPath -match "^[A-Za-z]:\\$") {
        return "$fullPath."
    }
    return $fullPath.TrimEnd("\")
}

function ConvertTo-CommandLineArg {
    param([string]$Value)

    if ($Value -notmatch '[\s"]') {
        return $Value
    }

    $escaped = $Value.Replace('"', '\"')
    if ($escaped.EndsWith("\")) {
        $escaped += "\"
    }
    return '"' + $escaped + '"'
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

function New-ServerUrls {
    param(
        [int]$Port,
        [string]$RootPath
    )

    $urls = New-Object System.Collections.Generic.List[string]
    $urls.Add("PC local:          http://127.0.0.1:$Port/")
    $urls.Add("Android emulator:  http://10.0.2.2:$Port/")
    foreach ($address in Get-LanAddresses) {
        $urls.Add(("Phone/LAN:         http://{0}:{1}/  ({2})" -f $address.IPAddress, $Port, $address.InterfaceAlias))
    }
    $urls.Add("")
    $urls.Add("Example JSON:      http://127.0.0.1:$Port/loadtest/data/manifest.json")
    $urls.Add("Example image:     http://127.0.0.1:$Port/loadtest/images/test_image_01_1280x720.png")
    $urls.Add("")
    $urls.Add("Mounted folder:    $RootPath")
    return $urls.ToArray()
}

function Append-Log {
    param(
        [System.Windows.Forms.TextBox]$TextBox,
        [string]$Message
    )

    $TextBox.AppendText($Message + [Environment]::NewLine)
}

function Set-RunningState {
    param(
        [bool]$Running,
        [System.Windows.Forms.Button]$StartButton,
        [System.Windows.Forms.Button]$StopButton,
        [System.Windows.Forms.Button]$OpenButton,
        [System.Windows.Forms.Button]$CopyButton,
        [System.Windows.Forms.Label]$StatusLabel
    )

    $StartButton.Enabled = -not $Running
    $StopButton.Enabled = $Running
    $OpenButton.Enabled = $Running
    $CopyButton.Enabled = $Running
    if ($Running) {
        $StatusLabel.Text = "Status: running"
        $StatusLabel.ForeColor = [System.Drawing.Color]::DarkGreen
    }
    else {
        $StatusLabel.Text = "Status: stopped"
        $StatusLabel.ForeColor = [System.Drawing.Color]::DarkRed
    }
}

if ($SelfTest) {
    Write-Output "GUI launcher self-test OK"
    return
}

[System.Windows.Forms.Application]::EnableVisualStyles()

$form = New-Object System.Windows.Forms.Form
$form.Text = "StaticDock Resource Server"
$form.StartPosition = "CenterScreen"
$form.Size = New-Object System.Drawing.Size(780, 560)
$form.MinimumSize = New-Object System.Drawing.Size(720, 500)

$font = New-Object System.Drawing.Font("Segoe UI", 9)
$monoFont = New-Object System.Drawing.Font("Consolas", 9)
$form.Font = $font

$rootLabel = New-Object System.Windows.Forms.Label
$rootLabel.Text = "Folder"
$rootLabel.Location = New-Object System.Drawing.Point(16, 20)
$rootLabel.Size = New-Object System.Drawing.Size(70, 24)
$form.Controls.Add($rootLabel)

$rootText = New-Object System.Windows.Forms.TextBox
$rootText.Location = New-Object System.Drawing.Point(90, 18)
$rootText.Size = New-Object System.Drawing.Size(540, 24)
$rootText.Text = $scriptDir
$form.Controls.Add($rootText)

$browseButton = New-Object System.Windows.Forms.Button
$browseButton.Text = "Browse..."
$browseButton.Location = New-Object System.Drawing.Point(640, 16)
$browseButton.Size = New-Object System.Drawing.Size(100, 28)
$form.Controls.Add($browseButton)

$portLabel = New-Object System.Windows.Forms.Label
$portLabel.Text = "Port"
$portLabel.Location = New-Object System.Drawing.Point(16, 62)
$portLabel.Size = New-Object System.Drawing.Size(70, 24)
$form.Controls.Add($portLabel)

$portBox = New-Object System.Windows.Forms.NumericUpDown
$portBox.Location = New-Object System.Drawing.Point(90, 60)
$portBox.Size = New-Object System.Drawing.Size(100, 24)
$portBox.Minimum = 1
$portBox.Maximum = 65535
$portBox.Value = 8000
$form.Controls.Add($portBox)

$workersLabel = New-Object System.Windows.Forms.Label
$workersLabel.Text = "Workers"
$workersLabel.Location = New-Object System.Drawing.Point(220, 62)
$workersLabel.Size = New-Object System.Drawing.Size(70, 24)
$form.Controls.Add($workersLabel)

$workersBox = New-Object System.Windows.Forms.NumericUpDown
$workersBox.Location = New-Object System.Drawing.Point(290, 60)
$workersBox.Size = New-Object System.Drawing.Size(100, 24)
$workersBox.Minimum = 1
$workersBox.Maximum = 512
$workersBox.Value = 128
$form.Controls.Add($workersBox)

$queueLabel = New-Object System.Windows.Forms.Label
$queueLabel.Text = "Queue"
$queueLabel.Location = New-Object System.Drawing.Point(420, 62)
$queueLabel.Size = New-Object System.Drawing.Size(70, 24)
$form.Controls.Add($queueLabel)

$queueBox = New-Object System.Windows.Forms.NumericUpDown
$queueBox.Location = New-Object System.Drawing.Point(490, 60)
$queueBox.Size = New-Object System.Drawing.Size(100, 24)
$queueBox.Minimum = 1
$queueBox.Maximum = 20000
$queueBox.Value = 2048
$form.Controls.Add($queueBox)

$statusLabel = New-Object System.Windows.Forms.Label
$statusLabel.Text = "Status: stopped"
$statusLabel.ForeColor = [System.Drawing.Color]::DarkRed
$statusLabel.Location = New-Object System.Drawing.Point(16, 104)
$statusLabel.Size = New-Object System.Drawing.Size(240, 24)
$form.Controls.Add($statusLabel)

$startButton = New-Object System.Windows.Forms.Button
$startButton.Text = "Start"
$startButton.Location = New-Object System.Drawing.Point(16, 140)
$startButton.Size = New-Object System.Drawing.Size(100, 34)
$form.Controls.Add($startButton)

$stopButton = New-Object System.Windows.Forms.Button
$stopButton.Text = "Stop"
$stopButton.Location = New-Object System.Drawing.Point(128, 140)
$stopButton.Size = New-Object System.Drawing.Size(100, 34)
$stopButton.Enabled = $false
$form.Controls.Add($stopButton)

$openButton = New-Object System.Windows.Forms.Button
$openButton.Text = "Open Local"
$openButton.Location = New-Object System.Drawing.Point(240, 140)
$openButton.Size = New-Object System.Drawing.Size(110, 34)
$openButton.Enabled = $false
$form.Controls.Add($openButton)

$copyButton = New-Object System.Windows.Forms.Button
$copyButton.Text = "Copy URLs"
$copyButton.Location = New-Object System.Drawing.Point(362, 140)
$copyButton.Size = New-Object System.Drawing.Size(110, 34)
$copyButton.Enabled = $false
$form.Controls.Add($copyButton)

$logBox = New-Object System.Windows.Forms.TextBox
$logBox.Location = New-Object System.Drawing.Point(16, 190)
$logBox.Size = New-Object System.Drawing.Size(724, 300)
$logBox.Anchor = [System.Windows.Forms.AnchorStyles]::Top -bor
    [System.Windows.Forms.AnchorStyles]::Bottom -bor
    [System.Windows.Forms.AnchorStyles]::Left -bor
    [System.Windows.Forms.AnchorStyles]::Right
$logBox.Multiline = $true
$logBox.ScrollBars = "Vertical"
$logBox.ReadOnly = $true
$logBox.Font = $monoFont
$form.Controls.Add($logBox)

$browseButton.Add_Click({
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.Description = "Choose resource folder to mount"
    $dialog.SelectedPath = $rootText.Text
    if ($dialog.ShowDialog($form) -eq [System.Windows.Forms.DialogResult]::OK) {
        $rootText.Text = $dialog.SelectedPath
    }
})

$startButton.Add_Click({
    try {
        if (-not (Test-Path -LiteralPath $serverExe)) {
            [System.Windows.Forms.MessageBox]::Show($form, "static-dock.exe was not found in this folder.", "Missing server", "OK", "Error") | Out-Null
            return
        }

        $rootPath = Normalize-RootPath $rootText.Text
        if (-not (Test-Path -LiteralPath $rootPath -PathType Container)) {
            [System.Windows.Forms.MessageBox]::Show($form, "Selected folder does not exist.", "Invalid folder", "OK", "Error") | Out-Null
            return
        }

        $port = [int]$portBox.Value
        $workers = [int]$workersBox.Value
        $queue = [int]$queueBox.Value
        $script:activePort = $port
        $script:activeRoot = $rootPath

        $logBox.Clear()
        Append-Log $logBox "Starting server..."
        Append-Log $logBox "Executable: $serverExe"

        $args = @(
            "--root", $rootPath,
            "--port", [string]$port,
            "--workers", [string]$workers,
            "--queue", [string]$queue
        )

        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $serverExe
        $psi.WorkingDirectory = $rootPath
        $psi.UseShellExecute = $false
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.CreateNoWindow = $true
        $psi.Arguments = (($args | ForEach-Object { ConvertTo-CommandLineArg $_ }) -join " ")

        $script:serverProcess = New-Object System.Diagnostics.Process
        $script:serverProcess.StartInfo = $psi
        $script:serverProcess.EnableRaisingEvents = $true

        $script:serverProcess.add_OutputDataReceived({
            param($sender, $eventArgs)
            if ($eventArgs.Data) {
                $form.BeginInvoke([Action[string]]{
                    param($line)
                    $logBox.AppendText($line + [Environment]::NewLine)
                    if ($line -match '^Port:\s+(\d+)') {
                        $script:activePort = [int]$Matches[1]
                        if ($script:activeRoot) {
                            $script:lastUrls = New-ServerUrls -Port $script:activePort -RootPath $script:activeRoot
                        }
                    }
                }, $eventArgs.Data) | Out-Null
            }
        })

        $script:serverProcess.add_ErrorDataReceived({
            param($sender, $eventArgs)
            if ($eventArgs.Data) {
                $form.BeginInvoke([Action[string]]{
                    param($line)
                    $logBox.AppendText("[err] " + $line + [Environment]::NewLine)
                }, $eventArgs.Data) | Out-Null
            }
        })

        $script:serverProcess.add_Exited({
            $form.BeginInvoke([Action]{
                Set-RunningState $false $startButton $stopButton $openButton $copyButton $statusLabel
                Append-Log $logBox "Server stopped."
            }) | Out-Null
        })

        [void]$script:serverProcess.Start()
        $script:serverProcess.BeginOutputReadLine()
        $script:serverProcess.BeginErrorReadLine()

        $script:lastUrls = New-ServerUrls -Port $port -RootPath $rootPath
        Append-Log $logBox "If port $port is busy, the server may auto-select another port. Check the log above."

        Set-RunningState $true $startButton $stopButton $openButton $copyButton $statusLabel
    }
    catch {
        [System.Windows.Forms.MessageBox]::Show($form, $_.Exception.Message, "Start failed", "OK", "Error") | Out-Null
    }
})

$stopButton.Add_Click({
    try {
        if ($script:serverProcess -and -not $script:serverProcess.HasExited) {
            $script:serverProcess.Kill()
            $script:serverProcess.WaitForExit(2000) | Out-Null
        }
        Set-RunningState $false $startButton $stopButton $openButton $copyButton $statusLabel
    }
    catch {
        [System.Windows.Forms.MessageBox]::Show($form, $_.Exception.Message, "Stop failed", "OK", "Error") | Out-Null
    }
})

$openButton.Add_Click({
    $port = if ($script:activePort) { [int]$script:activePort } else { [int]$portBox.Value }
    Start-Process "http://127.0.0.1:$port/"
})

$copyButton.Add_Click({
    if ($script:lastUrls.Count -eq 0 -and $script:activePort -and $script:activeRoot) {
        $script:lastUrls = New-ServerUrls -Port $script:activePort -RootPath $script:activeRoot
    }
    if ($script:lastUrls.Count -gt 0) {
        [System.Windows.Forms.Clipboard]::SetText(($script:lastUrls -join [Environment]::NewLine))
    }
})

$form.Add_FormClosing({
    if ($script:serverProcess -and -not $script:serverProcess.HasExited) {
        $script:serverProcess.Kill()
    }
})

Append-Log $logBox "Choose a folder, then click Start."
Append-Log $logBox "The Rust server exe will do the actual file serving."
Append-Log $logBox "Windows Firewall may ask for permission the first time."

[System.Windows.Forms.Application]::Run($form)
