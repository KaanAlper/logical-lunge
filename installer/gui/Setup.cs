using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.IO;
using System.Runtime.InteropServices;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// Logical Lunge Setup: the windowed face of the install. It collects the choices, runs install.ps1 (embedded in this
// exe) hidden with LL_DRIVER, and shows its phases and setup.ps1's steps. The install itself lives in one place:
// install.ps1 + installer\setup.ps1, the same path as the one-line install.
static class Program
{
    [STAThread]
    static void Main(string[] args)
    {
        // --shots <folder>: every page drawn to a PNG (CI looks at them; nothing is installed)
        if (args.Length == 2 && args[0] == "--shots") { SetupForm.Shots(args[1]); return; }
        try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new SetupForm());
    }

    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr value);
}

sealed class SetupForm : Form
{
    // ---------------------------------------------------------------- texts (Turkish / English)
    bool tr = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr";
    string T(string turkish, string english) { return tr ? turkish : english; }

    static readonly string[][] Languages =
    {
        new[] { "system", "" }, new[] { "tr", "Türkçe" }, new[] { "en", "English" }, new[] { "de", "Deutsch" },
        new[] { "fr", "Français" }, new[] { "es", "Español" }, new[] { "it", "Italiano" }, new[] { "pt", "Português" },
        new[] { "ru", "Русский" }, new[] { "uk", "Українська" }, new[] { "pl", "Polski" }, new[] { "ja", "日本語" },
        new[] { "zh", "中文" }, new[] { "ko", "한국어" }, new[] { "ar", "العربية" },
    };
    static readonly string[] Colors = { "#b69df8", "#8ab4f8", "#7fd4c9", "#a6d189", "#f5a3c7", "#ffb77c", "#f28b82" };

    // ---------------------------------------------------------------- choices
    string edition = "native-ui", language = "system", accentHex = "#b69df8", clock = "24";
    bool terminal = true, sensors = true, everything = true;

    // ---------------------------------------------------------------- install state
    enum Page { Interface, Personalize, Progress, Result }
    Page page = Page.Interface;
    string work, phase = "", version = "", setupProgress;
    int downloadPercent; long downloadDone, downloadTotal;
    string[] steps = new string[0];
    int stepN; string stepState = "running"; int stepPercent; string stepFile;
    string resultTitle, resultBody, resultDetail, resultLog, warnTitle, warnBody;
    string[] warn = new string[0];
    bool resultOk, cancelRequested;
    Process proc;
    readonly Timer timer = new Timer { Interval = 50 };
    int tick;
    DateTime slideStart = DateTime.UtcNow;

    // ---------------------------------------------------------------- drawing
    float k = 1f; // DIPs -> pixels
    static readonly Color Bg = Hex("#141218"), Surface = Hex("#1d1b20"), Surface2 = Hex("#2b2930"), Outline = Hex("#49454f"),
        Fg = Hex("#e6e0e9"), Sub = Hex("#cac4d0"), Dim = Hex("#938f99"), Ok = Hex("#a8dab5"), Err = Hex("#f2b8b5");
    Color Accent { get { return Hex(accentHex); } }
    readonly Dictionary<string, Font> fonts = new Dictionary<string, Font>();
    sealed class Hit { public RectangleF Rect; public string Id; public Action Click; }
    readonly List<Hit> hits = new List<Hit>();
    string hover;

    const float W = 760, H = 540;
    const double SlideMs = 220;

    public SetupForm()
    {
        Text = "Logical Lunge";
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.CenterScreen;
        BackColor = Bg;
        DoubleBuffered = true;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
        try { this.Icon = System.Drawing.Icon.ExtractAssociatedIcon(Application.ExecutablePath); } catch { }
        string marker = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), @"LogicalLunge\EDITION");
        try { if (File.Exists(marker)) { var e = File.ReadAllText(marker).Trim(); if (e == "web-ui" || e == "native-ui") edition = e; } } catch { }
        timer.Tick += (s, e) => OnTick();
        timer.Start();
    }

    protected override CreateParams CreateParams
    {
        get { var cp = base.CreateParams; cp.ClassStyle |= 0x20000; /* CS_DROPSHADOW */ return cp; }
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        k = DeviceDpi / 96f;
        ClientSize = new Size((int)(W * k), (int)(H * k));
        CenterToScreen();
        // Windows 11: rounded corners and the dark frame; Windows 10 ignores both
        int round = 2, dark = 1;
        DwmSetWindowAttribute(Handle, 33 /* DWMWA_WINDOW_CORNER_PREFERENCE */, ref round, 4);
        DwmSetWindowAttribute(Handle, 20 /* DWMWA_USE_IMMERSIVE_DARK_MODE */, ref dark, 4);
    }

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == 0x02E0 /* WM_DPICHANGED */)
        {
            k = (m.WParam.ToInt32() & 0xFFFF) / 96f;
            ClearFonts();
            var r = (RECT)Marshal.PtrToStructure(m.LParam, typeof(RECT));
            SetBounds(r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top);
            Invalidate();
            return;
        }
        base.WndProc(ref m);
    }

    // ---------------------------------------------------------------- pages

    protected override void OnPaint(PaintEventArgs e)
    {
        Render(e.Graphics);
    }

    void Render(Graphics g)
    {
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
        g.Clear(Bg);
        hits.Clear();
        g.ScaleTransform(k, k);

        // header: name, step dots, close
        FillCircle(g, Accent, 30, 30, 6);
        Str(g, "Logical Lunge", F(15, true), Fg, 44, 19, 300, 24);
        int dot = page == Page.Interface ? 0 : page == Page.Personalize ? 1 : 2;
        for (int i = 0; i < 3; i++) FillRound(g, i == dot ? Accent : Outline, W - 140 + i * 18, 26, i == dot ? 22 : 8, 8, 4);
        var close = new RectangleF(W - 52, 14, 36, 32);
        if (hover == "close") FillRound(g, Surface2, close.X, close.Y, close.Width, close.Height, 10);
        Glyph(g, "close", close, Sub);
        AddHit(close, "close", CloseClicked);

        // the page slides in from the right
        float t = (float)Math.Min(1.0, (DateTime.UtcNow - slideStart).TotalMilliseconds / SlideMs);
        float off = 28 * (1 - (1 - (1 - t) * (1 - t)));
        g.TranslateTransform(off, 0);
        if (page == Page.Interface) PaintInterface(g);
        else if (page == Page.Personalize) PaintPersonalize(g);
        else if (page == Page.Progress) PaintProgress(g);
        else PaintResult(g);
        g.ResetTransform();
    }

    void PaintInterface(Graphics g)
    {
        Str(g, T("Hangi arayüzü kurmak istersin?", "Which interface do you want?"), F(24, true), Fg, 40, 82, 680, 36);
        Str(g, T("İkisi de aynı masaüstü; sonradan değiştirebilirsin.", "Both are the same desktop; you can switch later."), F(13), Sub, 40, 120, 680, 22);
        Card(g, "native-ui", 40, 170, "Native", T("Bar, Super menüsü, sağ panel, ayarlar, Dock ve bildirimler tamamen yerel çizilir: en hızlısı.",
            "The bar, Super menu, right panel, settings, Dock and notifications are all drawn natively: the fastest."), T("Önerilen", "Recommended"));
        Card(g, "web-ui", 392, 170, "Web UI", T("Aynı masaüstü; bar ve paneller React / WebView2 pencereleriyle çizilir.",
            "The same desktop; the bar and panels are drawn in React / WebView2 windows."), null);
        Button(g, "next", T("Devam", "Next"), W - 40 - 140, H - 72, 140, true, () => Go(Page.Personalize));
    }

    void Card(Graphics g, string id, float x, float y, string title, string body, string badge)
    {
        var r = new RectangleF(x, y, 328, 250);
        bool sel = edition == id;
        FillRound(g, sel ? Mix(Surface2, Accent, 0.10f) : hover == id ? Surface2 : Surface, r.X, r.Y, r.Width, r.Height, 22);
        if (sel) DrawRound(g, Accent, 2, r.X + 1, r.Y + 1, r.Width - 2, r.Height - 2, 21);
        // a tiny bar illustration
        FillRound(g, Bg, x + 24, y + 26, 280, 86, 14);
        FillRound(g, sel ? Accent : Outline, x + 36, y + 38, id == "native-ui" ? 256 : 120, 12, 6);
        for (int i = 0; i < 4; i++) FillRound(g, Outline, x + 36 + i * 44, y + 62, 36, 36, 10);
        Str(g, title, F(18, true), Fg, x + 24, y + 128, 200, 28);
        if (badge != null)
        {
            float bw = Measure(g, badge, F(11, true)) + 24;
            FillRound(g, Mix(Surface, Accent, 0.25f), x + r.Width - 24 - bw, y + 131, bw, 22, 11);
            StrCenter(g, badge, F(11, true), Accent, new RectangleF(x + r.Width - 24 - bw, y + 131, bw, 22));
        }
        StrWrap(g, body, F(13), Sub, x + 24, y + 162, 280, 70);
        if (sel) { FillCircle(g, Accent, x + r.Width - 30, y + r.Height - 30, 12); Glyph(g, "check", new RectangleF(x + r.Width - 42, y + r.Height - 42, 24, 24), Hex("#1d1b20")); }
        AddHit(r, id, () => { edition = id; Invalidate(); });
    }

    void PaintPersonalize(Graphics g)
    {
        Str(g, T("Kişiselleştir", "Make it yours"), F(24, true), Fg, 40, 82, 680, 36);

        Str(g, T("Dil", "Language"), F(13, true), Sub, 40, 132, 200, 20);
        float cx = 40, cy = 158;
        foreach (var l in Languages)
        {
            string label = l[0] == "system" ? T("Sistem dili", "System language") : l[1];
            float w = Measure(g, label, F(12.5f)) + 28;
            if (cx + w > W - 40) { cx = 40; cy += 34; }
            Chip(g, "lang:" + l[0], label, cx, cy, w, language == l[0], () =>
            {
                language = l[0];
                if (language != "system") tr = language == "tr";
                else tr = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr";
                Invalidate();
            });
            cx += w + 8;
        }

        float y = cy + 44;
        Str(g, T("Vurgu rengi", "Accent color"), F(13, true), Sub, 40, y, 200, 20);
        Str(g, T("Saat", "Clock"), F(13, true), Sub, 420, y, 200, 20);
        for (int i = 0; i < Colors.Length; i++)
        {
            string c = Colors[i];
            var r = new RectangleF(40 + i * 44, y + 28, 32, 32);
            if (accentHex == c) DrawRound(g, Fg, 2, r.X - 4, r.Y - 4, r.Width + 8, r.Height + 8, 20);
            FillCircle(g, Hex(c), r.X + 16, r.Y + 16, hover == "color:" + c ? 17 : 16);
            AddHit(r, "color:" + c, () => { accentHex = c; Invalidate(); });
        }
        string now24 = DateTime.Now.ToString("HH:mm"), now12 = DateTime.Now.ToString("h:mm tt", System.Globalization.CultureInfo.InvariantCulture);
        Segment(g, 420, y + 26, new[] { "24", "12" }, new[] { T("24 saat", "24-hour") + "  " + now24, T("12 saat", "12-hour") + "  " + now12 });

        y += 84;
        Str(g, T("Ek bileşenler", "Extras"), F(13, true), Sub, 40, y, 300, 20);
        Toggle(g, "terminal", T("Terminal: WezTerm + fish + starship", "Terminal: WezTerm + fish + starship"), 40, y + 26, terminal, () => { terminal = !terminal; Invalidate(); });
        Toggle(g, "sensors", T("CPU sıcaklığı: PawnIO sürücüsü", "CPU temperature: PawnIO driver"), 40, y + 60, sensors, () => { sensors = !sensors; Invalidate(); });
        Toggle(g, "everything", T("Dosya araması: Everything", "File search: Everything"), 40, y + 94, everything, () => { everything = !everything; Invalidate(); });

        Button(g, "back", T("Geri", "Back"), 40, H - 72, 120, false, () => Go(Page.Interface));
        Button(g, "install", T("Kur", "Install"), W - 40 - 160, H - 72, 160, true, StartInstall);
        Str(g, T("Windows bir kez yönetici izni isteyecek.", "Windows will ask for permission once."), F(12), Dim, 180, H - 60, 380, 20);
    }

    void PaintProgress(Graphics g)
    {
        Str(g, cancelRequested ? T("Geri alınıyor…", "Rolling back…") : T("Kuruluyor", "Installing"), F(24, true), Fg, 40, 82, 600, 36);
        if (version.Length > 0)
        {
            string v = version.Replace("-" + edition, "");
            Str(g, v + " · " + (edition == "web-ui" ? "Web UI" : "Native"), F(13), Sub, 40, 120, 600, 22);
        }
        float y = 156;
        if (steps.Length == 0)
        {
            // before setup.ps1: this exe's own phases
            string[] ids = { "release", "download", "verify", "extract", "stop", "uac" };
            string[] labels =
            {
                T("Son sürüm aranıyor", "Finding the latest release"), T("İndiriliyor", "Downloading"), T("Doğrulanıyor", "Verifying"),
                T("Açılıyor", "Extracting"), T("Masaüstü durduruluyor", "Stopping the desktop"), T("Windows yönetici izni istiyor", "Windows asks for permission"),
            };
            int cur = Array.IndexOf(ids, phase);
            for (int i = 0; i < ids.Length; i++)
            {
                if (i > cur + 1 && i >= 4) break; // stop / uac show up when they happen
                StepRow(g, y, labels[i], i < cur ? 2 : i == cur ? 1 : 0);
                if (i == cur && phase == "download")
                {
                    ProgressBar(g, 76, y + 30, 420, downloadPercent / 100f);
                    string mb = downloadTotal > 0 ? string.Format("{0:0.0} / {1:0.0} MB", downloadDone / 1048576.0, downloadTotal / 1048576.0) : "";
                    Str(g, downloadPercent + "%   " + mb, F(12), Dim, 510, y + 22, 220, 20);
                    y += 26;
                }
                y += 34;
            }
        }
        else
        {
            StepRow(g, y, T("Hazırlık", "Preparation"), 2);
            y += 30;
            for (int i = 0; i < steps.Length; i++)
            {
                int state = i + 1 < stepN || stepState == "done" ? 2 : i + 1 == stepN ? (stepState == "error" ? 3 : 1) : 0;
                StepRow(g, y, steps[i], state);
                if (state == 1)
                {
                    ProgressBar(g, 76, y + 26, 420, Math.Max(0, Math.Min(99, stepPercent)) / 100f);
                    if (!string.IsNullOrEmpty(stepFile)) Str(g, stepFile, F(11.5f), Dim, 510, y + 18, 220, 20);
                    y += 18;
                }
                y += 24;
            }
        }
        if (!cancelRequested) Button(g, "cancel", T("İptal", "Cancel"), W - 40 - 120, H - 72, 120, false, RequestCancel);
    }

    void PaintResult(Graphics g)
    {
        var mark = resultOk ? Ok : Err;
        FillCircle(g, Mix(Bg, mark, 0.18f), 72, 118, 32);
        Glyph(g, resultOk ? "check" : "close", new RectangleF(52, 98, 40, 40), mark);
        Str(g, resultTitle ?? "", F(22, true), Fg, 120, 100, 600, 34);
        float y = 170;
        y = StrWrap(g, resultBody ?? "", F(13), Sub, 40, y, 680, 220) + 12;
        if (!string.IsNullOrEmpty(resultDetail)) y = StrWrap(g, resultDetail, F(12), Dim, 40, y, 680, 60) + 8;
        if (warn.Length > 0)
        {
            FillRound(g, Mix(Surface, Hex("#ffb77c"), 0.12f), 40, y, 680, 30 + warn.Length * 22 + 26, 16);
            Str(g, warnTitle ?? "", F(13, true), Hex("#ffb77c"), 56, y + 10, 640, 20);
            for (int i = 0; i < warn.Length; i++) Str(g, "•  " + warn[i], F(12.5f), Fg, 56, y + 34 + i * 22, 640, 20);
            StrWrap(g, warnBody ?? "", F(12), Dim, 56, y + 34 + warn.Length * 22, 640, 30);
        }
        if (!string.IsNullOrEmpty(resultLog)) Str(g, resultLog, F(11.5f), Dim, 40, H - 100, 560, 18);
        Button(g, "done", T("Kapat", "Close"), W - 40 - 140, H - 72, 140, true, Close);
    }

    // ---------------------------------------------------------------- controls (drawn)

    void Button(Graphics g, string id, string label, float x, float y, float w, bool primary, Action click)
    {
        bool hot = hover == id;
        var bg = primary ? (hot ? Mix(Accent, Color.White, 0.12f) : Accent) : (hot ? Surface2 : Surface);
        FillRound(g, bg, x, y, w, 44, 22);
        StrCenter(g, label, F(14, true), primary ? Hex("#1d1b20") : Fg, new RectangleF(x, y, w, 44));
        AddHit(new RectangleF(x, y, w, 44), id, click);
    }

    void Chip(Graphics g, string id, string label, float x, float y, float w, bool on, Action click)
    {
        FillRound(g, on ? Mix(Surface, Accent, 0.28f) : hover == id ? Surface2 : Surface, x, y, w, 28, 14);
        if (on) DrawRound(g, Accent, 1.2f, x + 0.6f, y + 0.6f, w - 1.2f, 26.8f, 13.4f);
        StrCenter(g, label, F(12.5f), on ? Fg : Sub, new RectangleF(x, y, w, 28));
        AddHit(new RectangleF(x, y, w, 28), id, click);
    }

    void Segment(Graphics g, float x, float y, string[] values, string[] labels)
    {
        float w = 146;
        FillRound(g, Surface, x, y, w * values.Length + 8, 40, 20);
        for (int i = 0; i < values.Length; i++)
        {
            string v = values[i];
            var r = new RectangleF(x + 4 + i * w, y + 4, w, 32);
            if (clock == v) FillRound(g, Accent, r.X, r.Y, r.Width, r.Height, 16);
            else if (hover == "clock:" + v) FillRound(g, Surface2, r.X, r.Y, r.Width, r.Height, 16);
            StrCenter(g, labels[i], F(12.5f), clock == v ? Hex("#1d1b20") : Sub, r);
            AddHit(r, "clock:" + v, () => { clock = v; Invalidate(); });
        }
    }

    void Toggle(Graphics g, string id, string label, float x, float y, bool on, Action click)
    {
        var row = new RectangleF(x, y, 680, 30);
        if (hover == id) FillRound(g, Surface, row.X - 8, row.Y - 2, row.Width + 16, row.Height + 4, 12);
        FillRound(g, on ? Accent : Surface2, x, y + 4, 40, 22, 11);
        if (!on) DrawRound(g, Outline, 1.2f, x + 0.6f, y + 4.6f, 38.8f, 20.8f, 10.4f);
        FillCircle(g, on ? Hex("#1d1b20") : Dim, on ? x + 29 : x + 11, y + 15, on ? 8 : 6);
        Str(g, label, F(13), on ? Fg : Sub, x + 54, y + 4, 600, 22);
        AddHit(row, id, click);
    }

    // state: 0 pending, 1 running, 2 done, 3 failed
    void StepRow(Graphics g, float y, string label, int state)
    {
        float cx = 56, cy = y + 11;
        if (state == 2) { FillCircle(g, Mix(Bg, Ok, 0.22f), cx, cy, 10); Glyph(g, "check", new RectangleF(cx - 8, cy - 8, 16, 16), Ok); }
        else if (state == 3) { FillCircle(g, Mix(Bg, Err, 0.22f), cx, cy, 10); Glyph(g, "close", new RectangleF(cx - 8, cy - 8, 16, 16), Err); }
        else if (state == 1)
        {
            using (var p = new Pen(Accent, 2.4f) { StartCap = LineCap.Round, EndCap = LineCap.Round })
                g.DrawArc(p, cx - 8, cy - 8, 16, 16, (tick * 18) % 360, 260);
        }
        else FillCircle(g, Outline, cx, cy, 3);
        Str(g, label, F(13, state == 1), state == 0 ? Dim : Fg, 76, y, 600, 22);
    }

    void ProgressBar(Graphics g, float x, float y, float w, float frac)
    {
        FillRound(g, Surface2, x, y, w, 6, 3);
        float fw = Math.Max(6, w * Math.Max(0, Math.Min(1, frac)));
        FillRound(g, Accent, x, y, fw, 6, 3);
    }

    // ---------------------------------------------------------------- input

    void AddHit(RectangleF r, string id, Action click) { hits.Add(new Hit { Rect = r, Id = id, Click = click }); }

    Hit HitAt(Point p)
    {
        float x = p.X / k, y = p.Y / k;
        for (int i = hits.Count - 1; i >= 0; i--) if (hits[i].Rect.Contains(x, y)) return hits[i];
        return null;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        var h = HitAt(e.Location);
        string id = h == null ? null : h.Id;
        Cursor = h == null ? Cursors.Default : Cursors.Hand;
        if (id != hover) { hover = id; Invalidate(); }
    }

    protected override void OnMouseLeave(EventArgs e)
    {
        if (hover != null) { hover = null; Invalidate(); }
    }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        if (e.Button != MouseButtons.Left) return;
        var h = HitAt(e.Location);
        if (h != null) { h.Click(); return; }
        // anywhere else moves the window
        ReleaseCapture();
        SendMessage(Handle, 0xA1 /* WM_NCLBUTTONDOWN */, (IntPtr)2 /* HTCAPTION */, IntPtr.Zero);
    }

    protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
    {
        if (keyData == Keys.Escape) { CloseClicked(); return true; }
        if (keyData == Keys.Enter)
        {
            if (page == Page.Interface) Go(Page.Personalize);
            else if (page == Page.Personalize) StartInstall();
            else if (page == Page.Result) Close();
            return true;
        }
        return base.ProcessCmdKey(ref msg, keyData);
    }

    void Go(Page p)
    {
        page = p;
        slideStart = DateTime.UtcNow;
        hover = null;
        StartFrames();
    }

    // The page slide is drawn on the compositor's frame clock: the 50 ms form timer gave it four or five frames and
    // Invalidate() waited behind every other message, so Devam / Geri moved in jumps. Each frame paints at once
    // (Refresh) and then waits for the next composition pass (DwmFlush, the display's refresh); the message loop runs
    // between frames, so clicks and keys stay live.
    bool animating;
    void StartFrames()
    {
        if (animating) { Invalidate(); return; }
        animating = true;
        BeginInvoke((Action)Frame);
    }

    void Frame()
    {
        if (IsDisposed) return;
        Refresh();
        if ((DateTime.UtcNow - slideStart).TotalMilliseconds >= SlideMs + 20) { animating = false; return; }
        if (DwmFlush() != 0) System.Threading.Thread.Sleep(8); // composition off: about the same pace
        BeginInvoke((Action)Frame);
    }

    void CloseClicked()
    {
        if (page == Page.Progress) RequestCancel();
        else Close();
    }

    // ---------------------------------------------------------------- the install

    void StartInstall()
    {
        try
        {
            work = Path.Combine(Path.GetTempPath(), "ll-setup-" + Guid.NewGuid().ToString("N").Substring(0, 8));
            Directory.CreateDirectory(work);
            string script = Path.Combine(work, "install.ps1");
            using (var res = typeof(Program).Assembly.GetManifestResourceStream("install.ps1"))
            using (var file = File.Create(script))
                res.CopyTo(file);
            var psi = new ProcessStartInfo("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -File \"" + script + "\"")
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = work,
            };
            psi.EnvironmentVariables["LL_DRIVER"] = work;
            psi.EnvironmentVariables["LL_EDITION"] = edition;
            psi.EnvironmentVariables["LL_LANGUAGE"] = language;
            psi.EnvironmentVariables["LL_FOCUS_COLOR"] = accentHex;
            psi.EnvironmentVariables["LL_CLOCK"] = clock;
            if (!terminal) psi.EnvironmentVariables["LL_NO_TERMINAL"] = "1";
            if (!sensors) psi.EnvironmentVariables["LL_NO_SENSORS"] = "1";
            if (!everything) psi.EnvironmentVariables["LL_NO_EVERYTHING"] = "1";
            proc = Process.Start(psi);
            phase = "release";
            Go(Page.Progress);
        }
        catch (Exception ex)
        {
            Finish(false, T("Kurulum başlatılamadı", "Setup could not start"), ex.Message, null, null);
        }
    }

    void RequestCancel()
    {
        if (cancelRequested || work == null) return;
        cancelRequested = true;
        try { File.WriteAllText(Path.Combine(work, "cancel"), ""); } catch { }
        Invalidate();
    }

    void OnTick()
    {
        tick++;
        if (page == Page.Progress && tick % 2 == 0) Poll();
        // the slide has its own frames (Frame)
        if (page == Page.Progress && !animating) Invalidate();
    }

    void Poll()
    {
        var st = ReadJson(Path.Combine(work, "status.json"));
        if (st != null)
        {
            phase = Str(st, "phase") ?? phase;
            version = Str(st, "version") ?? version;
            downloadPercent = Int(st, "percent", downloadPercent);
            downloadDone = Long(st, "done", downloadDone);
            downloadTotal = Long(st, "total", downloadTotal);
            setupProgress = Str(st, "progress") ?? setupProgress;
            var s = Strings(st, "steps");
            if (s.Length > 0) steps = s;
            if (phase == "done" || phase == "error" || phase == "cancelled")
            {
                warnTitle = Str(st, "warnTitle");
                warnBody = Str(st, "warnBody");
                warn = Strings(st, "warn");
                resultLog = Str(st, "log");
                Finish(phase == "done", Str(st, "title") ?? (phase == "done" ? T("Kurulum tamamlandı", "Installed") : T("Kurulum yapılamadı", "Setup failed")),
                    Str(st, "body") ?? Str(st, "message"), Str(st, "detail"), resultLog);
                return;
            }
        }
        if (setupProgress != null)
        {
            var p = ReadJson(setupProgress);
            if (p != null)
            {
                stepN = Int(p, "n", stepN);
                stepState = Str(p, "state") ?? stepState;
                stepPercent = Int(p, "percent", stepPercent);
                stepFile = Str(p, "file");
            }
        }
        // the script ended without a result (it was killed, or PowerShell itself failed)
        if (proc != null && proc.HasExited && (st == null || st.Count == 0 || !IsFinal(Str(st, "phase"))))
        {
            System.Threading.Thread.Sleep(150);
            st = ReadJson(Path.Combine(work, "status.json"));
            if (st == null || !IsFinal(Str(st, "phase")))
                Finish(false, T("Kurulum beklenmedik şekilde durdu", "Setup stopped unexpectedly"),
                    T("Hiçbir şey yarım kalmadı; yeniden deneyebilirsin.", "Nothing was left half-done; you can try again."), "exit " + proc.ExitCode, null);
        }
    }

    static bool IsFinal(string phase) { return phase == "done" || phase == "error" || phase == "cancelled"; }

    void Finish(bool ok, string title, string body, string detail, string log)
    {
        resultOk = ok;
        resultTitle = title;
        resultBody = body;
        resultDetail = detail;
        resultLog = log;
        Go(Page.Result);
    }

    protected override void OnFormClosing(FormClosingEventArgs e)
    {
        // still installing (closed from the taskbar): ask the install to roll back first
        if (page == Page.Progress && proc != null && !proc.HasExited)
        {
            RequestCancel();
            e.Cancel = true;
            return;
        }
        base.OnFormClosing(e);
    }

    protected override void OnFormClosed(FormClosedEventArgs e)
    {
        timer.Stop();
        try { if (work != null) Directory.Delete(work, true); } catch { }
        base.OnFormClosed(e);
    }

    // ---------------------------------------------------------------- screenshots (CI)

    public static void Shots(string dir)
    {
        Directory.CreateDirectory(dir);
        foreach (bool turkish in new[] { true, false })
        {
            string lang = turkish ? "tr" : "en";
            Shot(dir, lang + "-1-interface", turkish, f => { });
            Shot(dir, lang + "-2-personalize", turkish, f => { f.page = Page.Personalize; f.accentHex = "#8ab4f8"; f.language = lang; f.sensors = false; });
            Shot(dir, lang + "-3-download", turkish, f =>
            {
                f.page = Page.Progress; f.phase = "download"; f.version = "v0.3.0-native-ui";
                f.downloadPercent = 42; f.downloadDone = 14L << 20; f.downloadTotal = 33L << 20;
            });
            Shot(dir, lang + "-4-setup", turkish, f =>
            {
                f.page = Page.Progress; f.phase = "setup"; f.version = "v0.3.0-native-ui"; f.stepN = 4; f.stepPercent = 63; f.stepFile = @"app\lunge-shell.exe";
                f.steps = turkish
                    ? new[] { "Windows denetleniyor", "Gerekli bileşenler", "Masaüstü durduruluyor", "Dosyalar kopyalanıyor", "Ayarların yazılıyor", "Önceki sürümden taşınıyor", "Araçlar", "Terminal", "Windows ayarları", "Başlangıç görevleri", "Sahiplik", "Bitiriliyor" }
                    : new[] { "Checking Windows", "Required components", "Stopping the desktop", "Copying files", "Writing your settings", "Moving data from the old version", "Tools", "Terminal", "Windows settings", "Startup tasks", "Ownership", "Finishing" };
            });
            Shot(dir, lang + "-5-done", turkish, f =>
            {
                f.page = Page.Result; f.resultOk = true;
                f.resultTitle = turkish ? "Kurulum tamamlandı" : "All set";
                f.resultBody = turkish ? "Masaüstün birkaç saniye içinde açılıyor.\n\nSuper: arama ve uygulamalar   Super + Enter: terminal" : "Your desktop opens in a few seconds.\n\nSuper: search and apps   Super + Enter: terminal";
                f.warnTitle = turkish ? "Birkaç ek parça kurulamadı" : "A few extras couldn't be installed";
                f.warn = new[] { turkish ? "CPU sıcaklığı: PawnIO sürücüsü" : "CPU temperature: PawnIO driver" };
                f.warnBody = turkish ? "Masaüstün tam çalışıyor, yalnızca bunlar eksik." : "Your desktop works fully; only these are missing.";
            });
            Shot(dir, lang + "-6-error", turkish, f =>
            {
                f.page = Page.Result; f.resultOk = false;
                f.resultTitle = turkish ? "Olmadı, ama endişelenme" : "That didn't work, but don't worry";
                f.resultBody = turkish ? "Dosyalar kopyalanıyor adımında bir sorun çıktı. Her şey eski haline döndü." : "Something went wrong while copying files. Everything was put back.";
                f.resultDetail = "Access to the path 'C:\\Program Files\\LogicalLunge\\app' is denied.";
                f.resultLog = @"C:\Users\me\AppData\Local\Temp\logical-lunge-install.log";
            });
        }
    }

    static void Shot(string dir, string name, bool turkish, Action<SetupForm> setup)
    {
        using (var f = new SetupForm())
        {
            f.timer.Stop();
            f.tr = turkish;
            f.slideStart = DateTime.UtcNow.AddSeconds(-5);
            f.tick = 7;
            setup(f);
            using (var bmp = new Bitmap((int)W, (int)H))
            {
                using (var g = Graphics.FromImage(bmp)) f.Render(g);
                bmp.Save(Path.Combine(dir, name + ".png"), System.Drawing.Imaging.ImageFormat.Png);
            }
        }
    }

    // ---------------------------------------------------------------- helpers

    static Dictionary<string, object> ReadJson(string path)
    {
        try
        {
            if (path == null || !File.Exists(path)) return null;
            string text;
            using (var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
            using (var sr = new StreamReader(fs)) text = sr.ReadToEnd();
            return new JavaScriptSerializer().DeserializeObject(text) as Dictionary<string, object>;
        }
        catch { return null; }
    }

    static string Str(Dictionary<string, object> d, string key) { object v; return d.TryGetValue(key, out v) && v != null ? Convert.ToString(v) : null; }
    static int Int(Dictionary<string, object> d, string key, int def) { object v; try { return d.TryGetValue(key, out v) && v != null ? Convert.ToInt32(v) : def; } catch { return def; } }
    static long Long(Dictionary<string, object> d, string key, long def) { object v; try { return d.TryGetValue(key, out v) && v != null ? Convert.ToInt64(v) : def; } catch { return def; } }
    static string[] Strings(Dictionary<string, object> d, string key)
    {
        object v;
        var arr = d.TryGetValue(key, out v) ? v as object[] : null;
        if (arr == null) return new string[0];
        var list = new List<string>();
        foreach (var o in arr) if (o != null) list.Add(Convert.ToString(o));
        return list.ToArray();
    }

    Font F(float px, bool bold = false)
    {
        string key = px + (bold ? "b" : "");
        Font f;
        if (!fonts.TryGetValue(key, out f))
        {
            string family = FontExists("Segoe UI Variable Text") ? "Segoe UI Variable Text" : "Segoe UI";
            if (bold && FontExists("Segoe UI Variable Display")) family = "Segoe UI Variable Display";
            fonts[key] = f = new Font(family, px, bold ? FontStyle.Bold : FontStyle.Regular, GraphicsUnit.Pixel);
        }
        return f;
    }

    void ClearFonts() { foreach (var f in fonts.Values) f.Dispose(); fonts.Clear(); }

    static bool? hasVariable;
    static bool FontExists(string name)
    {
        if (name.StartsWith("Segoe UI Variable") && hasVariable.HasValue) return hasVariable.Value;
        using (var fam = new InstalledFontCollection())
            foreach (var f in fam.Families) if (f.Name == name) { if (name.StartsWith("Segoe UI Variable")) hasVariable = true; return true; }
        if (name.StartsWith("Segoe UI Variable")) hasVariable = false;
        return false;
    }

    static void Str(Graphics g, string s, Font f, Color c, float x, float y, float w, float h)
    {
        using (var b = new SolidBrush(c))
        using (var fmt = new StringFormat(StringFormatFlags.NoWrap) { Trimming = StringTrimming.EllipsisCharacter })
            g.DrawString(s, f, b, new RectangleF(x, y, w, h), fmt);
    }

    // centred in r (buttons, chips, segments): no measuring, so nothing is cut
    static void StrCenter(Graphics g, string s, Font f, Color c, RectangleF r)
    {
        using (var b = new SolidBrush(c))
        using (var fmt = new StringFormat(StringFormatFlags.NoWrap) { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center, Trimming = StringTrimming.EllipsisCharacter })
            g.DrawString(s, f, b, r, fmt);
    }

    // wrapped text; returns the bottom
    static float StrWrap(Graphics g, string s, Font f, Color c, float x, float y, float w, float maxH)
    {
        var size = g.MeasureString(s, f, (int)w);
        using (var b = new SolidBrush(c)) g.DrawString(s, f, b, new RectangleF(x, y, w, Math.Min(maxH, size.Height + 2)));
        return y + Math.Min(maxH, size.Height);
    }

    static float Measure(Graphics g, string s, Font f)
    {
        using (var fmt = new StringFormat(StringFormat.GenericTypographic)) return g.MeasureString(s, f, 1000, fmt).Width;
    }

    // Material-like glyphs drawn as lines (no icon font needed)
    static void Glyph(Graphics g, string name, RectangleF r, Color c)
    {
        using (var p = new Pen(c, Math.Max(1.5f, r.Width / 14f)) { StartCap = LineCap.Round, EndCap = LineCap.Round, LineJoin = LineJoin.Round })
        {
            float x = r.X, y = r.Y, w = r.Width, h = r.Height;
            if (name == "check") g.DrawLines(p, new[] { new PointF(x + w * 0.24f, y + h * 0.52f), new PointF(x + w * 0.43f, y + h * 0.70f), new PointF(x + w * 0.78f, y + h * 0.32f) });
            else if (name == "close")
            {
                float a = w * 0.32f;
                g.DrawLine(p, x + a, y + a, x + w - a, y + h - a);
                g.DrawLine(p, x + w - a, y + a, x + a, y + h - a);
            }
        }
    }

    static GraphicsPath RoundPath(float x, float y, float w, float h, float r)
    {
        var path = new GraphicsPath();
        float d = Math.Min(r * 2, Math.Min(w, h));
        if (d <= 0.5f) { path.AddRectangle(new RectangleF(x, y, w, h)); return path; }
        path.AddArc(x, y, d, d, 180, 90);
        path.AddArc(x + w - d, y, d, d, 270, 90);
        path.AddArc(x + w - d, y + h - d, d, d, 0, 90);
        path.AddArc(x, y + h - d, d, d, 90, 90);
        path.CloseFigure();
        return path;
    }

    static void FillRound(Graphics g, Color c, float x, float y, float w, float h, float r)
    {
        using (var b = new SolidBrush(c)) using (var p = RoundPath(x, y, w, h, r)) g.FillPath(b, p);
    }

    static void DrawRound(Graphics g, Color c, float width, float x, float y, float w, float h, float r)
    {
        using (var pen = new Pen(c, width)) using (var p = RoundPath(x, y, w, h, r)) g.DrawPath(pen, p);
    }

    static void FillCircle(Graphics g, Color c, float cx, float cy, float r)
    {
        using (var b = new SolidBrush(c)) g.FillEllipse(b, cx - r, cy - r, r * 2, r * 2);
    }

    static Color Hex(string h)
    {
        int v = Convert.ToInt32(h.TrimStart('#'), 16);
        return Color.FromArgb(255, (v >> 16) & 255, (v >> 8) & 255, v & 255);
    }

    static Color Mix(Color a, Color b, float t)
    {
        return Color.FromArgb(255, (int)(a.R + (b.R - a.R) * t), (int)(a.G + (b.G - a.G) * t), (int)(a.B + (b.B - a.B) * t));
    }

    [StructLayout(LayoutKind.Sequential)] struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);
    [DllImport("dwmapi.dll")] static extern int DwmFlush();
    [DllImport("user32.dll")] static extern bool ReleaseCapture();
    [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr hwnd, int msg, IntPtr wp, IntPtr lp);
}
