// ll-shell — Logical Lunge'u Windows'un KABUĞU yapar (explorer.exe yerine).
//
// Windows oturum açılınca kabuk olarak Winlogon\Shell değerindeki programı başlatır (normalde explorer.exe).
// scripts\shell-mode.ps1 on bu değeri "ll-helper.exe --shell" yapar; o zaman Explorer hiç başlamaz ve görev çubuğu,
// Başlat menüsü, masaüstü, arama gibi parçaları hiç çalışmaz. Onların yaptığı ve kabuğun devralması gereken işler:
//   - Bar (ll-bar.cs, WebView2 yok) ve tray ikonları (TrayHost: programlar ikonlarını Shell_TrayWnd'ye yollar)
//   - Açılışta başlayan programlar (Run kayıtları, Başlangıç klasörü; Görev Yöneticisi'nde kapatılanlar hariç)
//   - Ses ve medya tuşları (uygulamaların işlemediği WM_APPCOMMAND kabuğa gelir)
//   - Oturum ekranının kapanması (kabuk hazır olayı)
// Bir şey ters giderse Windows yine kullanılabilir kalır: oturum açarken Shift basılıysa, kabuk aynı oturumda
// üst üste çöktüyse ya da %LOCALAPPDATA%\logical-lunge\use-explorer varsa Explorer açılır.
//
// Kabuk ayrı bir süreç (ll-helper --shell): asıl ll-helper (kısayollar, animasyonlar) çökse de bar ve tray
// ikonları gitmez; kabuk çökerse Windows (AutoRestartShell) ve asıl helper'daki ShellWatch onu yeniden başlatır.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;
using Microsoft.Win32;

static class LLShell
{
    public const string BarTitle = "ll-bar";
    // Zebar'daki bar da, native bar da "bizim bar"
    public static bool IsBarTitle(string t) { return t == "Zebar - logical-lunge / bar" || t == BarTitle; }

    // Bu süreç Windows'un kabuğu mu (ll-helper --shell ve Explorer'a dönülmedi)
    public static bool IsShell;

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr OpenEvent(uint access, bool inherit, string name);
    [DllImport("kernel32.dll")] static extern bool SetEvent(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("user32.dll")] static extern bool RegisterShellHookWindow(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern uint RegisterWindowMessage(string s);

    public static string Dir { get { return System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "logical-lunge"); } }
    public static string Home { get { return Environment.GetFolderPath(Environment.SpecialFolder.UserProfile); } }

    // Oturuma özel, kalıcı olmayan kayıt anahtarı: oturum kapanınca Windows siler. Aynı oturumda kabuğun kaç kez
    // başladığı ve açılış programlarının çalışıp çalışmadığı burada (kabuk çöküp yeniden başlarsa programlar iki kez açılmasın).
    const string SessionKey = @"Software\LogicalLunge-Session";
    static RegistryKey Session() { return Registry.CurrentUser.CreateSubKey(SessionKey, RegistryKeyPermissionCheck.ReadWriteSubTree, RegistryOptions.Volatile); }

    public static void Run()
    {
        bool created;
        var mutex = new Mutex(true, "ll-shell-single", out created);
        if (!created) return;
        try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { } // PER_MONITOR_AWARE_V2

        string why = FallbackReason();
        if (why != null)
        {
            Slider.Log("kabuk: Explorer açılıyor (" + why + ")");
            SignalReady();
            StartExplorer();
            // Kapanırsak Windows kabuğu (bizi) yeniden başlatır ve döngüye girer: Explorer açıkken sessizce bekle
            GC.KeepAlive(mutex);
            Thread.Sleep(Timeout.Infinite);
            return;
        }
        IsShell = true;
        Slider.Log("kabuk: başlıyor (explorer.exe yerine)");

        AppDomain.CurrentDomain.UnhandledException += (s, e) => Slider.Log("KABUK ÇÖKTÜ: " + e.ExceptionObject);
        Application.ThreadException += (s, e) => Slider.Log("kabuk UI hata: " + e.Exception);
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
        Application.EnableVisualStyles();

        var ui = new ShellWindow();
        var h = ui.Handle;
        TrayHost.Start();
        BarHost.Start(ui);
        MediaWatch.Start();
        // Uygulamaların işlemediği ses / medya tuşları (DefWindowProc -> kabuk kancası): Explorer'ın işi
        ui.ShellHookMsg = RegisterWindowMessage("SHELLHOOK");
        RegisterShellHookWindow(h);
        SignalReady();

        ThreadPool.QueueUserWorkItem(_ =>
        {
            try { EnsureGlazeWM(); } catch (Exception ex) { Slider.Log("kabuk: GlazeWM: " + ex.Message); }
            try { StartupApps.RunOnce(); } catch (Exception ex) { Slider.Log("kabuk: açılış programları: " + ex.Message); }
        });

        Application.Run(ui);
        GC.KeepAlive(mutex);
    }

    static string FallbackReason()
    {
        if ((Native.GetAsyncKeyState(0x10) & 0x8000) != 0) return "Shift basılı";
        if (System.IO.File.Exists(System.IO.Path.Combine(Dir, "use-explorer"))) return "use-explorer dosyası";
        if (Maint.Running("explorer")) return "Explorer zaten çalışıyor";
        // Aynı oturumda 2 dakika içinde 3. başlangıç: kabuk çöküp duruyor
        try
        {
            using (var k = Session())
            {
                var now = DateTime.UtcNow;
                var keep = new List<string>();
                var old = k.GetValue("starts") as string[];
                if (old != null)
                    foreach (var s in old)
                    {
                        long t;
                        if (long.TryParse(s, out t) && (now - new DateTime(t, DateTimeKind.Utc)).TotalMinutes < 2) keep.Add(s);
                    }
                keep.Add(now.Ticks.ToString());
                k.SetValue("starts", keep.ToArray(), RegistryValueKind.MultiString);
                if (keep.Count >= 3) return "kabuk 2 dakikada " + keep.Count + ". kez başladı";
            }
        }
        catch { }
        return null;
    }

    public static void StartExplorer()
    {
        if (Maint.Running("explorer")) return;
        // Çalışan bir kabuk yokken açılan explorer.exe görev çubuğu ve masaüstüyle tam kabuk olur
        try { Process.Start(new ProcessStartInfo(System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "explorer.exe")) { UseShellExecute = false, WorkingDirectory = Home }); }
        catch (Exception ex) { Slider.Log("kabuk: explorer başlatılamadı: " + ex.Message); }
    }

    // Oturum açılış ekranı ("Hoş geldiniz") kabuk hazır deyince kalkar; demezsek Windows bir süre bekler.
    static void SignalReady()
    {
        foreach (var name in new[] { "ShellDesktopSwitchEvent", "Global\\ShellDesktopSwitchEvent", "msgina: ShellReadyEvent", "Global\\msgina: ShellReadyEvent" })
        {
            IntPtr ev = OpenEvent(0x0002 /*EVENT_MODIFY_STATE*/, false, name);
            if (ev == IntPtr.Zero) continue;
            SetEvent(ev); CloseHandle(ev);
        }
    }

    public static string GlazeExe()
    {
        foreach (var exe in new[] {
            System.IO.Path.Combine(Home, @".glzr\logical-lunge\bin\glazewm.exe"),
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), @"glzr.io\GlazeWM\glazewm.exe") })
            if (System.IO.File.Exists(exe)) return exe;
        return null;
    }

    // Kurulumu yapan kullanıcıda LL\GlazeWM görevi GlazeWM'i zaten başlatır. Lab imajında sonradan açılan hesaplarda o görev
    // yok: birkaç saniye içinde gelmezse kabuk başlatır (GlazeWM de açılış komutlarıyla Zebar'ı ve asıl helper'ı başlatır).
    static void EnsureGlazeWM()
    {
        for (int i = 0; i < 12; i++) { if (Maint.Running("glazewm")) return; Thread.Sleep(500); }
        string exe = GlazeExe();
        if (exe == null) { Slider.Log("kabuk: glazewm.exe bulunamadı"); return; }
        Process.Start(new ProcessStartInfo(exe) { UseShellExecute = true, WorkingDirectory = Home });
        Slider.Log("kabuk: GlazeWM başlatıldı");
    }

    // Asıl helper'dan: kabuk modu açıkken kabuk süreci (ll-helper --shell) yoksa ve Explorer da çalışmıyorsa başlat.
    // Winlogon çöken kabuğu genelde kendisi yeniden başlatır; bu, onun yapmadığı durumlar için.
    public static void StartWatch()
    {
        new Thread(() =>
        {
            Thread.Sleep(20000);
            while (true)
            {
                Thread.Sleep(10000);
                try
                {
                    if (Maint.Quiet() || !Configured()) continue;
                    Mutex m;
                    bool alive = Mutex.TryOpenExisting("ll-shell-single", out m);
                    if (m != null) m.Dispose();
                    if (alive || Maint.Running("explorer") || !Maint.Allow("shell-restarts")) continue;
                    Slider.Log("kabuk nöbetçisi: kabuk çalışmıyordu, yeniden başlatılıyor");
                    Process.Start(new ProcessStartInfo(Maint.HelperExe, "--shell") { UseShellExecute = true, WorkingDirectory = Home });
                }
                catch (Exception ex) { Slider.Log("kabuk nöbetçisi: " + ex.Message); }
            }
        }) { IsBackground = true, Priority = ThreadPriority.BelowNormal, Name = "shell-watch" }.Start();
    }

    // Winlogon\Shell (kullanıcının ya da makinenin) bizi mi gösteriyor
    public static bool Configured()
    {
        foreach (var root in new[] { Registry.CurrentUser, Registry.LocalMachine })
        {
            try
            {
                using (var k = root.OpenSubKey(@"Software\Microsoft\Windows NT\CurrentVersion\Winlogon"))
                {
                    var v = k == null ? null : k.GetValue("Shell") as string;
                    if (v != null) return v.IndexOf("ll-helper", StringComparison.OrdinalIgnoreCase) >= 0 && v.IndexOf("--shell", StringComparison.OrdinalIgnoreCase) >= 0;
                }
            }
            catch { }
        }
        return false;
    }

    // Asıl helper'a (127.0.0.1:6131) bir komut: workspace, overview, sağ panel...
    public static bool Helper(string path)
    {
        try
        {
            var rq = (System.Net.HttpWebRequest)System.Net.WebRequest.Create("http://127.0.0.1:6131" + path);
            rq.Method = "POST"; rq.ContentLength = 0; rq.Timeout = 800; rq.Proxy = null;
            using (var rs = (System.Net.HttpWebResponse)rq.GetResponse()) return (int)rs.StatusCode < 300;
        }
        catch { return false; }
    }
    public static void HelperAsync(string path) { ThreadPool.QueueUserWorkItem(_ => Helper(path)); }

    // ---- Widget'lara kabuktan haber (sağ panel, ekran klavyesi...) ----
    // Zebar widget'ları bu olayları Tauri olaylarıyla birbirine yolluyordu; native bar Tauri'de değil. Sağ panel
    // /shell-wait uzun yoklamasıyla bekler ve gelen olayı (ör. "sidebar-right-toggle") "ll:" önekiyle yayınlar.
    static readonly object sigLock = new object();
    static int sigSeq;
    static string sigWhat = "";
    public static readonly string[] Signals = { "sidebar-right-toggle", "osk-toggle", "session-toggle", "update-check" };
    public static void Signal(string what)
    {
        lock (sigLock) { sigSeq++; sigWhat = what; Monitor.PulseAll(sigLock); }
    }
    public static string WaitSignal(int since, int timeoutMs)
    {
        lock (sigLock)
        {
            if (since < 0) return sigSeq + " ";
            var sw = Stopwatch.StartNew();
            while (sigSeq == since)
            {
                int left = timeoutMs - (int)sw.ElapsedMilliseconds;
                if (left <= 0) break;
                Monitor.Wait(sigLock, left);
            }
            return sigSeq + " " + (sigSeq == since ? "" : sigWhat);
        }
    }
}

// Kabuğun görünmez ana penceresi: kabuk kancası mesajları (ses / medya tuşları) buraya gelir
class ShellWindow : Form
{
    public uint ShellHookMsg;

    public ShellWindow()
    {
        ShowInTaskbar = false; FormBorderStyle = FormBorderStyle.None; Opacity = 0;
        WindowState = FormWindowState.Minimized; Text = "ll-shell";
        Load += (s, e) => Hide();
    }

    protected override void WndProc(ref Message m)
    {
        if (ShellHookMsg != 0 && (uint)m.Msg == ShellHookMsg && m.WParam.ToInt64() == 12 /*HSHELL_APPCOMMAND*/)
        {
            int cmd = (int)((m.LParam.ToInt64() >> 16) & 0x0FFF); // GET_APPCOMMAND_LPARAM
            if (AppCommand(cmd)) { m.Result = (IntPtr)1; return; }
        }
        base.WndProc(ref m);
    }

    static bool AppCommand(int cmd)
    {
        switch (cmd)
        {
            case 8: Volume.ToggleMute(); BarHost.ShowOsd("volume", Volume.Muted ? 0 : Volume.Level); return true;
            case 9: Volume.Step(-2); BarHost.ShowOsd("volume", Volume.Level); return true;   // ii: 2'şer
            case 10: Volume.Step(2); BarHost.ShowOsd("volume", Volume.Level); return true;
            case 11: MediaWatch.Send("next"); return true;
            case 12: MediaWatch.Send("prev"); return true;
            case 13: MediaWatch.Send("stop"); return true;
            case 14: case 46: case 47: MediaWatch.Send("toggle"); return true;
            case 50: Mic.SetAll(!Mic.IsMuted()); BarHost.Refresh(); return true; // APPCOMMAND_MIC_ON_OFF_TOGGLE
        }
        return false;
    }
}

// ---------------- Açılış programları ----------------
// Explorer oturum açılınca Run kayıtlarını ve Başlangıç klasörlerini çalıştırır; kabuk biz olunca bu iş bizde.
// Görev Yöneticisi > Başlangıç'ta kapatılanlar (StartupApproved) atlanır. Oturum başına bir kez.
static class StartupApps
{
    const string Approved = @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\";

    public static void RunOnce()
    {
        using (var k = Registry.CurrentUser.CreateSubKey(@"Software\LogicalLunge-Session", RegistryKeyPermissionCheck.ReadWriteSubTree, RegistryOptions.Volatile))
        {
            if (k.GetValue("startup") != null) return;
            k.SetValue("startup", DateTime.Now.ToString("o"));
        }
        Thread.Sleep(2500); // önce masaüstü (GlazeWM, bar) ayağa kalksın
        int n = 0;
        n += RunKey(Registry.LocalMachine, @"Software\Microsoft\Windows\CurrentVersion\Run", "Run", false);
        n += RunKey(Registry.LocalMachine, @"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run", "Run32", false);
        n += RunKey(Registry.CurrentUser, @"Software\Microsoft\Windows\CurrentVersion\Run", "Run", false);
        n += RunKey(Registry.CurrentUser, @"Software\Microsoft\Windows\CurrentVersion\RunOnce", null, true);
        n += RunFolder(Environment.GetFolderPath(Environment.SpecialFolder.CommonStartup));
        n += RunFolder(Environment.GetFolderPath(Environment.SpecialFolder.Startup));
        Slider.Log("kabuk: " + n + " açılış programı başlatıldı");
    }

    // StartupApproved: ilk bayt tekse (03, 07...) kullanıcı kapatmış
    static bool Disabled(string group, string name)
    {
        foreach (var root in new[] { Registry.CurrentUser, Registry.LocalMachine })
        {
            try
            {
                using (var k = root.OpenSubKey(Approved + group))
                {
                    var b = k == null ? null : k.GetValue(name) as byte[];
                    if (b != null && b.Length > 0) return (b[0] & 1) != 0;
                }
            }
            catch { }
        }
        return false;
    }

    static bool Ours(string cmd)
    {
        string c = cmd.ToLowerInvariant();
        return c.Contains("glazewm") || c.Contains("zebar") || c.Contains("ll-helper");
    }

    static int RunKey(RegistryKey root, string path, string group, bool once)
    {
        int n = 0;
        try
        {
            using (var k = root.OpenSubKey(path, once))
            {
                if (k == null) return 0;
                foreach (var name in k.GetValueNames())
                {
                    string cmd = k.GetValue(name) as string;
                    if (once) { try { k.DeleteValue(name); } catch { } }
                    if (string.IsNullOrWhiteSpace(cmd) || Ours(cmd)) continue;
                    if (group != null && Disabled(group, name)) continue;
                    if (Launch(cmd)) n++;
                }
            }
        }
        catch (Exception ex) { Slider.Log("kabuk: " + path + ": " + ex.Message); }
        return n;
    }

    static int RunFolder(string dir)
    {
        int n = 0;
        if (string.IsNullOrEmpty(dir) || !System.IO.Directory.Exists(dir)) return 0;
        foreach (var f in System.IO.Directory.GetFiles(dir))
        {
            string name = System.IO.Path.GetFileName(f);
            if (name.Equals("desktop.ini", StringComparison.OrdinalIgnoreCase) || Disabled("StartupFolder", name)) continue;
            try { Process.Start(new ProcessStartInfo(f) { UseShellExecute = true, WorkingDirectory = dir }); n++; }
            catch (Exception ex) { Slider.Log("kabuk: " + name + ": " + ex.Message); }
        }
        return n;
    }

    // Run değerleri tam komut satırı: "C:\Program Files\X\x.exe" /min ya da tırnaksız C:\Program Files\X\x.exe /min
    static bool Launch(string cmd)
    {
        cmd = Environment.ExpandEnvironmentVariables(cmd.Trim());
        string exe = null, args = "";
        if (cmd.StartsWith("\""))
        {
            int e = cmd.IndexOf('"', 1);
            if (e > 1) { exe = cmd.Substring(1, e - 1); args = cmd.Substring(e + 1).Trim(); }
        }
        else
        {
            // Boşluklarda bölünmüş en kısa var olan dosya yolu; yoksa ilk kelime (PATH'teki rundll32 gibi)
            int from = 0;
            while (exe == null)
            {
                int sp = cmd.IndexOf(' ', from);
                string cand = sp < 0 ? cmd : cmd.Substring(0, sp);
                if (System.IO.File.Exists(cand) || System.IO.File.Exists(cand + ".exe")) { exe = cand; args = sp < 0 ? "" : cmd.Substring(sp + 1).Trim(); }
                else if (sp < 0) break;
                from = sp + 1;
            }
            if (exe == null)
            {
                int sp = cmd.IndexOf(' ');
                exe = sp < 0 ? cmd : cmd.Substring(0, sp);
                args = sp < 0 ? "" : cmd.Substring(sp + 1).Trim();
            }
        }
        if (string.IsNullOrEmpty(exe)) return false;
        try
        {
            string wd = System.IO.Path.IsPathRooted(exe) ? System.IO.Path.GetDirectoryName(exe) : LLShell.Home;
            Process.Start(new ProcessStartInfo(exe, args) { UseShellExecute = true, WorkingDirectory = wd });
            return true;
        }
        catch (Exception ex) { Slider.Log("kabuk: başlatılamadı: " + cmd + " (" + ex.Message + ")"); return false; }
    }
}

// ---------------- Tray (bildirim alanı) ----------------
// Programlar tray ikonlarını Shell_NotifyIcon ile ekler; Windows bunu "Shell_TrayWnd" sınıfındaki pencereye WM_COPYDATA
// olarak iletir (normalde Explorer'ın görev çubuğu). Kabuk biz olunca o pencere bizde: ikonlar native bar'da çizilir,
// tıklamalar programa, programın beklediği biçimde (sürüm 3 / 4) geri yollanır.
static class TrayHost
{
    public class Icon
    {
        public IntPtr Wnd; public uint Id; public Guid Guid; public uint CallbackMsg; public uint Version;
        public Bitmap Image; public string Tip = ""; public bool Hidden;
        public Rectangle Screen; // bar'da son çizildiği yer (Shell_NotifyIconGetRect için)
        public string Key { get { return Guid != Guid.Empty ? Guid.ToString() : Wnd.ToInt64() + ":" + Id; } }
    }

    delegate IntPtr WndProcFn(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WNDCLASSEX
    {
        public int cbSize; public uint style; public IntPtr lpfnWndProc; public int cbClsExtra, cbWndExtra;
        public IntPtr hInstance, hIcon, hCursor, hbrBackground; public string lpszMenuName, lpszClassName; public IntPtr hIconSm;
    }
    [StructLayout(LayoutKind.Sequential)] struct COPYDATASTRUCT { public IntPtr dwData; public int cbData; public IntPtr lpData; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern ushort RegisterClassEx(ref WNDCLASSEX wc);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowEx(int ex, string cls, string title, int style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
    [DllImport("user32.dll")] static extern IntPtr DefWindowProc(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr CopyIcon(IntPtr h);
    [DllImport("user32.dll")] static extern bool DestroyIcon(IntPtr h);
    [DllImport("user32.dll")] static extern bool AllowSetForegroundWindow(uint pid);
    [DllImport("user32.dll")] static extern bool SendNotifyMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool ChangeWindowMessageFilterEx(IntPtr h, uint msg, uint action, IntPtr info);

    static WndProcFn proc; // çöp toplayıcı silmesin
    static IntPtr tray;
    static readonly List<Icon> icons = new List<Icon>();
    public static event Action Changed;

    public static List<Icon> Visible()
    {
        var l = new List<Icon>();
        lock (icons) foreach (var i in icons) if (!i.Hidden && i.Image != null) l.Add(i);
        return l;
    }

    // Kabuğun UI thread'inde
    public static void Start()
    {
        proc = WndProc;
        IntPtr inst = Native.GetModuleHandle(null);
        foreach (var cls in new[] { "Shell_TrayWnd", "TrayNotifyWnd" })
        {
            var wc = new WNDCLASSEX { cbSize = Marshal.SizeOf(typeof(WNDCLASSEX)), lpfnWndProc = Marshal.GetFunctionPointerForDelegate(proc), hInstance = inst, lpszClassName = cls };
            RegisterClassEx(ref wc);
        }
        // Görünmez, sıfır boyutlu üst düzey pencere: FindWindow("Shell_TrayWnd") bulabilsin diye message-only değil
        tray = CreateWindowEx(0x80 /*TOOLWINDOW*/ | 0x8 /*TOPMOST*/, "Shell_TrayWnd", "", unchecked((int)0x80000000) /*POPUP*/, 0, 0, 0, 0, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);
        // Bazı programlar önce tepsi alt penceresini arar
        CreateWindowEx(0, "TrayNotifyWnd", "", 0x40000000 /*CHILD*/, 0, 0, 0, 0, tray, IntPtr.Zero, inst, IntPtr.Zero);
        uint created = LLShell.RegisterWindowMessage("TaskbarCreated");
        // Düşük bütünlükteki süreçler de ikon ekleyebilsin (UIPI)
        ChangeWindowMessageFilterEx(tray, 0x004A /*WM_COPYDATA*/, 1, IntPtr.Zero);
        ChangeWindowMessageFilterEx(tray, created, 1, IntPtr.Zero);
        // Çalışan programlar ikonlarını yeniden eklesin (Explorer yeniden başlayınca da aynısı olur)
        SendNotifyMessage(new IntPtr(0xFFFF) /*HWND_BROADCAST*/, created, IntPtr.Zero, IntPtr.Zero);
        Slider.Log("tray: Shell_TrayWnd hazır");

        // Kapanan programların ikonları (NIM_DELETE demeden çıkanlar) temizlensin
        var t = new System.Windows.Forms.Timer { Interval = 3000 };
        t.Tick += (s, e) =>
        {
            bool any = false;
            lock (icons) any = icons.RemoveAll(i => { bool dead = !Native.IsWindow(i.Wnd); if (dead && i.Image != null) i.Image.Dispose(); return dead; }) > 0;
            if (any) Raise();
        };
        t.Start();
    }

    static void Raise() { var c = Changed; if (c != null) c(); }

    static IntPtr WndProc(IntPtr h, uint msg, IntPtr w, IntPtr l)
    {
        if (msg == 0x004A /*WM_COPYDATA*/)
        {
            try
            {
                var cds = (COPYDATASTRUCT)Marshal.PtrToStructure(l, typeof(COPYDATASTRUCT));
                long kind = cds.dwData.ToInt64();
                if (kind == 1) return (IntPtr)(NotifyIcon(cds.lpData, cds.cbData) ? 1 : 0);
                if (kind == 3) return IconRect(cds.lpData, cds.cbData);
                return IntPtr.Zero; // 0: AppBar mesajları (SHAppBarMessage) — görev çubuğu yok
            }
            catch (Exception ex) { Slider.Log("tray: " + ex.Message); return IntPtr.Zero; }
        }
        return DefWindowProc(h, msg, w, l);
    }

    // SHELLTRAYDATA { DWORD magic; DWORD message; NOTIFYICONDATAW (32 bit tutamaçlarla) } — 64 bit Windows'ta da tutamaçlar 4 bayt
    const int NID = 8;
    static bool NotifyIcon(IntPtr p, int len)
    {
        if (len < NID + 24 || Marshal.ReadInt32(p, 0) != 0x34753423) return false;
        int message = Marshal.ReadInt32(p, 4);
        int cbSize = Marshal.ReadInt32(p, NID + 0);
        int avail = Math.Min(cbSize, len - NID);
        IntPtr wnd = new IntPtr((long)(uint)Marshal.ReadInt32(p, NID + 4));
        uint id = (uint)Marshal.ReadInt32(p, NID + 8);
        uint flags = (uint)Marshal.ReadInt32(p, NID + 12);
        Guid guid = Guid.Empty;
        if ((flags & 0x20) != 0 && avail >= 952) { var gb = new byte[16]; Marshal.Copy(p + NID + 936, gb, 0, 16); guid = new Guid(gb); }

        Icon ic = null;
        lock (icons)
            foreach (var i in icons)
                if (guid != Guid.Empty ? i.Guid == guid : (i.Wnd == wnd && i.Id == id)) { ic = i; break; }

        switch (message)
        {
            case 0: // NIM_ADD
                if (ic != null) return false;
                ic = new Icon { Wnd = wnd, Id = id, Guid = guid };
                Apply(ic, p, avail, flags);
                lock (icons) icons.Add(ic);
                break;
            case 1: // NIM_MODIFY
                if (ic == null) return false;
                Apply(ic, p, avail, flags);
                break;
            case 2: // NIM_DELETE
                if (ic == null) return false;
                lock (icons) icons.Remove(ic);
                if (ic.Image != null) ic.Image.Dispose();
                break;
            case 3: return ic != null; // NIM_SETFOCUS
            case 4: // NIM_SETVERSION
                if (ic == null) return false;
                if (avail >= 804) ic.Version = (uint)Marshal.ReadInt32(p, NID + 800);
                return true;
            default: return false;
        }
        Raise();
        return true;
    }

    static void Apply(Icon ic, IntPtr p, int avail, uint flags)
    {
        if ((flags & 0x1) != 0) ic.CallbackMsg = (uint)Marshal.ReadInt32(p, NID + 16);        // NIF_MESSAGE
        if ((flags & 0x2) != 0)                                                                // NIF_ICON
        {
            IntPtr hi = new IntPtr((long)(uint)Marshal.ReadInt32(p, NID + 20));
            Bitmap bmp = null;
            if (hi != IntPtr.Zero)
            {
                IntPtr copy = CopyIcon(hi);
                if (copy != IntPtr.Zero)
                {
                    try { using (var i = System.Drawing.Icon.FromHandle(copy)) bmp = i.ToBitmap(); } catch { }
                    DestroyIcon(copy);
                }
            }
            if (ic.Image != null) ic.Image.Dispose();
            ic.Image = bmp;
        }
        if ((flags & 0x4) != 0)                                                                // NIF_TIP
        {
            int chars = avail >= 280 ? 128 : 64; // eski NOTIFYICONDATA'da 64 karakter
            ic.Tip = Marshal.PtrToStringUni(p + NID + 24, chars);
            int z = ic.Tip.IndexOf('\0'); if (z >= 0) ic.Tip = ic.Tip.Substring(0, z);
        }
        if ((flags & 0x8) != 0 && avail >= 288)                                                // NIF_STATE
        {
            uint state = (uint)Marshal.ReadInt32(p, NID + 280), mask = (uint)Marshal.ReadInt32(p, NID + 284);
            if ((mask & 1) != 0) ic.Hidden = (state & 1) != 0;                                 // NIS_HIDDEN
        }
    }

    // Shell_NotifyIconGetRect: 1. mesaj sol/üst, 2. mesaj genişlik/yükseklik (MAKELONG)
    static IntPtr IconRect(IntPtr p, int len)
    {
        if (len < 36) return IntPtr.Zero;
        int message = Marshal.ReadInt32(p, 4);
        IntPtr wnd = new IntPtr((long)(uint)Marshal.ReadInt32(p, 12));
        uint id = (uint)Marshal.ReadInt32(p, 16);
        var gb = new byte[16]; Marshal.Copy(p + 20, gb, 0, 16);
        var guid = new Guid(gb);
        lock (icons)
            foreach (var i in icons)
                if ((guid != Guid.Empty && i.Guid == guid) || (i.Wnd == wnd && i.Id == id))
                {
                    var r = i.Screen;
                    if (r.IsEmpty) return IntPtr.Zero;
                    return message == 1 ? MakeLong(r.Left, r.Top) : MakeLong(r.Width, r.Height);
                }
        return IntPtr.Zero;
    }
    static IntPtr MakeLong(int lo, int hi) { return new IntPtr((int)(((uint)(ushort)lo) | ((uint)(ushort)hi << 16))); }

    // Tıklamayı programa yolla. Sürüm 4: wParam = imleç, lParam = (mesaj, ikon kimliği); daha eskisi: wParam = kimlik, lParam = mesaj.
    public static void Click(Icon ic, int button, bool dbl, Point screen)
    {
        if (!Native.IsWindow(ic.Wnd)) return;
        uint pid; Native.GetWindowThreadProcessId(ic.Wnd, out pid);
        AllowSetForegroundWindow(pid); // programın menüsü / penceresi öne gelebilsin
        if (button == 0)
        {
            if (dbl) Post(ic, 0x0203, screen);                // WM_LBUTTONDBLCLK
            else { Post(ic, 0x0201, screen); Post(ic, 0x0202, screen); if (ic.Version >= 4) Post(ic, 0x0400, screen); } // NIN_SELECT
        }
        else if (button == 1) { Post(ic, 0x0204, screen); Post(ic, 0x0205, screen); if (ic.Version >= 4) Post(ic, 0x007B, screen); } // WM_CONTEXTMENU
        else { Post(ic, 0x0207, screen); Post(ic, 0x0208, screen); }
    }
    public static void Hover(Icon ic, Point screen) { if (Native.IsWindow(ic.Wnd)) Post(ic, 0x0200, screen); }

    static void Post(Icon ic, int mouseMsg, Point pt)
    {
        if (ic.CallbackMsg == 0) return;
        if (ic.Version >= 4)
            Native.PostMessage(ic.Wnd, ic.CallbackMsg, MakeLong(pt.X, pt.Y), MakeLong(mouseMsg, (int)ic.Id));
        else
            Native.PostMessage(ic.Wnd, ic.CallbackMsg, new IntPtr((long)ic.Id), new IntPtr(mouseMsg));
    }
}
