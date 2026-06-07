using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Windows.Forms;

static class StaticDockGuiProgram
{
    [STAThread]
    static void Main()
    {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new StaticDockForm());
    }
}

public class StaticDockForm : Form
{
    const int DefaultPort = 8787;
    const int DefaultWorkers = 128;
    const int DefaultQueue = 2048;

    readonly string dir;
    readonly string exe;
    readonly string iconFile;
    readonly string pidFile;
    readonly string settingsDir;
    readonly string settingsFile;
    Process server;
    int actualPort = DefaultPort;
    bool loading;
    string lang = "zh";

    Label titleLabel, subLabel, status, langLabel, rootLabel, portLabel, workersLabel, queueLabel, hint, addressLabel;
    TextBox rootText, addressBox, logBox;
    NumericUpDown portBox, workersBox, queueBox;
    Button browseBtn, startBtn, stopBtn, openBtn, folderBtn, copyBtn, resetBtn, checkPortBtn;
    CheckBox autoStartBox, openAfterStartBox;
    ComboBox langBox;
    Timer restartTimer;
    string lastUrls = "";

    public StaticDockForm()
    {
        dir = AppDomain.CurrentDomain.BaseDirectory.TrimEnd('\\');
        exe = Path.Combine(dir, "static-dock.exe");
        iconFile = Path.Combine(dir, "assets\\icon.png");
        pidFile = Path.Combine(dir, "static-dock-server.pid");
        settingsDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "StaticDock");
        settingsFile = Path.Combine(settingsDir, "settings.ini");
        BuildUi();
        LoadSettings();
        ApplyLanguage();
    }

    string DefaultRoot()
    {
        string resources = Path.Combine(dir, "resources");
        return Directory.Exists(resources) ? resources : dir;
    }

    string T(string key)
    {
        bool en = lang == "en";
        switch (key)
        {
            case "title": return en ? "StaticDock Resource Harbor" : "StaticDock 静态资源码头";
            case "language": return en ? "Language" : "语言";
            case "subtitle": return en ? "Default mount: resources. Changes restart the server automatically." : "默认挂载 resources 资源库；修改目录/端口后运行中会自动重启";
            case "stopped": return en ? "Stopped" : "已停止";
            case "running": return en ? "Running" : "运行中";
            case "root": return en ? "Resource folder" : "资源目录";
            case "browse": return en ? "Browse..." : "选择...";
            case "port": return en ? "Port" : "端口";
            case "workers": return en ? "Worker threads" : "工作线程";
            case "queue": return en ? "Queue size" : "队列长度";
            case "hintDefault": return en ? "Default port: 8787. Settings are saved automatically." : "默认端口 8787，设置会自动保存";
            case "hintRunning": return en ? "Server is running. Setting changes restart automatically." : "服务运行中；修改设置会自动重启";
            case "hintChanged": return en ? "Settings changed. Restarting soon..." : "设置已变更，将自动重启...";
            case "start": return en ? "Start" : "启动服务";
            case "stop": return en ? "Stop" : "停止服务";
            case "open": return en ? "Open Manager" : "打开资源库";
            case "autostart": return en ? "Auto start" : "启动时自动开启服务";
            case "openAfterStart": return en ? "Open browser after start" : "启动后自动打开资源库";
            case "openFolder": return en ? "Open Folder" : "打开文件夹";
            case "checkPort": return en ? "Check Port" : "检测端口";
            case "addresses": return en ? "Main addresses" : "常用访问地址";
            case "copy": return en ? "Copy URLs" : "复制地址";
            case "reset": return en ? "Reset" : "恢复默认";
            case "bootLog": return en ? "GUI is ready. Default mount is resources. The browser opens a resource manager page." : "GUI 已启动。默认挂载 resources 目录，打开后是资源管理页面。";
            case "restartLog": return en ? "Settings changed. Restarting server automatically..." : "设置已变更，正在自动重启服务...";
            case "chooseFolder": return en ? "Choose the resource folder to mount" : "选择要挂载的资源目录";
            case "resetAsk": return en ? "Resetting defaults requires restarting the server. Continue?" : "恢复默认设置需要重启服务，是否继续？";
            case "resetDone": return en ? "Defaults restored: resources / 8787 / 128 worker threads / 2048 queue." : "已恢复默认设置：resources / 8787 / 128 工作线程 / 2048 队列。";
            case "missingExe": return en ? "static-dock.exe was not found: " : "找不到 static-dock.exe：";
            case "missingRoot": return en ? "Resource folder does not exist: " : "资源目录不存在：";
            case "starting": return en ? "Starting StaticDock..." : "正在启动 StaticDock...";
            case "program": return en ? "Program: " : "程序：";
            case "folder": return en ? "Folder: " : "目录：";
            case "startFailedFast": return en ? "The server exited immediately. Exit code: " : "服务启动后立即退出，退出码：";
            case "launchSummary": return en ? "Server is running. Main addresses:" : "服务已启动，常用地址：";
            case "serverConfig": return en ? "Current settings:" : "当前配置：";
            case "openTip": return en ? "Click Open Manager to browse files in the browser." : "点击“打开资源库”可在浏览器中浏览文件。";
            case "started": return en ? "Server started. PID: " : "服务已启动，PID：";
            case "startFailed": return en ? "Start failed: " : "启动失败：";
            case "startFailedTitle": return en ? "Start failed" : "启动失败";
            case "copied": return en ? "URLs copied to clipboard." : "访问地址已复制。";
            case "portFree": return en ? "Port is available: " : "端口可用：";
            case "portBusy": return en ? "Port is already in use: " : "端口已被占用：";
            case "managerUrl": return en ? "Resource manager: " : "资源管理页：";
            case "emulatorUrl": return en ? "Android emulator: " : "安卓模拟器：";
            case "lanUrl": return en ? "Phone/LAN: " : "手机/局域网：";
            case "mountRoot": return en ? "Mounted folder: " : "挂载目录：";
            case "stopping": return en ? "Stopping server..." : "正在停止服务...";
            case "stoppedLog": return en ? "Server stopped. Processes cleaned: " : "服务已停止，清理进程数：";
            case "noneStopped": return en ? "No StaticDock server process was found in this folder." : "没有发现当前目录的服务进程。";
            case "stopError": return en ? "Stop error: " : "停止异常：";
            case "closeAsk": return en ? "The server is still running. Stop it before closing?" : "服务仍在运行，关闭窗口时是否停止服务？";
        }
        return key;
    }

    void BuildUi()
    {
        Text = T("title");
        TryLoadWindowIcon();
        StartPosition = FormStartPosition.CenterScreen;
        Size = new Size(940, 640);
        MinimumSize = new Size(850, 570);
        Font = new Font("Microsoft YaHei UI", 9f);
        BackColor = Color.FromArgb(245, 247, 251);

        var header = new Panel { Left = 0, Top = 0, Width = 940, Height = 92, BackColor = Color.FromArgb(30, 64, 175), Anchor = AnchorStyles.Left | AnchorStyles.Top | AnchorStyles.Right };
        Controls.Add(header);
        titleLabel = new Label { ForeColor = Color.White, Font = new Font("Microsoft YaHei UI", 19f, FontStyle.Bold), Left = 24, Top = 14, Width = 450, Height = 36 };
        header.Controls.Add(titleLabel);
        subLabel = new Label { ForeColor = Color.FromArgb(219, 234, 254), Left = 26, Top = 56, Width = 650, Height = 24 };
        header.Controls.Add(subLabel);
        status = new Label { ForeColor = Color.White, BackColor = Color.FromArgb(239, 68, 68), TextAlign = ContentAlignment.MiddleCenter, Left = 820, Top = 30, Width = 90, Height = 28, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        header.Controls.Add(status);

        langLabel = new Label { ForeColor = Color.FromArgb(219, 234, 254), Left = 650, Top = 34, Width = 70, Height = 24, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        header.Controls.Add(langLabel);
        langBox = new ComboBox { Left = 720, Top = 30, Width = 85, Height = 28, Anchor = AnchorStyles.Top | AnchorStyles.Right, DropDownStyle = ComboBoxStyle.DropDownList };
        langBox.Items.Add("中文");
        langBox.Items.Add("English");
        langBox.SelectedIndex = 0;
        langBox.SelectedIndexChanged += delegate { if (!loading) { lang = langBox.SelectedIndex == 1 ? "en" : "zh"; SaveSettings(); ApplyLanguage(); } };
        header.Controls.Add(langBox);

        rootLabel = MakeLabel(24, 118, 80, 24);
        rootText = new TextBox { Left = 116, Top = 116, Width = 610, Height = 26, Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right };
        rootText.TextChanged += SettingChanged;
        Controls.Add(rootText);
        browseBtn = new Button { Left = 742, Top = 114, Width = 110, Height = 30, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        browseBtn.Click += delegate { BrowseRoot(); };
        Controls.Add(browseBtn);

        portLabel = MakeLabel(24, 160, 70, 24);
        portBox = new NumericUpDown { Left = 116, Top = 158, Width = 90, Minimum = 1, Maximum = 65535, Value = DefaultPort };
        portBox.ValueChanged += SettingChanged;
        Controls.Add(portBox);
        workersLabel = MakeLabel(230, 160, 90, 24);
        workersBox = new NumericUpDown { Left = 330, Top = 158, Width = 90, Minimum = 1, Maximum = 512, Value = DefaultWorkers };
        workersBox.ValueChanged += SettingChanged;
        Controls.Add(workersBox);
        queueLabel = MakeLabel(450, 160, 90, 24);
        queueBox = new NumericUpDown { Left = 548, Top = 158, Width = 100, Minimum = 1, Maximum = 20000, Value = DefaultQueue };
        queueBox.ValueChanged += SettingChanged;
        Controls.Add(queueBox);

        hint = new Label { Left = 670, Top = 160, Width = 240, Height = 24, ForeColor = Color.FromArgb(100, 116, 139), Anchor = AnchorStyles.Top | AnchorStyles.Right };
        Controls.Add(hint);

        startBtn = AddButton(24, 206); startBtn.Click += delegate { StartServer(); };
        stopBtn = AddButton(146, 206); stopBtn.Click += delegate { StopServer(); };
        openBtn = AddButton(268, 206); openBtn.Click += delegate { Process.Start("http://127.0.0.1:" + actualPort + "/?lang=" + lang); };
        folderBtn = AddButton(404, 206); folderBtn.Click += delegate { OpenResourceFolder(); };
        copyBtn = AddButton(540, 206); copyBtn.Click += delegate { if (!String.IsNullOrEmpty(lastUrls)) Clipboard.SetText(lastUrls); Log(T("copied")); };
        resetBtn = AddButton(662, 206); resetBtn.Click += delegate { ResetDefaults(true); };
        checkPortBtn = AddButton(784, 206); checkPortBtn.Click += delegate { CheckPort(); };

        logBox = new TextBox { Left = 24, Top = 382, Width = 880, Height = 178, Anchor = AnchorStyles.Top | AnchorStyles.Bottom | AnchorStyles.Left | AnchorStyles.Right, Multiline = true, ScrollBars = ScrollBars.Vertical, ReadOnly = true, Font = new Font("Consolas", 9f), BackColor = Color.FromArgb(15, 23, 42), ForeColor = Color.FromArgb(226, 232, 240) };
        Controls.Add(logBox);

        autoStartBox = new CheckBox { Left = 24, Top = 252, Width = 180, Height = 24 };
        autoStartBox.CheckedChanged += SettingChanged;
        Controls.Add(autoStartBox);
        openAfterStartBox = new CheckBox { Left = 220, Top = 252, Width = 220, Height = 24 };
        openAfterStartBox.CheckedChanged += SettingChanged;
        Controls.Add(openAfterStartBox);

        addressLabel = MakeLabel(24, 292, 160, 24);
        addressLabel.Text = T("addresses");
        addressBox = new TextBox { Left = 24, Top = 320, Width = 880, Height = 52, Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right, Multiline = true, ReadOnly = true, Font = new Font("Consolas", 9f), BackColor = Color.White, ForeColor = Color.FromArgb(15, 23, 42), Text = "-" };
        Controls.Add(addressBox);

        restartTimer = new Timer();
        restartTimer.Interval = 900;
        restartTimer.Tick += delegate { restartTimer.Stop(); if (IsRunning()) { Log(T("restartLog")); StartServer(); } };
        SetRunning(false);
    }

    Label MakeLabel(int x, int y, int w, int h) { var l = new Label { Left = x, Top = y, Width = w, Height = h, ForeColor = Color.FromArgb(75, 85, 99) }; Controls.Add(l); return l; }
    Button AddButton(int x, int y) { var b = new Button { Left = x, Top = y, Width = 112, Height = 34 }; Controls.Add(b); return b; }
    void Log(string s) { if (logBox != null) logBox.AppendText(s + Environment.NewLine); }
    bool IsRunning() { return server != null && !server.HasExited; }

    void ApplyLanguage()
    {
        Text = T("title");
        titleLabel.Text = T("title");
        langLabel.Text = T("language");
        subLabel.Text = T("subtitle");
        rootLabel.Text = T("root");
        browseBtn.Text = T("browse");
        portLabel.Text = T("port");
        workersLabel.Text = T("workers");
        queueLabel.Text = T("queue");
        startBtn.Text = T("start");
        stopBtn.Text = T("stop");
        openBtn.Text = T("open");
        folderBtn.Text = T("openFolder");
        copyBtn.Text = T("copy");
        resetBtn.Text = T("reset");
        checkPortBtn.Text = T("checkPort");
        addressLabel.Text = T("addresses");
        autoStartBox.Text = T("autostart");
        openAfterStartBox.Text = T("openAfterStart");
        hint.Text = IsRunning() ? T("hintRunning") : T("hintDefault");
        status.Text = IsRunning() ? T("running") : T("stopped");
    }

    void SetRunning(bool r)
    {
        startBtn.Enabled = !r; stopBtn.Enabled = r; openBtn.Enabled = r; copyBtn.Enabled = r;
        status.Text = r ? T("running") : T("stopped");
        status.BackColor = r ? Color.FromArgb(22, 163, 74) : Color.FromArgb(239, 68, 68);
    }

    void TryLoadWindowIcon()
    {
        try
        {
            Icon embedded = Icon.ExtractAssociatedIcon(Application.ExecutablePath);
            if (embedded != null)
            {
                Icon = embedded;
                return;
            }
        }
        catch { }

        try
        {
            if (File.Exists(iconFile))
            {
                using (var bitmap = new Bitmap(iconFile))
                {
                    IntPtr handle = bitmap.GetHicon();
                    Icon = Icon.FromHandle(handle);
                }
            }
        }
        catch { }
    }

    void OpenResourceFolder()
    {
        try
        {
            string root = Path.GetFullPath(rootText.Text.Trim());
            if (!Directory.Exists(root)) throw new Exception(T("missingRoot") + root);
            Process.Start("explorer.exe", root);
        }
        catch (Exception ex)
        {
            Log(T("startFailed") + ex.Message);
        }
    }

    void CheckPort()
    {
        int port = (int)portBox.Value;
        try
        {
            TcpListener listener = new TcpListener(IPAddress.Loopback, port);
            listener.Start();
            listener.Stop();
            Log(T("portFree") + port);
        }
        catch
        {
            Log(T("portBusy") + port);
        }
    }

    void BrowseRoot()
    {
        using (var d = new FolderBrowserDialog())
        {
            d.Description = T("chooseFolder");
            d.SelectedPath = rootText.Text;
            if (d.ShowDialog(this) == DialogResult.OK) rootText.Text = d.SelectedPath;
        }
    }

    void SettingChanged(object sender, EventArgs e)
    {
        if (loading) return;
        SaveSettings();
        if (IsRunning()) { hint.Text = T("hintChanged"); restartTimer.Stop(); restartTimer.Start(); }
    }

    void ResetDefaults(bool ask)
    {
        if (ask && IsRunning() && MessageBox.Show(this, T("resetAsk"), T("title"), MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes) return;
        bool wasRunning = IsRunning();
        if (wasRunning) StopServer(false);
        loading = true;
        rootText.Text = DefaultRoot(); portBox.Value = DefaultPort; workersBox.Value = DefaultWorkers; queueBox.Value = DefaultQueue;
        autoStartBox.Checked = false; openAfterStartBox.Checked = false;
        loading = false;
        SaveSettings();
        Log(T("resetDone"));
        if (wasRunning) StartServer();
    }

    void LoadSettings()
    {
        loading = true;
        rootText.Text = DefaultRoot(); portBox.Value = DefaultPort; workersBox.Value = DefaultWorkers; queueBox.Value = DefaultQueue;
        autoStartBox.Checked = false; openAfterStartBox.Checked = false;
        try
        {
            if (File.Exists(settingsFile))
            {
                foreach (string line in File.ReadAllLines(settingsFile))
                {
                    int eq = line.IndexOf('='); if (eq <= 0) continue;
                    string k = line.Substring(0, eq); string v = line.Substring(eq + 1);
                    if (k == "root" && Directory.Exists(v)) rootText.Text = v;
                    else if (k == "port") SetNum(portBox, v);
                    else if (k == "workers") SetNum(workersBox, v);
                    else if (k == "queue") SetNum(queueBox, v);
                    else if (k == "lang") lang = v == "en" ? "en" : "zh";
                    else if (k == "autoStart") autoStartBox.Checked = v == "1" || v.Equals("true", StringComparison.OrdinalIgnoreCase);
                    else if (k == "openAfterStart") openAfterStartBox.Checked = v == "1" || v.Equals("true", StringComparison.OrdinalIgnoreCase);
                }
            }
        }
        catch { }
        langBox.SelectedIndex = lang == "en" ? 1 : 0;
        loading = false;
        SaveSettings();
        Log(T("bootLog"));
        if (autoStartBox.Checked) StartServer();
    }

    void SetNum(NumericUpDown box, string text) { decimal v; if (Decimal.TryParse(text, out v) && v >= box.Minimum && v <= box.Maximum) box.Value = v; }
    void SaveSettings()
    {
        try
        {
            if (!Directory.Exists(settingsDir)) Directory.CreateDirectory(settingsDir);
            File.WriteAllLines(settingsFile, new string[] { "root=" + rootText.Text, "port=" + (int)portBox.Value, "workers=" + (int)workersBox.Value, "queue=" + (int)queueBox.Value, "lang=" + lang, "autoStart=" + (autoStartBox.Checked ? "1" : "0"), "openAfterStart=" + (openAfterStartBox.Checked ? "1" : "0") });
        }
        catch { }
    }

    static string Q(string s) { return "\"" + s.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\""; }

    void StartServer()
    {
        try
        {
            if (!File.Exists(exe)) throw new Exception(T("missingExe") + exe);
            string root = Path.GetFullPath(rootText.Text.Trim());
            if (!Directory.Exists(root)) throw new Exception(T("missingRoot") + root);
            StopServer(false); logBox.Clear();
            int requestedPort = (int)portBox.Value; actualPort = requestedPort;
            var psi = new ProcessStartInfo();
            psi.FileName = exe; psi.WorkingDirectory = root; psi.UseShellExecute = false; psi.CreateNoWindow = true; psi.RedirectStandardOutput = true; psi.RedirectStandardError = true;
            psi.Arguments = "--root " + Q(root) + " --port " + requestedPort + " --workers " + (int)workersBox.Value + " --queue " + (int)queueBox.Value;
            server = new Process(); server.StartInfo = psi;
            server.OutputDataReceived += delegate(object sender, DataReceivedEventArgs e) { if (e.Data != null) BeginInvoke(new Action(delegate { if (e.Data.StartsWith("Port:")) { int p; if (Int32.TryParse(e.Data.Substring(5).Trim(), out p)) actualPort = p; } })); };
            server.ErrorDataReceived += delegate(object sender, DataReceivedEventArgs e) { if (e.Data != null) BeginInvoke(new Action(delegate { if (!IsBenignNetworkError(e.Data)) Log("[error] " + e.Data); })); };
            server.Start(); server.BeginOutputReadLine(); server.BeginErrorReadLine(); File.WriteAllText(pidFile, server.Id.ToString());
            System.Threading.Thread.Sleep(800);
            if (server.HasExited) throw new Exception(T("startFailedFast") + server.ExitCode);
            lastUrls = BuildUrls(actualPort, root); File.WriteAllText(Path.Combine(dir, "static-dock-urls.txt"), lastUrls, System.Text.Encoding.UTF8);
            addressBox.Text = lastUrls;
            try { Clipboard.SetText(lastUrls); } catch { }
            SaveSettings();
            Log(T("launchSummary"));
            Log(lastUrls);
            Log(T("serverConfig"));
            Log(T("mountRoot") + root);
            Log(T("port") + ": " + actualPort + "    " + T("workers") + ": " + (int)workersBox.Value + "    " + T("queue") + ": " + (int)queueBox.Value);
            Log(T("openTip"));
            SetRunning(true); hint.Text = T("hintRunning");
            if (openAfterStartBox.Checked) Process.Start("http://127.0.0.1:" + actualPort + "/?lang=" + lang);
        }
        catch (Exception ex)
        {
            Log(T("startFailed") + ex.Message); MessageBox.Show(this, ex.Message, T("startFailedTitle"), MessageBoxButtons.OK, MessageBoxIcon.Error); SetRunning(false);
        }
    }

    bool IsBenignNetworkError(string s) { return s.IndexOf("10054") >= 0 || s.IndexOf("10060") >= 0 || s.IndexOf("强迫关闭") >= 0; }

    string BuildUrls(int port, string root)
    {
        var lines = new List<string>();
        lines.Add(T("managerUrl") + "http://127.0.0.1:" + port + "/?lang=" + lang);
        lines.Add(T("emulatorUrl") + "http://10.0.2.2:" + port + "/?lang=" + lang);
        foreach (var ip in Dns.GetHostAddresses(Dns.GetHostName()))
            if (ip.AddressFamily == AddressFamily.InterNetwork && !IPAddress.IsLoopback(ip)) lines.Add(T("lanUrl") + "http://" + ip + ":" + port + "/?lang=" + lang);
        lines.Add(""); lines.Add(T("mountRoot") + root);
        return String.Join(Environment.NewLine, lines.ToArray());
    }

    void StopServer() { StopServer(true); }
    void StopServer(bool verbose)
    {
        int stopped = 0;
        try
        {
            if (server != null && !server.HasExited) { server.Kill(); server.WaitForExit(1500); stopped++; }
            if (File.Exists(pidFile))
            {
                int pid;
                if (Int32.TryParse(File.ReadAllText(pidFile).Trim(), out pid))
                {
                    try { var p = Process.GetProcessById(pid); if (!p.HasExited && p.ProcessName.IndexOf("static-dock", StringComparison.OrdinalIgnoreCase) >= 0) { p.Kill(); stopped++; } } catch { }
                }
                try { File.Delete(pidFile); } catch { }
            }
            foreach (var p in Process.GetProcessesByName("static-dock"))
            {
                try { if (String.Equals(p.MainModule.FileName, exe, StringComparison.OrdinalIgnoreCase)) { p.Kill(); stopped++; } } catch { }
            }
        }
        catch (Exception ex) { Log(T("stopError") + ex.Message); }
        server = null; SetRunning(false);
        if (verbose) Log(stopped > 0 ? T("stoppedLog") + stopped : T("noneStopped"));
    }

    protected override void OnFormClosing(FormClosingEventArgs e)
    {
        SaveSettings();
        if (server != null && !server.HasExited)
        {
            if (MessageBox.Show(this, T("closeAsk"), T("title"), MessageBoxButtons.YesNo, MessageBoxIcon.Question) == DialogResult.Yes) StopServer(false);
        }
        base.OnFormClosing(e);
    }
}
