param(
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serverExe = Join-Path $scriptDir "static-dock.exe"
$errorLog = Join-Path $scriptDir "static-dock-gui-error.log"
$serverLog = Join-Path $scriptDir "static-dock-server.log"
$serverErrLog = Join-Path $scriptDir "static-dock-server-error.log"
$pidFile = Join-Path $scriptDir "static-dock-server.pid"
$urlFile = Join-Path $scriptDir "static-dock-urls.txt"
$script:serverProcess = $null
$script:lastUrls = @()

function Write-GuiLog {
    param([string]$Message)
    try {
        $time = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"
        Add-Content -LiteralPath $errorLog -Value "[$time] $Message" -Encoding UTF8
    }
    catch {}
}

function Append-Log {
    param([string]$Message)
    try {
        if ($script:logBox -and -not $script:logBox.IsDisposed) {
            $script:logBox.AppendText($Message + [Environment]::NewLine)
        }
    }
    catch {
        Write-GuiLog ("Append-Log failed: " + $_.Exception.Message)
    }
}

function Normalize-RootPath {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) { return $scriptDir }
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if ($fullPath -match "^[A-Za-z]:\\$") { return "$fullPath." }
    return $fullPath.TrimEnd("\")
}

function ConvertTo-CommandLineArg {
    param([string]$Value)
    if ($null -eq $Value) { return '""' }
    if ($Value -notmatch '[\s"]') { return $Value }
    $escaped = $Value.Replace('"', '\"')
    if ($escaped.EndsWith("\")) { $escaped += "\" }
    return '"' + $escaped + '"'
}

function Get-LanAddressesSafe {
    try {
        Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -notlike "127.*" -and $_.IPAddress -notlike "169.254.*" } |
            Sort-Object InterfaceMetric, InterfaceAlias |
            Select-Object -ExpandProperty IPAddress -Unique
    }
    catch {
        Write-GuiLog ("Get-LanAddressesSafe failed: " + $_.Exception.Message)
        @()
    }
}

function New-ServerUrls {
    param([int]$Port, [string]$RootPath)
    $urls = New-Object System.Collections.Generic.List[string]
    $urls.Add("本机访问:        http://127.0.0.1:$Port/")
    $urls.Add("Android 模拟器:  http://10.0.2.2:$Port/")
    foreach ($ip in Get-LanAddressesSafe) {
        $urls.Add("手机/局域网:     http://$ip`:$Port/")
    }
    $urls.Add("")
    $urls.Add("挂载目录:        $RootPath")
    return $urls.ToArray()
}

function Set-RunningState {
    param([bool]$Running)
    $script:startButton.Enabled = -not $Running
    $script:stopButton.Enabled = $Running
    $script:openButton.Enabled = $Running
    $script:copyButton.Enabled = $Running
    if ($Running) {
        $script:statusLabel.Text = "状态：运行中"
        $script:statusLabel.ForeColor = [System.Drawing.Color]::DarkGreen
    }
    else {
        $script:statusLabel.Text = "状态：已停止"
        $script:statusLabel.ForeColor = [System.Drawing.Color]::DarkRed
    }
}

function Stop-Server {
    try {
        Append-Log "正在停止服务..."
        $stopped = 0

        if ($script:serverProcess -and -not $script:serverProcess.HasExited) {
            try {
                Stop-Process -Id $script:serverProcess.Id -Force -ErrorAction Stop
                $script:serverProcess.WaitForExit(2000) | Out-Null
                $stopped++
            }
            catch { Write-GuiLog ("Stop tracked process failed: " + $_.Exception.Message) }
        }

        if (Test-Path -LiteralPath $pidFile) {
            try {
                $pidText = Get-Content -LiteralPath $pidFile -Raw
                $oldPid = [int]$pidText
                $oldProcess = Get-Process -Id $oldPid -ErrorAction SilentlyContinue
                if ($oldProcess) {
                    Stop-Process -Id $oldPid -Force -ErrorAction SilentlyContinue
                    $stopped++
                }
            }
            catch { Write-GuiLog ("Stop pid-file process failed: " + $_.Exception.Message) }
        }

        try {
            $targetExe = [System.IO.Path]::GetFullPath($serverExe)
            foreach ($proc in Get-Process -Name "static-dock" -ErrorAction SilentlyContinue) {
                try {
                    $procPath = [System.IO.Path]::GetFullPath($proc.Path)
                    if ($procPath -ieq $targetExe) {
                        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
                        $stopped++
                    }
                }
                catch {}
            }
        }
        catch { Write-GuiLog ("Stop residual processes failed: " + $_.Exception.Message) }

        try {
            $portToStop = [int]$script:portBox.Value
            $connections = Get-NetTCPConnection -LocalPort $portToStop -State Listen -ErrorAction SilentlyContinue
            foreach ($conn in $connections) {
                try {
                    $portProc = Get-Process -Id $conn.OwningProcess -ErrorAction SilentlyContinue
                    if ($portProc -and $portProc.ProcessName -eq "static-dock") {
                        Stop-Process -Id $portProc.Id -Force -ErrorAction SilentlyContinue
                        $stopped++
                    }
                }
                catch {}
            }
        }
        catch { Write-GuiLog ("Stop port owner failed: " + $_.Exception.Message) }

        Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
        $script:serverProcess = $null
        Set-RunningState $false
        if ($stopped -gt 0) { Append-Log "服务已停止，清理进程数: $stopped。" }
        else { Append-Log "没有发现当前目录的 static-dock 服务进程。" }
    }
    catch {
        Write-GuiLog ("Stop-Server failed: " + $_.Exception.ToString())
        [System.Windows.Forms.MessageBox]::Show($script:form, $_.Exception.Message, "停止失败", "OK", "Error") | Out-Null
    }
}

function Start-Server {
    try {
        if (-not (Test-Path -LiteralPath $serverExe)) { throw "找不到 static-dock.exe：$serverExe" }

        $rootPath = Normalize-RootPath $script:rootText.Text
        if (-not (Test-Path -LiteralPath $rootPath -PathType Container)) { throw "资源目录不存在：$rootPath" }

        $port = [int]$script:portBox.Value
        $workers = [int]$script:workersBox.Value
        $queue = [int]$script:queueBox.Value

        Stop-Server
        $script:logBox.Clear()
        Remove-Item -LiteralPath $serverLog,$serverErrLog,$urlFile -Force -ErrorAction SilentlyContinue

        Append-Log "正在启动 StaticDock..."
        Append-Log "程序: $serverExe"
        Append-Log "目录: $rootPath"
        Append-Log "端口: $port"

        $args = @("--root", $rootPath, "--port", [string]$port, "--workers", [string]$workers, "--queue", [string]$queue)
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $serverExe
        $psi.WorkingDirectory = $rootPath
        $psi.UseShellExecute = $false
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.CreateNoWindow = $true
        $psi.Arguments = (($args | ForEach-Object { ConvertTo-CommandLineArg $_ }) -join " ")

        $process = New-Object System.Diagnostics.Process
        $process.StartInfo = $psi
        $process.add_OutputDataReceived({
            param($sender, $eventArgs)
            if ($eventArgs.Data) { try { Add-Content -LiteralPath $serverLog -Value $eventArgs.Data -Encoding UTF8 } catch {} }
        })
        $process.add_ErrorDataReceived({
            param($sender, $eventArgs)
            if ($eventArgs.Data) { try { Add-Content -LiteralPath $serverErrLog -Value $eventArgs.Data -Encoding UTF8 } catch {} }
        })
        [void]$process.Start()
        $process.BeginOutputReadLine()
        $process.BeginErrorReadLine()
        $script:serverProcess = $process
        Set-Content -LiteralPath $pidFile -Value $process.Id -Encoding ASCII

        Start-Sleep -Milliseconds 800
        if ($process.HasExited) {
            $err = ""
            if (Test-Path -LiteralPath $serverErrLog) { $err = Get-Content -LiteralPath $serverErrLog -Raw }
            if ([string]::IsNullOrWhiteSpace($err) -and (Test-Path -LiteralPath $serverLog)) { $err = Get-Content -LiteralPath $serverLog -Raw }
            throw "服务启动后立即退出，退出码 $($process.ExitCode)。$err"
        }

        $script:lastUrls = New-ServerUrls -Port $port -RootPath $rootPath
        Set-Content -LiteralPath $urlFile -Value $script:lastUrls -Encoding UTF8
        Append-Log ""
        Append-Log "服务已启动，PID: $($process.Id)"
        Append-Log "访问地址："
        foreach ($url in $script:lastUrls) { Append-Log $url }
        Set-RunningState $true
    }
    catch {
        Write-GuiLog ("Start-Server failed: " + $_.Exception.ToString())
        Append-Log ("启动失败: " + $_.Exception.Message)
        [System.Windows.Forms.MessageBox]::Show($script:form, $_.Exception.Message, "启动失败", "OK", "Error") | Out-Null
        Set-RunningState $false
    }
}

try {
    Remove-Item -LiteralPath $errorLog -Force -ErrorAction SilentlyContinue
    Write-GuiLog "GUI script process entered"

    if ([System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
        Write-GuiLog "Not STA; relaunching self with -STA"
        Start-Process powershell.exe -ArgumentList @("-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-STA", "-File", $MyInvocation.MyCommand.Path) -WorkingDirectory $scriptDir | Out-Null
        exit 0
    }

    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing

    if ($SelfTest) {
        Write-Output "GUI launcher self-test OK"
        exit 0
    }

    Write-GuiLog "Assemblies loaded"
    [System.Windows.Forms.Application]::EnableVisualStyles()
    [System.Windows.Forms.Application]::SetCompatibleTextRenderingDefault($false)

    $script:form = New-Object System.Windows.Forms.Form
    $script:form.Text = "StaticDock 静态资源码头"
    $script:form.StartPosition = "CenterScreen"
    $script:form.Size = New-Object System.Drawing.Size(760, 520)
    $script:form.MinimumSize = New-Object System.Drawing.Size(720, 480)
    $script:form.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 9)

    $titleLabel = New-Object System.Windows.Forms.Label
    $titleLabel.Text = "StaticDock 静态资源码头"
    $titleLabel.Font = New-Object System.Drawing.Font("Microsoft YaHei UI", 15, [System.Drawing.FontStyle]::Bold)
    $titleLabel.Location = New-Object System.Drawing.Point(18, 14)
    $titleLabel.Size = New-Object System.Drawing.Size(360, 32)
    $script:form.Controls.Add($titleLabel)

    $script:statusLabel = New-Object System.Windows.Forms.Label
    $script:statusLabel.Location = New-Object System.Drawing.Point(590, 20)
    $script:statusLabel.Size = New-Object System.Drawing.Size(130, 24)
    $script:statusLabel.Anchor = [System.Windows.Forms.AnchorStyles]::Top -bor [System.Windows.Forms.AnchorStyles]::Right
    $script:form.Controls.Add($script:statusLabel)

    $rootLabel = New-Object System.Windows.Forms.Label
    $rootLabel.Text = "资源目录"
    $rootLabel.Location = New-Object System.Drawing.Point(20, 68)
    $rootLabel.Size = New-Object System.Drawing.Size(70, 24)
    $script:form.Controls.Add($rootLabel)

    $script:rootText = New-Object System.Windows.Forms.TextBox
    $script:rootText.Location = New-Object System.Drawing.Point(96, 66)
    $script:rootText.Size = New-Object System.Drawing.Size(510, 26)
    $script:rootText.Anchor = [System.Windows.Forms.AnchorStyles]::Top -bor [System.Windows.Forms.AnchorStyles]::Left -bor [System.Windows.Forms.AnchorStyles]::Right
    $script:rootText.Text = $scriptDir
    $script:form.Controls.Add($script:rootText)

    $browseButton = New-Object System.Windows.Forms.Button
    $browseButton.Text = "选择..."
    $browseButton.Location = New-Object System.Drawing.Point(620, 64)
    $browseButton.Size = New-Object System.Drawing.Size(90, 30)
    $browseButton.Anchor = [System.Windows.Forms.AnchorStyles]::Top -bor [System.Windows.Forms.AnchorStyles]::Right
    $script:form.Controls.Add($browseButton)

    $portLabel = New-Object System.Windows.Forms.Label
    $portLabel.Text = "端口"
    $portLabel.Location = New-Object System.Drawing.Point(20, 112)
    $portLabel.Size = New-Object System.Drawing.Size(50, 24)
    $script:form.Controls.Add($portLabel)

    $script:portBox = New-Object System.Windows.Forms.NumericUpDown
    $script:portBox.Location = New-Object System.Drawing.Point(70, 110)
    $script:portBox.Size = New-Object System.Drawing.Size(90, 26)
    $script:portBox.Minimum = 1
    $script:portBox.Maximum = 65535
    $script:portBox.Value = 8000
    $script:form.Controls.Add($script:portBox)

    $workersLabel = New-Object System.Windows.Forms.Label
    $workersLabel.Text = "Workers"
    $workersLabel.Location = New-Object System.Drawing.Point(190, 112)
    $workersLabel.Size = New-Object System.Drawing.Size(70, 24)
    $script:form.Controls.Add($workersLabel)

    $script:workersBox = New-Object System.Windows.Forms.NumericUpDown
    $script:workersBox.Location = New-Object System.Drawing.Point(260, 110)
    $script:workersBox.Size = New-Object System.Drawing.Size(90, 26)
    $script:workersBox.Minimum = 1
    $script:workersBox.Maximum = 512
    $script:workersBox.Value = 128
    $script:form.Controls.Add($script:workersBox)

    $queueLabel = New-Object System.Windows.Forms.Label
    $queueLabel.Text = "Queue"
    $queueLabel.Location = New-Object System.Drawing.Point(380, 112)
    $queueLabel.Size = New-Object System.Drawing.Size(60, 24)
    $script:form.Controls.Add($queueLabel)

    $script:queueBox = New-Object System.Windows.Forms.NumericUpDown
    $script:queueBox.Location = New-Object System.Drawing.Point(440, 110)
    $script:queueBox.Size = New-Object System.Drawing.Size(100, 26)
    $script:queueBox.Minimum = 1
    $script:queueBox.Maximum = 20000
    $script:queueBox.Value = 2048
    $script:form.Controls.Add($script:queueBox)

    $script:startButton = New-Object System.Windows.Forms.Button
    $script:startButton.Text = "启动服务"
    $script:startButton.Location = New-Object System.Drawing.Point(20, 158)
    $script:startButton.Size = New-Object System.Drawing.Size(100, 34)
    $script:form.Controls.Add($script:startButton)

    $script:stopButton = New-Object System.Windows.Forms.Button
    $script:stopButton.Text = "停止服务"
    $script:stopButton.Location = New-Object System.Drawing.Point(132, 158)
    $script:stopButton.Size = New-Object System.Drawing.Size(100, 34)
    $script:form.Controls.Add($script:stopButton)

    $script:openButton = New-Object System.Windows.Forms.Button
    $script:openButton.Text = "打开本机"
    $script:openButton.Location = New-Object System.Drawing.Point(244, 158)
    $script:openButton.Size = New-Object System.Drawing.Size(100, 34)
    $script:form.Controls.Add($script:openButton)

    $script:copyButton = New-Object System.Windows.Forms.Button
    $script:copyButton.Text = "复制地址"
    $script:copyButton.Location = New-Object System.Drawing.Point(356, 158)
    $script:copyButton.Size = New-Object System.Drawing.Size(100, 34)
    $script:form.Controls.Add($script:copyButton)

    $script:logBox = New-Object System.Windows.Forms.TextBox
    $script:logBox.Location = New-Object System.Drawing.Point(20, 210)
    $script:logBox.Size = New-Object System.Drawing.Size(700, 235)
    $script:logBox.Anchor = [System.Windows.Forms.AnchorStyles]::Top -bor [System.Windows.Forms.AnchorStyles]::Bottom -bor [System.Windows.Forms.AnchorStyles]::Left -bor [System.Windows.Forms.AnchorStyles]::Right
    $script:logBox.Multiline = $true
    $script:logBox.ScrollBars = "Vertical"
    $script:logBox.ReadOnly = $true
    $script:logBox.Font = New-Object System.Drawing.Font("Consolas", 9)
    $script:form.Controls.Add($script:logBox)

    $browseButton.Add_Click({
        try {
            $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
            $dialog.Description = "选择要挂载的资源目录"
            $dialog.SelectedPath = $script:rootText.Text
            if ($dialog.ShowDialog($script:form) -eq [System.Windows.Forms.DialogResult]::OK) { $script:rootText.Text = $dialog.SelectedPath }
        }
        catch {
            Write-GuiLog ("Browse failed: " + $_.Exception.ToString())
            [System.Windows.Forms.MessageBox]::Show($script:form, $_.Exception.Message, "选择目录失败", "OK", "Error") | Out-Null
        }
    })

    $script:startButton.Add_Click({ Start-Server })
    $script:stopButton.Add_Click({ Stop-Server })
    $script:openButton.Add_Click({ try { Start-Process "http://127.0.0.1:$([int]$script:portBox.Value)/" } catch { Write-GuiLog ("Open URL failed: " + $_.Exception.ToString()) } })
    $script:copyButton.Add_Click({
        try {
            if (-not $script:lastUrls -or $script:lastUrls.Count -eq 0) { $script:lastUrls = New-ServerUrls -Port ([int]$script:portBox.Value) -RootPath (Normalize-RootPath $script:rootText.Text) }
            [System.Windows.Forms.Clipboard]::SetText(($script:lastUrls -join [Environment]::NewLine))
            Append-Log "访问地址已复制到剪贴板。"
        }
        catch {
            Write-GuiLog ("Copy URLs failed: " + $_.Exception.ToString())
            [System.Windows.Forms.MessageBox]::Show($script:form, $_.Exception.Message, "复制失败", "OK", "Error") | Out-Null
        }
    })

    $script:form.Add_Shown({
        Write-GuiLog "Form shown"
        Append-Log "GUI 已启动。选择目录后点击“启动服务”。"
        Append-Log "如果启动失败，请查看 static-dock-gui-error.log / static-dock-server-error.log。"
    })

    $script:form.Add_FormClosing({
        try {
            if ($script:serverProcess -and -not $script:serverProcess.HasExited) {
                $answer = [System.Windows.Forms.MessageBox]::Show($script:form, "服务仍在运行，关闭窗口时是否停止服务？", "StaticDock", "YesNo", "Question")
                if ($answer -eq [System.Windows.Forms.DialogResult]::Yes) { Stop-Server }
            }
        }
        catch { Write-GuiLog ("FormClosing failed: " + $_.Exception.ToString()) }
    })

    Set-RunningState $false
    Write-GuiLog "Entering Application.Run"
    [System.Windows.Forms.Application]::Run($script:form)
    Write-GuiLog "Application.Run returned"
    exit 0
}
catch {
    Write-GuiLog ("Fatal GUI error: " + $_.Exception.ToString())
    try {
        Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
        [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, "StaticDock GUI fatal error", "OK", "Error") | Out-Null
    }
    catch {}
    exit 1
}
