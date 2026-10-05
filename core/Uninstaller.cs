using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
using System.Threading;
using System.Windows.Forms;

// ---------------- Kaldırıcı ----------------
// uninstall.ps1'in penceresi. Açılış örtüsü bütün ekranları kaplar (arkada bar, pencere yöneticisi, görev çubuğu gidip
// gelirken görünmez; kullanıcı yarım kalmış bir şeye tıklamaz), ana ekranda kurulumun penceresi gibi bir kart: önce soru
// (ayarlar ve veriler de silinsin mi, ek bileşenler de kaldırılsın mı), sonra adımlar sırayla, en sonda sonuç; kart
// kapanınca örtü yumuşakça çekilir. İşin kendisi uninstall.ps1'de (tek yer): bu pencere onu gizli çalıştırır, adımlarını
// <çalışma klasörü>\steps.txt'den okur ("adım durum" satırları). Kayıtlar (loglar) her durumda kalır.
// uninstall.ps1 kurulum klasöründen geçici bir kopyayla (lunge-uninstall.exe) başlatır: kurulum klasörü silinirken bu
// süreç çalışmaya devam eder; masaüstünü durduran kod süreçleri adlarıyla kapattığı için bu kopyaya dokunmaz.
// Yönetici izni örtüden önce istenir (kart tek başınayken, "Kaldır"a basınca): güvenli masaüstü kapalıysa izin penceresi
// sıradan bir penceredir ve örtünün arkasında kalıyordu (2026-10-05: kullanıcı Alt+Tab, ok tuşu ve Enter'la körlemesine
// onayladı). İzin verilince yönetici kopyası (-Await) bekler; örtü gelir; kullanıcının parçası (-UserPhase: masaüstünü
// kullanıcının yetkisiyle durdurmak) bitince yönetici kopyası devam eder.
static class Uninstaller
{
    // Ek bileşenler: kurulumun install-backup.json'a yazdıkları; bunlardan biri kuruluysa ortak bir seçenek çıkar
    internal static readonly string[] Extras = { "terminal", "fonts", "msys2", "pawnio", "everything" };

    // Kurulumun kaydından kurulu ek bileşenler (kayıt okunamazsa hiçbiri)
    internal static List<string> InstalledExtras(string backupJson)
    {
        var found = new List<string>();
        try
        {
            var d = new System.Web.Script.Serialization.JavaScriptSerializer().DeserializeObject(backupJson) as Dictionary<string, object>;
            object v;
            var list = d != null && d.TryGetValue("installed", out v) ? v as object[] : null;
            if (list == null) return found;
            foreach (var o in list)
            {
                string s = o as string;
                if (s != null && Array.IndexOf(Extras, s) >= 0 && !found.Contains(s)) found.Add(s);
            }
        }
        catch { }
        return found;
    }

    // steps.txt: "adım durum" satırları, son yazılan geçerli; "result <sonuç>" bitişi söyler
    internal static Dictionary<string, string> ReadSteps(string text, out string result, out string message)
    {
        var steps = new Dictionary<string, string>();
        result = null; message = null;
        foreach (var raw in (text ?? "").Split('\n'))
        {
            string line = raw.Trim().TrimStart('\uFEFF');
            int sp = line.IndexOf(' ');
            if (sp <= 0) continue;
            string id = line.Substring(0, sp), rest = line.Substring(sp + 1).Trim();
            if (id == "result") { result = rest; continue; }
            if (id == "message") { message = rest; continue; }
            steps[id] = rest;
        }
        return steps;
    }

    public static void Run(string work, string app)
    {
        bool created;
        using (new Mutex(true, @"Local\LogicalLunge.Uninstall", out created))
        {
            if (!created) return;
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            var covers = new List<Splash.Cover>();
            string backup = null;
            try { backup = File.ReadAllText(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"LogicalLunge\state\install-backup.json")); } catch { }
            var card = new UninstallCard(work, app, InstalledExtras(backup), Splash.Accent());
            int start = Environment.TickCount, covered = -1, leaving = -1;
            card.Leave = () => { if (leaving < 0) leaving = Environment.TickCount; };
            // Örtü (izinden sonra): her ekranda bir tane, kart ana ekrandakinin sahipliğinde (onun üstünde kalır)
            var keepTop = new System.Windows.Forms.Timer { Interval = 250 };
            card.Cover = () =>
            {
                if (covered >= 0) return;
                covered = Environment.TickCount;
                Splash.Cover main = null;
                foreach (var s in Screen.AllScreens)
                {
                    var f = new Splash.Cover(s.Bounds, s.Primary) { Opacity = 0 };
                    if (s.Primary) main = f;
                    f.Show();
                    covers.Add(f);
                }
                var targets = new List<KeyValuePair<Splash.Cover, Rectangle>>();
                foreach (var f in covers) targets.Add(new KeyValuePair<Splash.Cover, Rectangle>(f, f.Bounds));
                ThreadPool.QueueUserWorkItem(_ =>
                {
                    try
                    {
                        var img = Splash.Wallpaper();
                        if (img == null) return;
                        var virt = Splash.SpanStyle() ? SystemInformation.VirtualScreen : Rectangle.Empty;
                        foreach (var t in targets)
                        {
                            var bmp = Splash.Cover.Prepare(img, t.Value, virt, t.Key.BackColor);
                            var f = t.Key;
                            try { f.BeginInvoke((Action)(() => f.SetBackground(bmp))); } catch { bmp.Dispose(); }
                        }
                        img.Dispose();
                    }
                    catch { }
                });
                if (main != null) card.Owner = main;
                keepTop.Start();
                card.Activate();
            };
            var clock = new System.Windows.Forms.Timer { Interval = 16 };
            clock.Tick += (o, e) =>
            {
                int now = Environment.TickCount;
                double away = leaving < 0 ? 1 : Math.Pow(1 - Math.Min(1, (now - leaving) / 450.0), 3);
                double cover = covered < 0 ? 0 : (1 - Math.Pow(1 - Math.Min(1, (now - covered) / 300.0), 3)) * away;
                foreach (var f in covers) { f.Opacity = cover; if (f.Blending) f.Invalidate(); }
                if (!card.IsDisposed) card.Opacity = (1 - Math.Pow(1 - Math.Min(1, (now - start) / 300.0), 3)) * away;
                if (leaving >= 0 && now - leaving >= 450)
                {
                    clock.Stop();
                    Application.ExitThread();
                }
            };
            clock.Start();
            // Örtü en üstte kalsın: kaldırma sırasında açılan ya da öne gelen hiçbir şey araya girmesin (kart örtünün sahibi
            // olduğu için onun da üstünde kalır)
            keepTop.Tick += (o, e) => { foreach (var f in covers) Native.SetWindowPos(f.Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); };
            card.Show();
            card.Activate();
            Application.Run();
            keepTop.Stop();
            foreach (var f in covers) f.Dispose();
            card.Dispose();
            // Bu kopya ve çalışma klasörü: süreç bittikten sonra silinir
            try
            {
                Process.Start(new ProcessStartInfo("cmd.exe", "/c ping -n 3 127.0.0.1 >nul & rmdir /s /q \"" + work + "\"")
                { CreateNoWindow = true, UseShellExecute = false, WorkingDirectory = Path.GetTempPath() });
            }
            catch { }
        }
    }
}

sealed class UninstallCard : Form
{
    readonly bool tr = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr";
    string T(string turkish, string english) { return tr ? turkish : english; }

    enum Page { Confirm, Progress, Result }
    Page page = Page.Confirm;
    readonly string work, app;
    readonly List<string> extras;
    bool removeData, removeExtras = true;
    public Action Leave = () => { };
    public Action Cover = () => { };
    // Zaten yönetici değilse izin "Kaldır"da, örtüden önce istenir; izinle başlayan yönetici kopyası (waiter) bekler
    readonly bool admin = new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator);
    Process waiter;
    string userArgs, failure;

    // Adımlar: kimlik, etiket. "shell / tiling / core" süreçlerin kapanmasından, "windows" bu pencerenin onarımından,
    // geri kalanlar uninstall.ps1'den gelir.
    string[][] steps;
    readonly Dictionary<string, string> shown = new Dictionary<string, string>();
    int lastChange;
    Process proc;
    bool repairStarted;
    volatile string repairResult;
    string resultTitle, resultBody, resultDetail;
    bool resultOk;
    readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer { Interval = 50 };
    int tick;

    float k = 1f;
    static readonly Color Bg = Hex("#141218"), Surface = Hex("#1d1b20"), Surface2 = Hex("#2b2930"), Outline = Hex("#49454f"),
        Fg = Hex("#e6e0e9"), Sub = Hex("#cac4d0"), Dim = Hex("#938f99"), Ok = Hex("#a8dab5"), Err = Hex("#f2b8b5");
    readonly Color accent;
    readonly Dictionary<string, Font> fonts = new Dictionary<string, Font>();
    sealed class Hit { public RectangleF Rect; public string Id; public Action Click; }
    readonly List<Hit> hits = new List<Hit>();
    string hover;
    const float W = 640, H = 520;

    public UninstallCard(string work, string app, List<string> extras, Color accent)
    {
        this.work = work; this.app = app; this.extras = extras; this.accent = accent;
        Text = T("Logical Lunge'ı kaldır", "Uninstall Logical Lunge");
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.CenterScreen;
        ShowInTaskbar = true;
        TopMost = true;
        BackColor = Bg;
        Opacity = 0;
        DoubleBuffered = true;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
        try { Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath); } catch { }
        var list = new List<string[]>
        {
            new[] { "uac", T("Yönetici izni", "Administrator permission") },
            new[] { "shell", T("Bar ve kabuk kapatılıyor", "Closing the bar and shell") },
            new[] { "tiling", T("Pencere yöneticisi kapatılıyor", "Closing the window manager") },
            new[] { "core", T("Çekirdek kapatılıyor", "Closing the core") },
            new[] { "taskbar", T("Görev çubuğu geri getiriliyor", "Bringing back the taskbar") },
            new[] { "icons", T("Masaüstü simgeleri ve gizli pencereler gösteriliyor", "Showing desktop icons and hidden windows") },
            new[] { "windows", T("Uygulama pencereleri onarılıyor", "Repairing app windows") },
            new[] { "settings", T("Windows ayarları geri yükleniyor", "Restoring Windows settings") },
            new[] { "tasks", T("Başlangıç görevleri kaldırılıyor", "Removing startup tasks") },
        };
        if (extras.Count > 0) list.Add(new[] { "extras", T("Ek bileşenler", "Extras") });
        list.Add(new[] { "files", T("Program dosyaları siliniyor (kayıtlar kalıyor)", "Deleting program files (logs are kept)") });
        steps = list.ToArray();
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
        int round = 2, dark = 1;
        DwmSetWindowAttribute(Handle, 33 /* DWMWA_WINDOW_CORNER_PREFERENCE */, ref round, 4);
        DwmSetWindowAttribute(Handle, 20 /* DWMWA_USE_IMMERSIVE_DARK_MODE */, ref dark, 4);
    }

    // Alt+F4 yalnızca soru sayfasında (vazgeçmek demek); kaldırma sürerken kapatılamaz
    protected override void OnFormClosing(FormClosingEventArgs e)
    {
        if (e.CloseReason == CloseReason.UserClosing)
        {
            e.Cancel = true;
            if (page != Page.Progress) Leave();
            return;
        }
        base.OnFormClosing(e);
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        try { Render(e.Graphics); } catch { }
    }

    internal void Render(Graphics g)
    {
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
        g.Clear(Bg);
        hits.Clear();
        g.ScaleTransform(k, k);
        FillCircle(g, accent, 30, 30, 6);
        Str(g, "Logical Lunge", F(15, true), Fg, 44, 19, 300, 24);
        if (page == Page.Confirm) PaintConfirm(g);
        else if (page == Page.Progress) PaintProgress(g);
        else PaintResult(g);
        g.ResetTransform();
    }

    void PaintConfirm(Graphics g)
    {
        Str(g, T("Logical Lunge kaldırılsın mı?", "Uninstall Logical Lunge?"), F(24, true), Fg, 40, 76, 560, 36);
        float y = StrWrap(g, T("Görev çubuğun, masaüstü simgelerin ve Windows ayarların Logical Lunge'dan önceki hâline döner. Açık pencerelerin yerinde kalır.",
            "Your taskbar, desktop icons and Windows settings go back to how they were before Logical Lunge. Your open windows stay."), F(13), Sub, 40, 118, 560, 60) + 22;
        y = Toggle(g, "data", T("Ayarlarımı ve verilerimi de sil", "Also delete my settings and data"),
            T("Ayar dosyaları, pano geçmişi, yapılacaklar, kısayol ve gece ışığı ayarları, indirilen duvar kağıtları.",
              "Config files, clipboard history, to-dos, shortcut and night-light settings, downloaded wallpapers."), 40, y, removeData, () => { removeData = !removeData; Invalidate(); });
        if (extras.Count > 0)
        {
            y = Toggle(g, "extras", T("Ek bileşenleri de kaldır", "Also remove the extras"), ExtrasText() + "\n" + (removeExtras
                ? T("Kapatırsan kalırlar.", "Turn off to keep them.")
                : T("Kalırlar: WezTerm ve Everything kullanıcı programlarına taşınır.", "They stay: WezTerm and Everything move to your user programs.")),
                40, y + 8, removeExtras, () => { removeExtras = !removeExtras; Invalidate(); });
        }
        Str(g, T("Kayıtlar (loglar) her durumda saklanır.", "Logs are always kept."), F(12), Dim, 40, H - 112, 560, 20);
        Button(g, "cancel", T("Vazgeç", "Cancel"), 40, H - 72, 130, false, () => Leave());
        Button(g, "go", T("Kaldır", "Uninstall"), W - 40 - 150, H - 72, 150, true, Start);
        Str(g, T("Windows bir kez yönetici izni isteyecek.", "Windows will ask for permission once."), F(12), Dim, 190, H - 60, 260, 20);
    }

    string ExtrasText()
    {
        var parts = new List<string>();
        if (extras.Contains("terminal") || extras.Contains("msys2") || extras.Contains("fonts")) parts.Add(T("terminal (WezTerm, fish, starship, Nerd Font)", "terminal (WezTerm, fish, starship, Nerd Font)"));
        if (extras.Contains("pawnio")) parts.Add(T("CPU sıcaklığı sürücüsü (PawnIO)", "CPU temperature driver (PawnIO)"));
        if (extras.Contains("everything")) parts.Add(T("dosya araması (Everything)", "file search (Everything)"));
        string s = string.Join(", ", parts.ToArray());
        return s.Length > 0 ? char.ToUpper(s[0]) + s.Substring(1) + "." : "";
    }

    void PaintProgress(Graphics g)
    {
        Str(g, T("Kaldırılıyor", "Uninstalling"), F(24, true), Fg, 40, 76, 560, 36);
        float y = 128;
        foreach (var s in steps)
        {
            string st;
            if (!shown.TryGetValue(s[0], out st)) st = "";
            string label = s[1];
            if (s[0] == "extras") label = removeExtras ? T("Ek bileşenler kaldırılıyor", "Removing the extras") : T("Ek bileşenler taşınıyor", "Moving the extras");
            StepRow(g, y, label, st == "done" || st == "skip" ? 2 : st == "fail" ? 3 : st == "run" ? 1 : 0);
            y += 30;
        }
    }

    void PaintResult(Graphics g)
    {
        var mark = resultOk ? Ok : Err;
        FillCircle(g, Mix(Bg, mark, 0.18f), 72, 112, 32);
        Glyph(g, resultOk ? "check" : "close", new RectangleF(52, 92, 40, 40), mark);
        Str(g, resultTitle ?? "", F(22, true), Fg, 120, 94, 480, 34);
        float y = StrWrap(g, resultBody ?? "", F(13), Sub, 40, 164, 560, 200) + 12;
        if (!string.IsNullOrEmpty(resultDetail)) StrWrap(g, resultDetail, F(12), Dim, 40, y, 560, 120);
        Button(g, "close", T("Kapat", "Close"), W - 40 - 140, H - 72, 140, true, () => Leave());
    }

    // ---------------------------------------------------------------- the uninstall

    void Start()
    {
        try
        {
            File.WriteAllText(Path.Combine(work, "steps.txt"), "");
            foreach (var name in new[] { "go", "ready", "cancel" }) File.Delete(Path.Combine(work, name));
            var args = "-NoProfile -ExecutionPolicy Bypass -File \"" + Path.Combine(work, "uninstall.ps1") + "\" -Driver \"" + work + "\" " +
                (removeData ? "-RemoveConfig" : "-KeepConfig") + " -Extras " + (removeExtras ? "remove" : "keep");
            if (!admin)
            {
                // İzin penceresi kartın önüne gelsin: kart izin sorulurken en üstte değil
                TopMost = false;
                try
                {
                    waiter = Process.Start(new ProcessStartInfo("powershell.exe", args + " -Elevated -Await -Owner " + Process.GetCurrentProcess().Id +
                        " -UserProfile \"" + Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) + "\" -UserSid " + WindowsIdentity.GetCurrent().User.Value)
                    { UseShellExecute = true, Verb = "runas", WindowStyle = ProcessWindowStyle.Hidden, ErrorDialogParentHandle = Handle, WorkingDirectory = work });
                }
                catch (System.ComponentModel.Win32Exception ex)
                {
                    if (ex.NativeErrorCode != 1223 /*ERROR_CANCELLED*/) throw;
                    Finish(false, T("Kaldırma yapılmadı", "Nothing was uninstalled"),
                        T("Yönetici izni verilmedi; hiçbir şey değişmedi.", "Permission was not given; nothing changed."), null);
                    return;
                }
                finally { TopMost = true; }
                if (waiter == null) throw new InvalidOperationException(T("Yönetici kopyası başlamadı", "The elevated copy did not start"));
                userArgs = args + " -UserPhase";
            }
            Cover();
            page = Page.Progress;
            lastChange = Environment.TickCount;
            if (admin) proc = Process.Start(new ProcessStartInfo("powershell.exe", args) { UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = work });
            Invalidate();
        }
        catch (Exception ex)
        {
            Finish(false, T("Kaldırma başlatılamadı", "The uninstall could not start"), ex.Message, null);
        }
    }

    // Yönetici kopyası beklemeye geçince (ready) kullanıcının parçası başlar; o parça go yazmadan biterse (masaüstü
    // durdurulamadı) yönetici kopyası cancel ile bırakılır
    void StepWaiter()
    {
        if (waiter == null) return;
        try
        {
            if (proc == null)
            {
                if (File.Exists(Path.Combine(work, "ready")))
                    proc = Process.Start(new ProcessStartInfo("powershell.exe", userArgs) { UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = work });
                return;
            }
            if (proc.HasExited && !File.Exists(Path.Combine(work, "go"))) Cancel();
        }
        catch (Exception ex) { failure = ex.Message; Cancel(); }
    }

    void Cancel()
    {
        try { if (!File.Exists(Path.Combine(work, "cancel"))) File.WriteAllText(Path.Combine(work, "cancel"), "cancel"); } catch { }
    }

    int ExitCode()
    {
        try { return (waiter ?? proc).ExitCode; } catch { return -1; }
    }

    void OnTick()
    {
        tick++;
        if (page != Page.Progress) return;
        string text = "";
        try
        {
            using (var fs = new FileStream(Path.Combine(work, "steps.txt"), FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
            using (var sr = new StreamReader(fs, Encoding.UTF8)) text = sr.ReadToEnd();
        }
        catch { }
        StepWaiter();
        string result, message;
        var reported = Uninstaller.ReadSteps(text, out result, out message);
        if (message == null) message = failure;
        if (waiter != null) reported["uac"] = "done";
        // Masaüstünün parçaları kapandıkça (uninstall.ps1 hepsini birlikte durdurur; sırası burada görünür)
        if (Gone(Names.Shell)) reported["shell"] = "done";
        if (Gone(Names.Tiling)) reported["tiling"] = "done";
        if (Gone(Names.Core) && reported.ContainsKey("tiling")) reported["core"] = "done";
        foreach (var s in new[] { "shell", "tiling", "core" }) if (!reported.ContainsKey(s) && proc != null) { reported[s] = "run"; break; }
        // Pencereler: görev çubuğu ve gizli pencereler geri geldikten sonra, bu süreçte (kullanıcının pencereleri onun
        // yetkisiyle onarılır)
        string icons;
        if (!repairStarted && reported.TryGetValue("icons", out icons) && (icons == "done" || icons == "fail"))
        {
            repairStarted = true;
            // STA: kabuğun COM arayüzleri (ShellWindows, ImmersiveShell)
            var repair = new Thread(() =>
            {
                try { repairResult = WindowRepair.Shell() + "; " + WindowRepair.Run(); } catch (Exception ex) { repairResult = "! " + ex.Message; }
            }) { IsBackground = true };
            repair.SetApartmentState(ApartmentState.STA);
            repair.Start();
        }
        if (repairStarted) reported["windows"] = repairResult == null ? "run" : repairResult.StartsWith("!") ? "fail" : "done";
        // Ekranda adımlar sırayla ilerler: aynı anda biten adımlar birer birer (her biri en az 180 ms) işaretlenir
        int now = Environment.TickCount;
        if (now - lastChange >= 180)
            foreach (var s in steps)
            {
                string want, have;
                if (!reported.TryGetValue(s[0], out want)) continue;
                shown.TryGetValue(s[0], out have);
                if (have == want) continue;
                if ((have == "done" || have == "skip") && want == "run") continue;
                shown[s[0]] = want;
                lastChange = now;
                break;
            }
        bool caughtUp = true;
        foreach (var kv in reported) { string have; if (!shown.TryGetValue(kv.Key, out have) || have != kv.Value) caughtUp = false; }
        // Bitiş: yönetici kopyasıyla çalışılıyorsa onun bitmesi (kullanıcının parçası ondan önce biter)
        bool ended = waiter != null ? waiter.HasExited : proc != null && proc.HasExited;
        if (ended && caughtUp && (repairResult != null || !repairStarted))
        {
            if (result == null) { Thread.Sleep(150); text = ReadAll(); reported = Uninstaller.ReadSteps(text, out result, out message); }
            if (result == "done")
            {
                StartKeptEverything();
                Finish(true, T("Logical Lunge kaldırıldı", "Logical Lunge has been removed"),
                    T("Windows eski hâline döndü. Kayıtlar saklandı:", "Windows is back to how it was. The logs are kept:") + "\n" +
                    Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"LogicalLunge\logs"), message);
            }
            else if (result == "cancelled")
                Finish(false, T("Kaldırma yapılmadı", "Nothing was uninstalled"),
                    T("Yönetici izni verilmedi; hiçbir şey silinmedi. Masaüstün geri geliyor.", "Permission was not given; nothing was deleted. Your desktop comes back."), message);
            else
                Finish(false, T("Kaldırma tamamlanamadı", "The uninstall did not finish"),
                    T("Program dosyaları ya da bazı Windows ayarları geride kaldı. Kurtarma kayıtları %LOCALAPPDATA%\\LogicalLunge\\state içinde; kaldırmayı yeniden çalıştırabilirsin.",
                      "Program files or some Windows settings were left behind. Recovery records are in %LOCALAPPDATA%\\LogicalLunge\\state; you can run the uninstall again."),
                    message ?? (result == null ? "exit " + ExitCode() : result));
            return;
        }
        Invalidate();
    }

    string ReadAll()
    {
        try
        {
            using (var fs = new FileStream(Path.Combine(work, "steps.txt"), FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
            using (var sr = new StreamReader(fs, Encoding.UTF8)) return sr.ReadToEnd();
        }
        catch { return ""; }
    }

    // Bu kurulumun bu adlı süreci kalmadı mı
    bool Gone(string name)
    {
        string prefix = app.TrimEnd('\\') + "\\";
        foreach (var p in Process.GetProcessesByName(name))
            using (p)
            {
                string path = null;
                try { path = p.MainModule.FileName; } catch { }
                if (path == null || path.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)) return false;
            }
        return true;
    }

    // Kalan Everything (kullanıcının programlarına taşındı): yönetici kaldırıcı değil kullanıcı başlatır (arama kanalı ister)
    void StartKeptEverything()
    {
        if (removeExtras || !extras.Contains("everything")) return;
        try
        {
            string exe = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"Programs\Everything\Everything.exe");
            if (File.Exists(exe) && Process.GetProcessesByName("Everything").Length == 0) Process.Start(exe, "-startup");
        }
        catch { }
    }

    void Finish(bool ok, string title, string body, string detail)
    {
        resultOk = ok; resultTitle = title; resultBody = body; resultDetail = detail;
        page = Page.Result;
        Invalidate();
    }

    // ---------------------------------------------------------------- controls (the installer's look)

    void Button(Graphics g, string id, string label, float x, float y, float w, bool primary, Action click)
    {
        bool hot = hover == id;
        var bg = primary ? (hot ? Mix(accent, Color.White, 0.12f) : accent) : (hot ? Surface2 : Surface);
        FillRound(g, bg, x, y, w, 44, 22);
        StrCenter(g, label, F(14, true), primary ? Hex("#1d1b20") : Fg, new RectangleF(x, y, w, 44));
        AddHit(new RectangleF(x, y, w, 44), id, click);
    }

    // returns the bottom
    float Toggle(Graphics g, string id, string label, string detail, float x, float y, bool on, Action click)
    {
        float bottom = StrWrapMeasure(g, detail, F(12), 500) + y + 30;
        var row = new RectangleF(x, y, 560, bottom - y);
        if (hover == id) FillRound(g, Surface, row.X - 8, row.Y - 4, row.Width + 16, row.Height + 8, 12);
        FillRound(g, on ? accent : Surface2, x, y + 4, 40, 22, 11);
        if (!on) DrawRound(g, Outline, 1.2f, x + 0.6f, y + 4.6f, 38.8f, 20.8f, 10.4f);
        FillCircle(g, on ? Hex("#1d1b20") : Dim, on ? x + 29 : x + 11, y + 15, on ? 8 : 6);
        Str(g, label, F(13.5f, true), on ? Fg : Sub, x + 54, y + 3, 500, 22);
        StrWrap(g, detail, F(12), Dim, x + 54, y + 27, 500, 80);
        AddHit(row, id, click);
        return bottom + 10;
    }

    // state: 0 pending, 1 running, 2 done, 3 failed
    void StepRow(Graphics g, float y, string label, int state)
    {
        float cx = 56, cy = y + 11;
        if (state == 2) { FillCircle(g, Mix(Bg, Ok, 0.22f), cx, cy, 10); Glyph(g, "check", new RectangleF(cx - 8, cy - 8, 16, 16), Ok); }
        else if (state == 3) { FillCircle(g, Mix(Bg, Err, 0.22f), cx, cy, 10); Glyph(g, "close", new RectangleF(cx - 8, cy - 8, 16, 16), Err); }
        else if (state == 1)
        {
            using (var p = new Pen(accent, 2.4f) { StartCap = LineCap.Round, EndCap = LineCap.Round })
                g.DrawArc(p, cx - 8, cy - 8, 16, 16, (tick * 18) % 360, 260);
        }
        else FillCircle(g, Outline, cx, cy, 3);
        Str(g, label, F(13, state == 1), state == 0 ? Dim : Fg, 76, y, 520, 22);
    }

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

    protected override void OnMouseLeave(EventArgs e) { if (hover != null) { hover = null; Invalidate(); } }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        if (e.Button != MouseButtons.Left) return;
        var h = HitAt(e.Location);
        if (h != null) h.Click();
    }

    protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
    {
        if (keyData == Keys.Escape) { if (page != Page.Progress) Leave(); return true; }
        if (keyData == Keys.Enter)
        {
            if (page == Page.Confirm) Start();
            else if (page == Page.Result) Leave();
            return true;
        }
        return base.ProcessCmdKey(ref msg, keyData);
    }

    // ---------------------------------------------------------------- screenshots (tests look at the pages)

    internal static void Shots(string dir)
    {
        Directory.CreateDirectory(dir);
        var all = new List<string>(Uninstaller.Extras);
        Shot(dir, "1-confirm", all, c => { });
        Shot(dir, "2-confirm-keep", all, c => { c.removeExtras = false; c.removeData = true; });
        Shot(dir, "3-confirm-no-extras", new List<string>(), c => { });
        Shot(dir, "4-progress", all, c =>
        {
            c.page = Page.Progress; c.tick = 7;
            foreach (var s in new[] { "shell", "tiling", "core", "uac", "taskbar" }) c.shown[s] = "done";
            c.shown["icons"] = "run";
        });
        Shot(dir, "5-done", all, c => c.Finish(true, "Logical Lunge kaldırıldı", "Windows eski hâline döndü. Kayıtlar saklandı:\nC:\\Users\\me\\AppData\\Local\\LogicalLunge\\logs", null));
    }

    static void Shot(string dir, string name, List<string> extras, Action<UninstallCard> setup)
    {
        using (var c = new UninstallCard(Path.GetTempPath(), @"C:\Program Files\LogicalLunge", extras, Hex("#a6d189")))
        {
            c.timer.Stop();
            setup(c);
            using (var bmp = new Bitmap((int)W, (int)H))
            {
                using (var g = Graphics.FromImage(bmp)) c.Render(g);
                bmp.Save(Path.Combine(dir, name + ".png"), System.Drawing.Imaging.ImageFormat.Png);
            }
        }
    }

    // ---------------------------------------------------------------- drawing helpers (the installer's)

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

    static bool FontExists(string name)
    {
        using (var fam = new InstalledFontCollection())
            foreach (var f in fam.Families) if (f.Name == name) return true;
        return false;
    }

    static void Str(Graphics g, string s, Font f, Color c, float x, float y, float w, float h)
    {
        using (var b = new SolidBrush(c))
        using (var fmt = new StringFormat(StringFormatFlags.NoWrap) { Trimming = StringTrimming.EllipsisCharacter })
            g.DrawString(s, f, b, new RectangleF(x, y, w, h), fmt);
    }

    static void StrCenter(Graphics g, string s, Font f, Color c, RectangleF r)
    {
        using (var b = new SolidBrush(c))
        using (var fmt = new StringFormat(StringFormatFlags.NoWrap) { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center, Trimming = StringTrimming.EllipsisCharacter })
            g.DrawString(s, f, b, r, fmt);
    }

    static float StrWrap(Graphics g, string s, Font f, Color c, float x, float y, float w, float maxH)
    {
        var size = g.MeasureString(s, f, (int)w);
        using (var b = new SolidBrush(c)) g.DrawString(s, f, b, new RectangleF(x, y, w, Math.Min(maxH, size.Height + 2)));
        return y + Math.Min(maxH, size.Height);
    }

    static float StrWrapMeasure(Graphics g, string s, Font f, float w) { return g.MeasureString(s, f, (int)w).Height; }

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

    [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);
}

// ---------------- Pencere onarımı ----------------
// Masaüstü kapandıktan sonra başka uygulamaların pencerelerinde bizden kalan: köşe yuvarlayıcının ya da yuva kesmesinin
// bölgesi (kalırsa başlık çubuğu kesik kalır, pencere tutulup taşınamaz: Alt+F4 gerekirdi), pencere yöneticisinin
// yerleştirip ekran dışında bıraktığı ya da başlığı ekranın üstünde kalmış pencereler, hata kutusu yakalanırken saydam
// kalmış kutular. Yalnızca bizim dokunduğumuz bilinenlere: pencere yöneticisinin yuva işareti (LungeSlotLT/RB) taşıyan
// pencereler ve tam olarak köşe yuvarlayıcının verdiği şekildeki bölgeler; uygulamanın kendi bölgesine dokunulmaz.
static class WindowRepair
{
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetProp(IntPtr h, string name);
    [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr h, IntPtr rgn);
    [DllImport("gdi32.dll")] static extern bool EqualRgn(IntPtr a, IntPtr b);
    [DllImport("user32.dll")] static extern bool GetLayeredWindowAttributes(IntPtr h, out uint key, out byte alpha, out uint flags);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] struct AppBar { public int size; public IntPtr hwnd; public uint callback, edge; public int left, top, right, bottom; public IntPtr param; }
    [DllImport("shell32.dll")] static extern IntPtr SHAppBarMessage(uint msg, ref AppBar data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder name, int size);
    [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr h, int cmd);

    // Kabuk, kullanıcının yetkisiyle: kabuğun gizlediği (cloak) uygulama pencereleri, masaüstü simgeleri ve sabit görev
    // çubuğu. Yönetici kopyası kayıt defterini yazar, ama Explorer'ın canlı görünümüne (ShellWindows) yükseltilmiş bir
    // süreçten ulaşılamayabiliyor: kaldırmadan sonra simgeler gizli, görev çubuğu otomatik gizlemede kaldı (2026-10-05).
    public static string Shell()
    {
        var parts = new List<string>();
        try { parts.Add(Orphans.Uncloak() + " uncloaked"); } catch (Exception ex) { parts.Add("uncloak ! " + ex.Message); }
        try { ShowIcons(); parts.Add("icons"); } catch (Exception ex) { parts.Add("icons ! " + ex.Message); }
        try { parts.Add(FixTaskbar() ? "taskbar" : "taskbar ?"); } catch (Exception ex) { parts.Add("taskbar ! " + ex.Message); }
        return string.Join(", ", parts.ToArray());
    }

    // Masaüstünün IFolderView2'si: ShellWindows -> IServiceProvider -> IShellBrowser -> etkin görünüm
    // https://devblogs.microsoft.com/oldnewthing/20130318-00/?p=4933
    [ComImport, Guid("6d5140c1-7436-11ce-8034-00aa006009fa"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface Provider { [PreserveSig] int QueryService(ref Guid service, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out object result); }
    [ComImport, Guid("000214e2-0000-0000-c000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface Browser
    {
        void GetWindow(); void ContextSensitiveHelp(); void InsertMenusSB(); void SetMenuSB(); void RemoveMenusSB();
        void SetStatusTextSB(); void EnableModelessSB(); void TranslateAcceleratorSB(); void BrowseObject();
        void GetViewStateStream(); void GetControlWindow(); void SendControlMsg();
        void QueryActiveShellView([MarshalAs(UnmanagedType.Interface)] out object view);
    }
    [ComImport, Guid("1af3a467-214f-4298-908e-06b03e0b39f9"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface FolderView2
    {
        void GetCurrentViewMode(); void SetCurrentViewMode(); void GetFolder(); void Item(); void ItemCount(); void Items();
        void GetSelectionMarkedItem(); void GetFocusedItem(); void GetItemPosition(); void GetSpacing(); void GetDefaultSpacing();
        void GetAutoArrange(); void SelectItem(); void SelectAndPositionItems();
        void SetGroupBy(); void GetGroupBy(); void SetViewProperty(); void GetViewProperty(); void SetTileViewProperties();
        void SetExtendedTileViewProperties(); void SetText(); void SetCurrentFolderFlags(uint mask, uint flags);
    }

    static void ShowIcons()
    {
        object windows = null, desktop = null, browser = null, view = null;
        try
        {
            windows = Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("9ba05972-f6a8-11cf-a442-00a0c90a8f39")));
            object[] args = { 0, null /*VT_EMPTY*/, 8 /*SWC_DESKTOP*/, 0, 1 /*SWFO_NEEDDISPATCH*/ };
            var modifier = new System.Reflection.ParameterModifier(5); modifier[3] = true;
            desktop = windows.GetType().InvokeMember("FindWindowSW", System.Reflection.BindingFlags.InvokeMethod, null, windows, args, new[] { modifier }, null, null);
            Guid service = new Guid("4c96be40-915c-11cf-99d3-00aa004ae837"), iid = typeof(Browser).GUID;
            Marshal.ThrowExceptionForHR(((Provider)desktop).QueryService(ref service, ref iid, out browser));
            ((Browser)browser).QueryActiveShellView(out view);
            ((FolderView2)view).SetCurrentFolderFlags(0x1000 /*FWF_NOICONS*/, 0);
        }
        finally
        {
            foreach (object o in new object[] { view, browser, desktop, windows }) if (o != null && Marshal.IsComObject(o)) Marshal.ReleaseComObject(o);
        }
    }

    // Görev çubukları görünür ve otomatik gizlemesiz (ABS_ALWAYSONTOP)
    static bool FixTaskbar()
    {
        IntPtr tray = FindWindow("Shell_TrayWnd", null);
        if (tray == IntPtr.Zero) return false;
        var data = new AppBar { size = Marshal.SizeOf(typeof(AppBar)), hwnd = tray, param = (IntPtr)2 };
        SHAppBarMessage(10 /*ABM_SETSTATE*/, ref data);
        EnumWindows((h, l) =>
        {
            var name = new StringBuilder(64); GetClassName(h, name, 64);
            if (name.ToString() == "Shell_TrayWnd" || name.ToString() == "Shell_SecondaryTrayWnd") ShowWindowAsync(h, 8 /*SW_SHOWNA*/);
            return true;
        }, IntPtr.Zero);
        return (SHAppBarMessage(4 /*ABM_GETSTATE*/, ref data).ToInt64() & 1) == 0;
    }

    // Onarılanların kısa özeti; includeOwnProcess yalnızca testler için (kaldırıcının kendi örtüsüne dokunulmaz)
    public static string Run(bool includeOwnProcess = false)
    {
        uint self = (uint)Process.GetCurrentProcess().Id;
        int regions = 0, moved = 0, shown = 0;
        var windows = new List<IntPtr>();
        EnumWindows((h, l) => { windows.Add(h); return true; }, IntPtr.Zero);
        foreach (var h in windows)
        {
            try
            {
                if (!Native.IsWindowVisible(h) || IsIconic(h)) continue;
                uint pid; Native.GetWindowThreadProcessId(h, out pid);
                if (pid == self && !includeOwnProcess) continue;
                bool ours = GetProp(h, "LungeSlotLT") != IntPtr.Zero || GetProp(h, "LungeSlotRB") != IntPtr.Zero;
                if (Region(h, ours)) regions++;
                if (ours && IntoView(h)) moved++;
                if (Reveal(h)) shown++;
            }
            catch { }
        }
        return regions + " region, " + moved + " moved, " + shown + " shown";
    }

    // Bölge bizimse kaldır: yuva işaretli pencerede her bölge bizimdir; işaretsizde yalnızca köşe yuvarlayıcının o pencereye
    // vereceği şeklin tıpkısı (yuvarlak ya da köşeli)
    internal static bool Region(IntPtr h, bool ours)
    {
        Native.RECT box;
        if (Native.GetWindowRgnBox(h, out box) == 0 /*ERROR: bölgesi yok*/) return false;
        if (!ours && !RounderShaped(h)) return false;
        return Native.SetWindowRgn(h, IntPtr.Zero, true) != 0;
    }

    static bool RounderShaped(IntPtr h)
    {
        Native.RECT wr, fr;
        if (!Native.GetWindowRect(h, out wr)) return false;
        if (Native.DwmGetWindowAttribute(h, Native.DWMWA_EXTENDED_FRAME_BOUNDS, out fr, Marshal.SizeOf(typeof(Native.RECT))) != 0) fr = wr;
        int l = fr.Left - wr.Left, t = fr.Top - wr.Top, r = l + (fr.Right - fr.Left), b = t + (fr.Bottom - fr.Top);
        IntPtr actual = Native.CreateRectRgn(0, 0, 0, 0);
        try
        {
            if (GetWindowRgn(h, actual) <= 1) return false;
            foreach (bool square in new[] { false, true })
            {
                IntPtr ll = Rounder.MakeRegion(square, l, t, r, b);
                try { if (EqualRgn(actual, ll)) return true; }
                finally { Native.DeleteObject(ll); }
            }
            return false;
        }
        finally { Native.DeleteObject(actual); }
    }

    // Pencere yöneticisinin yerleştirdiği pencere hiçbir ekranda değilse ya da başlığı ekranın üstünde kaldıysa: en yakın
    // ekranın çalışma alanına (boyutu sığdığı kadar)
    internal static bool IntoView(IntPtr h)
    {
        Native.RECT r;
        if (!Native.GetWindowRect(h, out r)) return false;
        var rect = Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom);
        var screen = Screen.FromRectangle(rect);
        var work = screen.WorkingArea;
        // görünen ve tutulabilen bir şerit (başlık) var mı: üst kenarın 40 piksellik kısmı bir ekranda
        var title = new Rectangle(rect.Left, rect.Top, rect.Width, Math.Min(40, rect.Height));
        bool reachable = false;
        foreach (var s in Screen.AllScreens) { var i = Rectangle.Intersect(title, s.WorkingArea); if (i.Width >= 40 && i.Height >= 20) { reachable = true; break; } }
        if (reachable) return false;
        int w = Math.Min(rect.Width, work.Width), hgt = Math.Min(rect.Height, work.Height);
        int x = Math.Max(work.Left, Math.Min(rect.Left, work.Right - w)), y = Math.Max(work.Top, Math.Min(rect.Top, work.Bottom - hgt));
        return Native.SetWindowPos(h, IntPtr.Zero, x, y, w, hgt, 0x0004 | 0x0010 | 0x0200); // NOZORDER | NOACTIVATE | NOOWNERZORDER
    }

    // Hata kutusu yakalanırken (iletişim kutusu: #32770) tam saydam kalmış bir kutu: görünür olur
    internal static bool Reveal(IntPtr h)
    {
        int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        if ((ex & 0x00080000) == 0) return false;
        var cls = new StringBuilder(16); Native.GetClassName(h, cls, 16);
        if (cls.ToString() != "#32770") return false;
        uint key, flags; byte alpha;
        if (!GetLayeredWindowAttributes(h, out key, out alpha, out flags) || (flags & 0x2) == 0 || alpha != 0) return false;
        return Native.SetLayeredWindowAttributes(h, 0, 255, 0x2);
    }
}
