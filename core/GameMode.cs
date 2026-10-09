// Oyun kipi: tek sinyal, sahibi çekirdek. Öndeki pencere monitörünün tamamını kaplıyorsa (özel tam ekran ya da kenarlıksız
// tam ekran bir oyun, tam ekran bir video) ve Windows da bunu doğruluyorsa (SHQueryUserNotificationState: QUNS_BUSY ya da
// QUNS_RUNNING_D3D_FULL_SCREEN) oyun kipi açılır. Açıkken kabuk (bar, masaüstü widget'ları) çizmez ve yoklamaz, canlı duvar
// kağıdı o monitörde durur, pencere yöneticisinin kenarlıkları ve animasyonları boşta bekler, tepsi casusu ve yoklayıcılar
// geri çekilir, kimse "her zaman üstte"yi yeniden istemez, o monitörde fareyle odak kapalıdır. Algılama olaylarla: ön plan
// değişimi ve öndeki pencerenin konum değişimi (yalnızca onun sürecine kurulan bir kanca); yoklama yok.
using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

static class GameMode
{
    [DllImport("shell32.dll")] static extern int SHQueryUserNotificationState(out int state);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr mon, ref MONITORINFOEX info);
    [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct MONITORINFOEX
    {
        public int cbSize; public Native.RECT rcMonitor, rcWork; public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szDevice;
    }

    const int QUNS_BUSY = 2, QUNS_RUNNING_D3D_FULL_SCREEN = 3;
    const uint WINEVENT_OUTOFCONTEXT = 0x0000, MONITOR_DEFAULTTONULL = 0;
    // canlı duvar kağıdının mesajı (live-wallpaper ui.rs: WM_APP_GAME_MODE): wParam 1 açık / 0 kapalı, lParam monitör
    const uint WM_APP_GAME_MODE = 0x8000 + 5;
    const string WALLPAPER_CLASS = "LogicalLunge.LiveWallpaper";
    // Windows'un doğrulaması kaplamadan biraz sonra gelebilir: bir kez daha bakılır
    const int CONFIRM_AGAIN_MS = 1000;

    static readonly object gate = new object();
    static volatile bool on;
    static IntPtr monitor;
    static Rectangle bounds;
    static string device = "";
    static Native.WinEventDelegate fgCb, locCb;
    static IntPtr locHook, watched;
    static System.Windows.Forms.Timer again;
    static readonly TilingClient wm = new TilingClient();
    static readonly uint OwnPid = (uint)System.Diagnostics.Process.GetCurrentProcess().Id;

    public static bool On { get { return on; } }
    public static string Device { get { lock (gate) return device; } }

    // Nokta oyunun monitöründe mi (fareyle odak orada kapalı)
    public static bool Covers(Point p)
    {
        if (!on) return false;
        lock (gate) return on && bounds.Contains(p);
    }

    public static void Start()
    {
        var t = new Thread(() =>
        {
            fgCb = Callback.Guard("oyun kipi (ön plan)", OnForeground);
            locCb = Callback.Guard("oyun kipi (konum)", OnLocation);
            Native.SetWinEventHook(Native.EVENT_SYSTEM_FOREGROUND, Native.EVENT_SYSTEM_FOREGROUND, IntPtr.Zero, fgCb, 0, 0, WINEVENT_OUTOFCONTEXT);
            again = new System.Windows.Forms.Timer { Interval = CONFIRM_AGAIN_MS };
            again.Tick += (s, e) => { again.Stop(); Evaluate(false); };
            Watch(Native.GetAncestor(Native.GetForegroundWindow(), 2));
            Evaluate(true);
            Application.Run();
        }) { IsBackground = true, Name = "game-mode" };
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
    }

    static void OnForeground(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        Watch(Native.GetAncestor(hwnd, 2));
        Evaluate(true);
    }

    static void OnLocation(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        if (idObject != 0 || idChild != 0 || hwnd != watched) return; // OBJID_WINDOW, öndeki pencerenin kendisi
        Evaluate(true);
    }

    // Konum kancası yalnızca öndeki pencerenin sürecine (imleç ve diğer süreçlerin olayları gelmez)
    static void Watch(IntPtr root)
    {
        if (root == watched && locHook != IntPtr.Zero) return;
        if (locHook != IntPtr.Zero) { UnhookWinEvent(locHook); locHook = IntPtr.Zero; }
        watched = root;
        if (root == IntPtr.Zero) return;
        uint pid;
        Native.GetWindowThreadProcessId(root, out pid);
        if (pid == 0) return;
        locHook = Native.SetWinEventHook(Native.EVENT_OBJECT_LOCATIONCHANGE, Native.EVENT_OBJECT_LOCATIONCHANGE, IntPtr.Zero, locCb, pid, 0, WINEVENT_OUTOFCONTEXT);
    }

    // Kabuğun kendi yüzeyleri (overview, değiştirici, bar, masaüstü) oyun değildir
    static bool Ours(IntPtr root)
    {
        var cls = new StringBuilder(64);
        Native.GetClassName(root, cls, 64);
        string c = cls.ToString();
        if (c == "Progman" || c == "WorkerW" || c == "Shell_TrayWnd" || c == WALLPAPER_CLASS) return true;
        uint pid;
        Native.GetWindowThreadProcessId(root, out pid);
        if (pid == OwnPid) return true;
        string name = ProcInfo.Name(pid) ?? "";
        return name.Equals(Names.Shell, StringComparison.OrdinalIgnoreCase) || name.Equals(Names.Tiling, StringComparison.OrdinalIgnoreCase)
            || name.Equals(LiveWallpaper.Name, StringComparison.OrdinalIgnoreCase);
    }

    // recheck: Windows henüz doğrulamadıysa bir kez daha bakılsın
    static void Evaluate(bool recheck)
    {
        try
        {
            IntPtr root = watched;
            IntPtr mon = IntPtr.Zero;
            var info = new MONITORINFOEX { cbSize = Marshal.SizeOf(typeof(MONITORINFOEX)) };
            bool covers = false;
            if (root != IntPtr.Zero && Native.IsWindowVisible(root) && !Native.IsIconic(root) && !Ours(root))
            {
                mon = MonitorFromWindow(root, MONITOR_DEFAULTTONULL);
                Native.RECT r;
                if (mon != IntPtr.Zero && GetMonitorInfo(mon, ref info) && Native.GetWindowRect(root, out r))
                {
                    var m = info.rcMonitor;
                    covers = r.Left <= m.Left + 1 && r.Top <= m.Top + 1 && r.Right >= m.Right - 1 && r.Bottom >= m.Bottom - 1;
                }
            }
            bool game = false;
            if (covers)
            {
                int st;
                game = SHQueryUserNotificationState(out st) == 0 && (st == QUNS_BUSY || st == QUNS_RUNNING_D3D_FULL_SCREEN);
                if (!game && recheck) again.Start();
            }
            var m2 = info.rcMonitor;
            Set(game, mon, Rectangle.FromLTRB(m2.Left, m2.Top, m2.Right, m2.Bottom), info.szDevice ?? "");
        }
        catch (Exception ex) { Slider.Log("oyun kipi: " + ex.GetBaseException().Message); }
    }

    static void Set(bool game, IntPtr mon, Rectangle rect, string dev)
    {
        IntPtr oldMon;
        lock (gate)
        {
            if (game == on && (!game || mon == monitor)) return;
            oldMon = monitor;
            on = game;
            monitor = game ? mon : IntPtr.Zero;
            bounds = game ? rect : Rectangle.Empty;
            device = game ? dev : "";
        }
        Slider.Log(game ? "oyun kipi açık: " + dev + " (tam ekran uygulama önde)" : "oyun kipi kapalı");
        // başka bir monitöre geçen oyun: önceki monitörün duvar kağıdı devam etsin
        if (game && oldMon != IntPtr.Zero && oldMon != mon) Wallpaper(false, oldMon);
        Wallpaper(game, game ? mon : oldMon);
        Broadcast(game, dev);
        if (Keep.Round != null) Keep.Round.Quiet(game);
    }

    // Kabuğa (olay akışı) ve pencere yöneticisine (IPC): ikisi de kanca thread'ini beklemesin
    static void Broadcast(bool game, string dev)
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            Toasts.Emit(game ? "ll:game-mode-on:" + dev : "ll:game-mode-off");
            try { wm.Command("wm-game-mode " + (game ? "on" : "off")); }
            catch (Exception ex) { Slider.Log("oyun kipi: pencere yöneticisine söylenemedi: " + ex.Message); }
        });
    }

    static void Wallpaper(bool game, IntPtr mon)
    {
        IntPtr player = FindWindow(WALLPAPER_CLASS, null);
        if (player != IntPtr.Zero) PostMessage(player, WM_APP_GAME_MODE, (IntPtr)(game ? 1 : 0), mon);
    }

    // Yeniden bağlanan kabuk (olay akışı) o anki durumu da bilsin
    public static void Replay()
    {
        if (on) Toasts.Emit("ll:game-mode-on:" + Device);
    }
}
