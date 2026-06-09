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
    bool advancedVisible;
    string lang = "zh";
    string lastUrls = "";
    string localUrl = "";
    string lanUrl = "";
    string emulatorUrl = "";

    Label titleLabel, subLabel, statusLabel, langLabel, rootLabel, portLabel, workersLabel, queueLabel, hintLabel, addressLabel;
    Label localTitle, lanTitle, emulatorTitle, advancedTitle;
    ComboBox rootText;
    TextBox localText, lanText, emulatorText, logBox;
    NumericUpDown portBox, workersBox, queueBox;
    Button browseBtn, openFolderBtn, startBtn, stopBtn, checkPortBtn, resetBtn, advancedBtn;
    Button openLocalBtn, copyLocalBtn, copyLanBtn, copyEmulatorBtn, copyAllBtn;
    CheckBox autoStartBox, openAfterStartBox;
    ComboBox langBox;
    Panel advancedPanel;
    Timer restartTimer, monitorTimer;

    public StaticDockForm()
    {
        dir = AppDomain.CurrentDomain.BaseDirectory.TrimEnd('\\');
        exe = Path.Combine(dir, "static-dock.exe");
        iconFile = Path.Combine(dir, "assets\\icon.png");
        settingsDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "StaticDock");
        pidFile = Path.Combine(settingsDir, "static-dock-server.pid");
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
            case "subtitle": return en ? "Turn a local folder into a LAN resource service." : "把本地文件夹一键变成局域网资源服务";
            case "language": return en ? "Language" : "语言";
            case "running": return en ? "Running" : "运行中";
            case "stopped": return en ? "Stopped" : "已停止";
            case "root": return en ? "Resource folder" : "资源目录";
            case "browse": return en ? "Browse" : "选择目录";
            case "openFolder": return en ? "Open Folder" : "打开文件夹";
            case "start": return en ? "Start" : "启动服务";
            case "stop": return en ? "Stop" : "停止服务";
            case "checkPort": return en ? "Check Port" : "检测端口";
            case "reset": return en ? "Reset" : "恢复默认";
            case "addresses": return en ? "Access addresses" : "访问地址";
            case "local": return en ? "Local" : "本机";
            case "lan": return en ? "LAN / Phone" : "局域网/手机";
            case "emulator": return en ? "Android emulator" : "安卓模拟器";
            case "open": return en ? "Open" : "打开";
            case "copy": return en ? "Copy" : "复制";
            case "copyAll": return en ? "Copy All" : "复制全部";
            case "advancedShow": return en ? "Show advanced settings" : "显示高级设置";
            case "advancedHide": return en ? "Hide advanced settings" : "隐藏高级设置";
            case "advanced": return en ? "Advanced settings" : "高级设置";
            case "port": return en ? "Port" : "端口";
            case "workers": return en ? "Worker threads" : "工作线程";
            case "queue": return en ? "Queue size" : "队列长度";
            case "autostart": return en ? "Auto start service when GUI opens" : "启动时自动开启服务";
            case "openAfterStart": return en ? "Open resource manager after start" : "启动后自动打开资源库";
            case "hintDefault": return en ? "Default: resources folder, port 8787. Settings are saved automatically." : "默认挂载 resources，端口 8787。设置会自动保存。";
            case "hintRunning": return en ? "Server is running. Changing folder or port restarts automatically." : "服务运行中，修改目录或端口会自动重启。";
            case "hintChanged": return en ? "Settings changed. Restarting soon..." : "设置已变更，即将自动重启...";
            case "bootLog": return en ? "GUI is ready." : "GUI 已启动。";
            case "restartLog": return en ? "Settings changed. Restarting server..." : "设置已变更，正在自动重启服务...";
            case "chooseFolder": return en ? "Choose the resource folder to mount" : "选择要挂载的资源目录";
            case "missingExe": return en ? "static-dock.exe was not found: " : "找不到 static-dock.exe：";
            case "missingRoot": return en ? "Resource folder does not exist: " : "资源目录不存在：";
            case "launchSummary": return en ? "Server is running." : "服务已启动。";
            case "serverConfig": return en ? "Current settings:" : "当前配置：";
            case "openTip": return en ? "Use the address buttons above to open or copy URLs." : "可使用上方地址按钮打开或复制访问地址。";
            case "startFailedFast": return en ? "The server exited immediately. Exit code: " : "服务启动后立即退出，退出码：";
            case "startFailed": return en ? "Start failed: " : "启动失败：";
            case "startFailedTitle": return en ? "Start failed" : "启动失败";
            case "copied": return en ? "Copied to clipboard." : "已复制到剪贴板。";
            case "portFree": return en ? "Port is available: " : "端口可用：";
            case "portBusy": return en ? "Port is already in use. StaticDock may choose another port: " : "端口已被占用，StaticDock 可能会自动选择备用端口：";
            case "resetAsk": return en ? "Resetting defaults requires restarting the server. Continue?" : "恢复默认设置需要重启服务，是否继续？";
            case "resetDone": return en ? "Defaults restored." : "已恢复默认设置。";
            case "stoppedLog": return en ? "Server stopped. Processes cleaned: " : "服务已停止，清理进程数：";
            case "noneStopped": return en ? "No StaticDock server process was found in this folder." : "没有发现当前目录的服务进程。";
            case "stopError": return en ? "Stop error: " : "停止异常：";
            case "serverExited": return en ? "Server process exited." : "服务进程已退出。";
            case "closeAsk": return en ? "The server is still running. Stop it before closing?" : "服务仍在运行，关闭窗口时是否停止服务？";
        }
        return key;
    }

    void BuildUi()
    {
        Text = T("title");
        TryLoadWindowIcon();
        StartPosition = FormStartPosition.CenterScreen;
        Size = new Size(980, 680);
        MinimumSize = new Size(900, 620);
        Font = new Font("Microsoft YaHei UI", 9f);
        BackColor = Color.FromArgb(245, 247, 251);

        var header = new Panel { Left = 0, Top = 0, Width = 980, Height = 92, BackColor = Color.FromArgb(30, 64, 175), Anchor = AnchorStyles.Left | AnchorStyles.Top | AnchorStyles.Right };
        Controls.Add(header);
        titleLabel = new Label { ForeColor = Color.White, Font = new Font("Microsoft YaHei UI", 19f, FontStyle.Bold), Left = 24, Top = 14, Width = 440, Height = 36 };
        header.Controls.Add(titleLabel);
        subLabel = new Label { ForeColor = Color.FromArgb(219, 234, 254), Left = 26, Top = 56, Width = 650, Height = 24 };
        header.Controls.Add(subLabel);
        langLabel = new Label { ForeColor = Color.FromArgb(219, 234, 254), Left = 690, Top = 34, Width = 70, Height = 24, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        header.Controls.Add(langLabel);
        langBox = new ComboBox { Left = 755, Top = 30, Width = 90, Height = 28, Anchor = AnchorStyles.Top | AnchorStyles.Right, DropDownStyle = ComboBoxStyle.DropDownList };
        langBox.Items.Add("中文"); langBox.Items.Add("English"); langBox.SelectedIndex = 0;
        langBox.SelectedIndexChanged += delegate { if (!loading) { lang = langBox.SelectedIndex == 1 ? "en" : "zh"; SaveSettings(); ApplyLanguage(); RefreshUrls(); } };
        header.Controls.Add(langBox);
        statusLabel = new Label { ForeColor = Color.White, BackColor = Color.FromArgb(239, 68, 68), TextAlign = ContentAlignment.MiddleCenter, Left = 865, Top = 30, Width = 90, Height = 28, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        header.Controls.Add(statusLabel);

        var rootPanel = Card(24, 112, 930, 92);
        rootLabel = MakeLabel(rootPanel, 16, 14, 110, 24);
        rootText = new ComboBox { Left = 16, Top = 42, Width = 650, Height = 26, Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right, DropDownStyle = ComboBoxStyle.DropDown };
        rootText.TextChanged += SettingChanged;
        rootText.SelectedIndexChanged += SettingChanged;
        rootPanel.Controls.Add(rootText);
        browseBtn = new Button { Left = 682, Top = 39, Width = 110, Height = 32, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        browseBtn.Click += delegate { BrowseRoot(); };
        rootPanel.Controls.Add(browseBtn);
        openFolderBtn = new Button { Left = 805, Top = 39, Width = 110, Height = 32, Anchor = AnchorStyles.Top | AnchorStyles.Right };
        openFolderBtn.Click += delegate { OpenResourceFolder(); };
        rootPanel.Controls.Add(openFolderBtn);

        var controlPanel = Card(24, 216, 930, 86);
        startBtn = Button(controlPanel, 16, 24, 110, 34); startBtn.Click += delegate { StartServer(); };
        stopBtn = Button(controlPanel, 138, 24, 110, 34); stopBtn.Click += delegate { StopServer(); };
        checkPortBtn = Button(controlPanel, 260, 24, 110, 34); checkPortBtn.Click += delegate { CheckPort(); };
        resetBtn = Button(controlPanel, 382, 24, 110, 34); resetBtn.Click += delegate { ResetDefaults(true); };
        autoStartBox = new CheckBox { Left = 520, Top = 16, Width = 210, Height = 24 };
        autoStartBox.CheckedChanged += SettingChanged;
        controlPanel.Controls.Add(autoStartBox);
        openAfterStartBox = new CheckBox { Left = 520, Top = 46, Width = 260, Height = 24 };
        openAfterStartBox.CheckedChanged += SettingChanged;
        controlPanel.Controls.Add(openAfterStartBox);
        hintLabel = new Label { Left = 750, Top = 28, Width = 160, Height = 36, ForeColor = Color.FromArgb(100, 116, 139), Anchor = AnchorStyles.Top | AnchorStyles.Right };
        controlPanel.Controls.Add(hintLabel);

        var addrPanel = Card(24, 314, 930, 150);
        addressLabel = MakeLabel(addrPanel, 16, 12, 180, 24);
        localTitle = MakeLabel(addrPanel, 16, 44, 110, 24);
        localText = AddressBox(addrPanel, 130, 42);
        openLocalBtn = SmallButton(addrPanel, 720, 40); openLocalBtn.Click += delegate { OpenUrl(localUrl); };
        copyLocalBtn = SmallButton(addrPanel, 805, 40); copyLocalBtn.Click += delegate { CopyUrl(localUrl); };
        lanTitle = MakeLabel(addrPanel, 16, 76, 110, 24);
        lanText = AddressBox(addrPanel, 130, 74);
        copyLanBtn = SmallButton(addrPanel, 805, 72); copyLanBtn.Click += delegate { CopyUrl(lanUrl); };
        emulatorTitle = MakeLabel(addrPanel, 16, 108, 110, 24);
        emulatorText = AddressBox(addrPanel, 130, 106);
        copyEmulatorBtn = SmallButton(addrPanel, 805, 104); copyEmulatorBtn.Click += delegate { CopyUrl(emulatorUrl); };
        copyAllBtn = SmallButton(addrPanel, 720, 104); copyAllBtn.Click += delegate { CopyUrl(lastUrls); };

        advancedPanel = Card(24, 476, 930, 72);
        advancedTitle = MakeLabel(advancedPanel, 16, 12, 160, 24);
        advancedBtn = Button(advancedPanel, 760, 20, 140, 32); advancedBtn.Click += delegate { SetAdvancedVisible(!advancedVisible); };
        portLabel = MakeLabel(advancedPanel, 16, 42, 70, 24);
        portBox = new NumericUpDown { Left = 88, Top = 40, Width = 90, Minimum = 1, Maximum = 65535, Value = DefaultPort };
        portBox.ValueChanged += SettingChanged; advancedPanel.Controls.Add(portBox);
        workersLabel = MakeLabel(advancedPanel, 210, 42, 100, 24);
        workersBox = new NumericUpDown { Left = 315, Top = 40, Width = 90, Minimum = 1, Maximum = 512, Value = DefaultWorkers };
        workersBox.ValueChanged += SettingChanged; advancedPanel.Controls.Add(workersBox);
        queueLabel = MakeLabel(advancedPanel, 435, 42, 100, 24);
        queueBox = new NumericUpDown { Left = 540, Top = 40, Width = 110, Minimum = 1, Maximum = 20000, Value = DefaultQueue };
        queueBox.ValueChanged += SettingChanged; advancedPanel.Controls.Add(queueBox);

        logBox = new TextBox { Left = 24, Top = 560, Width = 930, Height = 78, Anchor = AnchorStyles.Top | AnchorStyles.Bottom | AnchorStyles.Left | AnchorStyles.Right, Multiline = true, ScrollBars = ScrollBars.Vertical, ReadOnly = true, Font = new Font("Consolas", 9f), BackColor = Color.FromArgb(15, 23, 42), ForeColor = Color.FromArgb(226, 232, 240) };
        Controls.Add(logBox);

        restartTimer = new Timer(); restartTimer.Interval = 900;
        restartTimer.Tick += delegate { restartTimer.Stop(); if (IsRunning()) { Log(T("restartLog")); StartServer(); } };
        monitorTimer = new Timer(); monitorTimer.Interval = 2000;
        monitorTimer.Tick += delegate { MonitorServer(); }; monitorTimer.Start();
        SetAdvancedVisible(false);
        SetRunning(false);
        RefreshUrls();
    }

    Panel Card(int x, int y, int w, int h)
    {
        var p = new Panel { Left = x, Top = y, Width = w, Height = h, Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right, BackColor = Color.White, BorderStyle = BorderStyle.FixedSingle };
        Controls.Add(p);
        return p;
    }

    Label MakeLabel(Control parent, int x, int y, int w, int h) { var l = new Label { Left = x, Top = y, Width = w, Height = h, ForeColor = Color.FromArgb(75, 85, 99) }; parent.Controls.Add(l); return l; }
    Button Button(Control parent, int x, int y, int w, int h) { var b = new Button { Left = x, Top = y, Width = w, Height = h }; parent.Controls.Add(b); return b; }
    Button SmallButton(Control parent, int x, int y) { return Button(parent, x, y, 72, 28); }
    TextBox AddressBox(Control parent, int x, int y) { var t = new TextBox { Left = x, Top = y, Width = 570, Height = 24, ReadOnly = true, Font = new Font("Consolas", 9f), Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right }; parent.Controls.Add(t); return t; }
    void Log(string s) { if (logBox != null) logBox.AppendText(s + Environment.NewLine); }
    bool IsRunning() { return server != null && !server.HasExited; }

    void AddRecentRoot(string root)
    {
        if (String.IsNullOrWhiteSpace(root)) return;
        root = root.Trim();
        for (int i = rootText.Items.Count - 1; i >= 0; i--)
            if (String.Equals(rootText.Items[i].ToString(), root, StringComparison.OrdinalIgnoreCase)) rootText.Items.RemoveAt(i);
        rootText.Items.Insert(0, root);
        while (rootText.Items.Count > 8) rootText.Items.RemoveAt(rootText.Items.Count - 1);
    }

    string RecentRootsLine()
    {
        var values = new List<string>();
        foreach (object item in rootText.Items) values.Add(item.ToString().Replace("|", ""));
        return String.Join("|", values.ToArray());
    }

    void LoadRecentRoots(string text)
    {
        foreach (string item in text.Split('|'))
        {
            string root = item.Trim();
            if (Directory.Exists(root)) AddRecentRoot(root);
        }
    }

    void ApplyLanguage()
    {
        Text = T("title"); titleLabel.Text = T("title"); subLabel.Text = T("subtitle"); langLabel.Text = T("language");
        statusLabel.Text = IsRunning() ? T("running") : T("stopped");
        rootLabel.Text = T("root"); browseBtn.Text = T("browse"); openFolderBtn.Text = T("openFolder");
        startBtn.Text = T("start"); stopBtn.Text = T("stop"); checkPortBtn.Text = T("checkPort"); resetBtn.Text = T("reset");
        autoStartBox.Text = T("autostart"); openAfterStartBox.Text = T("openAfterStart"); hintLabel.Text = IsRunning() ? T("hintRunning") : T("hintDefault");
        addressLabel.Text = T("addresses"); localTitle.Text = T("local"); lanTitle.Text = T("lan"); emulatorTitle.Text = T("emulator");
        openLocalBtn.Text = T("open"); copyLocalBtn.Text = T("copy"); copyLanBtn.Text = T("copy"); copyEmulatorBtn.Text = T("copy"); copyAllBtn.Text = T("copyAll");
        advancedTitle.Text = T("advanced"); portLabel.Text = T("port"); workersLabel.Text = T("workers"); queueLabel.Text = T("queue"); advancedBtn.Text = advancedVisible ? T("advancedHide") : T("advancedShow");
        RefreshUrls();
    }

    void SetAdvancedVisible(bool visible)
    {
        advancedVisible = visible;
        workersLabel.Visible = visible; workersBox.Visible = visible; queueLabel.Visible = visible; queueBox.Visible = visible;
        advancedBtn.Text = visible ? T("advancedHide") : T("advancedShow");
    }

    void SetRunning(bool r)
    {
        startBtn.Enabled = !r; stopBtn.Enabled = r; openLocalBtn.Enabled = r; copyLocalBtn.Enabled = r; copyLanBtn.Enabled = r; copyEmulatorBtn.Enabled = r; copyAllBtn.Enabled = r;
        statusLabel.Text = r ? T("running") : T("stopped");
        statusLabel.BackColor = r ? Color.FromArgb(22, 163, 74) : Color.FromArgb(239, 68, 68);
        hintLabel.Text = r ? T("hintRunning") : T("hintDefault");
        if (!r) { localText.Text = "-"; lanText.Text = "-"; emulatorText.Text = "-"; }
    }

    void RefreshUrls()
    {
        int port = actualPort > 0 ? actualPort : (int)portBox.Value;
        localUrl = "http://127.0.0.1:" + port + "/?lang=" + lang;
        emulatorUrl = "http://10.0.2.2:" + port + "/?lang=" + lang;
        lanUrl = FirstLanUrl(port);
        if (IsRunning()) { localText.Text = localUrl; lanText.Text = String.IsNullOrEmpty(lanUrl) ? "-" : lanUrl; emulatorText.Text = emulatorUrl; lastUrls = BuildUrls(port, rootText.Text); }
    }

    string FirstLanUrl(int port)
    {
        IPAddress fallback = null;
        foreach (var ip in Dns.GetHostAddresses(Dns.GetHostName()))
        {
            if (ip.AddressFamily != AddressFamily.InterNetwork || IPAddress.IsLoopback(ip) || IsSpecialIpv4(ip)) continue;
            if (IsPrivateLanIpv4(ip)) return "http://" + ip + ":" + port + "/?lang=" + lang;
            if (fallback == null) fallback = ip;
        }
        return fallback == null ? "" : "http://" + fallback + ":" + port + "/?lang=" + lang;
    }

    bool IsPrivateLanIpv4(IPAddress ip)
    {
        byte[] b = ip.GetAddressBytes();
        return b[0] == 10 || (b[0] == 172 && b[1] >= 16 && b[1] <= 31) || (b[0] == 192 && b[1] == 168);
    }

    bool IsSpecialIpv4(IPAddress ip)
    {
        byte[] b = ip.GetAddressBytes();
        return b[0] == 0 || b[0] == 127 || (b[0] == 169 && b[1] == 254) || (b[0] == 198 && (b[1] == 18 || b[1] == 19)) || b[0] >= 224;
    }

    void TryLoadWindowIcon()
    {
        try { Icon embedded = Icon.ExtractAssociatedIcon(Application.ExecutablePath); if (embedded != null) { Icon = embedded; return; } } catch { }
        try { if (File.Exists(iconFile)) using (var bitmap = new Bitmap(iconFile)) Icon = Icon.FromHandle(bitmap.GetHicon()); } catch { }
    }

    void MonitorServer()
    {
        if (server != null && server.HasExited) { server = null; SetRunning(false); Log(T("serverExited")); }
    }

    void OpenUrl(string url) { if (!String.IsNullOrEmpty(url) && url != "-") Process.Start(url); }
    void CopyUrl(string url) { if (!String.IsNullOrEmpty(url) && url != "-") { Clipboard.SetText(url); Log(T("copied")); } }

    void OpenResourceFolder()
    {
        try { string root = Path.GetFullPath(rootText.Text.Trim()); if (!Directory.Exists(root)) throw new Exception(T("missingRoot") + root); Process.Start("explorer.exe", root); }
        catch (Exception ex) { Log(T("startFailed") + ex.Message); }
    }

    void CheckPort()
    {
        int port = (int)portBox.Value;
        try { TcpListener listener = new TcpListener(IPAddress.Any, port); listener.Start(); listener.Stop(); Log(T("portFree") + port); }
        catch { Log(T("portBusy") + port); }
    }

    void BrowseRoot()
    {
        using (var d = new FolderBrowserDialog()) { d.Description = T("chooseFolder"); d.SelectedPath = rootText.Text; if (d.ShowDialog(this) == DialogResult.OK) { rootText.Text = d.SelectedPath; AddRecentRoot(d.SelectedPath); } }
    }

    void SettingChanged(object sender, EventArgs e)
    {
        if (loading) return;
        SaveSettings(); RefreshUrls();
        if (IsRunning()) { hintLabel.Text = T("hintChanged"); restartTimer.Stop(); restartTimer.Start(); }
    }

    void ResetDefaults(bool ask)
    {
        if (ask && IsRunning() && MessageBox.Show(this, T("resetAsk"), T("title"), MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes) return;
        bool wasRunning = IsRunning(); if (wasRunning) StopServer(false);
        loading = true;
        rootText.Text = DefaultRoot(); AddRecentRoot(rootText.Text); portBox.Value = DefaultPort; workersBox.Value = DefaultWorkers; queueBox.Value = DefaultQueue; autoStartBox.Checked = false; openAfterStartBox.Checked = false;
        loading = false; SaveSettings(); RefreshUrls(); Log(T("resetDone")); if (wasRunning) StartServer();
    }

    void LoadSettings()
    {
        loading = true;
        rootText.Items.Clear();
        rootText.Text = DefaultRoot(); AddRecentRoot(rootText.Text); portBox.Value = DefaultPort; workersBox.Value = DefaultWorkers; queueBox.Value = DefaultQueue; autoStartBox.Checked = false; openAfterStartBox.Checked = false;
        try
        {
            if (File.Exists(settingsFile)) foreach (string line in File.ReadAllLines(settingsFile))
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
                else if (k == "recentRoots") LoadRecentRoots(v);
            }
        }
        catch { }
        langBox.SelectedIndex = lang == "en" ? 1 : 0;
        loading = false; SaveSettings(); Log(T("bootLog")); if (autoStartBox.Checked) StartServer();
    }

    void SetNum(NumericUpDown box, string text) { decimal v; if (Decimal.TryParse(text, out v) && v >= box.Minimum && v <= box.Maximum) box.Value = v; }
    void SaveSettings()
    {
        try
        {
            if (!Directory.Exists(settingsDir)) Directory.CreateDirectory(settingsDir);
            File.WriteAllLines(settingsFile, new string[] { "root=" + rootText.Text, "port=" + (int)portBox.Value, "workers=" + (int)workersBox.Value, "queue=" + (int)queueBox.Value, "lang=" + lang, "autoStart=" + (autoStartBox.Checked ? "1" : "0"), "openAfterStart=" + (openAfterStartBox.Checked ? "1" : "0"), "recentRoots=" + RecentRootsLine() });
        }
        catch { }
    }

    static string Q(string s) { return "\"" + s.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\""; }

    void StartServer()
    {
        try
        {
            if (!File.Exists(exe)) throw new Exception(T("missingExe") + exe);
            string root = Path.GetFullPath(rootText.Text.Trim()); if (!Directory.Exists(root)) throw new Exception(T("missingRoot") + root);
            AddRecentRoot(root);
            StopServer(false); logBox.Clear(); actualPort = (int)portBox.Value;
            var psi = new ProcessStartInfo();
            psi.FileName = exe; psi.WorkingDirectory = root; psi.UseShellExecute = false; psi.CreateNoWindow = true; psi.RedirectStandardOutput = true; psi.RedirectStandardError = true;
            psi.Arguments = "--root " + Q(root) + " --port " + actualPort + " --workers " + (int)workersBox.Value + " --queue " + (int)queueBox.Value;
            server = new Process(); server.StartInfo = psi;
            server.OutputDataReceived += delegate(object sender, DataReceivedEventArgs e) { if (e.Data != null) { if (e.Data.StartsWith("Port:")) { int p; if (Int32.TryParse(e.Data.Substring(5).Trim(), out p)) actualPort = p; } BeginInvoke(new Action(delegate { RefreshUrls(); })); } };
            server.ErrorDataReceived += delegate(object sender, DataReceivedEventArgs e) { if (e.Data != null) BeginInvoke(new Action(delegate { if (!IsBenignNetworkError(e.Data)) Log("[error] " + e.Data); })); };
            server.Start(); server.BeginOutputReadLine(); server.BeginErrorReadLine(); TryWriteText(pidFile, server.Id.ToString());
            System.Threading.Thread.Sleep(800); if (server.HasExited) throw new Exception(T("startFailedFast") + server.ExitCode);
            RefreshUrls(); lastUrls = BuildUrls(actualPort, root); TryWriteText(Path.Combine(settingsDir, "static-dock-urls.txt"), lastUrls);
            try { Clipboard.SetText(lastUrls); } catch { }
            SetRunning(true); RefreshUrls(); SaveSettings();
            Log(T("launchSummary")); Log(T("serverConfig")); Log(T("port") + ": " + actualPort + "    " + T("workers") + ": " + (int)workersBox.Value + "    " + T("queue") + ": " + (int)queueBox.Value); Log(T("openTip"));
            if (openAfterStartBox.Checked) OpenUrl(localUrl);
        }
        catch (Exception ex) { Log(T("startFailed") + ex.Message); MessageBox.Show(this, ex.Message, T("startFailedTitle"), MessageBoxButtons.OK, MessageBoxIcon.Error); SetRunning(false); }
    }

    bool IsBenignNetworkError(string s) { return s.IndexOf("10054") >= 0 || s.IndexOf("10060") >= 0 || s.IndexOf("强迫关闭") >= 0; }

    void TryWriteText(string path, string text)
    {
        try
        {
            string parent = Path.GetDirectoryName(path);
            if (!String.IsNullOrEmpty(parent) && !Directory.Exists(parent)) Directory.CreateDirectory(parent);
            File.WriteAllText(path, text, System.Text.Encoding.UTF8);
        }
        catch { }
    }

    string BuildUrls(int port, string root)
    {
        string local = "http://127.0.0.1:" + port + "/?lang=" + lang;
        string lan = FirstLanUrl(port);
        string emulator = "http://10.0.2.2:" + port + "/?lang=" + lang;
        var lines = new List<string>();
        lines.Add(T("local") + ": " + local);
        if (!String.IsNullOrEmpty(lan)) lines.Add(T("lan") + ": " + lan);
        lines.Add(T("emulator") + ": " + emulator);
        lines.Add(""); lines.Add(T("root") + ": " + root);
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
                int pid; if (Int32.TryParse(File.ReadAllText(pidFile).Trim(), out pid)) try { var p = Process.GetProcessById(pid); if (!p.HasExited && p.ProcessName.IndexOf("static-dock", StringComparison.OrdinalIgnoreCase) >= 0) { p.Kill(); stopped++; } } catch { }
                try { File.Delete(pidFile); } catch { }
            }
            foreach (var p in Process.GetProcessesByName("static-dock")) try { if (String.Equals(p.MainModule.FileName, exe, StringComparison.OrdinalIgnoreCase)) { p.Kill(); stopped++; } } catch { }
        }
        catch (Exception ex) { Log(T("stopError") + ex.Message); }
        server = null; SetRunning(false); if (verbose) Log(stopped > 0 ? T("stoppedLog") + stopped : T("noneStopped"));
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
