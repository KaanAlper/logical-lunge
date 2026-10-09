// lunge — Logical Lunge'un çekirdeği ve kök süreci. Pencere yöneticisini (lunge-tiling) ve kabuğu (lunge-shell) alt
// süreç olarak açar ve korur (Supervisor, nöbetçiler); pencere yöneticisinin yapamadığı, Hyprland/ii'de olan şeyleri yapar:
//   1) Workspace geçişinde "slide" animasyonu (Hyprland: animation workspaces, slide, menu_decel)
//      DWM thumbnail'leri ile: eski workspace'in canlı görüntüsü kayarak çıkar, yenisi girer.
//   2) Tüm pencerelerde yuvarlak köşe (Hyprland decoration.rounding) — Win10'da DWM yapmadığı
//      için SetWindowRgn ile.
//   3) Tek başına Super -> ii overview (arama) aç/kapa; Başlat menüsü açılmaz.
//
// Kısayollar (tiling config'den buraya taşındı, animasyonlu olsunlar diye):
//   Super+Ctrl+←/→          workspace sol/sağ
//   Super+Ctrl+Shift+←/→    pencereyi taşı + takip et
//   Super+1..0              workspace'e git
//   Super+←/↑/→/↓           odak yalnızca mevcut workspace içinde (asla başka workspace/monitöre atlamaz)
//
// Derleme: build.ps1 (csc, .NET Framework 4)
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Net.WebSockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// ---------------- Yollar ve adlar (tek yerde) ----------------
// Kurulum: lunge.exe'nin klasörü (%ProgramFiles%\LogicalLunge; yönetici korumalı, çünkü çekirdek ve pencere yöneticisi
// yönetici haklarıyla çalışır): exe'ler, ui, scripts, tools, VERSION. Çalışırken yazılan her şey kullanıcının klasörlerinde.
// Kullanıcının düzenlediği ayarlar: ~\.config\logical-lunge (config.yaml, keybinds.json).
// Uygulama verisi: %LOCALAPPDATA%\LogicalLunge (state, logs, update; WebView verisi shell'de).
static class Paths
{
    public static readonly string Install = AppDomain.CurrentDomain.BaseDirectory.TrimEnd('\\');
    public static readonly string Home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
    static readonly string DataRoot = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LogicalLunge");

    public static string In(string rel) { return System.IO.Path.Combine(Install, rel); }
    public static string Core { get { return In("lunge.exe"); } }
    public static string Tiling { get { return In("lunge-tiling.exe"); } }
    public static string TilingCli { get { return In("lunge-tiling-cli.exe"); } }
    public static string Shell { get { return In("lunge-shell.exe"); } }
    public static string Ui { get { return In("ui"); } }
    public static string UiPack(string rel) { return System.IO.Path.Combine(Ui, "logical-lunge", rel); }
    public static string Script(string name) { return In(System.IO.Path.Combine("scripts", name)); }
    public static string Tool(string rel) { return In(System.IO.Path.Combine("tools", rel)); }
    public static string Version { get { return In("VERSION"); } }

    public static string ConfigDir { get { return Dir(System.IO.Path.Combine(Home, @".config\logical-lunge")); } }
    public static string ConfigFile { get { return System.IO.Path.Combine(ConfigDir, "config.yaml"); } }
    public static string StateDir { get { return Dir(System.IO.Path.Combine(DataRoot, "state")); } }
    public static string State(string name) { return System.IO.Path.Combine(StateDir, name); }
    // Super menüsünün uygulama listesi (kurulum klasörü yönetici korumalı: kullanıcının yazdığı her şey veri klasöründe)
    public static string AppsJson { get { return State("apps.json"); } }
    public static string LogsDir { get { return Dir(System.IO.Path.Combine(DataRoot, "logs")); } }
    public static string DataDir(string name) { return Dir(System.IO.Path.Combine(DataRoot, name)); }

    static string Dir(string d) { try { System.IO.Directory.CreateDirectory(d); } catch { } return d; }
}

// Süreç adı / yolu: OpenProcess + QueryFullProcessImageName + CloseHandle (mikrosaniyeler, iz bırakmaz). Önceden
// Process.GetProcessById(pid).ProcessName her çağrıda sistemdeki tüm süreçlerin anlık görüntüsünü alıyordu (her yeni
// pencerede); MainModule hedef sürecin tüm modüllerini sayıyor ve açtığı tutamacı çöp toplayıcıya kadar tutuyordu.
static class ProcInfo
{
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool QueryFullProcessImageName(IntPtr h, int flags, StringBuilder name, ref int size);

    public static string Path(uint pid)
    {
        if (pid == 0) return null;
        IntPtr h = OpenProcess(0x1000, false, pid); // PROCESS_QUERY_LIMITED_INFORMATION
        if (h == IntPtr.Zero) return null;
        try
        {
            var sb = new StringBuilder(1024); int n = sb.Capacity;
            return QueryFullProcessImageName(h, 0, sb, ref n) ? sb.ToString(0, n) : null;
        }
        finally { CloseHandle(h); }
    }

    // Process.ProcessName biçiminde (uzantısız dosya adı); bulunamazsa ""
    public static string Name(uint pid)
    {
        string p = Path(pid);
        if (p != null) return System.IO.Path.GetFileNameWithoutExtension(p);
        try { using (var pr = Process.GetProcessById((int)pid)) return pr.ProcessName; } catch { return ""; }
    }
}

// Süreç adları ve widget pencere başlıkları (shell "Logical Lunge · <widget>" koyar)
static class Names
{
    public const string Core = "lunge", Tiling = "lunge-tiling", Shell = "lunge-shell";
    public const string Bar = "Logical Lunge · bar", Toast = "Logical Lunge · toast", Update = "Logical Lunge · update",
        Osk = "Logical Lunge · osk", Sidebar = "Logical Lunge · sidebar-right", Settings = "Logical Lunge · settings",
        TitlePrefix = "Logical Lunge ·", DesktopWidget = "Logical Lunge · widget",
        // açılış örtüsü (canlı duvar kağıdı onu "masaüstünü örten pencere" saymaz: arkasında ilk karesini çizsin)
        StartupCover = "Logical Lunge · açılış";
}

static class Native
{
    public delegate IntPtr LowLevelKeyboardProc(int nCode, IntPtr wParam, IntPtr lParam);
    public delegate void WinEventDelegate(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time);
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    public struct KBDLLHOOKSTRUCT { public uint vkCode, scanCode, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct DWM_THUMBNAIL_PROPERTIES
    {
        public uint dwFlags; public RECT rcDestination; public RECT rcSource;
        public byte opacity; public bool fVisible; public bool fSourceClientAreaOnly;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct WINDOWPLACEMENT { public int length, flags, showCmd; public Point min, max; public RECT normal; }

    [DllImport("user32.dll")] public static extern IntPtr SetWindowsHookEx(int id, LowLevelKeyboardProc fn, IntPtr mod, uint tid);
    [DllImport("user32.dll")] public static extern IntPtr CallNextHookEx(IntPtr h, int n, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool UnhookWindowsHookEx(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr h);
    [DllImport("kernel32.dll")] public static extern IntPtr GetModuleHandle(string name);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    // Çekirdeğin kendi ürettiği tuşların imzası (dwExtraInfo): kanca yalnızca bunları atlar. Telefondan / uzak
    // bağlantıdan / ekran klavyesinden gelen yapay tuşlar gerçek tuş gibi işlenir (eskiden hepsi atlanıyordu: Win
    // tuşu doğrudan Windows'a gidip Başlat menüsünü açıyordu).
    public static readonly UIntPtr LL_MARK = (UIntPtr)0x4C4C4B31u; // "LLK1"
    [DllImport("user32.dll")] public static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr mod, WinEventDelegate fn, uint pid, uint tid, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", SetLastError = true)] public static extern int SetWindowRgn(IntPtr h, IntPtr rgn, bool redraw);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetLayeredWindowAttributes(IntPtr h, uint key, byte alpha, uint flags);
    [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr h, IntPtr rect, IntPtr rgn, uint flags);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateRoundRectRgn(int l, int t, int r, int b, int w, int h);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateRectRgn(int l, int t, int r, int b);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
    [DllImport("user32.dll")] public static extern int GetWindowRgnBox(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int idx);
    [DllImport("user32.dll")] public static extern int SetWindowLong(IntPtr h, int idx, int v);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowPlacement(IntPtr h, ref WINDOWPLACEMENT wp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc fn, IntPtr l);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [StructLayout(LayoutKind.Sequential)] public struct LASTINPUTINFO { public uint cbSize, dwTime; }
    [DllImport("user32.dll")] public static extern bool GetLastInputInfo(ref LASTINPUTINFO info);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int v, int size);
    [DllImport("dwmapi.dll", EntryPoint = "DwmRegisterThumbnail")] static extern int DwmRegisterThumbnail0(IntPtr dest, IntPtr src, out IntPtr thumb);
    [DllImport("dwmapi.dll", EntryPoint = "DwmUnregisterThumbnail")] static extern int DwmUnregisterThumbnail0(IntPtr thumb);
    // DWM'de şu an kayıtlı önizleme sayısı: animasyon log'una yazılır. Kullandıkça artıyorsa bir yerde silinmiyor
    // demektir (DWM her karede hepsini taşır ve geçişler giderek yavaşlar).
    public static int LiveThumbs;
    public static int DwmRegisterThumbnail(IntPtr dest, IntPtr src, out IntPtr thumb)
    {
        int hr = DwmRegisterThumbnail0(dest, src, out thumb);
        if (hr == 0) Interlocked.Increment(ref LiveThumbs);
        return hr;
    }
    public static int DwmUnregisterThumbnail(IntPtr thumb)
    {
        int hr = DwmUnregisterThumbnail0(thumb);
        if (hr == 0) Interlocked.Decrement(ref LiveThumbs);
        return hr;
    }
    [DllImport("dwmapi.dll")] public static extern int DwmUpdateThumbnailProperties(IntPtr thumb, ref DWM_THUMBNAIL_PROPERTIES p);
    [DllImport("dwmapi.dll")] public static extern int DwmFlush();
    [StructLayout(LayoutKind.Sequential, Pack = 1)] public struct UNSIGNED_RATIO { public uint uiNumerator, uiDenominator; }
    // dwmapi.h: pshpack1 içinde tanımlı; cbSize tam tutmazsa çağrı reddedilir
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    public struct DWM_TIMING_INFO
    {
        public uint cbSize; public UNSIGNED_RATIO rateRefresh; public ulong qpcRefreshPeriod; public UNSIGNED_RATIO rateCompose;
        public ulong qpcVBlank, cRefresh; public uint cDXRefresh; public ulong qpcCompose, cFrame; public uint cDXPresent;
        public ulong cRefreshFrame, cFrameSubmitted; public uint cDXPresentSubmitted; public ulong cFrameConfirmed; public uint cDXPresentConfirmed;
        public ulong cRefreshConfirmed; public uint cDXRefreshConfirmed; public ulong cFramesLate; public uint cFramesOutstanding;
        public ulong cFrameDisplayed, qpcFrameDisplayed, cRefreshFrameDisplayed, cFrameComplete, qpcFrameComplete, cFramePending, qpcFramePending;
        public ulong cFramesDisplayed, cFramesComplete, cFramesPending, cFramesAvailable, cFramesDropped, cFramesMissed;
        public ulong cRefreshNextDisplayed, cRefreshNextPresented, cRefreshesDisplayed, cRefreshesPresented, cRefreshStarted;
        public ulong cPixelsReceived, cPixelsDrawn, cBuffersEmpty;
    }
    [DllImport("dwmapi.dll")] public static extern int DwmGetCompositionTimingInfo(IntPtr hwnd, ref DWM_TIMING_INFO info);
    [StructLayout(LayoutKind.Sequential)] public struct SIZE { public int cx, cy; }
    public delegate IntPtr LowLevelMouseProc(int nCode, IntPtr wParam, IntPtr lParam);
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct MSLLHOOKSTRUCT { public POINT pt; public uint mouseData, flags, time; public IntPtr extra; }
    [DllImport("user32.dll")] public static extern IntPtr SetWindowsHookEx(int id, LowLevelMouseProc fn, IntPtr mod, uint tid);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
    public const int WH_MOUSE_LL = 14;
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc fn, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUT
    {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public KEYBDINPUT ki;          // 64-bit düzeni
        [FieldOffset(8)] public long pad0; [FieldOffset(16)] public long pad1; [FieldOffset(24)] public long pad2; [FieldOffset(32)] public long pad3;
    }
    [DllImport("dwmapi.dll")] public static extern int DwmQueryThumbnailSourceSize(IntPtr thumb, out SIZE size);

    public const int WH_KEYBOARD_LL = 13;
    public const int WM_KEYDOWN = 0x100, WM_KEYUP = 0x101, WM_SYSKEYDOWN = 0x104, WM_SYSKEYUP = 0x105;
    public const uint LLKHF_INJECTED = 0x10;
    public const int GWL_STYLE = -16, GWL_EXSTYLE = -20;
    public const int WS_CAPTION = 0x00C00000, WS_CHILD = 0x40000000, WS_POPUP = unchecked((int)0x80000000);
    public const int WS_EX_TOOLWINDOW = 0x80, WS_EX_NOACTIVATE = 0x08000000, WS_EX_TOPMOST = 0x8, WS_EX_TRANSPARENT = 0x20;
    public const int DWMWA_EXTENDED_FRAME_BOUNDS = 9, DWMWA_CLOAKED = 14;
    public const uint DWM_TNP_RECTDESTINATION = 0x1, DWM_TNP_RECTSOURCE = 0x2, DWM_TNP_OPACITY = 0x4, DWM_TNP_VISIBLE = 0x8, DWM_TNP_SOURCECLIENTAREAONLY = 0x10;
    public const uint EVENT_OBJECT_SHOW = 0x8002, EVENT_OBJECT_LOCATIONCHANGE = 0x800B, EVENT_SYSTEM_FOREGROUND = 0x0003;
}

// ---------------- tiling IPC (ws://localhost:6123) ----------------
class TilingClient
{
    ClientWebSocket ws;
    readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };
    readonly object gate = new object();

    // Süre sınırı: pencere yöneticisi takılınca çağıran donmasın. Mesaj döngüsü olan thread'ler (arayüz, klavye kancası)
    // toplam en fazla ~1,5 sn bekler, arka plan işleri 5 sn (iki deneme de bu sürenin içinde). Önceden bağlanma (3 sn), gönderme (1,5 sn) ve
    // her ileti için 2 sn'lik okuma iki denemede toplanıyor, kilit sırası da eklenince arayüz 13 sn'ye kadar donuyordu.
    Dictionary<string, object> Send(string message)
    {
        bool interactive = System.Windows.Forms.Application.MessageLoop;
        int budget = interactive ? 1500 : 5000;
        var sw = Stopwatch.StartNew();
        if (!Monitor.TryEnter(gate, budget)) { LogError("kilit bekleme süresi doldu (" + message + ")"); return null; }
        try
        {
            for (int attempt = 0; attempt < 2; attempt++)
            {
                try
                {
                    int left = budget - (int)sw.ElapsedMilliseconds;
                    if (left <= 0) break;
                    if (ws == null || ws.State != WebSocketState.Open)
                    {
                        Drop();
                        ws = new ClientWebSocket();
                        ws.Options.Proxy = null; // yoksa WPAD proxy araması bağlantıyı saniyelerce geciktiriyor
                        if (!ws.ConnectAsync(new Uri("ws://127.0.0.1:6123"), CancellationToken.None).Wait(Math.Min(3000, left)))
                        { Drop(); LogError("bağlanılamadı"); continue; }
                    }
                    var bytes = Encoding.UTF8.GetBytes(message);
                    left = budget - (int)sw.ElapsedMilliseconds;
                    if (left <= 0 || !ws.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, CancellationToken.None).Wait(Math.Min(1500, left)))
                    { Drop(); LogError("gönderilemedi"); continue; }
                    while (true)
                    {
                        left = budget - (int)sw.ElapsedMilliseconds;
                        string text = left > 0 ? Receive(Math.Min(2000, left)) : null;
                        if (text == null) break;
                        var obj = json.DeserializeObject(text) as Dictionary<string, object>;
                        if (obj == null) continue;
                        object type, cm;
                        obj.TryGetValue("messageType", out type);
                        obj.TryGetValue("clientMessage", out cm);
                        if ((type as string) == "client_response" && (cm as string) == message) return obj;
                    }
                }
                catch (Exception ex) { LogError(ex.GetBaseException().Message); Drop(); }
            }
            return null;
        }
        finally { Monitor.Exit(gate); }
    }

    // Yarım kalan okuma / bağlantı bırakılmaz: ClientWebSocket aynı anda tek okuma kabul eder, süresi dolan okuma bekler
    // durumda kalırsa sonraki her istek hata veriyordu
    void Drop()
    {
        if (ws == null) return;
        try { ws.Abort(); ws.Dispose(); } catch { }
        ws = null;
    }

    string Receive(int timeoutMs)
    {
        var buf = new byte[1 << 16];
        var sb = new StringBuilder();
        while (true)
        {
            var t = ws.ReceiveAsync(new ArraySegment<byte>(buf), CancellationToken.None);
            if (!t.Wait(timeoutMs)) { Drop(); return null; }
            var r = t.Result;
            if (r.MessageType == WebSocketMessageType.Close) { Drop(); return null; }
            sb.Append(Encoding.UTF8.GetString(buf, 0, r.Count));
            if (r.EndOfMessage) return sb.ToString();
        }
    }

    // Pencere yöneticisi kapalıyken her istek bir hata satırı yazıyordu; satırlar sayılar (port, süre, kod) yüzünden
    // birebir aynı olmadığı için günlük birleştirmesi tutmuyordu. Rakamlar atılmış metin anahtardır: aynı anahtar
    // dakikada bir yazılır, arada kaç kez tekrarlandığı eklenir.
    static readonly Dictionary<string, int[]> errorSeen = new Dictionary<string, int[]>();
    static void LogError(string text)
    {
        string key = System.Text.RegularExpressions.Regex.Replace(text, @"\d+", "#");
        int now = Environment.TickCount, skipped;
        lock (errorSeen)
        {
            int[] e;
            if (errorSeen.TryGetValue(key, out e) && now - e[0] < 60000) { e[1]++; return; }
            skipped = e != null ? e[1] : 0;
            if (errorSeen.Count > 64) errorSeen.Clear();
            errorSeen[key] = new[] { now, 0 };
        }
        Slider.Log("ipc error: " + text + (skipped > 0 ? " (arada " + skipped + " kez daha)" : ""));
    }

    public List<Dictionary<string, object>> Monitors()
    {
        var res = Send("query monitors");
        var list = new List<Dictionary<string, object>>();
        if (res == null) return list;
        var data = res["data"] as Dictionary<string, object>;
        if (data == null) return list;
        foreach (var m in (object[])data["monitors"]) list.Add((Dictionary<string, object>)m);
        return list;
    }

    public List<Dictionary<string, object>> Workspaces()
    {
        var res = Send("query workspaces");
        var list = new List<Dictionary<string, object>>();
        if (res == null || !J.Bool(res, "success")) return list;
        object data, items;
        if (!res.TryGetValue("data", out data) || !(data is Dictionary<string, object>)) return list;
        if (!((Dictionary<string, object>)data).TryGetValue("workspaces", out items) || !(items is object[])) return list;
        foreach (var item in (object[])items) if (item is Dictionary<string, object>) list.Add((Dictionary<string, object>)item);
        return list;
    }

    public bool TryCommand(string cmd, out string error)
    {
        var response = Send("command " + cmd);
        if (response == null) { error = "Pencere yöneticisine ulaşılamadı."; return false; }
        if (!J.Bool(response, "success")) { error = J.Str(response, "error"); return false; }
        error = null;
        return true;
    }

    public void Command(string cmd) { Send("command " + cmd); }

    // tiling IPC'ye yanıt veriyor mu (pencere yöneticisi nöbetçisi için)
    public bool Ping() { return Send("query monitors") != null; }
}

static class J
{
    public static object[] Children(Dictionary<string, object> n)
    {
        object c; return n.TryGetValue("children", out c) && c is object[] ? (object[])c : new object[0];
    }
    public static string Str(Dictionary<string, object> n, string k) { object v; return n.TryGetValue(k, out v) && v != null ? v.ToString() : ""; }
    public static bool Bool(Dictionary<string, object> n, string k) { object v; return n.TryGetValue(k, out v) && v is bool && (bool)v; }
    public static int Int(Dictionary<string, object> n, string k) { object v; return n.TryGetValue(k, out v) && v != null ? Convert.ToInt32(v) : 0; }

    public static void WindowNodes(Dictionary<string, object> node, List<Dictionary<string, object>> into)
    {
        foreach (Dictionary<string, object> c in Children(node))
        {
            if (Str(c, "type") == "window") into.Add(c);
            else WindowNodes(c, into);
        }
    }

    public static void Windows(Dictionary<string, object> node, List<IntPtr> into)
    {
        foreach (Dictionary<string, object> c in Children(node))
        {
            if (Str(c, "type") == "window") into.Add(new IntPtr(Convert.ToInt64(c["handle"])));
            else Windows(c, into);
        }
    }
}

// ---------------- Slide overlay ----------------
// Kenarlık katmanı: animasyon katmanının hemen üstünde, yüzeysiz (WS_EX_NOREDIRECTIONBITMAP, tamamen şeffaf) pencere.
// Kenarlık önizlemeleri burada ÖNCEDEN kaydedilip havuzda bekler; dondurma anında kayıt yapılmaz, yalnızca yerleştirilir.
// (Kenarlıklar pencere önizlemelerinin üstünde olmalı; aynı katmanda bu, her dondurmada pencerelerden sonra yeniden kayıt
// demekti. Pencere kapanırken DWM meşgulken bu kayıt 15-35 ms sürüyor, katman tiling pencereleri kaydırdıktan sonra
// açılıyordu.)
class RingLayer : Form
{
    [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr h, int attr, ref int value, int size);
    public IntPtr Hwnd;
    int ownerThread;
    Rectangle placed;
    bool ready;
    public readonly Stack<IntPtr[]> PoolA = new Stack<IntPtr[]>(), PoolI = new Stack<IntPtr[]>();
    public int RetainA = 2, RetainI = 2;

    public RingLayer()
    {
        FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true; StartPosition = FormStartPosition.Manual;
        Text = "lunge-slide-rings";
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override CreateParams CreateParams
    {
        get { var cp = base.CreateParams; cp.ExStyle |= Native.WS_EX_TOOLWINDOW | Native.WS_EX_NOACTIVATE | Native.WS_EX_TOPMOST | 0x00200000; return cp; } // NOREDIRECTIONBITMAP
    }
    protected override void OnPaintBackground(PaintEventArgs e) { } // yüzey yok
    public void Prepare(Rectangle r)
    {
        if (Hwnd == IntPtr.Zero) { CreateControl(); Hwnd = Handle; ownerThread = Thread.CurrentThread.ManagedThreadId; }
        if (ready && r == placed) return;
        Cloak(true);
        Bounds = r;
        if (!ready) { Show(); ready = true; }
        placed = r;
    }
    public void Reveal()
    {
        bool own = Thread.CurrentThread.ManagedThreadId == ownerThread;
        Native.SetWindowPos(Hwnd, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010 | 0x0400 | (own ? 0u : 0x4000u)); // animasyon katmanının üstüne
        Cloak(false);
    }
    public void Conceal() { Cloak(true); }
    void Cloak(bool on) { int v = on ? 1 : 0; DwmSetWindowAttribute(Hwnd, 13, ref v, 4); }
}

class Overlay : Form
{
    [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr h, int attr, ref int value, int size);
    public IntPtr Hwnd;
    public readonly RingLayer Rings = new RingLayer();
    int ownerThread;
    Rectangle placed;
    bool ready;

    public Overlay()
    {
        FormBorderStyle = FormBorderStyle.None;
        ShowInTaskbar = false;
        TopMost = true;
        BackColor = Color.Black;
        StartPosition = FormStartPosition.Manual;
        Text = "lunge-slide";
    }

    // Katman hep "gösterilmiş" durur ama DWM ile gizlenir (DWMWA_CLOAK); açıp kapatmak yalnızca bu bayrak. Her animasyonda
    // Show() tam ekran pencereyi yeniden gösterip boyuyordu (~25-35 ms) ve katman açıkken kaydedilen her önizleme ~0,7 ms
    // tutuyordu; gizliyken kayıt ~0,03 ms, açmak + DWM karesi ~3,5 ms. Gizli pencere tıklama almaz (sanal masaüstündeki
    // pencereler gibi; WindowFromPoint altındaki pencereyi bulur). Boyama yalnızca katman başka monitöre geçince.
    public void Prepare(Rectangle r)
    {
        if (Hwnd == IntPtr.Zero) { CreateControl(); Hwnd = Handle; ownerThread = Thread.CurrentThread.ManagedThreadId; }
        Rings.Prepare(r);
        if (ready && r == placed) return;
        Cloak(true);
        Bounds = r;
        if (!ready) { Show(); ready = true; }
        Refresh();
        placed = r;
    }
    public void Reveal()
    {
        // Sonradan açılan en üstteki pencerelerin (kenarlık motoru, shell) üstüne çık. Sahibi olmayan thread'den eşzamansız:
        // UI thread'i başka bir animasyondaysa beklemesin.
        bool own = Thread.CurrentThread.ManagedThreadId == ownerThread;
        Native.SetWindowPos(Hwnd, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010 | 0x0400 | (own ? 0u : 0x4000u)); // TOPMOST, NOSIZE|NOMOVE|NOACTIVATE|NOOWNERZORDER
        Cloak(false);
        Rings.Reveal(); // kenarlık katmanı en üstte
    }
    public void Conceal() { Rings.Conceal(); Cloak(true); }
    void Cloak(bool on) { int v = on ? 1 : 0; DwmSetWindowAttribute(Hwnd, 13, ref v, 4); } // DWMWA_CLOAK
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= Native.WS_EX_TOOLWINDOW | Native.WS_EX_NOACTIVATE | Native.WS_EX_TOPMOST;
            return cp;
        }
    }
}

// Animasyon katmanı gerçek pencereleri örttüğü için kenarlık motoru'ın kenarlığı animasyon boyunca görünmüyordu; odaklı
// pencerenin kenarlığını katmanın içinde biz çiziyoruz. Önceden bu, her karede yeniden boyutlanan ayrı bir pencereydi
// (SetWindowPos + SetWindowRgn): büyük pencerede kare başına 10-50 ms (15-30 fps) tutuyordu ve animasyon döngüsü boyamaya
// izin vermediği için yeni açılan alan beyaz/siyah yanıp sönüyordu. Şimdi kenarlık, ekran dışında duran küçük bir şablon
// pencereden (kenar yumuşatmalı halka resmi) alınan DWM önizlemeleriyle çizilir: 4 köşe sabit boyutta, 4 kenar tek
// piksellik şeritten gerilir (9 dilim). Kare başına yalnızca önizleme dikdörtgenleri güncellenir (~0,01 ms/çağrı) ve
// kenarlık pencerelerle AYNI DWM karesinde hareket eder.
static class BorderStyle
{
    public static Color Active = Color.FromArgb(0xcc, 0xb6, 0x9d, 0xf8), Inactive = Color.FromArgb(0x99, 0x3a, 0x3a, 0x40);
    public static int Width = 2, Radius = 14;
    static BorderStyle() { Load(); }

    // config.yaml değişince yeniden okunur (ayarlar penceresinden odak rengi ya da elle düzenleme; bkz. ConfigWatch)
    public static void Load()
    {
        try
        {
            // Kenarlıkları pencere yöneticisi çiziyor: ayarları config.yaml'daki borders: bölümünde
            string cfg = System.IO.File.ReadAllText(Paths.ConfigFile);
            Active = Parse(cfg, "active_color", Active);
            Inactive = Parse(cfg, "inactive_color", Inactive);
            var m = System.Text.RegularExpressions.Regex.Match(cfg, @"border_width:\s*(\d+)");
            if (m.Success) Width = Math.Max(1, int.Parse(m.Groups[1].Value));
            m = System.Text.RegularExpressions.Regex.Match(cfg, @"border_radius:\s*(\d+)");
            if (m.Success) Radius = int.Parse(m.Groups[1].Value);
        }
        catch { }
    }
    static Color Parse(string cfg, string key, Color def)
    {
        var m = System.Text.RegularExpressions.Regex.Match(cfg, @"(?<![A-Za-z_])" + key + @":\s*[""']?#([0-9a-fA-F]{6})([0-9a-fA-F]{2})?");
        if (!m.Success) return def;
        int rgb = Convert.ToInt32(m.Groups[1].Value, 16);
        int al = m.Groups[2].Success ? Convert.ToInt32(m.Groups[2].Value, 16) : 255;
        return Color.FromArgb(al, (rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    }
}

// ---------------- Hareket eğrileri ve süreleri (Hyprland'deki bezier / animation satırları) ----------------
// config.yaml'daki üst düzey animations: bölümünden okunur, dosya değişince yeniden okunur. Bölüm yoksa ya da bir değer
// hatalıysa o değer için varsayılan kalır (bugüne dek sabit olan değerler; eğri adları ii'nin Hyprland ayarındaki gibi).
//   animations:
//     beziers:
//       benim_egrim: [0.2, 0.9, 0.1, 1]
//     workspaces: { duration: 520, curve: menu_decel }
static class Anims
{
    public sealed class Curve
    {
        public readonly string Name; public readonly double X1, Y1, X2, Y2;
        public Curve(string name, double x1, double y1, double x2, double y2)
        {
            // x değerleri [0,1] dışında eğri geriye döner (zaman ters akar): Hyprland gibi sınırla; y serbest (taşma)
            Name = name; X1 = Math.Max(0, Math.Min(1, x1)); Y1 = y1; X2 = Math.Max(0, Math.Min(1, x2)); Y2 = y2;
        }
        public double At(double t) { return t <= 0 ? 0 : t >= 1 ? 1 : Slider.Bezier(X1, Y1, X2, Y2, t); }
    }
    public sealed class Spec
    {
        public readonly int Ms; public readonly Curve Curve; public readonly int Popin;
        public Spec(int ms, Curve curve, int popin = 0) { Ms = ms; Curve = curve; Popin = popin; }
    }

    static Dictionary<string, Curve> Defaults()
    {
        var d = new Dictionary<string, Curve>(StringComparer.OrdinalIgnoreCase);
        Action<string, double, double, double, double> add = (n, a, b, c, e) => d[n] = new Curve(n, a, b, c, e);
        add("expressiveFastSpatial", 0.42, 1.67, 0.21, 0.90);
        add("expressiveSlowSpatial", 0.39, 1.29, 0.35, 0.98);
        add("expressiveDefaultSpatial", 0.38, 1.21, 0.22, 1.00);
        add("emphasizedDecel", 0.05, 0.7, 0.1, 1);
        add("emphasizedAccel", 0.3, 0, 0.8, 0.15);
        add("standardDecel", 0, 0, 0, 1);
        add("menu_decel", 0.1, 1, 0, 1);
        add("menu_accel", 0.52, 0.03, 0.72, 0.08);
        add("linear", 0, 0, 1, 1);
        return d;
    }

    // Kaydırma (Super+sayı, Super+Ctrl+←/→), taşı+takip et (Super+Ctrl+Shift+←/→: pencereyi yanında götürür),
    // pencerelerin yer değiştirmesi (açma / kapama / taşıma), yeni pencerenin büyüyerek belirmesi (popin: başlangıç boyu %)
    public static volatile Spec Workspaces, Carry, WindowsMove, WindowsIn;
    static Anims() { Apply(Defaults(), null); }

    static void Apply(Dictionary<string, Curve> curves, Dictionary<string, string> specs)
    {
        Func<string, int, string, int, Spec> make = (key, ms, curve, popin) =>
        {
            string body;
            if (specs != null && specs.TryGetValue(key, out body))
            {
                var d = System.Text.RegularExpressions.Regex.Match(body, @"(?:^|[,\s])duration\s*:\s*(\d+)");
                if (d.Success) ms = Math.Min(5000, int.Parse(d.Groups[1].Value));
                var c = System.Text.RegularExpressions.Regex.Match(body, @"(?:^|[,\s])curve\s*:\s*[""']?([A-Za-z_][\w-]*)");
                if (c.Success)
                {
                    if (curves.ContainsKey(c.Groups[1].Value)) curve = c.Groups[1].Value;
                    else Slider.Log("hareketler: " + key + " için bilinmeyen eğri '" + c.Groups[1].Value + "', " + curve + " kullanılıyor");
                }
                var p = System.Text.RegularExpressions.Regex.Match(body, @"(?:^|[,\s])popin\s*:\s*(\d+)");
                if (p.Success) popin = Math.Max(10, Math.Min(100, int.Parse(p.Groups[1].Value)));
            }
            return new Spec(ms, curves[curve], popin);
        };
        Workspaces = make("workspaces", 520, "menu_decel", 0);
        Carry = make("workspacescarry", 340, "menu_decel", 0);
        WindowsMove = make("windowsmove", 300, "emphasizedDecel", 0);
        WindowsIn = make("windowsin", 300, "emphasizedDecel", 80);
    }

    public static void Load()
    {
        try
        {
            var curves = Defaults();
            var specs = new Dictionary<string, string>();
            string cfg = System.IO.File.Exists(Paths.ConfigFile) ? System.IO.File.ReadAllText(Paths.ConfigFile) : "";
            // Yalnızca üst düzey bölüm (kenarlıkların borders: altında kendi animations: anahtarı var)
            var blk = System.Text.RegularExpressions.Regex.Match(cfg, @"(?m)^animations:[ \t]*(?:#.*)?\r?\n((?:(?:[ \t]+[^\r\n]*|[ \t]*)(?:\r?\n|$))*)");
            if (blk.Success)
            {
                string b = blk.Groups[1].Value;
                var inv = System.Globalization.CultureInfo.InvariantCulture;
                const string num = @"\s*(-?\d+(?:\.\d+)?)\s*";
                foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(b, @"(?m)^[ \t]+([A-Za-z_][\w-]*)[ \t]*:[ \t]*\[" + num + "," + num + "," + num + "," + num + @"\]"))
                    curves[m.Groups[1].Value] = new Curve(m.Groups[1].Value, double.Parse(m.Groups[2].Value, inv), double.Parse(m.Groups[3].Value, inv), double.Parse(m.Groups[4].Value, inv), double.Parse(m.Groups[5].Value, inv));
                // workspaces_carry / workspacesCarry / windows-move: aynı ad
                foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(b, @"(?m)^[ \t]+([A-Za-z_][\w-]*)[ \t]*:[ \t]*\{([^}\r\n]*)\}"))
                    specs[m.Groups[1].Value.Replace("_", "").Replace("-", "").ToLowerInvariant()] = m.Groups[2].Value;
            }
            Apply(curves, specs);
            Slider.Log("hareketler: kayma " + Workspaces.Ms + " ms " + Workspaces.Curve.Name + ", taşı+takip " + Carry.Ms + " ms " + Carry.Curve.Name
                + ", yer değiştirme " + WindowsMove.Ms + " ms " + WindowsMove.Curve.Name + ", açılış " + WindowsIn.Ms + " ms " + WindowsIn.Curve.Name + " %" + WindowsIn.Popin
                + (blk.Success ? "" : " (varsayılan)"));
        }
        catch (Exception ex) { Slider.Log("hareketler: " + ex.Message + " (varsayılanlar kaldı)"); }
    }
}

class RingTemplate : Form
{
    [StructLayout(LayoutKind.Sequential)] struct PT { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct SZ { public int cx, cy; }
    [StructLayout(LayoutKind.Sequential)] struct BLEND { public byte Op, Flags, Alpha, Format; }
    [DllImport("user32.dll")] static extern bool UpdateLayeredWindow(IntPtr h, IntPtr hdcDst, ref PT pptDst, ref SZ psize, IntPtr hdcSrc, ref PT pptSrc, uint crKey, ref BLEND pblend, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);

    public readonly int Bw, C, S, M;
    public readonly IntPtr Hwnd;
    const int X = -20000, Y = -20000; // ekran dışı; DWM önizlemesi yine de çizer

    public RingTemplate(Color color, int bw, int radius)
    {
        FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; StartPosition = FormStartPosition.Manual;
        Text = "lunge-ring-src";
        Bw = bw; Radius = radius; C = radius + bw + 1; S = 2 * C + 9; M = S / 2;
        Bounds = new Rectangle(X, Y, S, S);
        CreateControl(); Hwnd = Handle;
        Show();
        Paint(color);
    }
    readonly int Radius;

    // Halkayı bu renkle yeniden çizer; şablondan alınan DWM önizlemeleri (animasyon kenarlıkları) anında yeni rengi alır
    public void Paint(Color color)
    {
        int bw = Bw, radius = Radius;
        using (var bmp = new Bitmap(S, S, System.Drawing.Imaging.PixelFormat.Format32bppArgb))
        {
            using (var g = Graphics.FromImage(bmp))
            {
                g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
                g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
                g.Clear(Color.Transparent);
                float o = bw / 2f;
                using (var pen = new Pen(color, bw))
                    using (var rp = new GraphicsPathHelper(new RectangleF(o, o, S - bw, S - bw), Math.Max(1f, radius - o))) g.DrawPath(pen, rp.Path);
            }
            IntPtr screenDc = GetDC(IntPtr.Zero), memDc = CreateCompatibleDC(screenDc), hbm = bmp.GetHbitmap(Color.FromArgb(0)), old = SelectObject(memDc, hbm);
            try
            {
                var dst = new PT { x = X, y = Y }; var sz = new SZ { cx = S, cy = S }; var src = new PT();
                var bl = new BLEND { Op = 0, Flags = 0, Alpha = 255, Format = 1 }; // AC_SRC_OVER, AC_SRC_ALPHA
                UpdateLayeredWindow(Hwnd, screenDc, ref dst, ref sz, memDc, ref src, 0, ref bl, 2); // ULW_ALPHA
            }
            finally { SelectObject(memDc, old); Native.DeleteObject(hbm); DeleteDC(memDc); ReleaseDC(IntPtr.Zero, screenDc); }
        }
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override CreateParams CreateParams
    {
        get { var cp = base.CreateParams; cp.ExStyle |= 0x80 | 0x08000000 | 0x00080000 | 0x20; return cp; } // TOOLWINDOW | NOACTIVATE | LAYERED | TRANSPARENT
    }

    // 9 dilim: 0-3 köşeler (sol üst, sağ üst, sol alt, sağ alt), 4-7 kenarlar (üst, alt, sol, sağ)
    public Native.RECT Slice(int i)
    {
        switch (i)
        {
            case 0: return R(0, 0, C, C);
            case 1: return R(S - C, 0, S, C);
            case 2: return R(0, S - C, C, S);
            case 3: return R(S - C, S - C, S, S);
            case 4: return R(M, 0, M + 1, C);
            case 5: return R(M, S - C, M + 1, S);
            case 6: return R(0, M, C, M + 1);
            default: return R(S - C, M, S, M + 1);
        }
    }
    public static Native.RECT R(int l, int t, int r, int b) { return new Native.RECT { Left = l, Top = t, Right = r, Bottom = b }; }
}

// Animasyon kare ölçümü: her kare "güncelleme" (önizleme çağrıları), "flush" (DwmFlush: DWM'in kareyi
// bitirmesini bekleme) ve "boşluk" (iki kare arası başka şey: thread'in kesilmesi, GC) olarak parçalanır; takılmanın
// bizde mi DWM'de mi olduğunu ayırmak için.
// Kare temposu her karede DwmFlush. Zamanlayıcıyla (2f49b4a'daki FramePacer) güncelleme DWM'in kareyi topladığı ana denk
// gelip kayıyordu: ekranda gösterilen kareler (tools/dev/frame-bench) akıcı %88 -> %81, atlanan kare iki katı, gecikme
// +1,7 ms; yük altında da aynı yön. Ölçüm burada değil ekranda: zamanlayıcıyla bu sayaç "zamanında" der, ekran takılır.
class FrameStats
{
    readonly Stopwatch sw = Stopwatch.StartNew();
    double t0, t1, lastEnd = -1, sumUpd, sumFlush, sumGap, maxTotal;
    string worst = "";
    public int Frames;
    // Kare aralığı dağılımı (yenileme periyoduna göre): zamanında / 1 vsync kaçırdı / 2+ kaçırdı. Ortalama aynı olsa da
    // karışık aralıklar (7-14-7-14 ms) göze takılma olarak görünür.
    readonly double period = RefreshPeriodMs();
    int onTime, miss1, miss2;
    public static double RefreshPeriodMs()
    {
        try
        {
            var t = new Native.DWM_TIMING_INFO(); t.cbSize = (uint)Marshal.SizeOf(typeof(Native.DWM_TIMING_INFO));
            if (Native.DwmGetCompositionTimingInfo(IntPtr.Zero, ref t) == 0 && t.rateRefresh.uiNumerator > 0)
                return 1000.0 * t.rateRefresh.uiDenominator / t.rateRefresh.uiNumerator;
        }
        catch { }
        return 1000.0 / 60;
    }
    readonly int gc0, gc1, gc2;

    public FrameStats()
    {
        gc0 = GC.CollectionCount(0); gc1 = GC.CollectionCount(1); gc2 = GC.CollectionCount(2);
    }
    public void Begin() { t0 = sw.Elapsed.TotalMilliseconds; }
    public void Updated() { t1 = sw.Elapsed.TotalMilliseconds; }
    public void Flushed()
    {
        double t2 = sw.Elapsed.TotalMilliseconds;
        double gap = lastEnd < 0 ? 0 : t0 - lastEnd, upd = t1 - t0, fl = t2 - t1;
        double total = lastEnd < 0 ? t2 - t0 : t2 - lastEnd;
        sumUpd += upd; sumFlush += fl; sumGap += gap;
        if (total > maxTotal) { maxTotal = total; worst = string.Format("güncelleme {0:0.0} + flush {1:0.0} + boşluk {2:0.0}", upd, fl, gap); }
        if (lastEnd >= 0) { if (total < period * 1.5) onTime++; else if (total < period * 2.5) miss1++; else miss2++; }
        lastEnd = t2; Frames++;
    }
    public string Report()
    {
        var sb = new StringBuilder();
        sb.AppendFormat("[{0} kare/{1:0} ms; en uzun {2:0.0} ms = {3}; toplam güncelleme {4:0} flush {5:0} boşluk {6:0} ms; GC {7}/{8}/{9}",
            Frames, sw.Elapsed.TotalMilliseconds, maxTotal, worst, sumUpd, sumFlush, sumGap,
            GC.CollectionCount(0) - gc0, GC.CollectionCount(1) - gc1, GC.CollectionCount(2) - gc2);
        sb.AppendFormat("; aralık {0:0.0} ms: zamanında {1} / 1 kaçık {2} / 2+ kaçık {3}", period, onTime, miss1, miss2);
        sb.Append("]");
        PerfGuard.Record(onTime, miss1 + miss2);
        return sb.ToString();
    }
}

// Animasyon zamanı, karenin ekranda görüneceği ana göre: DWM'in şimdiden sonraki ilk vsync'i. "Şimdi"ye göre hesaplanınca
// güncellemenin karenin neresinde yapıldığına göre değişen bir hata kalıyordu (geç kalan karede pencere eğrinin gerisinde
// görünüp sonra sıçrıyordu). Hyprland değerleri gerçek zamana göre ilerletir; burada ayrıca gösterim anı hedeflenir. İlk
// karenin gösterim anı 0'dır. DWM zaman bilgisi alınamazsa "şimdi" kullanılır.
class PresentClock
{
    static readonly double ToMs = 1000.0 / Stopwatch.Frequency;
    long baseQpc = -1;
    public double Ms()
    {
        long now = Stopwatch.GetTimestamp(), next = now;
        var t = new Native.DWM_TIMING_INFO(); t.cbSize = (uint)Marshal.SizeOf(typeof(Native.DWM_TIMING_INFO));
        if (Native.DwmGetCompositionTimingInfo(IntPtr.Zero, ref t) == 0 && t.qpcRefreshPeriod > 0 && t.qpcVBlank > 0)
        {
            long period = (long)t.qpcRefreshPeriod, vb = (long)t.qpcVBlank;
            next = vb + (Math.Max(0, now - vb) / period + 1) * period;
        }
        if (baseQpc < 0) baseQpc = next;
        return (next - baseQpc) * ToMs;
    }
}

class Slider
{
    [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point pt, uint flags);
    [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr hmon, int type, out uint dx, out uint dy);
    // shell'in bar'ı 40 CSS px: DPI ölçeği %125/%150 olan monitörde 50/60 fiziksel piksel. Katmanlar bar'ın altından
    // başlamalı; ölçek monitör başına değişebilir (2-3 monitörlü kurulumlar).
    static int BarPx(int cx, int cy)
    {
        try
        {
            IntPtr m = MonitorFromPoint(new Point(cx, cy), 2); // MONITOR_DEFAULTTONEAREST
            uint dx, dy;
            if (GetDpiForMonitor(m, 0, out dx, out dy) == 0 && dx > 0) return (int)Math.Round(BAR_H * dx / 96.0);
        }
        catch { }
        return BAR_H;
    }
    const int BAR_H = 40;             // ii baseBarHeight — bar sabit kalır, altı kayar
    // Süreler ve eğriler config.yaml'dan (Anims): kayma 520 ms menu_decel (Hyprland workspaces speed 7 ~700 ms, kuyruğu
    // kısaltıldı), taşı+takip 340 ms, pencere hareketi 300 ms emphasizedDecel, açılış popin %80
    const int GAP = 50;                // Hyprland general.gaps_workspaces = 50
    // Hyprland r+1 / r-1 (pencere yöneticisinin --next/--prev-workspace'iyle aynı kural): config sırasında bu monitörde
    // yaşayan ya da açılacak bir sonraki workspace; başka monitörde gösterilen ya da başka monitöre bağlı olanlar atlanır,
    // uçta başa sarılmaz (1'de sola basınca 30'a gitmiyordu artık). Yoksa null.
    static string AdjacentWorkspace(List<Dictionary<string, object>> mons, Dictionary<string, object> mon, string current, int direction)
    {
        try {
            List<WorkspaceConfigText.Entry> entries;
            string error;
            if (!WorkspaceConfigText.TryRead(System.IO.File.ReadAllText(Paths.ConfigFile), out entries, out error)) return null;
            int index = entries.FindIndex(entry => entry.Number.ToString() == current);
            if (index < 0) return null;
            for (int i = index + direction; i >= 0 && i < entries.Count; i += direction)
            {
                string name = entries[i].Number.ToString();
                if (OnMonitor(mons, mon, name, entries[i].Monitor)) return name;
            }
        } catch { }
        return null;
    }

    // Workspace bu monitörde mi (ya da açılınca burada mı açılır): açıksa bulunduğu monitör, kapalıysa bağlı olduğu
    // monitör (bind_to_monitor: monitör sırası; o monitör takılı değilse odaktaki monitörde açılır)
    static bool OnMonitor(List<Dictionary<string, object>> mons, Dictionary<string, object> mon, string name, int? bound)
    {
        foreach (var m in mons)
            foreach (Dictionary<string, object> w in J.Children(m))
                if (J.Str(w, "name") == name) return J.Str(m, "id") == J.Str(mon, "id");
        return bound == null || bound.Value < 0 || bound.Value >= mons.Count || J.Str(mons[bound.Value], "id") == J.Str(mon, "id");
    }

    readonly TilingClient tiling;
    // Her monitörün kendi katmanı hazır ve gizli bekler (Warm): tek katmanı başka monitöre taşımak yeniden boyutlama ve
    // boyama demekti (~15 ms). Listede olmayan bir dikdörtgen (monitör düzeni değişti) yedek katmanı taşıyarak kullanır.
    Overlay overlay = new Overlay();
    readonly Overlay spare;
    readonly Dictionary<Rectangle, Overlay> overlays = new Dictionary<Rectangle, Overlay>();
    void UseOverlay(Rectangle r)
    {
        Overlay o;
        lock (overlays) if (!overlays.TryGetValue(r, out o)) o = spare;
        overlay = o;
        o.Prepare(r);
        ovW = r.Width; ovH = r.Height;
    }
    int ovW, ovH, culledCount;
    public volatile bool Interrupt;
    // Animasyon sürerken başka işler (dwindle yön komutu) tiling'i meşgul etmesin. Başladığı an saklanır: bekçi
    // (StartWatchdog) takılı kalan bir animasyonu (katman ekranda donmuş) bulup kaldırır.
    static volatile bool animating;
    static int animSince, swipeTouched;
    public static bool Animating
    {
        get { return animating; }
        set { if (value && !animating) animSince = Environment.TickCount; animating = value; }
    }
    // Katmana kayıtlı pencere önizlemeleri: bir hata animasyonu yarıda keserse bekçi hepsini bırakabilsin
    static readonly HashSet<IntPtr> liveThumbs = new HashSet<IntPtr>();
    static void Unregister(IntPtr id)
    {
        lock (liveThumbs) liveThumbs.Remove(id);
        Native.DwmUnregisterThumbnail(id);
    }

    // En uzun animasyon ~1 sn (+ tiling'i en fazla 1,5 sn bekleme); 10 sn'dir süren (parmak kaydırmasında son
    // hareketten bu yana) bir animasyon takılmıştır: katman kaldırılır, önizlemeler bırakılır.
    const int STUCK_MS = 10000;
    public void StartWatchdog()
    {
        new System.Threading.Timer(_ =>
        {
            if (!Stuck()) return;
            try { Ui.BeginInvoke((Action)(() => { if (Stuck()) Recover("animasyon " + (Environment.TickCount - animSince) / 1000 + " sn'dir bitmedi"); })); } catch { }
        }, null, 2000, 2000);
    }
    bool Stuck()
    {
        if (!animating) return false;
        int since = swipe != null ? Math.Max(0, Environment.TickCount - swipeTouched) : Environment.TickCount - animSince;
        return since > STUCK_MS;
    }

    // Yarıda kalan animasyonun temizliği (UI thread'inde): bütün katmanlar gizlenir, kenarlık / sabit pencere takımları
    // döner, kayıtlı önizlemeler bırakılır
    public void Recover(string why)
    {
        Log("animasyon kurtarma: " + why);
        swipe = null;
        var all = new List<Overlay> { overlay, spare };
        lock (overlays) all.AddRange(overlays.Values);
        foreach (var o in all) { try { o.Conceal(); } catch { } }
        try { RingsClear(); } catch { }
        try { PinsClear(); } catch { }
        List<IntPtr> ids;
        lock (liveThumbs) { ids = new List<IntPtr>(liveThumbs); liveThumbs.Clear(); }
        foreach (var id in ids) Native.DwmUnregisterThumbnail(id);
        Animating = false;
    }
    // Hyprland'de art arda basışta animasyon akmaya devam eder: bir önceki geçişten bu yana geçen süre
    // tam süreden kısaysa yeni animasyonu o kadar kısalt (en az MIN_MS). Tek basış tam uzunlukta kalır.
    static long lastSlideStart, lastMoveStart;
    const int MIN_MS = 180;
    static int Adaptive(ref long last, int full)
    {
        long now = Environment.TickCount;
        long since = unchecked((int)(now - last)); // TickCount'un 49,7 günlük dönüşünde de doğru fark
        last = now;
        return since > 0 && since < full ? (int)Math.Max(MIN_MS, since) : full;
    }

    // Ana helper açılışta: katman hazır ve gizli beklesin, ilk animasyon da bekletmesin
    // Monitör düzeni değişince (takıldı / çıkarıldı / çözünürlük) artık olmayan dikdörtgenlerin katmanları kapatılır:
    // önceden 8'e kadar birikip gizli pencereleri ve kenarlık havuzlarını (her biri onlarca DWM önizlemesi) tutuyordu.
    public void Warm()
    {
        lock (overlays)
        {
            var want = new HashSet<Rectangle>();
            foreach (var sc in Screen.AllScreens)
            {
                var b = sc.Bounds;
                int barH = BarPx(b.X + b.Width / 2, b.Y + b.Height / 2);
                want.Add(new Rectangle(b.X, b.Y + barH, b.Width, b.Height - barH));
            }
            if (!Animating)
                foreach (var kv in new List<KeyValuePair<Rectangle, Overlay>>(overlays))
                {
                    var o = kv.Value;
                    if (want.Contains(kv.Key) || o == overlay) continue; // kullanılan katman sonraki seferde
                    overlays.Remove(kv.Key);
                    if (o == spare) continue; // yedek kalır (yeni dikdörtgenler onu taşır)
                    try { EmptyRingPools(o.Rings); o.Rings.Dispose(); o.Dispose(); } catch (Exception ex) { Log("katman kapatma: " + ex.Message); }
                }
            foreach (var r in want)
            {
                if (overlays.ContainsKey(r) || overlays.Count >= 8) continue;
                var o = overlays.Count == 0 && !overlays.ContainsValue(spare) ? spare : new Overlay();
                o.Prepare(r);
                FillRingPools(o.Rings);
                overlays[r] = o;
            }
        }
    }

    static RingTemplate ringSrc, ringSrcInactive;

    // Odak rengi değişti: animasyon kenarlıklarının şablonlarını yeniden çiz (UI thread'inde)
    public static void RepaintRings()
    {
        try
        {
            if (ringSrc != null) ringSrc.Paint(BorderStyle.Active);
            if (ringSrcInactive != null && BorderStyle.Inactive.A > 0) ringSrcInactive.Paint(BorderStyle.Inactive);
        }
        catch (Exception ex) { Log("halka rengi: " + ex.Message); }
    }

    public Slider(TilingClient g)
    {
        tiling = g; spare = overlay;
        if (ringSrc == null)
        {
            try
            {
                ringSrc = new RingTemplate(BorderStyle.Active, BorderStyle.Width, BorderStyle.Radius);
                if (BorderStyle.Inactive.A > 0) ringSrcInactive = new RingTemplate(BorderStyle.Inactive, BorderStyle.Width, BorderStyle.Radius);
            }
            catch (Exception ex) { Log("ring: " + ex.Message); }
        }
    }

    // Kenarlıklar (kenarlık motoru'ınkiler katmanın altında kalır): odaklı pencereye etkin, diğerlerine pasif renkte, her biri
    // şablondan 8 DWM önizlemesi. Kenarlık katmanındaki havuzdan alınır (önceden, katman gizliyken kaydedilmiş); havuz
    // boşsa o an kaydedilir. Animasyon bitince takımlar gizlenip havuza döner.
    readonly List<Thumb> ringed = new List<Thumb>();
    static int RingPoolTarget(bool active, int windows)
    {
        return active ? 2 : Math.Max(2, Math.Min(12, windows + 1));
    }
    static IntPtr[] RegisterRingSet(RingTemplate src, IntPtr dest)
    {
        if (src == null) return null;
        var ids = new IntPtr[8];
        for (int i = 0; i < 8; i++)
        {
            if (Native.DwmRegisterThumbnail(dest, src.Hwnd, out ids[i]) != 0)
            {
                for (int j = 0; j < i; j++) Native.DwmUnregisterThumbnail(ids[j]);
                return null;
            }
            var p = new Native.DWM_THUMBNAIL_PROPERTIES
            {
                dwFlags = Native.DWM_TNP_RECTSOURCE | Native.DWM_TNP_VISIBLE | Native.DWM_TNP_OPACITY | Native.DWM_TNP_SOURCECLIENTAREAONLY,
                rcSource = src.Slice(i), fVisible = false, opacity = 255, fSourceClientAreaOnly = false
            };
            Native.DwmUpdateThumbnailProperties(ids[i], ref p);
        }
        return ids;
    }
    static IntPtr[] TakeRingSet(RingLayer layer, bool active)
    {
        var pool = active ? layer.PoolA : layer.PoolI;
        lock (pool) if (pool.Count > 0) return pool.Pop();
        return RegisterRingSet(active ? ringSrc : ringSrcInactive, layer.Hwnd);
    }
    static void ReturnRingSet(RingLayer layer, IntPtr[] ids, bool active)
    {
        if (ids == null || layer == null) return;
        HideRingSet(ids);
        var pool = active ? layer.PoolA : layer.PoolI;
        lock (pool) pool.Push(ids);
    }
    // Havuzu doldur (katman gizliyken: kayıt ucuz)
    static void FillRingPools(RingLayer layer, int incomingReserve = 0)
    {
        if (ringSrc == null || layer.Hwnd == IntPtr.Zero) return;
        for (int k = 0; k < 2; k++)
        {
            bool active = k == 0;
            var pool = active ? layer.PoolA : layer.PoolI;
            int want = active ? layer.RetainA : Math.Max(layer.RetainI, incomingReserve);
            if (!active && ringSrcInactive == null) continue;
            while (true)
            {
                lock (pool) if (pool.Count >= want) break;
                var ids = RegisterRingSet(active ? ringSrc : ringSrcInactive, layer.Hwnd);
                if (ids == null) break;
                lock (pool) pool.Push(ids);
            }
        }
    }
    void RingAdd(Thumb t, bool active)
    {
        if (t == null || !t.IsWin || ringed.Contains(t) || ringSrc == null) return;
        t.Layer = overlay.Rings;
        // Pasif takım herkese, etkin takım yalnızca odaklıya (odak değişirse RingsFocus o an havuzdan alır)
        if (active) t.RingA = TakeRingSet(t.Layer, true);
        if (ringSrcInactive != null) t.RingI = TakeRingSet(t.Layer, false);
        t.RingActive = active;
        ringed.Add(t);
    }
    void RingsAttach(IEnumerable<Thumb> ts, IntPtr focused)
    {
        RingsClear();
        if (TestNoRings()) return;
        var windows = new List<Thumb>();
        foreach (var t in ts) if (t != null && t.IsWin) windows.Add(t);
        overlay.Rings.RetainI = RingPoolTarget(false, windows.Count);
        overlay.Rings.RetainA = RingPoolTarget(true, 1);
        TrimRingPools(overlay.Rings);
        FillRingPools(overlay.Rings, windows.Count + 1); // includes an arriving window before Reveal; retained pool stays capped
        foreach (var t in windows) RingAdd(t, t.Src == focused);
    }
    // Yalnızca ölçüm için (A/B): %LOCALAPPDATA%\LogicalLunge\state\test-no-rings varken animasyonlarda kenarlık halkası yok.
    // Kaydırma takılmasının halkaların (pencere başına 8 önizleme) mı pencere önizlemelerinin mi olduğunu ayırmak için.
    static bool TestNoRings()
    {
        try { return System.IO.File.Exists(Paths.State(@"test-no-rings")); }
        catch { return false; }
    }
    // Odak değişti: etkin/pasif takımı değiştir (eskisi gizlenir, yenisi bir sonraki RingPlace'te yerleşir)
    void RingsFocus(IntPtr focused)
    {
        // Release old focus sets first, so a gesture never allocates a new set on the visible layer.
        foreach (var t in ringed)
            if (t.Src != focused && t.RingA != null) { ReturnRingSet(t.Layer, t.RingA, true); t.RingA = null; }
        foreach (var t in ringed)
        {
            bool a = t.Src == focused;
            if (a == t.RingActive) continue;
            HideRingSet(t.RingActive ? t.RingA : t.RingI);
            if (a && t.RingA == null) t.RingA = TakeRingSet(t.Layer, true);
            t.RingActive = a;
        }
    }
    static void HideRingSet(IntPtr[] ids)
    {
        if (ids == null) return;
        var p = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_VISIBLE, fVisible = false };
        foreach (var id in ids) Native.DwmUpdateThumbnailProperties(id, ref p);
    }
    // winRect: pencere dikdörtgeni (katman koordinatı). Halka görünen çerçevenin kenarına ortalanır (kenarlık motoru gibi).
    static void RingPlace(Thumb t, Native.RECT winRect, byte opacity)
    {
        if (t == null) return;
        var ids = t.RingActive ? t.RingA : t.RingI;
        var src = t.RingActive ? ringSrc : ringSrcInactive;
        if (ids == null || src == null) return;
        var fr = Deflate(winRect, t.FrameIns);
        int bl = src.Bw / 2, br = src.Bw - bl, c = src.C;
        int x0 = fr.Left - bl, y0 = fr.Top - bl, x1 = fr.Right + br, y1 = fr.Bottom + br;
        bool vis = opacity > 0 && x1 - x0 > 2 * c + 1 && y1 - y0 > 2 * c + 1;
        for (int i = 0; i < 8; i++)
        {
            Native.RECT d;
            switch (i)
            {
                case 0: d = RingTemplate.R(x0, y0, x0 + c, y0 + c); break;
                case 1: d = RingTemplate.R(x1 - c, y0, x1, y0 + c); break;
                case 2: d = RingTemplate.R(x0, y1 - c, x0 + c, y1); break;
                case 3: d = RingTemplate.R(x1 - c, y1 - c, x1, y1); break;
                case 4: d = RingTemplate.R(x0 + c, y0, x1 - c, y0 + c); break;
                case 5: d = RingTemplate.R(x0 + c, y1 - c, x1 - c, y1); break;
                case 6: d = RingTemplate.R(x0, y0 + c, x0 + c, y1 - c); break;
                default: d = RingTemplate.R(x1 - c, y0 + c, x1, y1 - c); break;
            }
            var p = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_VISIBLE | Native.DWM_TNP_OPACITY, rcDestination = d, fVisible = vis, opacity = opacity };
            Native.DwmUpdateThumbnailProperties(ids[i], ref p);
        }
    }
    void RingsClear()
    {
        foreach (var t in ringed)
        {
            ReturnRingSet(t.Layer, t.RingA, true);
            ReturnRingSet(t.Layer, t.RingI, false);
            t.RingA = t.RingI = null;
        }
        ringed.Clear();
        TrimRingPools(overlay.Rings);
        FillRingPools(overlay.Rings); // sonraki animasyon için (katman gizliyken)
    }

    // Kalabalık bir workspace'te havuz o an kaydedilen takımlarla büyür (pencere başına 8 önizleme) ve geri dönen
    // her takım havuzda kalırdı: animasyondan sonra hazır bekleyen sayıya indirilir
    static void TrimRingPools(RingLayer layer)
    {
        if (layer == null) return;
        foreach (var pool in new[] { layer.PoolA, layer.PoolI })
        {
            int want = pool == layer.PoolA ? layer.RetainA : layer.RetainI;
            var extra = new List<IntPtr[]>();
            lock (pool) while (pool.Count > want) extra.Add(pool.Pop());
            foreach (var ids in extra) foreach (var id in ids) Native.DwmUnregisterThumbnail(id);
        }
    }
    static void EmptyRingPools(RingLayer layer)
    {
        foreach (var pool in new[] { layer.PoolA, layer.PoolI })
        {
            var all = new List<IntPtr[]>();
            lock (pool) while (pool.Count > 0) all.Add(pool.Pop());
            foreach (var ids in all) foreach (var id in ids) Native.DwmUnregisterThumbnail(id);
        }
    }

    static Native.RECT Unshift(Native.RECT r, int ox, int oy)
    {
        return new Native.RECT { Left = r.Left + ox, Top = r.Top + oy, Right = r.Right + ox, Bottom = r.Bottom + oy };
    }

    static IntPtr FocusedTop() { return Native.GetAncestor(Native.GetForegroundWindow(), 2); }

    // Match the WM's descendant focus order before its asynchronous foreground change.
    // Layout order can differ from the last focused window, including inside splits.
    static Dictionary<string, object> WorkspaceFocusNode(Dictionary<string, object> node)
    {
        if (node == null) return null;
        if (J.Str(node, "type") == "window") return node;
        object order;
        var children = J.Children(node);
        var ids = node.TryGetValue("childFocusOrder", out order) ? order as object[] : null;
        if (ids != null)
            foreach (var id in ids)
                foreach (Dictionary<string, object> child in children)
                    if (id != null && J.Str(child, "id") == id.ToString())
                    {
                        var focused = WorkspaceFocusNode(child);
                        if (focused != null) return focused;
                    }
        foreach (Dictionary<string, object> child in children)
        {
            var focused = WorkspaceFocusNode(child);
            if (focused != null) return focused;
        }
        return null;
    }

    static IntPtr WorkspaceFocusHandle(Dictionary<string, object> workspace)
    {
        var window = WorkspaceFocusNode(workspace);
        if (window == null) return IntPtr.Zero;
        object stateValue, handle;
        var state = window.TryGetValue("state", out stateValue) ? stateValue as Dictionary<string, object> : null;
        if (state != null && J.Str(state, "type") == "minimized") return IntPtr.Zero;
        return window.TryGetValue("handle", out handle) && handle != null ? new IntPtr(Convert.ToInt64(handle)) : IntPtr.Zero;
    }

    static IntPtr WorkspacePreviewFocus(double progress, IntPtr oldFocus, IntPtr prevFocus, IntPtr nextFocus,
        bool hasPrev, bool hasNext)
    {
        if (progress > 0 && hasNext) return nextFocus;
        if (progress < 0 && hasPrev) return prevFocus;
        return oldFocus;
    }

    // Passive desktop widgets are part of the background scene. Their live
    // previews belong above the wallpaper but below moving application windows;
    // putting them in PinsAttach's top layer would make them cover tiled apps.
    internal static bool DesktopWidgetEligible(string title, string cls, string process, bool visible, bool cloaked, bool topmost)
    {
        return visible && !cloaked && !topmost && title == Names.DesktopWidget &&
            (cls == "LungeNativeBar" || cls == "Tauri Window") && string.Equals(process, Names.Shell, StringComparison.OrdinalIgnoreCase);
    }
    internal static void AddDesktopThumbnails(List<Thumb> scene, List<KeyValuePair<IntPtr, Native.RECT>> sources,
        int ox, int oy, Func<IntPtr, Native.RECT, Thumb> register)
    {
        // EnumWindows enumerates front to back; register back to front.
        for (int i = sources.Count - 1; i >= 0; i--)
        {
            var t = register(sources[i].Key, Shift(sources[i].Value, ox, oy));
            if (t != null) scene.Add(t);
        }
    }
    void DesktopWidgetsAttach(List<Thumb> scene, Rectangle mon, int ox, int oy)
    {
        var sources = new List<KeyValuePair<IntPtr, Native.RECT>>();
        var title = new StringBuilder(64); var cls = new StringBuilder(64);
        Native.EnumWindows(delegate (IntPtr h, IntPtr unused)
        {
            if (!Native.IsWindowVisible(h) || Native.IsIconic(h)) return true;
            title.Length = 0; Native.GetWindowText(h, title, 64);
            if (title.ToString() != Names.DesktopWidget) return true;
            cls.Length = 0; Native.GetClassName(h, cls, 64);
            uint pid; Native.GetWindowThreadProcessId(h, out pid);
            int cloaked;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cloaked, 4) != 0) return true;
            if (!DesktopWidgetEligible(title.ToString(), cls.ToString(), ProcInfo.Name(pid), true, cloaked != 0,
                (Native.GetWindowLong(h, Native.GWL_EXSTYLE) & 0x8) != 0)) return true;
            Native.RECT r;
            if (Native.GetWindowRect(h, out r) && mon.IntersectsWith(Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom)))
                sources.Add(new KeyValuePair<IntPtr, Native.RECT>(h, r));
            return true;
        }, IntPtr.Zero);
        AddDesktopThumbnails(scene, sources, ox, oy, (h, dest) => Register(h, dest, null));
    }

    // Hyprland bezier "menu_decel" = (0.1, 1), (0, 1)
    // İlerleme 0..1; süre 0 ise (hareket kapalı) hemen son kare (0/0 sonsuz döngüye sokardı)
    static double Prog(double at, int ms) { return ms <= 0 ? 1.0 : Math.Min(1.0, at / ms); }

    internal static double Bezier(double x1, double y1, double x2, double y2, double t)
    {
        double lo = 0, hi = 1, u = t;
        for (int i = 0; i < 24; i++)
        {
            u = (lo + hi) / 2;
            double x = 3 * (1 - u) * (1 - u) * u * x1 + 3 * (1 - u) * u * u * x2 + u * u * u;
            if (x < t) lo = u; else hi = u;
        }
        return 3 * (1 - u) * (1 - u) * u * y1 + 3 * (1 - u) * u * u * y2 + u * u * u;
    }

    public class Thumb { public IntPtr Id; public Native.RECT Dest; public IntPtr Src; public int Cx, Cy; public Native.RECT Ins; public bool IsWin; public Native.RECT FrameIns; public IntPtr[] RingA, RingI; public bool RingActive; public RingLayer Layer; public bool Culled; }

    // Ekran klavyesi, sağ panel, bildirimler monitöre "yapışık": workspace kayarken animasyon
    // katmanının altında kalmasınlar, en üstte sabit dursunlar.
    static readonly string[] Pinned = { Names.Osk, Names.Sidebar, Names.Toast, Names.Update, Names.Settings };
    // PiP gibi her workspace'te sabit duran, en üstte tutulan ve tiling'in yönetmediği pencereler animasyon katmanının
    // altında kalıp geçiş boyunca kayboluyor, sonra "yapıştırılmış resim" gibi geri geliyordu. Canlı önizlemeleri kenarlık
    // katmanının en üstüne, kendi yerlerine konur: katman açıldığı karede görünürler, geçiş boyunca sabit kalırlar.
    readonly List<IntPtr> pinIds = new List<IntPtr>();
    static readonly Dictionary<uint, string> pinProcs = new Dictionary<uint, string>();
    static readonly HashSet<string> pinSkipProcs = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { Names.Core, Names.Shell, Names.Tiling, "explorer", "ShellExperienceHost", "StartMenuExperienceHost", "SearchApp", "SearchUI", "TextInputHost", "LockApp" };
    void PinsAttach(Rectangle monArea, int ox, int oy, IEnumerable<Thumb> animated)
    {
        PinsClear();
        var skip = new HashSet<IntPtr>();
        if (animated != null) foreach (var t in animated) if (t != null) skip.Add(t.Src);
        var found = new List<KeyValuePair<IntPtr, Native.RECT>>();
        var title = new StringBuilder(64);
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            if (skip.Contains(h) || !Native.IsWindowVisible(h) || Native.IsIconic(h)) return true;
            // Bizim yapışık pencerelerimiz (bildirim, güncelleme kartı, ekran klavyesi, sağ panel) de PiP gibi: yalnızca
            // RaisePinned ile öne alınınca katman açıldığı an altında kalıyor, kaydırmanın başında kaybolup geri geliyorlardı
            title.Length = 0; Native.GetWindowText(h, title, 64);
            string tt = title.ToString();
            // Bar'ın açılır menüleri (tepsi oku, takvim, ses...) bar penceresinin içinde: pencere menü kadar aşağı uzar ve
            // uzayan kısmı katmanın altında kalıp kaydırma boyunca kayboluyordu. Uzamışsa bar da yapışık.
            bool bar = tt == Names.Bar;
            bool ours = bar || Array.IndexOf(Pinned, tt) >= 0;
            if (!ours && (Native.GetWindowLong(h, Native.GWL_EXSTYLE) & 0x8) == 0) return true; // WS_EX_TOPMOST
            int cl;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0) return true;
            Native.RECT r; Native.GetWindowRect(h, out r);
            if (r.Right - r.Left < 40 || r.Bottom - r.Top < 40) return true; // boş bildirim penceresi (içerik yok) de burada elenir
            if (!monArea.IntersectsWith(Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom))) return true;
            if (bar && r.Bottom <= monArea.Top + 2) return true; // menü kapalı: bar katmanın üstünde, önizleme gerekmez
            if (ours) { found.Add(new KeyValuePair<IntPtr, Native.RECT>(h, r)); return true; }
            uint pid; Native.GetWindowThreadProcessId(h, out pid);
            string pn;
            lock (pinProcs)
            {
                if (!pinProcs.TryGetValue(pid, out pn))
                {
                    pn = ProcInfo.Name(pid);
                    if (pinProcs.Count > 500) pinProcs.Clear();
                    pinProcs[pid] = pn;
                }
            }
            if (pn.Length == 0 || pinSkipProcs.Contains(pn)) return true;
            found.Add(new KeyValuePair<IntPtr, Native.RECT>(h, r));
            return true;
        }, IntPtr.Zero);
        // EnumWindows üstten alta sıralar: alttakini önce kaydet ki üstteki en üstte kalsın
        for (int i = found.Count - 1; i >= 0; i--)
        {
            IntPtr id;
            if (Native.DwmRegisterThumbnail(overlay.Rings.Hwnd, found[i].Key, out id) != 0) continue;
            var r = found[i].Value;
            var pr = new Native.DWM_THUMBNAIL_PROPERTIES
            {
                dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_VISIBLE | Native.DWM_TNP_OPACITY | Native.DWM_TNP_SOURCECLIENTAREAONLY,
                rcDestination = new Native.RECT { Left = r.Left - ox, Top = r.Top - oy, Right = r.Right - ox, Bottom = r.Bottom - oy },
                fVisible = true, opacity = 255, fSourceClientAreaOnly = false
            };
            Native.DwmUpdateThumbnailProperties(id, ref pr);
            pinIds.Add(id);
        }
    }
    void PinsClear()
    {
        foreach (var id in pinIds) Native.DwmUnregisterThumbnail(id);
        pinIds.Clear();
    }

    static void RaisePinned()
    {
        foreach (var title in Pinned)
        {
            IntPtr h = Native.FindWindow(null, title);
            if (h != IntPtr.Zero && Native.IsWindowVisible(h))
                Native.SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010 | 0x4000); // TOPMOST, NOMOVE|NOSIZE|NOACTIVATE|ASYNCWINDOWPOS
        }
    }

    // Duvar kağıdı penceresi önbellekte: aramak (EnumWindows) pencere açılıp kapanırken 20-40 ms sürebiliyordu. Önbellek 5 sn'den
    // eskiyse arka planda tazelenir; pencere geçersizse (explorer yeniden başladı) hemen aranır.
    static IntPtr wallCache;
    static int wallAt, wallRefreshing;
    static IntPtr WallpaperSource(out Native.RECT srcRect)
    {
        IntPtr c = wallCache;
        if (c != IntPtr.Zero && Native.IsWindow(c) && Native.IsWindowVisible(c))
        {
            if (Environment.TickCount - wallAt > 5000 && Interlocked.Exchange(ref wallRefreshing, 1) == 0)
                ThreadPool.QueueUserWorkItem(_ => { try { Native.RECT r; FindWallpaper(out r); } finally { wallRefreshing = 0; } });
            Native.GetWindowRect(c, out srcRect);
            return c;
        }
        return FindWallpaper(out srcRect);
    }
    static IntPtr FindWallpaper(out Native.RECT srcRect)
    {
        // Lively vb. canlı duvar kağıtları WorkerW'de; yoksa Progman duvar kağıdını çizer.
        IntPtr progman = Native.FindWindow("Progman", null);
        IntPtr wall = IntPtr.Zero;
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            var sb = new StringBuilder(64);
            Native.GetClassName(h, sb, 64);
            if (sb.ToString() == "WorkerW" && Native.IsWindowVisible(h) &&
                Native.FindWindowEx(h, IntPtr.Zero, "SHELLDLL_DefView", null) == IntPtr.Zero)
            { wall = h; return false; }
            return true;
        }, IntPtr.Zero);
        if (wall == IntPtr.Zero) wall = progman;
        Native.GetWindowRect(wall, out srcRect);
        wallCache = wall; wallAt = Environment.TickCount;
        return wall;
    }

    Thumb Register(IntPtr src, Native.RECT dest, Native.RECT? source)
    {
        IntPtr id;
        if (Native.DwmRegisterThumbnail(overlay.Hwnd, src, out id) != 0) return null;
        lock (liveThumbs) liveThumbs.Add(id);
        var t = new Thumb { Id = id, Dest = dest, Src = src };
        var p = new Native.DWM_THUMBNAIL_PROPERTIES
        {
            dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_VISIBLE | Native.DWM_TNP_OPACITY | Native.DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination = dest, opacity = 255, fVisible = true, fSourceClientAreaOnly = false
        };
        if (source.HasValue) { p.dwFlags |= Native.DWM_TNP_RECTSOURCE; p.rcSource = source.Value; }
        Native.DwmUpdateThumbnailProperties(id, ref p);
        return t;
    }

    // Pencere önizlemesini ölçeklemeden yerleştir: hedef boyut = DWM'nin bildirdiği kaynak boyutu.
    // (Hedefi pencere dikdörtgenine germek, kaynak farklı boyuttaysa geçişin başında/sonunda
    // pencerelerin hafifçe küçülüp büyümesine yol açıyordu.)
    // Pencerenin ekranda görünen hali (DWM kaynağının tamamı) hangi dikdörtgene çizilmeli: kaynak boyutu
    // pencere, çerçeve ya da bölge kutusuyla eşleşir. Animasyonun başı ve sonu hep buradan hesaplanır ki
    // katman kalkınca pencere "oturmasın".
    // Animasyon dikdörtgenleri pencere dikdörtgenidir (GetWindowRect). DWM önizlemesi gerçek pencerenin ekranda
    // gösterdiğinin aynısını (saydam gölge kenarları dahil) gösterir; başlangıç ve bitiş aynı türden olunca
    // boşluklar sabit kalır. "Görünen çerçeve" her uygulamada güvenilir değil: WezTerm onun dışına da çiziyor.
    public static Native.RECT WinRect(IntPtr h)
    {
        Native.RECT r;
        Native.GetWindowRect(h, out r);
        return r;
    }

    // Görünen çerçeve (kenarlık motoru kenarlığı buna çizilir).
    public static Native.RECT FrameRect(IntPtr h)
    {
        Native.RECT fr;
        if (Native.DwmGetWindowAttribute(h, Native.DWMWA_EXTENDED_FRAME_BOUNDS, out fr, Marshal.SizeOf(typeof(Native.RECT))) != 0)
            Native.GetWindowRect(h, out fr);
        return fr;
    }

    static Native.RECT VisualDest(IntPtr h, IntPtr thumb, int ox, int oy)
    {
        return Shift(WinRect(h), ox, oy);
    }

    // DWM kaynağı pencere dikdörtgeninin neresini kaplıyor: pencerenin tamamı, yalnızca çerçeve ya da
    // SetWindowRgn bölgesinin kutusu (yuvarlatılmış pencereler). Dönen değer pencere kenarlarından içe payladır.
    static Native.RECT SourceInsets(IntPtr h, Native.SIZE src)
    {
        Native.RECT wr, box;
        Native.GetWindowRect(h, out wr);
        int ww = wr.Right - wr.Left, wh = wr.Bottom - wr.Top;
        if (src.cx == ww && src.cy == wh) return new Native.RECT();
        var fr = FrameRect(h);
        if (src.cx == fr.Right - fr.Left && src.cy == fr.Bottom - fr.Top)
            return new Native.RECT { Left = fr.Left - wr.Left, Top = fr.Top - wr.Top, Right = wr.Right - fr.Right, Bottom = wr.Bottom - fr.Bottom };
        if (Native.GetWindowRgnBox(h, out box) != 0 && src.cx == box.Right - box.Left && src.cy == box.Bottom - box.Top)
            return new Native.RECT { Left = box.Left, Top = box.Top, Right = ww - box.Right, Bottom = wh - box.Bottom };
        return new Native.RECT();
    }

    // Pencere dikdörtgeninden görünen çerçeveye içe paylar (kenarlık halkası için).
    public static Native.RECT FrameInsets(IntPtr h)
    {
        var wr = WinRect(h); var fr = FrameRect(h);
        return new Native.RECT { Left = Math.Max(0, fr.Left - wr.Left), Top = Math.Max(0, fr.Top - wr.Top), Right = Math.Max(0, wr.Right - fr.Right), Bottom = Math.Max(0, wr.Bottom - fr.Bottom) };
    }

    static Native.RECT Deflate(Native.RECT r, Native.RECT d)
    {
        return new Native.RECT { Left = r.Left + d.Left, Top = r.Top + d.Top, Right = r.Right - d.Right, Bottom = r.Bottom - d.Bottom };
    }
    public static Native.RECT Inflate(Native.RECT r, Native.RECT d)
    {
        return new Native.RECT { Left = r.Left - d.Left, Top = r.Top - d.Top, Right = r.Right + d.Right, Bottom = r.Bottom + d.Bottom };
    }
    // tiling'in verdiği yerleşim dikdörtgeni (görünen çerçeve) -> pencere dikdörtgeni (gölge payları dahil)
    public static Native.RECT WindowRectForFrame(IntPtr h, Native.RECT frame) { return Inflate(frame, FrameInsets(h)); }
    static bool SameRect(Native.RECT a, Native.RECT b) { return a.Left == b.Left && a.Top == b.Top && a.Right == b.Right && a.Bottom == b.Bottom; }

    // tiling pencereleri SetWindowPos ile (kısmen eşzamansız) taşır: dikdörtgenler iki ölçüm arka arkaya aynı
    // kalana kadar bekle (en fazla ~150 ms), sonra bitiş konumlarını oku.
    // Yeni pencere tiling'in verdiği yere gerçekten oturana kadar bekle (görünen çerçeve, gölge kenarları hariç).
    // Yoksa belirme animasyonu pencerenin açıldığı yerde (ekran ortası) oynar ve pencere sonra zıplar.
    public static void WaitPlaced(long h, Native.RECT target, int maxMs)
    {
        var hw = new IntPtr(h);
        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < maxMs)
        {
            Native.RECT fr;
            if (Native.DwmGetWindowAttribute(hw, Native.DWMWA_EXTENDED_FRAME_BOUNDS, out fr, Marshal.SizeOf(typeof(Native.RECT))) != 0 && !Native.GetWindowRect(hw, out fr)) return;
            if (Math.Abs(fr.Left - target.Left) <= 12 && Math.Abs(fr.Top - target.Top) <= 12 &&
                Math.Abs(fr.Right - target.Right) <= 12 && Math.Abs(fr.Bottom - target.Bottom) <= 12)
            {
                if (sw.ElapsedMilliseconds > 20) Log("yeni pencere yerine oturdu: " + sw.ElapsedMilliseconds + "ms");
                return;
            }
            Thread.Sleep(5);
        }
        Log("yeni pencere yerine oturmadı (" + maxMs + "ms)");
    }

    public static void WaitSettled(IEnumerable<long> handles)
    {
        var last = new Dictionary<long, Native.RECT>();
        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < 150)
        {
            bool same = last.Count > 0;
            var now = new Dictionary<long, Native.RECT>();
            foreach (var h in handles)
            {
                Native.RECT r; Native.GetWindowRect(new IntPtr(h), out r); now[h] = r;
                Native.RECT o;
                if (!last.TryGetValue(h, out o) || o.Left != r.Left || o.Top != r.Top || o.Right != r.Right || o.Bottom != r.Bottom) same = false;
            }
            if (same && sw.ElapsedMilliseconds >= 16) return;
            last = now;
            Thread.Sleep(8);
        }
    }

    // Önizlemeyi yerleştir: hedef pencere dikdörtgenidir; kaynak onun bir kısmını kaplıyorsa aynı paylarla içe
    // alınır. Boyut değişirken Hyprland gibi ölçeklenir.
    static void PlaceVisible(Thumb t, Native.RECT dest, bool query = true)
    {
        Native.SIZE src;
        var r = dest;
        if (!query && t.Cx > 0) r = Deflate(dest, t.Ins);
        else if (Native.DwmQueryThumbnailSourceSize(t.Id, out src) == 0 && src.cx > 0 && src.cy > 0)
        {
            if (src.cx != t.Cx || src.cy != t.Cy) { t.Ins = SourceInsets(t.Src, src); t.Cx = src.cx; t.Cy = src.cy; }
            r = Deflate(dest, t.Ins);
        }
        var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION, rcDestination = r };
        Native.DwmUpdateThumbnailProperties(t.Id, ref pr);
    }

    public class Frozen { public int Ox, Oy; public Rectangle Mon; public readonly List<Thumb> All = new List<Thumb>(); public readonly Dictionary<long, Thumb> Win = new Dictionary<long, Thumb>(); public Overlay Ov; }

    // Dondur: katmanı aç, pencereleri şu anki görünür yerlerinde (ya da verilen eski ekran dikdörtgenlerinde)
    // canlı görüntüleriyle göster. Arkasında tiling ne yaparsa yapsın kullanıcı zıplama görmez. UI thread'inde.
    // Animasyon nesli: her dondurma / kaydırma başlangıcında artar. Arka plandaki önbellek tazelemesi sorgusu sürerken bir
    // animasyon başladıysa sonucu atar (yoksa kapanan pencerenin henüz silinmiş hali önbelleğe yazılıp animasyonu bozuyordu).
    public static int Gen;

    // wholeMonitor: the layer covers the bar too (a window going into or out of fullscreen covers it), the bar being
    // a still image beneath the windows
    public Frozen Freeze(Rectangle mon, IEnumerable<long> handles, Dictionary<long, Native.RECT> startScreen, long hidden = 0, bool wholeMonitor = false)
    {
        try { return FreezeCore(mon, handles, startScreen, hidden, wholeMonitor); }
        catch (Exception ex) { Recover("donma: " + ex.Message); throw; }
    }

    // A window covering its whole monitor (to the pixel the OS can be off by): a fullscreen game or video
    internal static bool CoversMonitor(Native.RECT r, int mx, int my, int mw, int mh)
    {
        return r.Left <= mx + 1 && r.Top <= my + 1 && r.Right >= mx + mw - 1 && r.Bottom >= my + mh - 1;
    }

    static bool AnyCovers(Dictionary<string, object> ws, int mx, int my, int mw, int mh)
    {
        var wins = new List<IntPtr>();
        J.Windows(ws, wins);
        foreach (var h in wins)
            if (Native.IsWindow(h) && !Native.IsIconic(h) && CoversMonitor(WinRect(h), mx, my, mw, mh)) return true;
        return false;
    }

    // The bar as a still image on the layer (beneath the windows registered after it), for layers that cover it
    void BarAttach(List<Thumb> scene, Rectangle mon, int ox, int oy)
    {
        var bars = new List<KeyValuePair<IntPtr, Native.RECT>>();
        var title = new StringBuilder(64);
        Native.EnumWindows(delegate (IntPtr h, IntPtr unused)
        {
            if (!Native.IsWindowVisible(h) || Native.IsIconic(h)) return true;
            title.Length = 0; Native.GetWindowText(h, title, 64);
            if (title.ToString() != Names.Bar) return true;
            int cloaked;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cloaked, 4) == 0 && cloaked != 0) return true;
            Native.RECT r;
            if (Native.GetWindowRect(h, out r) && mon.IntersectsWith(Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom)))
                bars.Add(new KeyValuePair<IntPtr, Native.RECT>(h, r));
            return true;
        }, IntPtr.Zero);
        foreach (var b in bars)
        {
            var t = Register(b.Key, Shift(b.Value, ox, oy), null);
            if (t != null) scene.Add(t);
        }
    }

    Frozen FreezeCore(Rectangle mon, IEnumerable<long> handles, Dictionary<long, Native.RECT> startScreen, long hidden, bool wholeMonitor)
    {
        SwipeAbort(); // aynı katman: süren parmak kaydırması bitsin
        Interrupt = false;
        Interlocked.Increment(ref Gen);
        var fz = Stopwatch.StartNew();
        var fzs = new StringBuilder();
        Action<string> step = n => { fzs.Append(n + " " + fz.Elapsed.TotalMilliseconds.ToString("0.0") + " "); };
        int barH = wholeMonitor ? 0 : BarPx(mon.X + mon.Width / 2, mon.Y + mon.Height / 2);
        int ox = mon.X, oy = mon.Y + barH;
        var f = new Frozen { Ox = ox, Oy = oy, Mon = mon };
        UseOverlay(new Rectangle(mon.X, oy, mon.Width, mon.Height - barH));
        f.Ov = overlay;
        step("boyut");
        Native.RECT wsrc;
        IntPtr wall = WallpaperSource(out wsrc);
        step("duvar");
        if (wall != IntPtr.Zero)
        {
            var src = new Native.RECT { Left = mon.X - wsrc.Left, Top = oy - wsrc.Top, Right = mon.X - wsrc.Left + mon.Width, Bottom = oy - wsrc.Top + mon.Height - barH };
            var wt = Register(wall, new Native.RECT { Left = 0, Top = 0, Right = mon.Width, Bottom = mon.Height - barH }, src);
            if (wt != null) f.All.Add(wt);
        }
        DesktopWidgetsAttach(f.All, new Rectangle(mon.X, oy, mon.Width, mon.Height - barH), ox, oy);
        if (wholeMonitor) BarAttach(f.All, mon, ox, oy);
        foreach (var h in handles)
        {
            var hw = new IntPtr(h);
            if (!Native.IsWindowVisible(hw) || Native.IsIconic(hw)) continue; // küçültülmüş: ekranda yok
            var t = RegisterWin(hw, new Native.RECT());
            if (t == null) continue;
            Native.RECT old;
            if (startScreen != null && startScreen.TryGetValue(h, out old)) t.Dest = Shift(old, ox, oy);
            else t.Dest = VisualDest(hw, t.Id, ox, oy);
            if (h == hidden)
            {
                var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_OPACITY, rcDestination = t.Dest, opacity = 0 };
                Native.DwmUpdateThumbnailProperties(t.Id, ref pr); // yeni pencere: Finish'te %80'den belirir
            }
            else PlaceVisible(t, t.Dest);
            f.All.Add(t); f.Win[h] = t;
        }
        step("kayıt");
        RingsAttach(f.Win.Values, FocusedTop()); // kenarlıklar pencerelerin üstünde; katman gizliyken kaydı ucuz
        foreach (var t in f.Win.Values) RingPlace(t, t.Dest, (byte)(t.Src.ToInt64() == hidden ? 0 : 255));
        PinsAttach(new Rectangle(mon.X, oy, mon.Width, mon.Height - barH), ox, oy, f.All); // the bar drawn in the scene is no pin
        step("kenar");
        Animating = true;
        overlay.Reveal();
        RaisePinned();
        step("göster");
        Native.DwmFlush();
        step("flush");
        Log("donma adımları (ms, birikimli): " + fzs);
        return f;
    }

    // Bitir: her pencereyi son yerine kaydır/ölçekle (emphasizedDecel), yeni açılan pencere %80'den büyüyüp belirir
    // (Hyprland windowsIn: popin 80%), sonra katmanı kaldır. UI thread'inde.
    // targetFrames: tiling'in hesapladığı son yerleşim (görünen çerçeve, ekran koordinatı). Verilince animasyon pencerelerin
    // gerçekten yer değiştirmesini BEKLEMEDEN başlar (önceden 16-150 ms bekleniyordu); pencere yer değiştirdiği an hedef onun
    // gerçek yeridir (en küçük boyutu olan uygulama tiling'in hesabından farklı yere oturabilir).
    class Anim { public Thumb T; public IntPtr H; public Native.RECT Start, End, Before; public bool Moved, Resizes; public int Cx0, Cy0; public long SrcAt = -1; }

    // Where each window's image is on screen right now while a layout animation plays (screen coordinates): a new
    // change in the middle of one starts from there instead of jumping to the windows' real places first.
    static readonly object ShownLock = new object();
    static readonly Dictionary<long, Native.RECT> Shown = new Dictionary<long, Native.RECT>();
    static int ShownAt = Environment.TickCount - 100000;
    public static Dictionary<long, Native.RECT> ShownNow(IEnumerable<long> handles)
    {
        var r = new Dictionary<long, Native.RECT>();
        bool live = unchecked(Environment.TickCount - ShownAt) < 100;
        lock (ShownLock)
            foreach (var h in handles)
            {
                Native.RECT s;
                if (live && Shown.TryGetValue(h, out s)) r[h] = s;
                else if (Native.IsWindow(new IntPtr(h))) r[h] = WinRect(new IntPtr(h));
            }
        return r;
    }

    // Hata animasyonu yarıda keserse katman ekranda donmuş kalmasın: temizlenir, hata çağırana gider
    public void Finish(Frozen f, IEnumerable<long> endHandles, long popin, int durationMs, Dictionary<long, Native.RECT> targetFrames = null)
    {
        try { FinishCore(f, endHandles, popin, durationMs, targetFrames); }
        catch (Exception ex) { Recover("bitiş: " + ex.Message); throw; }
    }

    void FinishCore(Frozen f, IEnumerable<long> endHandles, long popin, int durationMs, Dictionary<long, Native.RECT> targetFrames)
    {
        if (f.Ov != null) overlay = f.Ov;
        lock (ShownLock) Shown.Clear(); // the freeze that led here already read them
        // Yer değiştiren pencereler windows_move eğrisiyle (durationMs), yeni pencere windows_in süresi ve eğrisiyle
        var inSpec = Anims.WindowsIn; var moveCurve = Anims.WindowsMove.Curve;
        var items = new List<Anim>();
        Thumb pop = null;
        var keep = new HashSet<long>();
        foreach (var h in endHandles)
        {
            var hw = new IntPtr(h);
            if (!Native.IsWindowVisible(hw) || Native.IsIconic(hw)) continue; // küçültülmüş: ekranda yok
            keep.Add(h);
            Thumb t;
            bool isNew = !f.Win.TryGetValue(h, out t);
            if (isNew) { t = RegisterWin(hw, new Native.RECT()); if (t == null) continue; f.All.Add(t); f.Win[h] = t; }
            var live = VisualDest(hw, IntPtr.Zero, f.Ox, f.Oy);
            Native.RECT tf, end;
            // Hedef yalnızca pencere henüz yerine geçmemişse kullanılır; zaten oradaysa gerçek yeri (piksel farkı olmasın)
            if (targetFrames != null && targetFrames.TryGetValue(h, out tf) && !SameRect(FrameRect(hw), tf)) end = Shift(Inflate(tf, t.FrameIns), f.Ox, f.Oy);
            else end = live;
            var a = new Anim { T = t, H = hw, Start = t.Dest, End = end, Before = isNew ? live : t.Dest };
            if (isNew || h == popin)
            {
                int cx = (end.Left + end.Right) / 2, cy = (end.Top + end.Bottom) / 2;
                double half = inSpec.Popin / 200.0; // popin %80: yarı genişlik 0.4
                int hw2 = (int)((end.Right - end.Left) * half), hh = (int)((end.Bottom - end.Top) * half);
                a.Start = new Native.RECT { Left = cx - hw2, Top = cy - hh, Right = cx + hw2, Bottom = cy + hh };
                pop = t;
            }
            a.Resizes = isNew || (a.Start.Right - a.Start.Left) != (end.Right - end.Left) || (a.Start.Bottom - a.Start.Top) != (end.Bottom - end.Top);
            a.Cx0 = t.Cx; a.Cy0 = t.Cy;
            items.Add(a);
        }
        // Artık bu workspace'te olmayan (kapanan / taşınan) pencereler katmanda kalmasın
        foreach (var kv in f.Win)
            if (!keep.Contains(kv.Key))
            {
                var hide = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_VISIBLE, fVisible = false };
                Native.DwmUpdateThumbnailProperties(kv.Value.Id, ref hide);
                HideRingSet(kv.Value.RingA); HideRingSet(kv.Value.RingI);
            }

        // Kenarlıklar: yalnızca burada ilk kez görülen pencerelere kayıt (katman açıkken kayıt pahalı); odak değiştiyse renk
        IntPtr focusedNow = FocusedTop();
        foreach (var a in items) RingAdd(a.T, a.T.Src == focusedNow);
        RingsFocus(focusedNow);

        var sw = Stopwatch.StartNew();
        var pc = new PresentClock();
        var fs = new FrameStats();
        int frames = 0; long lastFrame = 0, maxGap = 0;
        while (!Interrupt)
        {
            fs.Begin();
            long nowMs = sw.ElapsedMilliseconds;
            if (frames > 0 && nowMs - lastFrame > maxGap) maxGap = nowMs - lastFrame;
            lastFrame = nowMs; frames++;
            double at = pc.Ms();
            double p = Prog(at, durationMs), pIn = pop != null ? Prog(at, inSpec.Ms) : 1.0;
            double e = moveCurve.At(p), eIn = inSpec.Curve.At(pIn);
            foreach (var a in items)
            {
                if (!a.Moved)
                {
                    var live = VisualDest(a.H, IntPtr.Zero, f.Ox, f.Oy);
                    if (!SameRect(live, a.Before)) { a.Moved = true; a.End = live; }
                }
                else a.End = VisualDest(a.H, IntPtr.Zero, f.Ox, f.Oy);
                var r = Lerp(a.Start, a.End, a.T == pop ? eIn : e);
                byte op = 255;
                if (a.T == pop)
                {
                    // Hyprland windowsIn "popin 80%": ölçekli büyüyerek ve belirerek
                    op = (byte)Math.Min(255, (int)(255 * Math.Min(1.0, pIn * 2.5)));
                    var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_OPACITY, rcDestination = r, opacity = op };
                    Native.DwmUpdateThumbnailProperties(a.T.Id, ref pr);
                }
                else
                {
                    PlaceVisible(a.T, r, a.Resizes);
                    if (a.Resizes && a.SrcAt < 0 && (a.T.Cx != a.Cx0 || a.T.Cy != a.Cy0)) a.SrcAt = nowMs;
                }
                RingPlace(a.T, r, op);
                lock (ShownLock) Shown[a.H.ToInt64()] = Unshift(r, f.Ox, f.Oy);
            }
            ShownAt = Environment.TickCount;
            fs.Updated();
            Native.DwmFlush();
            fs.Flushed();
            if (p >= 1.0 && pIn >= 1.0) break;
        }
        var sb = new StringBuilder();
        foreach (var a in items) if (a.Resizes && a.T != pop) sb.Append(" | içerik " + a.Cx0 + "x" + a.Cy0 + "->" + a.T.Cx + "x" + a.T.Cy + (a.SrcAt >= 0 ? " @" + a.SrcAt + "ms" : " (değişmedi)"));
        Log("anim: " + frames + " kare / " + sw.ElapsedMilliseconds + " ms, en uzun kare " + maxGap + " ms, " + items.Count + " pencere" + sb + " " + fs.Report() + " önizleme=" + Native.LiveThumbs);
        overlay.Conceal();
        RingsClear();
        PinsClear();
        foreach (var t in f.All) Unregister(t.Id);
        Animating = false;
    }

    Thumb RegisterWin(IntPtr hw, Native.RECT dest)
    {
        var t = Register(hw, dest, null);
        if (t == null) return null;
        t.IsWin = true; t.FrameIns = FrameInsets(hw);
        return t;
    }

    Thumb RegisterWindow(IntPtr h, int ox, int oy)
    {
        // Küçültülmüş pencere ekranda yok (Windows onu yine de "görünür" sayar): önizlemesi ve 8 parçalık kenarlık
        // halkası her karede boşuna güncelleniyor, kaydı da DWM meşgulken pencere başına birkaç ms sürüyordu.
        // Workspace'te küçültülmüş pencere biriktikçe geçiş belirgin şekilde yavaşlıyordu.
        if (!Native.IsWindow(h) || Native.IsIconic(h)) return null;
        var t = RegisterWin(h, Shift(WinRect(h), ox, oy));
        if (t == null) return null;
        PlaceVisible(t, t.Dest);
        return t;
    }

    // Kaydırma: boyut değişmez, kaynak boyutu kayıtta okundu
    Native.RECT Move(Thumb t, int dx)
    {
        var r = t.Dest; r.Left += dx; r.Right += dx;
        PlaceSliding(t, r, false);
        return r;
    }

    // Oyunlardaki gibi görünmeyeni çizme: katmanın tamamen dışına kaymış pencerenin önizlemesi ve 8 parçalık kenarlık
    // halkası gizlenir ve her karede güncellenmez (kaydırmada pencerelerin yarısı her an ekran dışında; DWM onları da
    // işliyordu). Görüş alanına girince yeniden gösterilir. Halka çerçevenin biraz dışına taştığı için pay bırakılır.
    void PlaceSliding(Thumb t, Native.RECT r, bool query)
    {
        const int PAD = 32;
        bool outside = r.Right <= -PAD || r.Left >= ovW + PAD || r.Bottom <= -PAD || r.Top >= ovH + PAD;
        if (outside && ovW > 0)
        {
            if (t.Culled) return;
            t.Culled = true; culledCount++;
            var hide = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_VISIBLE, fVisible = false };
            Native.DwmUpdateThumbnailProperties(t.Id, ref hide);
            HideRingSet(t.RingA); HideRingSet(t.RingI);
            return;
        }
        if (t.Culled)
        {
            t.Culled = false;
            var show = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_VISIBLE, fVisible = true };
            Native.DwmUpdateThumbnailProperties(t.Id, ref show);
        }
        PlaceVisible(t, r, query);
        RingPlace(t, r, 255);
    }

    static Dictionary<string, object> FocusedMonitor(List<Dictionary<string, object>> mons, out Dictionary<string, object> ws)
    {
        ws = null;
        foreach (var m in mons)
            foreach (Dictionary<string, object> w in J.Children(m))
                if (J.Bool(w, "hasFocus")) { ws = w; return m; }
        foreach (var m in mons)
            foreach (Dictionary<string, object> w in J.Children(m))
                if (J.Bool(w, "isDisplayed")) { ws = w; return m; }
        return null;
    }

    // commands: tiling'e gönderilecekler. dirHint: +1 sağ, -1 sol, 0 = isimden hesapla.
    // ---- Pencere aç/kapa animasyonu (Hyprland windowsMove: speed 3 ≈ 300ms emphasizedDecel,
    // windowsIn: popin 80%). tiling pencereleri anında yerleştirir; biz eski yerleşimden yenisine
    // canlı DWM önizlemelerini kaydırıp ölçekleyerek geçiş yapıyoruz, sonra gerçek pencereler görünür.
    public static int MoveMs { get { return Anims.WindowsMove.Ms; } }

    static Native.RECT Lerp(Native.RECT a, Native.RECT b, double e)
    {
        return new Native.RECT
        {
            Left = (int)Math.Round(a.Left + (b.Left - a.Left) * e),
            Top = (int)Math.Round(a.Top + (b.Top - a.Top) * e),
            Right = (int)Math.Round(a.Right + (b.Right - a.Right) * e),
            Bottom = (int)Math.Round(a.Bottom + (b.Bottom - a.Bottom) * e)
        };
    }

    static Native.RECT Shift(Native.RECT r, int ox, int oy)
    {
        return new Native.RECT { Left = r.Left - ox, Top = r.Top - oy, Right = r.Right - ox, Bottom = r.Bottom - oy };
    }

    // Hyprland dwindle yeni pencereyi odaktakine değil FARENİN ALTINDAKİ pencereye açar. tiling
    // hep odaktakinin yanına koyduğu için: terminal açmadan hemen önce fare altındakini odakla.
    public void FocusUnderCursor()
    {
        var p = Cursor.Position;
        IntPtr under = Native.WindowFromPoint(p);
        long handle = under == IntPtr.Zero ? 0 : Native.GetAncestor(under, 2).ToInt64();
        var mons = tiling.Monitors();
        foreach (var m in mons)
            foreach (Dictionary<string, object> ws in J.Children(m))
            {
                if (!J.Bool(ws, "isDisplayed")) continue;
                var wins = new List<Dictionary<string, object>>();
                J.WindowNodes(ws, wins);
                foreach (var w in wins)
                {
                    object hv;
                    if (w.TryGetValue("handle", out hv) && Convert.ToInt64(hv) == handle)
                    {
                        if (!J.Bool(w, "hasFocus")) tiling.Command("focus --container-id " + J.Str(w, "id"));
                        return;
                    }
                }
            }
        // Farenin altında yönetilen pencere yok (boş workspace, masaüstü): farenin olduğu
        // monitörde gösterilen workspace'i odakla ki yeni pencere orada açılsın.
        foreach (var m in mons)
        {
            int mx = J.Int(m, "x"), my = J.Int(m, "y");
            if (p.X < mx || p.Y < my || p.X >= mx + J.Int(m, "width") || p.Y >= my + J.Int(m, "height")) continue;
            foreach (Dictionary<string, object> ws in J.Children(m))
                if (J.Bool(ws, "isDisplayed") && !J.Bool(ws, "hasFocus"))
                    tiling.Command("focus --workspace " + J.Str(ws, "name"));
            return;
        }
    }

    // Super+ok (focus) / Super+Shift+ok (move): yalnızca AYNI workspace içinde. tiling'in
    // "--direction" komutları o yönde pencere yoksa yan monitöre/workspace'e atlıyordu.
    // Sonunda fare hedef pencerenin ortasına taşınır (Hyprland'de odak değişince imleç de gider).
    public void Commands(string[] cmds) { foreach (var c in cmds) tiling.Command(c); }

    // Tutamacı verilen pencerenin pencere yöneticisindeki kimliği ve workspace'i (yönetilmiyorsa false)
    public bool FindHandle(long handle, out string id, out string workspace, out bool shown)
    {
        id = null; workspace = null; shown = false;
        foreach (var m in tiling.Monitors())
            foreach (Dictionary<string, object> ws in J.Children(m))
            {
                var wins = new List<Dictionary<string, object>>();
                J.WindowNodes(ws, wins);
                foreach (var w in wins)
                {
                    object hv;
                    if (w.TryGetValue("handle", out hv) && Convert.ToInt64(hv) == handle)
                    {
                        id = J.Str(w, "id"); workspace = J.Str(ws, "name"); shown = J.Bool(ws, "isDisplayed");
                        return true;
                    }
                }
            }
        return false;
    }

    public void FocusInWorkspace(string dir) { InWorkspace(dir, false); }
    public void MoveInWorkspace(string dir) { InWorkspace(dir, true); }

    static void WarpTo(Dictionary<string, object> w)
    {
        if (w == null) return;
        Cursor.Position = new Point(J.Int(w, "x") + J.Int(w, "width") / 2, J.Int(w, "y") + J.Int(w, "height") / 2);
    }

    // Workspace'in odaklı penceresinin (yoksa ilk penceresinin, o da yoksa monitörün) ortasına imleci götür
    static void WarpInto(Dictionary<string, object> monitor, Dictionary<string, object> ws)
    {
        var tw = new List<Dictionary<string, object>>();
        J.WindowNodes(ws, tw);
        Dictionary<string, object> fw = null;
        foreach (var x in tw) if (J.Bool(x, "hasFocus")) fw = x;
        if (fw == null && tw.Count > 0) fw = tw[0];
        if (fw != null) WarpTo(fw);
        else Cursor.Position = new Point(J.Int(monitor, "x") + J.Int(monitor, "width") / 2, J.Int(monitor, "y") + J.Int(monitor, "height") / 2);
    }

    // Hyprland'deki gibi pencere taşımaları da animasyonlu olsun: UI thread'inde AnimateLayout
    public static Control Ui;

    static Dictionary<string, object> Neighbor(List<Dictionary<string, object>> wins, Dictionary<string, object> cur, string dir)
    {
        int cx = J.Int(cur, "x"), cy = J.Int(cur, "y"), cw = J.Int(cur, "width"), ch = J.Int(cur, "height");
        int ccx = cx + cw / 2, ccy = cy + ch / 2;
        Dictionary<string, object> best = null;
        long bestScore = long.MaxValue;
        foreach (var w in wins)
        {
            if (J.Str(w, "id") == J.Str(cur, "id")) continue;
            int x = J.Int(w, "x"), y = J.Int(w, "y"), ww = J.Int(w, "width"), wh = J.Int(w, "height");
            int mx = x + ww / 2, my = y + wh / 2;
            long along, across;
            if (dir == "left") { if (mx >= ccx || x + ww > cx + 4) continue; along = cx - (x + ww); across = Math.Abs(my - ccy); }
            else if (dir == "right") { if (mx <= ccx || x < cx + cw - 4) continue; along = x - (cx + cw); across = Math.Abs(my - ccy); }
            else if (dir == "up") { if (my >= ccy || y + wh > cy + 4) continue; along = cy - (y + wh); across = Math.Abs(mx - ccx); }
            else { if (my <= ccy || y < cy + ch - 4) continue; along = y - (cy + ch); across = Math.Abs(mx - ccx); }
            // Önce en yakın sütun/satır, sonra aynı hizadaki
            long score = Math.Max(0, along) * 4 + across;
            if (score < bestScore) { bestScore = score; best = w; }
        }
        return best;
    }

    static Dictionary<string, object> ParentOf(Dictionary<string, object> node, string id)
    {
        foreach (Dictionary<string, object> ch in J.Children(node))
        {
            if (J.Str(ch, "id") == id) return node;
            var r = ParentOf(ch, id);
            if (r != null) return r;
        }
        return null;
    }

    // Odaktaki workspace'i yeniden sorgula: (workspace, pencereler, odaktaki)
    bool Current(out Dictionary<string, object> mon, out Dictionary<string, object> ws, out List<Dictionary<string, object>> wins, out Dictionary<string, object> cur)
    {
        wins = new List<Dictionary<string, object>>(); cur = null;
        mon = FocusedMonitor(tiling.Monitors(), out ws);
        if (mon == null || ws == null || !J.Bool(ws, "hasFocus")) return false;
        J.WindowNodes(ws, wins);
        foreach (var w in wins) if (J.Bool(w, "hasFocus")) cur = w;
        return cur != null;
    }

    static Dictionary<long, Native.RECT> Rects(List<Dictionary<string, object>> wins)
    {
        var r = new Dictionary<long, Native.RECT>();
        foreach (var w in wins)
        {
            object hv;
            if (!w.TryGetValue("handle", out hv) || hv == null) continue;
            int x = J.Int(w, "x"), y = J.Int(w, "y");
            r[Convert.ToInt64(hv)] = new Native.RECT { Left = x, Top = y, Right = x + J.Int(w, "width"), Bottom = y + J.Int(w, "height") };
        }
        return r;
    }

    // Öz-test (lunge.exe --anim-selftest): odaklı monitörü dondurup pencereleri AYNI yerlerine "animasyonla"
    // götürür. Doğruysa ekranda hiçbir şey kıpırdamaz; log'a kare süreleri yazılır.
    public void SelfTest()
    {
        Dictionary<string, object> mon, ws, cur; List<Dictionary<string, object>> wins;
        if (!Current(out mon, out ws, out wins, out cur)) { Log("selftest: odakta pencere yok"); return; }
        var monRect = new Rectangle(J.Int(mon, "x"), J.Int(mon, "y"), J.Int(mon, "width"), J.Int(mon, "height"));
        var targets = Rects(wins);
        var hs = new List<long>(targets.Keys);
        var sw = Stopwatch.StartNew();
        var f = Freeze(monRect, hs, null);
        Log("selftest: donma " + sw.ElapsedMilliseconds + " ms");
        Finish(f, hs, 0, MoveMs, targets);
    }

    static string StateType(Dictionary<string, object> w)
    {
        object st; var state = w != null && w.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
        return state != null ? J.Str(state, "type") : "";
    }

    void InWorkspace(string dir, bool move)
    {
        var clk = Stopwatch.StartNew();
        Dictionary<string, object> mon, ws, cur;
        List<Dictionary<string, object>> wins;
        if (!Current(out mon, out ws, out wins, out cur)) return; // boş workspace ya da pencere odakta değil

        var best = Neighbor(wins, cur, dir);
        if (!move)
        {
            if (best == null) return; // o yönde bu workspace'te pencere yok
            tiling.Command("focus --container-id " + J.Str(best, "id"));
            WarpTo(best);
            return;
        }

        string id = J.Str(cur, "id");
        if (best == null)
        {
            // O yönde komşu yok: tiling (fork) pencereyi o kenara çıkarır; pencere ekranın o yarısını alır, geri kalan
            // düzen öbür yarıda şeklini korur (Hyprland dwindle movetoroot). Örn. 2x2'de sağ üstteki sağa -> sağda boydan,
            // solda [sol üst / sol alt] sütunu ile eski sağ alttaki yan yana. Tek durum hariç: pencere doğrudan
            // workspace'in elemanıysa ve workspace zaten o eksendeyse tiling pencereyi diğer monitörün
            // workspace'ine atıyordu; orada hiçbir şey yapma. Tek pencerede de.
            var par0 = ParentOf(ws, J.Str(cur, "id"));
            string axis = dir == "left" || dir == "right" ? "horizontal" : "vertical";
            // yalnızca döşeli pencereler sayılır: bir döşeli + bir yüzen pencerede komut tiling'e gidiyor, o da pencereyi
            // yandaki monitöre atıyordu. Tam ekran pencere de kenarda monitör değiştirmez.
            int tiled = 0;
            foreach (var w in wins) if (StateType(w) == "tiling") tiled++;
            string curState = StateType(cur);
            if (curState == "fullscreen") return;
            if (par0 == null || (curState == "tiling" && tiled < 2)) return;
            if (J.Str(par0, "type") == "workspace" && J.Str(par0, "tilingDirection") == axis) return;
        }
        if (!Prefs.Animations || MoveMs <= 0) { tiling.Command("move --direction " + dir); return; } // animasyonlar kapalı (ayarlar / config.yaml)
        // Önce görüntüyü dondur (pencereler şu an nerede görünüyorsa orada), tiling arkada yerleştirsin
        var monRect = new Rectangle(J.Int(mon, "x"), J.Int(mon, "y"), J.Int(mon, "width"), J.Int(mon, "height"));
        var hs = new List<long>(Rects(wins).Keys);
        Frozen frozen = null;
        if (Ui != null)
        {
            Interrupt = true;
            try { frozen = (Frozen)Ui.Invoke((Func<Frozen>)(() => Freeze(monRect, hs, null))); } catch (Exception ex) { Log("freeze: " + ex.Message); }
        }
        long frozenAt = clk.ElapsedMilliseconds;
        // tiling (fork) Hyprland dwindle movewindow yapar: o yönde pencere varsa onu uzun kenarından böler; yoksa
        // bölme yönü değişir (yan yana iki pencerede Super+Shift+Yukarı -> odaktaki üstte tam genişlik).
        tiling.Command("move --direction " + dir);

        Dictionary<string, object> mA, wsA, curA; List<Dictionary<string, object>> winsA;
        bool ok = Current(out mA, out wsA, out winsA, out curA);
        var targets = ok ? Rects(winsA) : null;
        Log("taşı " + dir + ": donma " + frozenAt + " ms, tiling hazır " + clk.ElapsedMilliseconds + " ms");
        var endHs = ok ? new List<long>(targets.Keys) : hs;
        // Pencerelerin yerleşmesi beklenmez: hedef tiling'in hesabı, yer değişince gerçek yer (Finish)
        if (ok)
        {
            Dictionary<string, object> moved = null;
            foreach (var w in winsA) if (J.Str(w, "id") == id) moved = w;
            WarpTo(moved);
        }
        if (frozen != null)
        {
            int dur = Adaptive(ref lastMoveStart, MoveMs);
            Ui.BeginInvoke((Action)(() =>
            {
                try { Finish(frozen, endHs, 0, dur, targets); } catch (Exception ex) { Log("move anim: " + ex.Message); }
            }));
        }
    }
    // Satır çağıran iş parçacığında yalnızca kuyruğa girer; diske tek bir arka plan yazıcı toplu yazar. Eskiden her satır
    // çağıranın iş parçacığında (animasyon, kanca işleri) dosyayı açıp yazıp kapatıyordu: yavaş disk ya da tarayan bir
    // antivirüs kare kaçırtıyordu.
    public static void Log(string s)
    {
        try
        {
            // Aynı satır art arda gelirse (pencere yöneticisi yokken her denemede "ipc error") bir dakika boyunca tek satır
            // kalır, sonra kaç kez tekrarlandığı yazılır: kesintinin başı birkaç saatte dosyadan atılmıyordu
            string repeated = null;
            lock (logLock)
            {
                if (s == lastLine && logClock.ElapsedMilliseconds - lastLineAt < 60000) { repeats++; return; }
                if (repeats > 0) repeated = "  (önceki satır " + repeats + " kez daha)";
                lastLine = s; lastLineAt = logClock.ElapsedMilliseconds; repeats = 0;
            }
            var now = DateTime.Now;
            if (repeated != null) s = repeated + Environment.NewLine + now.ToString("HH:mm:ss.fff ") + s;
            LogWriter.Add(now, s);
        }
        catch { }
    }
    static readonly object logLock = new object();
    static readonly Stopwatch logClock = Stopwatch.StartNew();
    static string lastLine;
    static long lastLineAt;
    static int repeats;

    public void Run(string[] commands, int dirHint, string targetName)
    {
        try { RunCore(commands, dirHint, targetName); }
        catch (Exception ex) { if (Animating) Recover("kayma: " + ex.Message); throw; }
    }

    void RunCore(string[] commands, int dirHint, string targetName)
    {
        SwipeAbort();
        // Animasyonlar kapalı (ayarlar) ya da bu hareketin süresi 0 (config.yaml): workspace doğrudan değişir
        bool carry0 = commands.Length == 2 && commands[0].StartsWith("move --") && commands[1].StartsWith("focus --");
        if (!Prefs.Animations || (carry0 ? Anims.Carry : Anims.Workspaces).Ms <= 0) { foreach (var c in commands) tiling.Command(c); return; }
        var clock = Stopwatch.StartNew();
        Interrupt = false;
        var mons = tiling.Monitors();
        Dictionary<string, object> oldWs;
        var mon = FocusedMonitor(mons, out oldWs);
        Log("query " + clock.ElapsedMilliseconds + "ms monitors=" + mons.Count + " focusedMon=" + (mon != null));
        if (mon == null || oldWs == null) { foreach (var c in commands) tiling.Command(c); return; }

        string oldName = J.Str(oldWs, "name");
        if (targetName != null && targetName == oldName) return;

        // Hedef başka monitörde zaten gösteriliyorsa kaydırma yok: gerçekte yandaki ekrana geçiliyor.
        // Super+Ctrl+←/→ (next/prev) için de hedefi önceden tahmin et; fare de o monitöre gitsin (Hyprland gibi).
        string otherTarget = targetName;
        string lastCmd = commands[commands.Length - 1]; // tek geçiş ya da taşı+takip et (move ..., focus ...)
        if (otherTarget == null && commands.Length <= 2)
        {
            int cur0;
            if (int.TryParse(oldName, out cur0))
            {
                if (lastCmd == "focus --next-workspace") otherTarget = AdjacentWorkspace(mons, mon, oldName, 1);
                else if (lastCmd == "focus --prev-workspace") otherTarget = AdjacentWorkspace(mons, mon, oldName, -1);
            }
        }
        Dictionary<string, object> otherMon = null, otherWs = null, warpMon = null, warpWs = null;
        if (otherTarget != null)
            foreach (var m in mons)
                if (J.Str(m, "id") != J.Str(mon, "id"))
                    foreach (Dictionary<string, object> w in J.Children(m))
                        if (J.Str(w, "name") == otherTarget) { otherMon = m; otherWs = w; }
        if (otherMon != null)
        {
            // Workspace, bulunduğu monitörde gösterilir; kayma da orada olmalı. Önceden odaktaki monitörde oynuyordu:
            // ekran kayıp eski workspace'e dönüyor, hedef yandaki monitörde beliriyordu.
            Dictionary<string, object> shown = null;
            foreach (Dictionary<string, object> w in J.Children(otherMon)) if (J.Bool(w, "isDisplayed")) shown = w;
            if (J.Bool(otherWs, "isDisplayed") || shown == null || commands.Length != 1)
            {
                // Zaten orada gösteriliyor (yandaki ekrana geçiş) ya da pencere taşınıp takip ediliyor: animasyonsuz
                foreach (var c in commands) tiling.Command(c);
                WarpInto(otherMon, otherWs); // imleç odağın geçtiği monitöre (Hyprland gibi)
                Log("slide: hedef diğer monitörde, animasyonsuz");
                return;
            }
            mon = otherMon; oldWs = shown; oldName = J.Str(shown, "name");
            warpMon = otherMon; warpWs = otherWs;
            Log("slide: hedef diğer monitörde gizli, kayma orada " + oldName + " -> " + otherTarget);
        }

        int mx = J.Int(mon, "x"), my = J.Int(mon, "y"), mw = J.Int(mon, "width"), mh = J.Int(mon, "height");
        // A fullscreen window (a game, a video) covers the bar. The layer normally starts under the bar so the bar
        // stays put; a fullscreen window's top then went under the real bar as the slide began and came back at its
        // end (it seemed to shrink and grow). With one on either side the layer takes the whole monitor, the bar is a
        // still image beneath the sliding windows, and the fullscreen window slides whole.
        Dictionary<string, object> slideTarget = null;
        if (otherTarget != null)
            foreach (Dictionary<string, object> w in J.Children(mon)) if (J.Str(w, "name") == otherTarget) slideTarget = w;
        bool wholeMonitor = AnyCovers(oldWs, mx, my, mw, mh) || (slideTarget != null && AnyCovers(slideTarget, mx, my, mw, mh));
        int barH = wholeMonitor ? 0 : BarPx(mx + mw / 2, my + mh / 2);
        Interlocked.Increment(ref Gen);
        UseOverlay(new Rectangle(mx, my + barH, mw, mh - barH));
        int ox = mx, oy = my + barH;

        var thumbs = new List<Thumb>();
        var oldThumbs = new List<Thumb>();
        var newThumbs = new List<Thumb>();

        // Duvar kağıdı (sabit, Hyprland'de de kaymaz)
        Native.RECT wsrc;
        IntPtr wall = WallpaperSource(out wsrc);
        if (wall != IntPtr.Zero)
        {
            var src = new Native.RECT { Left = mx - wsrc.Left, Top = oy - wsrc.Top, Right = mx - wsrc.Left + mw, Bottom = oy - wsrc.Top + mh - barH };
            var t = Register(wall, new Native.RECT { Left = 0, Top = 0, Right = mw, Bottom = mh - barH }, src);
            if (t != null) thumbs.Add(t);
        }
        DesktopWidgetsAttach(thumbs, new Rectangle(mx, oy, mw, mh - barH), ox, oy);
        if (wholeMonitor) BarAttach(thumbs, new Rectangle(mx, my, mw, mh), ox, oy);

        // Super+Ctrl+Shift+←/→: pencereyi taşı ve takip et. Taşınan pencere yerinde kalır, workspace'ler onun
        // arkasında kayar (pencereyi yanında götürüyormuşsun gibi), sonra yeni yerleşimdeki yerine oturur.
        bool moveFollow = commands.Length == 2 && commands[0].StartsWith("move --") && commands[1].StartsWith("focus --");
        IntPtr carriedH = IntPtr.Zero;
        if (moveFollow)
        {
            var ow = new List<Dictionary<string, object>>();
            J.WindowNodes(oldWs, ow);
            foreach (var w in ow) if (J.Bool(w, "hasFocus")) carriedH = new IntPtr(Convert.ToInt64(w["handle"]));
            if (carriedH == IntPtr.Zero) moveFollow = false;
        }
        Thumb carried = null;
        var oldWins = new List<IntPtr>();
        J.Windows(oldWs, oldWins);
        foreach (var h in oldWins)
        {
            Native.RECT r;
            if (!Native.IsWindowVisible(h) || !Native.GetWindowRect(h, out r)) continue;
            var t = RegisterWindow(h, ox, oy);
            if (t == null) continue;
            thumbs.Add(t);
            if (moveFollow && h == carriedH) carried = t; else oldThumbs.Add(t);
        }

        Log("snapshot " + clock.ElapsedMilliseconds + "ms old=" + oldThumbs.Count);

        // ---- Hızlı yol: hedef workspace önceden belliyse (Super+sayı, Super+Ctrl+←/→) animasyon
        // tiling komutunu BEKLEMEDEN başlar. tiling'in geçişi pencere sayısıyla 100-200 ms sürüyor
        // ve animasyon ondan sonra başladığı için her geçişte önce donma hissi oluyordu. Gizli
        // workspace'in pencereleri konumlarını koruduğu için önizlemeleri komuttan önce hazırlanabilir.
        string predicted = targetName;
        string focusCmd = moveFollow ? commands[1] : commands[0];
        if (predicted == null && (commands.Length == 1 || moveFollow))
        {
            int cur;
            if (int.TryParse(oldName, out cur))
            {
                if (focusCmd == "focus --next-workspace") predicted = AdjacentWorkspace(mons, mon, oldName, 1);
                else if (focusCmd == "focus --prev-workspace") predicted = AdjacentWorkspace(mons, mon, oldName, -1);
            }
        }
        // Uçtaki workspace (bu monitörde ötesi yok): pencere yöneticisi de bir şey yapmaz; animasyon da başlamasın
        if (predicted == null && (commands.Length == 1 || moveFollow) && (focusCmd == "focus --next-workspace" || focusCmd == "focus --prev-workspace"))
        {
            Log("slide: bu monitörde " + (focusCmd.EndsWith("next-workspace") ? "sonraki" : "önceki") + " workspace yok");
            return;
        }
        if (predicted != null && (commands.Length == 1 || moveFollow))
        {
            Dictionary<string, object> target = null;
            foreach (var m in mons)
                foreach (Dictionary<string, object> w in J.Children(m))
                    if (J.Str(w, "name") == predicted) target = w;

            int fdir = dirHint;
            int a2, b2;
            if (fdir == 0) fdir = int.TryParse(oldName, out a2) && int.TryParse(predicted, out b2) && b2 < a2 ? -1 : 1;

            if (target != null)
            {
                var tw = new List<Dictionary<string, object>>();
                J.WindowNodes(target, tw);
                foreach (var w in tw)
                {
                    object st; var state = w.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
                    if (state != null && J.Str(state, "type") == "minimized") continue;
                    var t = RegisterWindow(new IntPtr(Convert.ToInt64(w["handle"])), ox, oy);
                    if (t != null) { newThumbs.Add(t); thumbs.Add(t); Move(t, fdir * (mw + GAP)); }
                }
            }

            long regMs = clock.ElapsedMilliseconds;
            // Kenarlık: taşınan pencerenin (taşı+takip) ya da odaklı pencerenin; tüm önizlemelerden sonra kaydedilir
            // Kenarlıklar: taşınan (taşı+takip) ya da odaklı pencereye etkin, diğerlerine pasif; önizlemelerden sonra
            RingsAttach(thumbs, carried != null ? carried.Src : WorkspaceFocusHandle(target));
            foreach (var t in oldThumbs) RingPlace(t, t.Dest, 255);
            foreach (var t in newThumbs) { var r0 = t.Dest; r0.Left += fdir * (mw + GAP); r0.Right += fdir * (mw + GAP); RingPlace(t, r0, 255); }
            if (carried != null) RingPlace(carried, carried.Dest, 255);
            PinsAttach(new Rectangle(mx, my + barH, mw, mh - barH), ox, oy, thumbs);
            overlay.Reveal();
            RaisePinned();
            Native.DwmFlush();
            Log("fast shown " + clock.ElapsedMilliseconds + "ms (kayıt " + regMs + "ms) new=" + newThumbs.Count);

            var cmdsAll = (string[])commands.Clone();
            var task = Task.Factory.StartNew(() => { foreach (var cm in cmdsAll) tiling.Command(cm); });
            var slide = moveFollow ? Anims.Carry : Anims.Workspaces; // taşıma daha kısa: pencere beklemeden yerine geçsin
            var settle = Anims.WindowsMove.Curve;
            int dur0 = Adaptive(ref lastSlideStart, slide.Ms);
            Animating = true;

            // Taşı+takip et: tiling komutu bittiği an (genelde kaymanın ilk ~50 ms'i) hedef workspace'teki pencereler
            // ve taşınan pencere yeni yerlerine doğru kaymayla AYNI ANDA ve esnemeden ilerler; ayrı bir "yerleşme"
            // adımı yok (Hyprland'de de pencere kayarken boyutlanır). Hedef her karede canlı okunur: tiling pencereyi
            // eşzamansız taşıdığı için ilk okuma eski yer olabilir.
            var from = new Dictionary<Thumb, Native.RECT>();
            if (moveFollow) { foreach (var t in newThumbs) from[t] = t.Dest; if (carried != null) from[carried] = carried.Dest; }
            Stopwatch swR = null;
            int durR = 0;
            int mfFrames = 0; long mfLast = 0, mfMax = 0, cmdDoneAt = -1, movedAt = -1;
            Native.RECT carriedStart = carried != null ? WinRect(carried.Src) : new Native.RECT();
            Func<bool> WindowsMoved = () =>
            {
                if (carried != null) { var nowR = WinRect(carried.Src); if (nowR.Left != carriedStart.Left || nowR.Top != carriedStart.Top || nowR.Right != carriedStart.Right || nowR.Bottom != carriedStart.Bottom) return true; }
                return false;
            };
            var sw0 = Stopwatch.StartNew();
            var pc = new PresentClock();
            double rStart = 0;
            var fs = new FrameStats();
            culledCount = 0;
            while (!Interrupt)
            {
                fs.Begin();
                long nowMs0 = sw0.ElapsedMilliseconds;
                double at = pc.Ms(); // bu karenin ekranda görüneceği an
                if (mfFrames > 0 && nowMs0 - mfLast > mfMax) mfMax = nowMs0 - mfLast;
                mfLast = nowMs0; mfFrames++;
                if (cmdDoneAt < 0 && task.IsCompleted) cmdDoneAt = nowMs0;
                if (movedAt < 0 && moveFollow && WindowsMoved()) movedAt = nowMs0;
                double p = Prog(at, dur0);
                double e = slide.Curve.At(p);
                int shift = (int)Math.Round(e * (mw + GAP));
                if (moveFollow && swR == null && (task.IsCompleted || WindowsMoved()))
                {
                    // Pencere yeni boyutuna geçti: yerleşme kayma ile BİRLİKTE, en az 300 ms'lik yumuşak bir geçişle
                    swR = Stopwatch.StartNew();
                    rStart = at;
                    durR = Math.Max(200, (int)(dur0 - at));
                }
                double pR = swR == null ? 0 : Prog(at - rStart, durR);
                double eR = settle.At(pR);
                foreach (var t in oldThumbs) Move(t, -fdir * shift);
                foreach (var t in newThumbs)
                {
                    int dx = fdir * (mw + GAP) - fdir * shift;
                    if (!moveFollow) { Move(t, dx); continue; }
                    var r = swR == null ? from[t] : Lerp(from[t], VisualDest(t.Src, t.Id, ox, oy), eR);
                    r.Left += dx; r.Right += dx;
                    PlaceSliding(t, r, swR != null);
                }
                if (moveFollow && carried != null)
                {
                    var rc = swR == null ? from[carried] : Lerp(from[carried], VisualDest(carried.Src, carried.Id, ox, oy), eR);
                    PlaceVisible(carried, rc, swR != null);
                    RingPlace(carried, rc, 255);
                }
                fs.Updated();
                // An app's own fullscreen on the target workspace is raised to the top by the window manager as
                // the slide runs (it is its workspace's focused window): it covered the layer and the workspace
                // appeared without a slide. The layer goes back on top every few frames.
                if (mfFrames % 4 == 1) { overlay.Reveal(); RaisePinned(); }
                Native.DwmFlush();
                fs.Flushed();
                if (p >= 1.0 && (!moveFollow || pR >= 1.0)) break;
                if (p >= 1.0 && swR == null && sw0.ElapsedMilliseconds > dur0 + 1500) break; // komut takıldı
            }
            long animEnd = clock.ElapsedMilliseconds;
            Log("slide" + (moveFollow ? "+taşı" : "") + ": " + mfFrames + " kare, en uzun kare " + mfMax + " ms, komut bitti " + cmdDoneAt + " ms, pencere yer değiştirdi " + movedAt + " ms " + fs.Report() + " önizleme=" + Native.LiveThumbs + " gizlenen=" + culledCount);
            // Katmanı tiling'in yanıtını değil GERÇEK durumu bekleyerek kaldır: eski workspace'in pencereleri gizlenip
            // (cloak) yenininkiler göründüğü an. tiling bazen pencereleri gösterdikten ~250 ms sonra yanıt veriyordu
            // ve hızlı basışta her geçiş bunu bekliyordu. Yanıt arkada gelmeye devam eder.
            Func<IntPtr, bool> cloaked = hw => { int cv; return Native.DwmGetWindowAttribute(hw, Native.DWMWA_CLOAKED, out cv, 4) == 0 && cv != 0; };
            var waitSw = Stopwatch.StartNew();
            bool viaState = false;
            while (!task.IsCompleted && waitSw.ElapsedMilliseconds < 1500)
            {
                bool done = oldThumbs.Count + newThumbs.Count > 0;
                foreach (var t in oldThumbs) if (!cloaked(t.Src)) { done = false; break; }
                if (done) foreach (var t in newThumbs) if (cloaked(t.Src)) { done = false; break; }
                if (done) { viaState = true; break; }
                Thread.Sleep(4);
            }
            overlay.Conceal(); RingsClear(); PinsClear();
            foreach (var t in thumbs) Unregister(t.Id);
            Animating = false;
            Log("fast done " + clock.ElapsedMilliseconds + "ms (animasyon " + dur0 + "ms, bitti " + animEnd + "ms, " + (viaState ? "pencereler hazır" : "komut " + (task.IsCompleted ? "bitti" : "sürüyor")) + ")");
            if (warpMon != null) WarpInto(warpMon, warpWs); // odak yandaki monitöre geçti
            return;
        }

        RingsAttach(oldThumbs, FocusedTop());
        foreach (var t in oldThumbs) RingPlace(t, t.Dest, 255);
        PinsAttach(new Rectangle(mx, my + barH, mw, mh - barH), ox, oy, thumbs);
        overlay.Reveal();
        RaisePinned();
        Native.DwmFlush();
        Log("shown " + clock.ElapsedMilliseconds + "ms");

        foreach (var c in commands) tiling.Command(c);
        Log("commanded " + clock.ElapsedMilliseconds + "ms");

        // tiling komuta hemen "tamam" der ama workspace'i birkaç ms sonra değiştirir:
        // gösterilen workspace değişene kadar kısa aralıklarla tekrar sor (en fazla ~250ms).
        Dictionary<string, object> newWs = null;
        for (int tries = 0; tries < 25; tries++)
        {
            mons = tiling.Monitors();
            newWs = null;
            foreach (var m in mons)
                if (J.Str(m, "id") == J.Str(mon, "id"))
                    foreach (Dictionary<string, object> w in J.Children(m))
                        if (J.Bool(w, "isDisplayed")) newWs = w;
            if (newWs != null && J.Str(newWs, "name") != oldName) break;
            Thread.Sleep(10);
        }
        Log("switched " + clock.ElapsedMilliseconds + "ms -> " + (newWs != null ? J.Str(newWs, "name") : "?"));

        int dir = dirHint;
        if (newWs != null && dir == 0)
        {
            int a, b;
            if (int.TryParse(oldName, out a) && int.TryParse(J.Str(newWs, "name"), out b)) dir = b > a ? 1 : -1;
            else dir = 1;
        }

        if (newWs != null && J.Str(newWs, "name") != oldName)
        {
            var newWins = new List<IntPtr>();
            J.Windows(newWs, newWins);
            foreach (var h in newWins)
            {
                var t = RegisterWindow(h, ox, oy);
                if (t != null) { newThumbs.Add(t); thumbs.Add(t); RingAdd(t, false); Move(t, dir * (mw + GAP)); }
            }
            RingsFocus(WorkspaceFocusHandle(newWs));
            foreach (var t in newThumbs) Move(t, dir * (mw + GAP));
            Native.DwmFlush();

            var pc = new PresentClock();
            while (!Interrupt)
            {
                double p = Prog(pc.Ms(), Anims.Workspaces.Ms);
                double e = Anims.Workspaces.Curve.At(p);
                int shift = (int)Math.Round(e * (mw + GAP));
                foreach (var t in oldThumbs) Move(t, -dir * shift);
                foreach (var t in newThumbs) Move(t, dir * (mw + GAP) - dir * shift);
                Native.DwmFlush();
                if (p >= 1.0) break;
            }
        }

        overlay.Conceal(); RingsClear(); PinsClear();
        foreach (var t in thumbs) Unregister(t.Id);
        Log("done " + clock.ElapsedMilliseconds + "ms new=" + newThumbs.Count);
        if (warpMon != null) WarpInto(warpMon, warpWs);
    }

    // ---- Parmakla kaydırma (dokunmatik yüzey; Hyprland workspace_swipe) ----
    // Workspace parmakla birlikte kayar. İki komşu workspace'in önizlemeleri de baştan hazırlanır: parmak hangi yöne
    // giderse o yan görünür, yön ortada değişebilir. Her dokunma raporunda yalnızca konumlar güncellenir (bekleme
    // yok, DWM kendi hızında birleştirir). Bırakınca yolun %30'unu geçtiyse ya da parmak o yöne hızlıysa geçiş
    // tamamlanır, değilse geri döner. Komşu: bir önceki / sonraki numara; başka monitördeki workspace
    // (gizli olsa da) buraya çekilmez (kenar sayılır). UI thread'inde çalışır (dokunma girdisi de orada gelir).
    sealed class SwipeScene
    {
        public IntPtr OldFocus, PrevFocus, NextFocus;
        public string OldName, PrevName, NextName;
        public int Mw;
        public readonly List<Thumb> All = new List<Thumb>(), Old = new List<Thumb>(), Prev = new List<Thumb>(), Next = new List<Thumb>();
        public double Progress;
    }
    SwipeScene swipe;
    public bool Swiping { get { return swipe != null; } }

    // Başka monitördeki workspace (gösterilsin ya da gizli olsun): WM onu kendi monitöründe açar, bu monitörde kaydırılamaz
    static bool LivesElsewhere(List<Dictionary<string, object>> mons, Dictionary<string, object> mon, string name)
    {
        foreach (var m in mons)
            if (J.Str(m, "id") != J.Str(mon, "id"))
                foreach (Dictionary<string, object> w in J.Children(m))
                    if (J.Str(w, "name") == name) return true;
        return false;
    }

    static Dictionary<string, object> WorkspaceNode(List<Dictionary<string, object>> mons, string name)
    {
        foreach (var m in mons)
            foreach (Dictionary<string, object> w in J.Children(m))
                if (J.Str(w, "name") == name) return w;
        return null;
    }

    public bool SwipeBegin()
    {
        swipeTouched = Environment.TickCount;
        try { return SwipeBeginCore(); }
        catch (Exception ex) { Recover("parmakla kaydırma başlangıcı: " + ex.Message); return false; }
    }

    bool SwipeBeginCore()
    {
        if (swipe != null) return true;
        if (!Prefs.Animations || Anims.Workspaces.Ms <= 0) return false;
        var clock = Stopwatch.StartNew();
        var mons = tiling.Monitors();
        Dictionary<string, object> oldWs;
        var mon = FocusedMonitor(mons, out oldWs);
        int cur;
        if (mon == null || oldWs == null || !int.TryParse(J.Str(oldWs, "name"), out cur)) return false;
        string prevCandidate = AdjacentWorkspace(mons, mon, cur.ToString(), -1);
        string nextCandidate = AdjacentWorkspace(mons, mon, cur.ToString(), 1);
        string prevName = prevCandidate != null && !LivesElsewhere(mons, mon, prevCandidate) ? prevCandidate : null;
        string nextName = nextCandidate != null && !LivesElsewhere(mons, mon, nextCandidate) ? nextCandidate : null;

        int mx = J.Int(mon, "x"), my = J.Int(mon, "y"), mw = J.Int(mon, "width"), mh = J.Int(mon, "height");
        int barH = BarPx(mx + mw / 2, my + mh / 2);
        Interlocked.Increment(ref Gen);
        UseOverlay(new Rectangle(mx, my + barH, mw, mh - barH));
        int ox = mx, oy = my + barH;
        var s = new SwipeScene { OldName = cur.ToString(), PrevName = prevName, NextName = nextName, Mw = mw };
        s.OldFocus = WorkspaceFocusHandle(oldWs);
        s.PrevFocus = WorkspaceFocusHandle(WorkspaceNode(mons, prevName));
        s.NextFocus = WorkspaceFocusHandle(WorkspaceNode(mons, nextName));

        // Duvar kağıdı (sabit)
        Native.RECT wsrc;
        IntPtr wall = WallpaperSource(out wsrc);
        if (wall != IntPtr.Zero)
        {
            var src = new Native.RECT { Left = mx - wsrc.Left, Top = oy - wsrc.Top, Right = mx - wsrc.Left + mw, Bottom = oy - wsrc.Top + mh - barH };
            var t = Register(wall, new Native.RECT { Left = 0, Top = 0, Right = mw, Bottom = mh - barH }, src);
            if (t != null) s.All.Add(t);
        }
        DesktopWidgetsAttach(s.All, new Rectangle(mx, oy, mw, mh - barH), ox, oy);
        var oldWins = new List<IntPtr>();
        J.Windows(oldWs, oldWins);
        foreach (var h in oldWins)
        {
            Native.RECT r;
            if (!Native.IsWindowVisible(h) || !Native.GetWindowRect(h, out r)) continue;
            var t = RegisterWindow(h, ox, oy);
            if (t != null) { s.All.Add(t); s.Old.Add(t); }
        }
        foreach (int side in new[] { -1, 1 })
        {
            string name = side < 0 ? prevName : nextName;
            var target = name == null ? null : WorkspaceNode(mons, name);
            if (target == null) continue; // henüz yok: boş workspace, yalnızca duvar kağıdı
            var tw = new List<Dictionary<string, object>>();
            J.WindowNodes(target, tw);
            foreach (var w in tw)
            {
                object st; var state = w.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
                if (state != null && J.Str(state, "type") == "minimized") continue;
                var t = RegisterWindow(new IntPtr(Convert.ToInt64(w["handle"])), ox, oy);
                if (t != null) { s.All.Add(t); (side < 0 ? s.Prev : s.Next).Add(t); }
            }
        }
        swipe = s;
        RingsAttach(s.All, s.OldFocus);
        SwipePlace(0);
        PinsAttach(new Rectangle(mx, my + barH, mw, mh - barH), ox, oy, s.All);
        overlay.Reveal();
        RaisePinned();
        Animating = true;
        Log("parmakla kaydırma: hazır " + clock.ElapsedMilliseconds + " ms, önceki " + (prevName ?? "-") + ", sonraki " + (nextName ?? "-") + ", " + s.All.Count + " önizleme");
        return true;
    }

    void SwipePlace(double p)
    {
        var s = swipe;
        if (s == null) return;
        RingsFocus(WorkspacePreviewFocus(p, s.OldFocus, s.PrevFocus, s.NextFocus, s.PrevName != null, s.NextName != null));
        int span = s.Mw + GAP;
        int shift = (int)Math.Round(p * span);
        foreach (var t in s.Old) Move(t, -shift);
        foreach (var t in s.Next) Move(t, span - shift);
        foreach (var t in s.Prev) Move(t, -span - shift);
    }

    // p: -1 (önceki workspace) .. +1 (sonraki). O yönde komşu yoksa lastik gibi biraz kayıp durur.
    public void SwipeUpdate(double p)
    {
        var s = swipe;
        if (s == null || double.IsNaN(p)) return;
        swipeTouched = Environment.TickCount;
        const double RUBBER = 0.06;
        if (p > 0 && s.NextName == null) p = RUBBER * (1 - Math.Exp(-p / RUBBER));
        else if (p < 0 && s.PrevName == null) p = -RUBBER * (1 - Math.Exp(p / RUBBER));
        p = Math.Max(-1, Math.Min(1, p));
        s.Progress = p;
        SwipePlace(p);
    }

    // Parmaklar kalktı. velocity: ilerleme / ms (+ sonrakine doğru).
    public void SwipeEnd(double velocity)
    {
        swipeTouched = Environment.TickCount;
        try { SwipeEndCore(velocity); }
        catch (Exception ex) { Recover("parmakla kaydırma sonu: " + ex.Message); }
    }

    void SwipeEndCore(double velocity)
    {
        var s = swipe;
        if (s == null) return;
        const double COMMIT = 0.2, FLICK = 0.0015; // hızlı fiske: ~0.7 sn'de bir workspace boyu
        if (double.IsNaN(velocity)) velocity = 0;
        double p = s.Progress;
        int target = 0;
        if (s.NextName != null && ((p > COMMIT && velocity > -FLICK) || (p > 0 && velocity > FLICK))) target = 1;
        else if (s.PrevName != null && ((p < -COMMIT && velocity < FLICK) || (p < 0 && velocity < -FLICK))) target = -1;
        string name = target > 0 ? s.NextName : target < 0 ? s.PrevName : null;
        Task task = null;
        if (name != null) task = Task.Factory.StartNew(() => tiling.Command("focus --workspace " + name));

        // Kalan yol workspace hareketinin eğrisiyle; süre kalan yolla orantılı (en az 120 ms)
        var spec = Anims.Workspaces;
        double from = p, to = target;
        int dur = spec.Ms <= 0 ? 0 : Math.Max(120, (int)(Math.Abs(to - from) * spec.Ms));
        var pc = new PresentClock();
        var fs = new FrameStats();
        int frames = 0;
        while (dur > 0)
        {
            fs.Begin(); frames++;
            double q = Prog(pc.Ms(), dur);
            SwipePlace(from + (to - from) * spec.Curve.At(q));
            fs.Updated();
            Native.DwmFlush();
            fs.Flushed();
            if (q >= 1.0) break;
        }
        // Katmanı gerçek durum hazır olunca kaldır (Run'daki gibi): eski workspace'in pencereleri gizlenmiş, yenininkiler
        // görünür olduğu an
        if (task != null)
        {
            var arriving = target > 0 ? s.Next : s.Prev;
            Func<IntPtr, bool> cloaked = hw => { int cv; return Native.DwmGetWindowAttribute(hw, Native.DWMWA_CLOAKED, out cv, 4) == 0 && cv != 0; };
            var waitSw = Stopwatch.StartNew();
            while (!task.IsCompleted && waitSw.ElapsedMilliseconds < 1500)
            {
                bool done = s.Old.Count + arriving.Count > 0;
                foreach (var t in s.Old) if (!cloaked(t.Src)) { done = false; break; }
                if (done) foreach (var t in arriving) if (cloaked(t.Src)) { done = false; break; }
                if (done) break;
                Thread.Sleep(4);
            }
        }
        SwipeClear();
        Log("parmakla kaydırma: " + (name == null ? "geri döndü" : "-> " + name) + ", bırakılan yer " + p.ToString("0.00") + ", hız " + (velocity * 1000).ToString("0.00") + "/sn, " + frames + " kare " + fs.Report());
    }

    // Klavyeyle kaydırma ya da taşıma başlarken süren parmak kaydırması hemen kapanır
    public void SwipeAbort()
    {
        if (swipe == null) return;
        SwipeClear();
        Log("parmakla kaydırma: yarıda bırakıldı");
    }

    void SwipeClear()
    {
        var s = swipe;
        if (s == null) return;
        swipe = null;
        overlay.Conceal(); RingsClear(); PinsClear();
        foreach (var t in s.All) Unregister(t.Id);
        Animating = false;
    }
}

// ---------------- Dwindle (Hyprland varsayılan layout'u) ----------------
// Hyprland dwindle: yeni pencere, odaktaki pencereyi UZUN kenarı boyunca ikiye böler
// (geniş -> yan yana, uzun -> alt alta) ve içe dönen bir spiral oluşur. tiling'de bu layout
// yok; odak her değiştiğinde odaktaki pencerenin en/boy oranına göre tiling yönünü ayarlıyoruz,
// böylece bir sonraki pencere dwindle'daki gibi yerleşiyor.
class Dwindle
{
    readonly TilingClient tiling;
    readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    public Dwindle(TilingClient g) { tiling = g; }

    // ---- Aç/kapa animasyonu için yerleşim hafızası ----
    Control ui; Slider slider;
    int animSeq;
    readonly TilingClient cacheTiling = new TilingClient();
    readonly object cacheLock = new object();
    Dictionary<long, Native.RECT> rects = new Dictionary<long, Native.RECT>();   // görünen pencereler
    Dictionary<long, string> monOf = new Dictionary<long, string>();              // pencere -> monitör id
    Dictionary<string, Rectangle> monRects = new Dictionary<string, Rectangle>();

    // Önbellek yalnızca bir şey değişince tazelenir: pencere yöneticisinden bir olay ya da görünen bir pencerenin yer
    // değiştirmesi (klavyeyle boyutlandırma vb.); olay yağmuru 30 ms'de birleşir. Bağlıyken 30 sn'lik güvenlik ağı,
    // bağlantı yokken 2 sn'lik kurtarma yoklaması; salt odak olayı geometriyi değiştirmez.
    // Önceden 300 ms'de bir tüm ağaç sorgulanıp JSON'u ayrıştırılıyordu: boşta bile iki süreçte sürekli iş, çekirdekte
    // sürekli çöp (kaymalarda çöp toplama duraklamaları).
    static readonly AutoResetEvent cacheDirty = new AutoResetEvent(true);
    volatile bool cacheConnected;
    volatile bool cacheHealthy;
    static int CacheWaitMs(bool connected, bool healthy) { return connected && healthy ? 30000 : 2000; }
    static bool SnapshotEvent(string eventType) { return eventType != "focus_changed"; }
    static volatile Dwindle current;
    public static void MarkDirty() { cacheDirty.Set(); }
    // Köşe yuvarlayıcının konum olayından (kendi thread'i): görünen, yönetilen bir pencereyse
    public static void WindowMoved(IntPtr h)
    {
        var d = current;
        if (d == null) return;
        bool known;
        lock (d.cacheLock) known = d.rects.ContainsKey(h.ToInt64());
        if (known) cacheDirty.Set();
    }

    public Dwindle(TilingClient g, Control ui, Slider slider) : this(g)
    {
        this.ui = ui; this.slider = slider;
        current = this;
        var t = new Thread(() =>
        {
            while (true)
            {
                cacheDirty.WaitOne(CacheWaitMs(cacheConnected, cacheHealthy));
                Thread.Sleep(30);
                while (Slider.Animating) Thread.Sleep(50); // animasyon bitince bir kez
                try { RefreshCache(); } catch { cacheHealthy = false; }
            }
        }) { IsBackground = true, Name = "dwindle-cache" };
        t.Start();
    }

    HashSet<long> tiledSet = new HashSet<long>(); // görünen workspace'lerdeki döşeli pencereler (kapanma ön-dondurması için)

    void Snapshot(out Dictionary<long, Native.RECT> r, out Dictionary<long, string> m, out Dictionary<string, Rectangle> mr, int gen = -1)
    {
        r = new Dictionary<long, Native.RECT>(); m = new Dictionary<long, string>(); mr = new Dictionary<string, Rectangle>();
        var tiled = new HashSet<long>();
        var monitors = cacheTiling.Monitors();
        if (monitors.Count == 0) throw new InvalidOperationException("WM snapshot unavailable");
        foreach (var mon in monitors)
        {
            string mid = J.Str(mon, "id");
            mr[mid] = new Rectangle(J.Int(mon, "x"), J.Int(mon, "y"), J.Int(mon, "width"), J.Int(mon, "height"));
            foreach (Dictionary<string, object> ws in J.Children(mon))
            {
                if (!J.Bool(ws, "isDisplayed")) continue;
                var wins = new List<Dictionary<string, object>>();
                J.WindowNodes(ws, wins);
                foreach (var w in wins)
                {
                    object hv;
                    if (!w.TryGetValue("handle", out hv)) continue;
                    long h = Convert.ToInt64(hv);
                    int x = J.Int(w, "x"), y = J.Int(w, "y");
                    r[h] = new Native.RECT { Left = x, Top = y, Right = x + J.Int(w, "width"), Bottom = y + J.Int(w, "height") };
                    m[h] = mid;
                    object st; var state = w.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
                    if (state != null && J.Str(state, "type") == "tiling") tiled.Add(h);
                }
            }
        }
        lock (cacheLock) if (gen == -1 || (gen == Slider.Gen && !Slider.Animating)) tiledSet = tiled;
    }

    // Pencerelerin ekranda görünen dikdörtgenleri (GetWindowRect): açma/kapama animasyonu diğer pencereleri
    // tam da görüldükleri eski yerlerinden kaydırsın
    Dictionary<long, Native.RECT> visual = new Dictionary<long, Native.RECT>();
    static Dictionary<long, Native.RECT> Visual(IEnumerable<long> handles)
    {
        var v = new Dictionary<long, Native.RECT>();
        foreach (var h in handles) { var hw = new IntPtr(h); if (Native.IsWindow(hw)) v[h] = Slider.WinRect(hw); }
        return v;
    }

    void RefreshCache()
    {
        int gen = Slider.Gen;
        Dictionary<long, Native.RECT> r; Dictionary<long, string> m; Dictionary<string, Rectangle> mr;
        Snapshot(out r, out m, out mr, gen);
        var v = Visual(r.Keys);
        lock (cacheLock)
        {
            if (gen != Slider.Gen || Slider.Animating) { cacheDirty.Set(); return; } // sorgu sürerken animasyon başladı: bu sonuç eski, sonra yine
            rects = r; monOf = m; monRects = mr; visual = v; cacheHealthy = true;
        }
    }

    // ---- Yeni pencere: Windows onu önce kendi varsayılan yerinde (ortada) gösterir, tiling birkaç on ms sonra
    // yerleştirir. Hyprland pencereyi son yerini alana kadar hiç göstermez: burada da pencere görünür olduğu an
    // (EVENT_OBJECT_SHOW) ekranı mevcut pencerelerle donduruyoruz; ortadaki pencere katmanın altında kalır,
    // tiling yer açınca son yerinde %80'den büyüyüp belirir. Yönetilmezse (açılış ekranı vb.) 0.9 sn'de kalkar.
    Native.WinEventDelegate showCb;
    readonly object pendLock = new object();
    Slider.Frozen pendFrozen;
    long pendHandle;
    int pendAt;
    static readonly HashSet<string> noFreezeProcs = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { Names.Shell, Names.Core, Names.Tiling, "ShellExperienceHost", "SearchUI", "SearchApp", "StartMenuExperienceHost",
          "LockApp", "TextInputHost", "ApplicationFrameHost", "lunge-songrec", "lunge-termcolors" };

    public void HookNewWindows()
    {
        if (ui == null) return;
        ui.BeginInvoke((Action)(() =>
        {
            showCb = Callback.Guard("yeni pencere olayı", OnWinEvent);
            // EVENT_OBJECT_DESTROY (0x8001) .. EVENT_OBJECT_SHOW (0x8002) .. EVENT_OBJECT_HIDE (0x8003)
            Native.SetWinEventHook(0x8001, 0x8003, IntPtr.Zero, showCb, 0, 0, 0x0002 | 0x0000); // OUTOFCONTEXT
        }));
    }

    void OnWindowShown(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        try
        {
            if (idObject != 0 || hwnd == IntPtr.Zero) return;
            if (Native.GetAncestor(hwnd, 2) != hwnd || !Native.IsWindowVisible(hwnd)) return;
            long h = hwnd.ToInt64();
            if (Slider.Animating || !Prefs.Animations) return;
            lock (cacheLock) if (rects.ContainsKey(h)) return;               // zaten yönetilen pencere (workspace dönüşü vb.)
            lock (pendLock) if (pendFrozen != null) return;
            int style = Native.GetWindowLong(hwnd, Native.GWL_STYLE), ex = Native.GetWindowLong(hwnd, Native.GWL_EXSTYLE);
            // Yalnızca döşenecek türden uygulama pencereleri (AutoFloat'ın yüzdürmeyeceği)
            if ((style & Native.WS_CAPTION) != Native.WS_CAPTION || (style & 0x00040000) == 0) return;
            if ((ex & Native.WS_EX_TOOLWINDOW) != 0 || (ex & Native.WS_EX_NOACTIVATE) != 0) return;
            if (Native.GetWindow(hwnd, 4) != IntPtr.Zero) return; // sahibi olan (diyalog)
            uint pid; Native.GetWindowThreadProcessId(hwnd, out pid);
            string proc;
            proc = ProcInfo.Name(pid); if (proc.Length == 0) return;
            if (noFreezeProcs.Contains(proc)) return;

            // Yeni pencere farenin olduğu monitördeki odaktaki workspace'e gelir (LaunchQueue fare altını odaklar)
            var cur = Cursor.Position;
            string mid = null; Rectangle mon = Rectangle.Empty;
            Dictionary<long, Native.RECT> vis; Dictionary<long, string> mo;
            lock (cacheLock)
            {
                foreach (var kv in monRects) if (kv.Value.Contains(cur)) { mid = kv.Key; mon = kv.Value; }
                vis = visual; mo = monOf;
            }
            if (mid == null) return;
            var hs = new List<long>();
            foreach (var kv in mo) if (kv.Value == mid) hs.Add(kv.Key);
            var start = new Dictionary<long, Native.RECT>();
            foreach (var x in hs) { Native.RECT r; if (vis.TryGetValue(x, out r)) start[x] = r; }

            var f = slider.Freeze(mon, hs, start, 0);   // UI thread'indeyiz
            lock (pendLock) { pendFrozen = f; pendHandle = h; pendAt = Environment.TickCount; }
            Slider.Log("yeni pencere dondu: " + proc);

            var timer = new System.Windows.Forms.Timer { Interval = 900 };
            timer.Tick += (s, e) =>
            {
                timer.Stop(); timer.Dispose();
                Slider.Frozen left = null;
                lock (pendLock) { if (pendFrozen == f) { left = f; pendFrozen = null; } }
                if (left != null)
                {
                    List<long> now; lock (cacheLock) now = new List<long>(rects.Keys);
                    slider.Finish(left, now, 0, 120); // yönetilmedi: katmanı yumuşakça kaldır
                }
            };
            timer.Start();
        }
        catch (Exception ex2) { Slider.Log("show hook: " + ex2.Message); }
    }

    void OnWinEvent(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        EventLag.Note("pencere", time);
        if (ev == Native.EVENT_OBJECT_SHOW) { OnWindowShown(hook, ev, hwnd, idObject, idChild, thread, time); return; }
        if (idObject != 0 || idChild != 0 || hwnd == IntPtr.Zero) return;
        try { OnWindowGone(hwnd); } catch (Exception ex) { Slider.Log("gone hook: " + ex.Message); }
    }

    // ---- Pencere kapanıyor / gizleniyor: Windows'un olayı tiling'in bildiriminden ~30-40 ms önce gelir; o arada tiling
    // kalan pencereleri yeniden yerleştirdiği için pencereler animasyon başlamadan zıplıyordu. Ekranı hemen, pencerelerin
    // görüldükleri yerlerde donduruyoruz; bildirim gelince AnimateChange bu katmanı alıp kaydırır. Gelmezse 0,4 sn'de kalkar.
    // Donma katmanı, beklenen yeni pencere yok olunca ya da kural onu yüzdürünce / tam ekran yapınca hemen kalkar.
    // Önceden 900 ms'lik güvenlik zamanlayıcısını bekliyordu: kısa ömürlü bir pencere (WezTerm'in açarken gösterdiği
    // yardımcı pencere) ekranı ~1 sn dondurup asıl pencereyi donmuş görüntünün altında bırakıyordu; o sürede yeni
    // pencere de dondurulamıyordu (açılış animasyonu oynuyor, terminal görünmüyordu).
    void ReleasePending(long h, string why)
    {
        Slider.Frozen left = null;
        lock (pendLock) { if (pendFrozen != null && pendHandle == h) { left = pendFrozen; pendFrozen = null; } }
        if (left == null) return;
        Slider.Log("donma bırakıldı: " + why);
        ui.BeginInvoke((Action)(() =>
        {
            try { List<long> now; lock (cacheLock) now = new List<long>(rects.Keys); slider.Finish(left, now, 0, 120); }
            catch (Exception ex) { Slider.Log("donma bırakma: " + ex.Message); }
        }));
    }

    void OnWindowGone(IntPtr hwnd)
    {
        long h = hwnd.ToInt64();
        ReleasePending(h, "yeni pencere hemen kapandı");
        if (Slider.Animating || !Prefs.Animations) return;
        string mid; Rectangle mon;
        Dictionary<long, Native.RECT> vis; Dictionary<long, string> mo; HashSet<long> tl;
        lock (cacheLock)
        {
            if (!tiledSet.Contains(h) || !monOf.TryGetValue(h, out mid) || !monRects.TryGetValue(mid, out mon)) return;
            vis = visual; mo = monOf; tl = tiledSet;
        }
        lock (pendLock) if (pendFrozen != null) return;
        var hs = new List<long>();
        bool otherTiled = false;
        foreach (var kv in mo)
            if (kv.Value == mid && kv.Key != h) { hs.Add(kv.Key); if (tl.Contains(kv.Key)) otherTiled = true; }
        if (!otherTiled) return; // yer değiştirecek başka döşeli pencere yok
        var start = new Dictionary<long, Native.RECT>();
        foreach (var x in hs) { Native.RECT r; if (vis.TryGetValue(x, out r)) start[x] = r; }

        var f = slider.Freeze(mon, hs, start, 0); // UI thread'indeyiz
        lock (pendLock) { pendFrozen = f; pendHandle = h; pendAt = Environment.TickCount; }
        Slider.Log("kapanan pencere dondu");

        var timer = new System.Windows.Forms.Timer { Interval = 400 };
        timer.Tick += (s, e) =>
        {
            timer.Stop(); timer.Dispose();
            Slider.Frozen left = null;
            lock (pendLock) { if (pendFrozen == f) { left = f; pendFrozen = null; } }
            if (left != null)
            {
                List<long> now; lock (cacheLock) now = new List<long>(rects.Keys);
                slider.Finish(left, now, 0, 120); // tiling bildirmedi: katmanı yumuşakça kaldır
            }
        };
        timer.Start();
    }

    Slider.Frozen TakePending(long h)
    {
        lock (pendLock)
        {
            if (pendFrozen == null || pendHandle != h || Environment.TickCount - pendAt > 2000) return null;
            var f = pendFrozen; pendFrozen = null; return f;
        }
    }

    // Pencere açıldı/kapandı. tiling yerleşimi zaten değiştirdi; katmanı hemen, pencerelerin GÖRÜLDÜKLERİ eski
    // yerlerinden (önbellek) açıp arkada fareye göre yerleştirmeyi de yapıyoruz, sonra hepsi gerçek yerine kayar.
    // (Eskiden: önce zıplama, 60 ms sonra katman, sonra fareye göre ikinci zıplama ve 400 ms sonra üçüncüsü.)
    // ---- A window's state changing (Super+F fullscreen, floating <-> tiling, the spoofed fullscreen): as Hyprland,
    // which animates the window's last picture from its old place to its new one and shows the app's fresh picture
    // when it comes, the app never repainting along the way. The screen is frozen with the windows where they are
    // (where their pictures are, if a layout animation is still playing: a quick second press goes on from there),
    // the window manager changes the state behind the layer, and the pictures move to the new layout.
    static readonly string[] stateCommands = {
        "toggle-fullscreen", "set-fullscreen", "toggle-floating", "set-floating", "toggle-tiling", "set-tiling",
        "toggle-fullscreen-spoof" };
    public static bool ChangesState(string[] commands)
    {
        foreach (var c in commands)
            foreach (var s in stateCommands)
                if (c == s || c.StartsWith(s + " ")) return true;
        return false;
    }

    // From the keyboard hook's worker thread: false when the commands were not run here
    public static bool AnimateState(string[] commands)
    {
        var d = current;
        if (d == null || d.ui == null || !Prefs.Animations || !ChangesState(commands)) return false;
        d.AnimateStateCore(commands);
        return true;
    }

    void AnimateStateCore(string[] commands)
    {
        var clk = Stopwatch.StartNew();
        long fh = Native.GetAncestor(Native.GetForegroundWindow(), 2).ToInt64();
        Dictionary<long, string> beforeMon; Dictionary<string, Rectangle> beforeMr;
        lock (cacheLock) { beforeMon = monOf; beforeMr = monRects; }
        string mid; Rectangle mon;
        if (!beforeMon.TryGetValue(fh, out mid) || !beforeMr.TryGetValue(mid, out mon)) { foreach (var c in commands) tiling.Command(c); return; }
        var hs = new List<long>();
        foreach (var kv in beforeMon) if (kv.Value == mid) hs.Add(kv.Key);
        var start = Slider.ShownNow(hs);
        // Going into or out of fullscreen the window covers the bar at one end: the layer covers it too
        Native.RECT fr;
        bool whole = start.TryGetValue(fh, out fr) && Slider.CoversMonitor(fr, mon.X, mon.Y, mon.Width, mon.Height)
            || Array.Exists(commands, c => c.Contains("fullscreen"));
        Slider.Frozen f = null;
        slider.Interrupt = true; // a layout animation still playing ends now (its pictures' places are in `start`)
        try { f = (Slider.Frozen)ui.Invoke((Func<Slider.Frozen>)(() => slider.Freeze(mon, hs, start, 0, whole))); }
        catch (Exception ex) { Slider.Log("durum donması: " + ex.Message); }
        foreach (var c in commands) tiling.Command(c);
        if (f == null) { RefreshCache(); return; }
        Dictionary<long, Native.RECT> after; Dictionary<long, string> afterMon; Dictionary<string, Rectangle> mr;
        try { Snapshot(out after, out afterMon, out mr); }
        catch (Exception ex)
        {
            Slider.Log("durum: " + ex.Message);
            ui.BeginInvoke((Action)(() => { try { slider.Finish(f, new List<long>(), 0, 1); } catch { } }));
            return;
        }
        var end = new List<long>();
        foreach (var kv in afterMon) if (kv.Value == mid) end.Add(kv.Key);
        var v = new Dictionary<long, Native.RECT>();
        foreach (var kv in after) v[kv.Key] = Slider.WindowRectForFrame(new IntPtr(kv.Key), kv.Value);
        lock (cacheLock) { rects = after; monOf = afterMon; monRects = mr; visual = v; }
        Slider.Log("durum değişti: " + string.Join(", ", commands) + ", donma+komut " + clk.ElapsedMilliseconds + " ms");
        ui.BeginInvoke((Action)(() =>
        {
            try { slider.Finish(f, end, 0, Slider.MoveMs, after); } catch (Exception ex) { Slider.Log("durum animasyonu: " + ex.Message); }
        }));
    }

    void AnimateChange(long anchorHandle, bool opened, Dictionary<string, object> win)
    {
        var clk = Stopwatch.StartNew();
        if (ui == null || !Prefs.Animations) return;
        Dictionary<long, Native.RECT> beforeVis; Dictionary<long, string> beforeMon;
        lock (cacheLock) { beforeVis = visual; beforeMon = monOf; }

        Dictionary<long, Native.RECT> after = null; Dictionary<long, string> afterMon = null; Dictionary<string, Rectangle> mr = null;
        string mid = null;
        Rectangle mon = Rectangle.Empty;
        var hs = new List<long>();
        var start = new Dictionary<long, Native.RECT>();
        long pop = opened ? anchorHandle : 0;

        // Açılışta SHOW, kapanışta HIDE/DESTROY anında dondurulmuş olabilir. Kapanış dondurulduysa ilk tiling sorgusu
        // gereksiz (monitör önbellekte): animasyon ~15-20 ms erken başlar.
        Slider.Frozen f = TakePending(anchorHandle);
        bool preClosed = f != null && !opened;
        if (preClosed)
        {
            mon = f.Mon;
            lock (cacheLock) foreach (var kv in monRects) if (kv.Value == f.Mon) mid = kv.Key;
            if (mid == null) preClosed = false;
        }
        if (!preClosed)
        {
            Snapshot(out after, out afterMon, out mr);
            if (!(opened ? afterMon : beforeMon).TryGetValue(anchorHandle, out mid) || !mr.TryGetValue(mid, out mon))
            {
                if (f != null) { var left = f; try { ui.Invoke((Action)(() => slider.Finish(left, new List<long>(), 0, 1))); } catch { } }
                RefreshCache();
                return;
            }
            foreach (var kv in afterMon) if (kv.Value == mid) hs.Add(kv.Key);
            foreach (var h in hs) { Native.RECT r; if (beforeVis.TryGetValue(h, out r)) start[h] = r; }
        }
        if (f != null && f.Mon != mon)
        {
            // Pencere başka monitöre geldi: o katmanı hemen kaldır, bu monitörde yeniden dondur
            var wrong = f; f = null;
            try { ui.Invoke((Action)(() => slider.Finish(wrong, new List<long>(), 0, 1))); } catch { }
        }
        if (f == null)
        {
            slider.Interrupt = true;
            // Kapanma kancası bu arada dondurmuş olabilir (ikisi de UI thread'inde sırayla çalışır): iki katman olmasın.
            // Açılış / kapanış kancası dondururken (UI thread'i ~50 ms meşgul) buraya gelindiyse bekleyen katman ancak
            // Invoke içinde bulunur: o zaman Freeze çalışmaz ve yukarıdaki Interrupt kalkık kalıp animasyonu 0 karede
            // bitiriyordu (yeni pencere hiç büyüyerek açılmıyordu). Burada UI thread'indeyiz, süren animasyon yok: indir.
            try
            {
                f = (Slider.Frozen)ui.Invoke((Func<Slider.Frozen>)(() =>
                {
                    var pending = TakePending(anchorHandle);
                    if (pending == null) return slider.Freeze(mon, hs, start, pop);
                    slider.Interrupt = false;
                    return pending;
                }));
            }
            catch (Exception ex) { Slider.Log("freeze: " + ex.Message); }
        }


        Snapshot(out after, out afterMon, out mr);
        var end = new List<long>();
        foreach (var kv in afterMon) if (kv.Value == mid) end.Add(kv.Key);
        // Pencerelerin yerleşmesi beklenmez (önceden 16-500 ms): hedef tiling'in yerleşimi, pencere yer değiştirdiği
        // an gerçek yeri (Finish). Önbellekteki görünür dikdörtgenler de hedef yerleşimden hesaplanır.
        var v = new Dictionary<long, Native.RECT>();
        foreach (var kv in after) v[kv.Key] = Slider.WindowRectForFrame(new IntPtr(kv.Key), kv.Value);
        lock (cacheLock) { rects = after; monOf = afterMon; monRects = mr; visual = v; }
        Slider.Log((opened ? "açıldı" : "kapandı") + ": " + start.Count + "->" + end.Count + " pencere, donma+hedef " + clk.ElapsedMilliseconds + " ms" + (f == null ? " (katman yok)" : ""));
        // Yeni pencere açılınca fare yerinde kalır (odak yeni pencerede): sonraki pencere de farenin altındaki pencereyi
        // farenin bulunduğu yarısından böler (Hyprland dwindle, force_split = 0). Fare yeni pencerenin ortasına
        // taşındığında her yeni pencere bir öncekini ortadan bölüyordu.
        if (f != null)
            ui.BeginInvoke((Action)(() =>
            {
                try { slider.Finish(f, end, pop, Slider.MoveMs, after); } catch (Exception ex) { Slider.Log("anim: " + ex.Message); }
            }));
    }
    // Uygulamaya özel kural yerine GENEL karar (Hyprland da sabit boyutlu pencereleri ve
    // diyalogları kendiliğinden yüzdürür):
    //   - tüm monitörü kaplayan çerçevesiz pencere (oyun)          -> tam ekran
    //   - sahibi olan pencere (diyalog, "İndirilenler" vb.)         -> yüzen
    //   - yeniden boyutlanamayan pencere (updater, istemci, sihirbaz) -> yüzen
    //   - başlıksız ve kenarsız açılır pencere (özel arayüzlü istemci) -> yüzen
    // Yüzen/tam ekran pencereler tiling ayarıyla her zaman üstte.
    bool AutoFloat(Dictionary<string, object> win)
    {
        object hv;
        if (!win.TryGetValue("handle", out hv) || hv == null) return false;
        IntPtr h = new IntPtr(Convert.ToInt64(hv));
        object st;
        var state = win.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
        if (state != null && J.Str(state, "type") != "tiling") return false; // tiling zaten yüzdürmüş

        int style = Native.GetWindowLong(h, Native.GWL_STYLE);
        bool caption = (style & Native.WS_CAPTION) == Native.WS_CAPTION;
        bool thick = (style & 0x00040000) != 0;      // WS_THICKFRAME
        bool popup = (style & Native.WS_POPUP) != 0;
        bool owned = Native.GetWindow(h, 4) != IntPtr.Zero; // GW_OWNER

        Native.RECT r;
        Native.GetWindowRect(h, out r);
        var scr = Screen.FromHandle(h).Bounds;
        bool coversMonitor = r.Left <= scr.Left && r.Top <= scr.Top && r.Right >= scr.Right && r.Bottom >= scr.Bottom;

        string id = J.Str(win, "id"), cmd = null;
        if (!caption && !thick && coversMonitor)
        {
            // Tam ekran yalnızca öyle kalan pencereye: bazı uygulamalar açılırken monitör boyunda çerçevesiz bir yardımcı
            // pencereyi bir an gösterip kapatıyor (WezTerm). Oyun 250 ms sonra da tam ekrandadır.
            string proc = J.Str(win, "processName"), title = J.Str(win, "title");
            ThreadPool.QueueUserWorkItem(_ =>
            {
                Thread.Sleep(250);
                try
                {
                    Native.RECT r2;
                    int st2 = Native.GetWindowLong(h, Native.GWL_STYLE);
                    bool still = Native.IsWindow(h) && Native.IsWindowVisible(h) && Native.GetWindowRect(h, out r2)
                        && (st2 & Native.WS_CAPTION) != Native.WS_CAPTION && (st2 & 0x00040000) == 0
                        && r2.Left <= scr.Left && r2.Top <= scr.Top && r2.Right >= scr.Right && r2.Bottom >= scr.Bottom;
                    if (!still) { Slider.Log("auto set-fullscreen atlandı (geçici pencere): " + proc + " | " + title); return; }
                    tiling.Command("--id " + id + " set-fullscreen");
                    Slider.Log("auto set-fullscreen: " + proc + " | " + title);
                }
                catch (Exception ex) { Slider.Log("auto set-fullscreen: " + ex.Message); }
            });
            return true;
        }
        else if (owned || !thick || (popup && !caption)) cmd = "set-floating --centered";
        if (cmd == null) return false;
        tiling.Command("--id " + id + " " + cmd);
        Slider.Log("auto " + cmd + ": " + J.Str(win, "processName") + " | " + J.Str(win, "title"));
        return true;
    }

    // ---- Odak geçmişi (Hyprland gibi): odaklı pencere kapanınca aynı workspace'te en son
    // odaklanan pencereye dön. tiling ağaçtaki komşuyu odaklıyordu; art arda Alt+F4'te
    // sıra karışıyor, ilk açılan pencere en sona kalmıyordu.
    const int AUTO_MS = 250; // kapanıştan hemen önce/sonra gelen odak, tiling'in otomatik seçimidir
    readonly List<string> mru = new List<string>();
    readonly List<KeyValuePair<string, long>> pending = new List<KeyValuePair<string, long>>();

    void CommitOld(long now)
    {
        var keep = new List<KeyValuePair<string, long>>();
        foreach (var p in pending)
        {
            if (unchecked((int)(now - p.Value)) > AUTO_MS) { mru.Remove(p.Key); mru.Insert(0, p.Key); }
            else keep.Add(p);
        }
        pending.Clear(); pending.AddRange(keep);
        if (mru.Count > 200) mru.RemoveRange(200, mru.Count - 200);
    }

    void OnFocused(string id)
    {
        if (string.IsNullOrEmpty(id)) return;
        long now = Environment.TickCount;
        CommitOld(now);
        pending.Add(new KeyValuePair<string, long>(id, now));
    }

    void OnClosed(string id)
    {
        if (string.IsNullOrEmpty(id)) return;
        long now = Environment.TickCount;
        CommitOld(now);
        bool wasFocused = (mru.Count > 0 && mru[0] == id) || pending.Exists(p => p.Key == id);
        // Kapanışın hemen öncesi/sonrasındaki odaklar tiling'in otomatik seçimi: geçmişe yazma
        pending.Clear();
        mru.Remove(id);
        if (!wasFocused) return;

        // Hedef: odaktaki workspace'te en son odaklanmış, hâlâ açık pencere
        var ids = new HashSet<string>();
        string focusedNow = null;
        foreach (var m in tiling.Monitors())
            foreach (Dictionary<string, object> ws in J.Children(m))
            {
                if (!J.Bool(ws, "hasFocus")) continue;
                var wins = new List<Dictionary<string, object>>();
                J.WindowNodes(ws, wins);
                foreach (var w in wins) { ids.Add(J.Str(w, "id")); if (J.Bool(w, "hasFocus")) focusedNow = J.Str(w, "id"); }
            }
        foreach (var candidate in mru)
        {
            if (!ids.Contains(candidate)) continue;
            if (candidate != focusedNow) tiling.Command("focus --container-id " + candidate);
            Slider.Log("close -> refocus " + (candidate == focusedNow ? "(zaten odakta)" : candidate));
            return;
        }
    }

    public void Start()
    {
        var t = new Thread(Loop) { IsBackground = true };
        t.Start();
    }

    void Loop()
    {
        while (true)
        {
            ClientWebSocket ws = null;
            try
            {
                ws = new ClientWebSocket();
                ws.Options.Proxy = null;
                if (!ws.ConnectAsync(new Uri("ws://127.0.0.1:6123"), CancellationToken.None).Wait(3000)) throw new TimeoutException("WM event connection timed out");
                var sub = Encoding.UTF8.GetBytes("sub --events focus_changed window_managed window_unmanaged focused_container_moved " +
                    "workspace_activated workspace_deactivated workspace_updated monitor_added monitor_updated monitor_removed tiling_direction_changed");
                cacheDirty.Set(); // yeniden bağlandı: aradaki değişiklikler
                if (!ws.SendAsync(new ArraySegment<byte>(sub), WebSocketMessageType.Text, true, CancellationToken.None).Wait(1500)) throw new TimeoutException("WM event subscription timed out");
                cacheConnected = true;
                var buf = new byte[1 << 16];
                while (ws.State == WebSocketState.Open)
                {
                    var sb = new StringBuilder();
                    WebSocketReceiveResult r;
                    do
                    {
                        r = ws.ReceiveAsync(new ArraySegment<byte>(buf), CancellationToken.None).Result;
                        if (r.MessageType == WebSocketMessageType.Close) break;
                        sb.Append(Encoding.UTF8.GetString(buf, 0, r.Count));
                    } while (!r.EndOfMessage);
                    if (r.MessageType == WebSocketMessageType.Close) break;
                    Handle(sb.ToString());
                }
            }
            catch (Exception ex) { Slider.Log("dwindle: " + ex.GetBaseException().Message); }
            finally { cacheConnected = false; cacheDirty.Set(); if (ws != null) ws.Dispose(); }
            Thread.Sleep(2000); // tiling yeniden başlarsa tekrar bağlan
        }
    }

    void Handle(string text)
    {
        var msg = json.DeserializeObject(text) as Dictionary<string, object>;
        if (msg == null || J.Str(msg, "messageType") != "event_subscription") return;
        var data = msg["data"] as Dictionary<string, object>;
        if (data == null) return;
        if (SnapshotEvent(J.Str(data, "eventType"))) cacheDirty.Set();
        if (J.Str(data, "eventType") == "window_unmanaged")
        {
            OnClosed(J.Str(data, "unmanagedId"));
            object uh;
            if (data.TryGetValue("unmanagedHandle", out uh) && uh != null) AnimateChange(Convert.ToInt64(uh), false, null);
            return;
        }
        object fc;
        Dictionary<string, object> win = null;
        if (data.TryGetValue("focusedContainer", out fc)) win = fc as Dictionary<string, object>;
        else if (data.TryGetValue("managedWindow", out fc)) win = fc as Dictionary<string, object>;
        if (win == null || J.Str(win, "type") != "window") return;
        if (J.Str(data, "eventType") == "focus_changed") { OnFocused(J.Str(win, "id")); }
        if (J.Str(data, "eventType") == "window_managed")
        {
            if (AutoFloat(win))
            {
                object ah;
                if (win.TryGetValue("handle", out ah) && ah != null) ReleasePending(Convert.ToInt64(ah), "kural yüzdürdü");
                LaunchQueue.Managed.Set();
                return;
            }
            LaunchQueue.Managed.Set(); // sıradaki açma devam etsin
            object nh;
            if (win.TryGetValue("handle", out nh) && nh != null) AnimateChange(Convert.ToInt64(nh), true, win);
        }
    }

}

// ---------------- Fareyle odak (Hyprland input.follow_mouse = 1) ----------------
// Yalnızca GERÇEK fare hareketinde imlecin altındaki pencereyi odaklar. Windows'un
// "üzerine gelince etkinleştir" özelliği fare kıpırdamadan da (pencere kapanıp yerleşim
// değişince) odak değiştiriyordu; bu da Alt+F4 sonrası odak geçmişini bozuyordu.
// Düşük seviyeli fare kancası sahte/sentetik hareketleri görmez.
static class DesktopClick
{
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);

    public static bool IsDesktopWindow(IntPtr h)
    {
        if (h == IntPtr.Zero) return false;
        IntPtr root = GetAncestor(h, 2); // GA_ROOT
        if (root == IntPtr.Zero) return false;
        var sb = new StringBuilder(16);
        GetClassName(root, sb, 16);
        string c = sb.ToString();
        return c == "Progman" || c == "WorkerW";
    }

    public static bool At(int x, int y) { return IsDesktopWindow(WindowFromPoint(new Point(x, y))); }

    // Masaüstünde bir yazı kutusu (simgenin yeniden adlandırma kutusu): onun tıklaması ve Enter'ı Explorer'ın kalır
    static bool IsEdit(IntPtr h)
    {
        if (h == IntPtr.Zero) return false;
        var sb = new StringBuilder(16);
        GetClassName(h, sb, 16);
        return sb.ToString() == "Edit";
    }

    public static bool EditAt(int x, int y) { return IsEdit(WindowFromPoint(new Point(x, y))); }

    [StructLayout(LayoutKind.Sequential)]
    struct GUITHREADINFO { public int cbSize, flags; public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret; public Native.RECT rcCaret; }
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread, ref GUITHREADINFO info);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);

    // Öndeki masaüstünde odak bir yazı kutusunda mı (yeniden adlandırılan simge)
    public static bool FocusIsEdit(IntPtr fg)
    {
        var info = new GUITHREADINFO { cbSize = Marshal.SizeOf(typeof(GUITHREADINFO)) };
        return GetGUIThreadInfo(GetWindowThreadProcessId(fg, IntPtr.Zero), ref info) && IsEdit(info.hwndFocus);
    }

    // Çift tıklama penceresi (kullanıcının Windows ayarı): süre ve dikdörtgen
    [DllImport("user32.dll")] static extern uint GetDoubleClickTime();
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    public static bool DoubleClick(uint t0, int x0, int y0, uint t1, int x1, int y1)
    {
        return t1 - t0 <= GetDoubleClickTime() && Math.Abs(x1 - x0) * 2 <= GetSystemMetrics(36) && Math.Abs(y1 - y0) * 2 <= GetSystemMetrics(37);
    }

    // Fare basılıyken sürükleme sayılacak kadar uzaklaştı mı (Windows'un sürükleme eşiği)
    public static bool Dragged(int x0, int y0, int x1, int y1)
    {
        return Math.Abs(x1 - x0) * 2 > GetSystemMetrics(68) || Math.Abs(y1 - y0) * 2 > GetSystemMetrics(69);
    }

    // Windows'un "tek tıklamayla aç" seçeneği (Klasör seçenekleri): açıkken masaüstündeki simge tek tıklamayla açılır.
    // Kayıt defterinden okunur; tıklama başına okumamak için 2 sn saklanır.
    [DllImport("shell32.dll")] static extern void SHGetSettings(out int flags, uint mask);
    const uint SSF_DOUBLECLICKINWEBVIEW = 0x80;
    static int singleCheckedAt = Environment.TickCount - 10000;
    static bool single;
    public static bool SingleClickOpen()
    {
        int now = Environment.TickCount;
        if (now - singleCheckedAt > 2000)
        {
            singleCheckedAt = now;
            try { int f; SHGetSettings(out f, SSF_DOUBLECLICKINWEBVIEW); single = (f & (1 << 5)) == 0; } catch { single = false; }
        }
        return single;
    }

    // Tutulan sol basışı Explorer'a geri ver (sürükleme başladı): basışın yerinde, işaretçiyle
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    public static void ReplayLeftDown(int x, int y)
    {
        int vx = GetSystemMetrics(76), vy = GetSystemMetrics(77), vw = Math.Max(2, GetSystemMetrics(78)), vh = Math.Max(2, GetSystemMetrics(79));
        int nx = (int)((x - vx) * 65535L / (vw - 1)), ny = (int)((y - vy) * 65535L / (vh - 1));
        mouse_event(0x0001 | 0x8000 | 0x4000, nx, ny, 0, Native.LL_MARK); // MOVE | ABSOLUTE | VIRTUALDESK
        mouse_event(0x0002, 0, 0, 0, Native.LL_MARK); // LEFTDOWN
    }
}

// A captured menu chord keeps its repeats/release even after focus or shell
// liveness changes. The hook's secure-desktop reset explicitly clears it.
sealed class DesktopMenuKeyState
{
    readonly HashSet<int> held = new HashSet<int>();
    public bool Contains(int vk) { return held.Contains(vk); }
    public void Clear() { held.Clear(); }
    public bool Handle(int vk, bool down, bool up, bool eligible, out bool open)
    {
        open = false;
        if (up && held.Remove(vk)) return true;
        if (!down) return false;
        if (held.Contains(vk)) return true;
        if (!eligible) return false;
        held.Add(vk); open = true;
        return true;
    }
}

class MouseFocus
{
    readonly TilingClient tiling;
    readonly AutoResetEvent moved = new AutoResetEvent(false);
    Native.LowLevelMouseProc proc;
    IntPtr hookHandle;
    volatile int lastX = int.MinValue, lastY = int.MinValue;

    public MouseFocus(TilingClient g) { tiling = g; }

    // Kanca, klavye kancasıyla aynı (başka iş yapmayan) thread'de kurulur; burada sadece sinyal verilir.
    public void InstallHook()
    {
        proc = Callback.Guard("fare kancası", (Native.LowLevelMouseProc)Hook);
        hookHandle = Native.SetWindowsHookEx(Native.WH_MOUSE_LL, proc, Native.GetModuleHandle(null), 0);
    }

    public void Reinstall()
    {
        IntPtr fresh = Native.SetWindowsHookEx(Native.WH_MOUSE_LL, proc, Native.GetModuleHandle(null), 0);
        if (fresh == IntPtr.Zero) return;
        IntPtr old = hookHandle; hookHandle = fresh;
        if (old != IntPtr.Zero) Native.UnhookWindowsHookEx(old);
    }

    // Kanca en son ne zaman çağrıldı (kanca bekçisi: Windows geç cevap veren kancayı sessizce söker)
    public static volatile int LastHookTick = Environment.TickCount;

    bool desktopRight;

    // Masaüstünde çift tıklama: simgeyi Explorer değil biz açarız (UserLaunch: önce denetlenir, açılamayan bir şeyde
    // Windows'un kutusu yerine bizim kartımız). İlk tıklama Explorer'ındır (seçim, sürükleme, kutu seçimi aynen kalır);
    // yalnızca çift tıklamayı tamamlayan ikinci basış ve bırakışı yutulur, açılacak simgeye kabuk bakar.
    bool leftDesk, swallowLeftUp;
    uint leftTime;
    int leftX, leftY;
    // "Tek tıklamayla aç" açıkken masaüstündeki sol basış tutulur: bırakılırsa tıklamadır (simge bizim yoldan açılır,
    // boşlukta seçim temizlenir); sürükleme eşiği aşılırsa basış Explorer'a geri verilir (sürükleme, kutu seçimi aynen).
    bool heldDown;
    int heldX, heldY;

    // Klavye kancası gibi ölçülür: yavaşsa nedeniyle log'a
    IntPtr Hook(int nCode, IntPtr wParam, IntPtr lParam)
    {
        LastHookTick = Environment.TickCount;
        var mark = InputLatency.Start();
        IntPtr r = HookInner(nCode, wParam, lParam);
        string slow = InputLatency.Slow(mark);
        if (slow != null) ThreadPool.QueueUserWorkItem(_ => Slider.Log("fare kancası yavaş: " + slow));
        return r;
    }

    IntPtr HookInner(int nCode, IntPtr wParam, IntPtr lParam)
    {
        int msg = wParam.ToInt32();
        if (nCode >= 0 && (msg == 0x204 || msg == 0x205)) // WM_RBUTTONDOWN / UP
        {
            var m = (Native.MSLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(Native.MSLLHOOKSTRUCT));
            if (msg == 0x204)
            {
                desktopRight = ShellState.Up && DesktopClick.At(m.pt.X, m.pt.Y);
                if (desktopRight) return (IntPtr)1; // dış tıklama değil: menüyü açan tıklama
                clickX = m.pt.X; clickY = m.pt.Y;
                clicked.Set();
            }
            else if (desktopRight)
            {
                desktopRight = false;
                ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:desktop-menu"));
                return (IntPtr)1;
            }
            return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
        }
        if (nCode >= 0 && msg == 0x200) // WM_MOUSEMOVE
        {
            var m = (Native.MSLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(Native.MSLLHOOKSTRUCT));
            if (heldDown && (m.flags & 1) == 0 && DesktopClick.Dragged(heldX, heldY, m.pt.X, m.pt.Y))
            {
                heldDown = false;
                DesktopClick.ReplayLeftDown(heldX, heldY);
            }
            if ((m.flags & 1) == 0 && (m.pt.X != lastX || m.pt.Y != lastY)) // LLMHF_INJECTED değil
            {
                lastX = m.pt.X; lastY = m.pt.Y;
                moved.Set();
            }
        }
        else if (nCode >= 0 && msg == 0x202 && swallowLeftUp) // çift tıklamanın yutulan basışının bırakışı
        {
            swallowLeftUp = false;
            return (IntPtr)1;
        }
        else if (nCode >= 0 && msg == 0x202 && heldDown) // tek tıklamayla aç: tutulan basış sürüklenmeden bırakıldı
        {
            heldDown = false;
            ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:desktop-click"));
            return (IntPtr)1;
        }
        else if (nCode >= 0 && (msg == 0x201 || msg == 0x207)) // sol / orta basış (sağ: yukarıda)
        {
            var m = (Native.MSLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(Native.MSLLHOOKSTRUCT));
            if (msg == 0x201 && (m.flags & 1) == 0) // gerçek sol basış (enjekte değil)
            {
                bool desk = ShellState.Up && DesktopClick.At(m.pt.X, m.pt.Y) && !DesktopClick.EditAt(m.pt.X, m.pt.Y);
                if (desk && DesktopClick.SingleClickOpen())
                {
                    heldDown = true; heldX = m.pt.X; heldY = m.pt.Y;
                    leftDesk = false;
                    return (IntPtr)1;
                }
                if (desk && leftDesk && DesktopClick.DoubleClick(leftTime, leftX, leftY, m.time, m.pt.X, m.pt.Y))
                {
                    leftDesk = false;
                    swallowLeftUp = true;
                    ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:desktop-open"));
                    return (IntPtr)1;
                }
                leftDesk = desk; leftTime = m.time; leftX = m.pt.X; leftY = m.pt.Y;
            }
            clickX = m.pt.X; clickY = m.pt.Y;
            clicked.Set(); // kanca hızlı kalsın: pencereye bakmak işçinin işi
        }
        return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
    }

    // Kabuğun dışına (bir pencereye, masaüstüne) tıklandı: açık menüler (tepsi, sağ panel) kapansın. Bar odak almadığı
    // için onlar "odak kaybı" olayını hiç görmüyordu. Kabuğun kendi pencerelerine tıklamak (sürükleme dahil) sayılmaz.
    readonly AutoResetEvent clicked = new AutoResetEvent(false);
    volatile int clickX, clickY;
    public void StartClickWorker()
    {
        new Thread(() =>
        {
            while (true)
            {
                clicked.WaitOne();
                try
                {
                    IntPtr under = Native.WindowFromPoint(new Point(clickX, clickY));
                    IntPtr root = under == IntPtr.Zero ? IntPtr.Zero : Native.GetAncestor(under, 2);
                    uint pid = 0;
                    if (root != IntPtr.Zero) Native.GetWindowThreadProcessId(root, out pid);
                    if (pid != 0 && ProcInfo.Name(pid).Equals(Names.Shell, StringComparison.OrdinalIgnoreCase)) continue;
                    Toasts.Emit("ll:outside-click");
                }
                catch (Exception ex) { Slider.Log("dış tıklama: " + ex.Message); }
            }
        }) { IsBackground = true, Name = "outside-click" }.Start();
    }

    public void StartWorker()
    {
        var t = new Thread(Worker) { IsBackground = true };
        t.Start();
    }

    void Worker()
    {
        IntPtr lastRoot = IntPtr.Zero;
        while (true)
        {
            moved.WaitOne();
            Thread.Sleep(15); // hareket akışını birleştir
            try
            {
                // Sürükleme / tıklama sırasında odak değiştirme
                if ((Native.GetAsyncKeyState(0x01) & 0x8000) != 0 || (Native.GetAsyncKeyState(0x02) & 0x8000) != 0) continue;
                var pt = new Point(lastX, lastY);
                IntPtr under = Native.WindowFromPoint(pt);
                if (under == IntPtr.Zero) continue;
                IntPtr root = Native.GetAncestor(under, 2);
                IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
                if (root == fg) continue;
                // Öndeki pencere bir diyalogsa (sahibi olan ya da kalıcı çerçeveli) fare hareketi odağı ondan çalmasın:
                // dosya seçme penceresi vb. fare gezdikçe arkaya düşüp gelmiyordu. Her zaman üstte olmak tek başına
                // yetmez: yüzen her pencere üstte durur ve fareyle odak o pencere öne gelince tamamen duruyordu.
                if (fg != IntPtr.Zero && (Native.GetWindow(fg, 4) != IntPtr.Zero ||
                    (Native.GetWindowLong(fg, Native.GWL_EXSTYLE) & 0x1 /*WS_EX_DLGMODALFRAME*/) != 0)) continue;
                if (root == lastRoot) continue; // son bakılan yönetilmeyen pencere (bar, masaüstü...)

                // Yalnızca tiling'in yönettiği ve odaktaki workspace'te görünen pencereler
                long handle = root.ToInt64();
                foreach (var m in tiling.Monitors())
                    foreach (Dictionary<string, object> ws in J.Children(m))
                    {
                        if (!J.Bool(ws, "isDisplayed")) continue;
                        var wins = new List<Dictionary<string, object>>();
                        J.WindowNodes(ws, wins);
                        // Hovering moves no focus to or from a fullscreen window: focusing another window on its
                        // workspace takes it out of fullscreen (as ii does), which only a click, a key or a new
                        // window should do, not the pointer passing by.
                        bool fullscreenThere = false;
                        foreach (var w in wins)
                        {
                            object st; var state = w.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
                            // büyütülmüş (Super+D) pencere tam ekran değil: üstünden geçince odak normal çalışır
                            if (state != null && J.Str(state, "type") == "fullscreen" && !J.Bool(state, "maximized")) fullscreenThere = true;
                        }
                        foreach (var w in wins)
                        {
                            object hv;
                            if (w.TryGetValue("handle", out hv) && Convert.ToInt64(hv) == handle)
                            {
                                if (!J.Bool(w, "hasFocus") && !fullscreenThere) tiling.Command("focus --container-id " + J.Str(w, "id"));
                                lastRoot = IntPtr.Zero;
                                goto done;
                            }
                        }
                    }
                lastRoot = root; // yönetilmiyor: tekrar sorgulama
                done:;
            }
            catch (Exception ex) { Slider.Log("mousefocus: " + ex.GetBaseException().Message); }
        }
    }
}

// ---------------- Görünmez kalmış pencereler ----------------
// tiling gizli workspace'lerin pencerelerini kabuğun "cloak" özelliğiyle gizler. tiling çökerse ya da zorla kapatılırsa
// (güncelleme, kaldırma) bu pencereler görünmez kalıyordu. lunge.exe --uncloak-orphans: kabuğun gizlediği uygulama
// pencerelerini geri getirir (tiling'in kendi kullandığı arayüzle). Askıya alınmış UWP pencerelerine dokunmaz.
static class Orphans
{
    // WM tags only the companion HWNDs it hides. Properties live with the HWND,
    // survive a WM crash and disappear on window destruction (no handle reuse).
    const string CompanionTag = "LogicalLunge.HiddenCompanion";
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetProp(IntPtr h, string name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr RemoveProp(IntPtr h, string name);
    [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr h, int how);
    static bool HasCompanionTag(IntPtr h) { return GetProp(h, CompanionTag) != IntPtr.Zero; }
    [ComImport, Guid("6D5140C1-7436-11CE-8034-00AA006009FA"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IServiceProviderLL { [return: MarshalAs(UnmanagedType.IUnknown)] object QueryService(ref Guid service, ref Guid riid); }
    [ComImport, Guid("372E1D3B-38D3-42E4-A15B-8AB2B178F513"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IApplicationViewLL { void m1(); void m2(); void m3(); void m4(); void m5(); void m6(); void m7(); void m8(); void m9(); [PreserveSig] int SetCloak(uint type, int flag); }
    [ComImport, Guid("1841C6D7-4F9D-42C0-AF41-8747538F10E5"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IApplicationViewCollectionLL { void m1(); void m2(); void m3(); [PreserveSig] int GetViewForHwnd(IntPtr hwnd, out IApplicationViewLL view); }

    // listOnly: yalnızca adayları yazdır. tiling çalışırken hiçbir şey yapılmaz: gizli workspace'lerin pencereleri
    // bilerek gizlidir, açılırlarsa ekrana dökülürlerdi.
    public static int Uncloak(bool listOnly = false)
    {
        if (!listOnly && Process.GetProcessesByName(Names.Tiling).Length > 0) { Slider.Log("uncloak: tiling çalışıyor, atlandı"); return -1; }
        var targets = new List<IntPtr>();
        var companions = new List<IntPtr>();
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            if (HasCompanionTag(h)) { companions.Add(h); return true; }
            if (!Native.IsWindowVisible(h)) return true;
            int cl;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cl, 4) != 0 || cl != 2) return true; // 2 = kabuk gizlemiş
            var c = new StringBuilder(128); Native.GetClassName(h, c, 128);
            string cs = c.ToString();
            if (cs == "Windows.UI.Core.CoreWindow" || cs == "ApplicationFrameWindow") return true; // askıdaki UWP
            int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
            if ((ex & Native.WS_EX_TOOLWINDOW) != 0) return true;
            // Tıklamayı geçiren şeffaf katmanlar (ör. görev çubuğu oyunu TaskBarHero): tiling bunları yönetmez; gizliyse öyle
            // kalsın (geri getirilince görev çubuğu gizliyken efektleri ekranın üstünde yüzüyordu)
            if ((ex & Native.WS_EX_TRANSPARENT) != 0 && (ex & 0x00080000) != 0) return true; // TRANSPARENT + LAYERED
            targets.Add(h);
            return true;
        }, IntPtr.Zero);
        if (listOnly)
        {
            targets.AddRange(companions);
            foreach (var h in targets) { var t = new StringBuilder(120); Native.GetWindowText(h, t, 120); Console.WriteLine(h.ToInt64() + " " + t); }
            return targets.Count;
        }
        int n = 0;
        foreach (var h in companions)
            if (ShowWindowAsync(h, 8 /* SW_SHOWNA: never take focus */)) { RemoveProp(h, CompanionTag); n++; }
        if (targets.Count == 0) return n;
        try
        {
            var shell = (IServiceProviderLL)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("C2F03A33-21F5-47FA-B4BB-156362A2F239")));
            var iid = typeof(IApplicationViewCollectionLL).GUID;
            var coll = (IApplicationViewCollectionLL)shell.QueryService(ref iid, ref iid);
            foreach (var h in targets)
            {
                IApplicationViewLL v;
                if (coll.GetViewForHwnd(h, out v) == 0 && v != null && v.SetCloak(1, 0) == 0) n++;
            }
        }
        catch (Exception ex) { Slider.Log("uncloak: " + ex.Message); }
        Slider.Log("görünmez kalmış " + targets.Count + " pencereden " + n + " tanesi geri getirildi");
        return n;
    }
}

// ---------------- Shell nöbetçisi ----------------
// Bar ve tüm paneller shell'de. shell hiç açılmazsa (ör. yeni kurulumda PATH) ya da açık olduğu halde widget sunucusu
// (127.0.0.1:6124) çalışmıyorsa (port o an önceki shell'de kaldıysa sunucusuz açılıyor, bar "bağlantı reddedildi"
// gösteriyordu) masaüstü yarım kalmasın: tiling çalışıyorken iki ardışık kontrolde (~10 sn) sorun sürerse shell'i temiz
// biçimde (port boşalana kadar bekleyip) yeniden başlat. Art arda başarısızlıkta beklemeyi uzatır.
static class ShellWatchdog
{
    const int PORT = 6124;

    static bool PortOpen()
    {
        try
        {
            using (var c = new System.Net.Sockets.TcpClient())
            {
                var ar = c.BeginConnect("127.0.0.1", PORT, null, null);
                bool ok = ar.AsyncWaitHandle.WaitOne(700) && c.Connected;
                try { c.EndConnect(ar); } catch { ok = false; }
                // bekleme tutamacı çöp toplayıcıya kalmasın (5 sn'de bir çağrılıyor)
                try { ar.AsyncWaitHandle.Close(); } catch { }
                return ok;
            }
        }
        catch { return false; }
    }

    static List<Process> Shells()
    {
        return new List<Process>(Process.GetProcessesByName(Names.Shell));
    }

    // 5 sn'de bir iki süreç tablosu taraması (GetProcessesByName bütün süreçleri listeler) yerine bulunan süreç
    // saklanır; çıkmadığı sürece tutamacına sormak yeter (HasExited: tek bir bekleme çağrısı). Çıkınca yeniden aranır.
    static readonly Dictionary<string, Process> seen = new Dictionary<string, Process>();
    static Process Find(string name)
    {
        Process p;
        if (seen.TryGetValue(name, out p))
        {
            try { if (!p.HasExited) return p; } catch { }
            p.Dispose();
            seen.Remove(name);
        }
        var ps = Process.GetProcessesByName(name);
        Process found = null;
        foreach (var q in ps)
        {
            if (found == null) { try { if (!q.HasExited) { found = q; continue; } } catch { } }
            q.Dispose();
        }
        if (found != null) seen[name] = found;
        return found;
    }

    static bool TilingRunning() { return Find(Names.Tiling) != null; }

    // Bar sayfaları yüklenince ve sonra 30 sn'de bir "canlıyım" der (POST /bar-alive?id=<sayfa yüklemesi>). shell ayakta ve
    // sunucusu açık olsa da bir bar hata sayfasında ya da donmuş kalabiliyordu (yenilemede eski shell'in sunucusuna bağlanıp
    // boş kalan bar gibi): son 150 sn'de canlı diyen bar sayısı bar penceresi sayısından azsa shell yeniden başlatılır.
    // Süreler tek yönlü saatle: duvar saati yaz saatinde ya da NTP düzeltmesinde atlar, bütün barlar bir anda "sessiz"
    // sayılıp kabuk boşuna yeniden başlıyordu. Saat çekirdekle başlar: çekirdeğin ne zamandır açık olduğu da ondan.
    static readonly Dictionary<string, long> barAlive = new Dictionary<string, long>();
    static readonly Stopwatch aliveClock = Stopwatch.StartNew();
    public static void BarAlive(string id)
    {
        lock (barAlive)
        {
            if (!barAlive.ContainsKey(id)) Slider.Log("bar canlı: " + (id.Length > 8 ? id.Substring(0, 8) : id));
            barAlive[id] = aliveClock.ElapsedMilliseconds;
            if (barAlive.Count > 50)
            {
                var old = new List<string>();
                foreach (var kv in barAlive) if (aliveClock.ElapsedMilliseconds - kv.Value > 300000) old.Add(kv.Key);
                foreach (var k in old) barAlive.Remove(k);
            }
        }
    }
    // Kabuğun açtığı her bar bu açılışta "canlıyım" dedi mi: null evet; değilse ne bekleniyor. Eski kabuğun barları (yeniden
    // başlatmadan önce) sayılmaz.
    public static string BarsReady()
    {
        var shell = Find(Names.Shell);
        if (shell == null) return "kabuk";
        DateTime started;
        try { started = shell.StartTime; } catch { return "kabuk"; }
        long since = aliveClock.ElapsedMilliseconds - (long)(DateTime.Now - started).TotalMilliseconds;
        int windows = BarWindows(shell.Id), alive = 0;
        lock (barAlive) foreach (var kv in barAlive) if (kv.Value >= since) alive++;
        if (windows == 0) return "bar";
        return alive >= windows ? null : "bar " + alive + "/" + windows;
    }
    // CI dayanıklılık testi (yalnızca LL_TEST=1): 30 sn'de bir bar penceresi sayısı ve son 45 sn'de "canlıyım" diyen bar
    // sayısı log'a yazılır; test barların yük altında da sustuğunu buradan görür. Normal çalışmada hiçbir şey yapmaz.
    static readonly bool testRun = Environment.GetEnvironmentVariable("LL_TEST") == "1";
    static long lastTestReport = -30000;
    static void TestReport(Process shell)
    {
        if (!testRun || aliveClock.ElapsedMilliseconds - lastTestReport < 30000) return;
        lastTestReport = aliveClock.ElapsedMilliseconds;
        int windows = BarWindows(shell.Id), recent = 0;
        lock (barAlive) foreach (var kv in barAlive) if (aliveClock.ElapsedMilliseconds - kv.Value <= 45000) recent++;
        Slider.Log("test: bars " + windows + " alive " + recent);
    }
    static int AliveBars()
    {
        int n = 0;
        lock (barAlive) foreach (var kv in barAlive) if (aliveClock.ElapsedMilliseconds - kv.Value <= 150000) n++;
        return n;
    }
    static int BarWindows(int pid)
    {
        int n = 0;
        var title = new StringBuilder(64);
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            uint p; Native.GetWindowThreadProcessId(h, out p);
            if (p != pid) return true;
            title.Length = 0; Native.GetWindowText(h, title, 64);
            if (title.ToString() == Names.Bar) n++;
            return true;
        }, IntPtr.Zero);
        return n;
    }
    // Bar'ların sessiz kalması bir sorun mu: shell ve helper yeterince uzun süredir açık, ekran kilitli değil
    static string SilentBars(Process shell)
    {
        if (aliveClock.ElapsedMilliseconds < 160000) return null; // helper yeni: bar'ların bir sonraki bildirimini bekle
        DateTime started;
        try { started = shell.StartTime; } catch { return null; }
        if ((DateTime.Now - started).TotalSeconds < 40) return null;
        if (!FocusGuard.OnDefaultDesktop()) return null; // kilit ekranında zamanlayıcılar yavaşlar
        int windows = BarWindows(shell.Id), alive = AliveBars();
        return windows > 0 && alive < windows ? "bar yanıt vermiyordu (" + alive + "/" + windows + " canlı)" : null;
    }

    // Kabuğu kurulum klasöründen başlatır. Kabuk her zaman normal kullanıcı olarak çalışır (çekirdek yönetici olsa da):
    // web içeriği yönetici olmaz ve kabuğun açtığı her şey (uygulamalar, komutlar) kullanıcı haklarıyla açılır.
    // Oturum açılışında Gezgin henüz hazır değilse 20 sn'ye kadar bekler. (ShellExecute: çekirdeğin tutamaçları kabuğa
    // miras kalmaz.)
    public static void StartShell(string why)
    {
        if (!System.IO.File.Exists(Paths.Shell)) { Slider.Log("shell nöbetçisi: " + why + ", " + Paths.Shell + " bulunamadı"); return; }
        if (UserLaunch.StartWhenReady(Paths.Shell, "", Paths.Home, 20000)) Slider.Log("shell nöbetçisi: " + why + ", başlatıldı");
        else Slider.Log("shell nöbetçisi: " + why + ", başlatılamadı");
    }

    public static void Restart(string why)
    {
        foreach (var p in Shells()) { try { p.Kill(); p.WaitForExit(3000); } catch { } finally { p.Dispose(); } }
        // Önceki süreç portu bırakana kadar bekle (yoksa yeni shell da sunucusuz açılabiliyor)
        var sw = Stopwatch.StartNew();
        while (PortOpen() && sw.ElapsedMilliseconds < 5000) Thread.Sleep(200);
        StartShell(why);
    }

    public static void Start()
    {
        new Thread(() =>
        {
            Thread.Sleep(15000); // açılışta shell'i Supervisor başlatır
            int bad = 0, failures = 0;
            while (true)
            {
                Thread.Sleep(5000);
                try
                {
                    if (!TilingRunning()) { bad = 0; continue; } // tiling kapalıyken (çıkış / yeniden başlatma) karışma
                    if (Maint.Quiet() || TilingWatchdog.Recovering) { bad = 0; continue; }
                    var shell = Find(Names.Shell);
                    if (shell != null) TestReport(shell);
                    string problem = shell == null ? "shell çalışmıyordu" : !PortOpen() ? "widget sunucusu (6124) yanıt vermiyordu" : SilentBars(shell);
                    if (problem != null && problem.StartsWith("bar ")) lock (barAlive) barAlive.Clear(); // yeniden başlayınca sayım sıfırdan
                    if (problem == null) { bad = 0; failures = 0; continue; }
                    if (++bad < 2) continue;
                    bad = 0;
                    if (shell == null) Toasts.SendLater("warn", "Kabuk durdu, yeniden başlatıldı", "Bar ve paneller yeniden açıldı.", "restart_alt");
                    Restart(problem);
                    failures++;
                    if (failures >= 3) Thread.Sleep(Math.Min(300000, 30000 * failures)); // sürekli başarısızsa sık sık deneme
                }
                catch (Exception ex) { Slider.Log("shell nöbetçisi: " + ex.Message); }
            }
        }) { IsBackground = true, Priority = ThreadPriority.BelowNormal }.Start();
    }
}

// ---------------- Arayüz tercihleri ve config izleme ----------------
// ~\.config\logical-lunge\prefs.json: {"language": "system" | "tr" | ..., "clock": "24" | "12", "animations": true,
// "focusColor": "#rrggbb", "theme": "dark" | "light"}. Widget'lar /prefs.json'dan okur (kurulum klasörü yönetici korumalı,
// yazılamaz); çekirdek animasyon tercihini kullanır. Kabuk teması burada tek kaynak: değişince (ayarlar, bar, panel ya da
// elle) kabuğa ll:theme-dark / ll:theme-light olayı gider; web widget'ları ve native bar aynı anda güncellenir.
// config.yaml değişince (ayarlar penceresi ya da elle) animasyon kenarlıklarının rengi yenilenir.
static class Prefs
{
    static volatile bool animations = true, gestures = true, winToasts = true, takeover = true;
    public static bool Animations { get { return animations; } }
    // Windows bildirimleri Logical Lunge kartı olarak (WinNotifications); Windows'un kendi balonları kapanır
    public static bool WinToasts { get { return winToasts; } }
    // Yalnızca çekirdek abone olur: --set-pref ile tercih yazan kısa ömürlü süreç balonlara dokunmaz
    public static event Action WinToastsChanged;
    // Windows'un yerini aldığımız parçaları (görev çubuğu, yerleşim önerileri ...) kaynağında kapat (ShellTakeover)
    public static bool Takeover { get { return takeover; } }
    public static event Action TakeoverChanged;
    // Dokunmatik yüzey hareketleri (3/4 parmak); dokunmatik yüzey yoksa etkisiz
    public static bool Gestures { get { return gestures; } }
    public static string FilePath { get { return System.IO.Path.Combine(Paths.ConfigDir, "prefs.json"); } }
    static readonly object gate = new object();

    public static Dictionary<string, object> Read()
    {
        bool corrupt;
        return TryRead(out corrupt) ?? new Dictionary<string, object>();
    }

    // null: okunamadı. corrupt: dosya bozuk (JSON değil); değilse o an yazılıyor / kilitli. Dosya yoksa boş sözlük.
    static Dictionary<string, object> TryRead(out bool corrupt)
    {
        corrupt = false;
        string text;
        try
        {
            if (!System.IO.File.Exists(FilePath)) return new Dictionary<string, object>();
            // Silme / yazma paylaşımıyla: okurken yazanın atomik değiştirmesini engellemez
            using (var fs = new System.IO.FileStream(FilePath, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
            using (var sr = new System.IO.StreamReader(fs, Encoding.UTF8)) text = sr.ReadToEnd();
        }
        catch { return null; }
        try { return new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(text) ?? new Dictionary<string, object>(); }
        catch { corrupt = text.Trim().Length > 0; return null; } // boş: yazılırken okundu
    }

    static readonly object loadGate = new object();
    static string theme; // son okunan kabuk teması (null: henüz okunmadı)
    static string last; // son okunan tercihler; değişince kabuğa ll:prefs (ör. bildirim süreleri hemen uygulanır)

    // Bildirimin ekranda kalma süresi (sn): bilgi / başarı ve uyarı / hata. toast.html'deki varsayılanlar bunlarla aynı.
    static readonly Dictionary<string, int> ToastDefaults = new Dictionary<string, int> { { "toastInfo", 3 }, { "toastError", 5 } };
    const int ToastMax = 60;

    public static int ToastSeconds(Dictionary<string, object> p, string key)
    {
        object v;
        return p.TryGetValue(key, out v) && v is int && (int)v >= 1 && (int)v <= ToastMax ? (int)v : ToastDefaults[key];
    }

    public static void Load()
    {
        lock (loadGate)
        {
            bool corrupt;
            var d = TryRead(out corrupt);
            if (d == null) return; // yazılırken okundu: yazma bitince dosya izleyicisi yeniden okutur
            object v;
            animations = !(d.TryGetValue("animations", out v) && v is bool && !(bool)v);
            gestures = !(d.TryGetValue("gestures", out v) && v is bool && !(bool)v);
            bool wt = !(d.TryGetValue("winToasts", out v) && v is bool && !(bool)v);
            if (wt != winToasts)
            {
                winToasts = wt;
                var changed = WinToastsChanged;
                if (changed != null) changed();
            }
            bool to = !(d.TryGetValue("takeover", out v) && v is bool && !(bool)v);
            if (to != takeover)
            {
                takeover = to;
                var changed = TakeoverChanged;
                if (changed != null) changed();
            }
            string th = d.TryGetValue("theme", out v) && "light".Equals(v) ? "light" : "dark";
            string was = theme;
            theme = th;
            if (was != null && was != th) Toasts.Emit("ll:theme-" + th);
            string all = new JavaScriptSerializer().Serialize(d);
            if (last != null && last != all) Toasts.Emit("ll:prefs");
            last = all;
        }
    }

    public static string Json()
    {
        return new JavaScriptSerializer().Serialize(Read());
    }

    // key: language | clock | animations | gestures | winToasts | takeover | focusColor | theme | toastInfo | toastError (sn); değer doğrulanır
    public static bool Set(string key, string value)
    {
        object val;
        int n;
        switch (key)
        {
            case "language":
                if (!System.Text.RegularExpressions.Regex.IsMatch(value, "^(system|[a-z]{2})$")) return false;
                val = value; break;
            case "clock":
                if (value != "24" && value != "12") return false;
                val = value; break;
            case "animations":
            case "gestures":
            case "winToasts":
            case "takeover":
            // rahatsız etme: native bildirim kartları gösterilmez
            case "dnd":
                if (value != "true" && value != "false") return false;
                val = value == "true"; break;
            case "focusColor":
                if (!System.Text.RegularExpressions.Regex.IsMatch(value, "^#[0-9a-fA-F]{6}$")) return false;
                val = value.ToLowerInvariant(); break;
            case "theme":
                if (value != "dark" && value != "light") return false;
                val = value; break;
            case "toastInfo":
            case "toastError":
                if (!int.TryParse(value, System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out n) || n < 1 || n > ToastMax) return false;
                val = n; break;
            case "uiScale":
                if (!int.TryParse(value, System.Globalization.NumberStyles.None, System.Globalization.CultureInfo.InvariantCulture, out n) || !UiScale.Valid(n)) return false;
                val = n; break;
            default: return false;
        }
        lock (gate)
        {
            // Okunamayan dosyanın üstüne tek anahtarla yazmak diğer tercihleri (dil, saat ...) silerdi: kilitliyse
            // kısa süre yeniden dene; bozuksa yedeğini alıp baştan başla.
            bool corrupt = false;
            Dictionary<string, object> d = null;
            for (int i = 0; i < 6 && d == null && !corrupt; i++)
            {
                d = TryRead(out corrupt);
                if (d == null && !corrupt) Thread.Sleep(50);
            }
            if (d == null)
            {
                if (!corrupt) return false;
                try { System.IO.File.Copy(FilePath, FilePath + ".bad", true); } catch { }
                d = new Dictionary<string, object>();
            }
            d[key] = val;
            if (!Files.WriteAtomic(FilePath, new JavaScriptSerializer().Serialize(d))) return false;
        }
        Load();
        if (key == "uiScale") UiScale.Apply((int)val);
        return true;
    }
}

static class UiScale
{
    public static readonly int[] Steps = { 85, 90, 100, 110, 125, 150 };
    const double BarDip = 40, Margin = 5;

    public static bool Valid(int pct) { return Array.IndexOf(Steps, pct) >= 0; }

    public static int Percent(Dictionary<string, object> prefs)
    {
        object v;
        return prefs.TryGetValue("uiScale", out v) && v is int && Valid((int)v) ? (int)v : 100;
    }

    // barın yüksekliği + kenar boşluğu (DIP)
    public static int TopGap(int pct) { return (int)Math.Round(BarDip * pct / 100.0 + Margin); }

    static readonly System.Text.RegularExpressions.Regex topLine = new System.Text.RegularExpressions.Regex(
        @"(^[ \t]*outer_gap:[ \t]*\r?\n(?:[ \t]+(?:right|bottom|left):.*\r?\n)*[ \t]+top:[ \t]*)'?\d+px'?",
        System.Text.RegularExpressions.RegexOptions.Multiline);

    // config.yaml metninde outer_gap'in top değeri; bulunamazsa metin olduğu gibi döner
    public static string WithTopGap(string yaml, int px)
    {
        return topLine.Replace(yaml, m => m.Groups[1].Value + "'" + px + "px'", 1);
    }

    public static void Apply(int pct)
    {
        try
        {
            string path = Paths.ConfigFile;
            if (!System.IO.File.Exists(path)) return;
            string text = System.IO.File.ReadAllText(path);
            string next = WithTopGap(text, TopGap(pct));
            if (next == text) return;
            if (!Files.WriteAtomic(path, next)) { Slider.Log("arayüz ölçeği: config.yaml yazılamadı"); return; }
            try { new TilingClient().Command("wm-reload-config"); } catch (Exception ex) { Slider.Log("arayüz ölçeği: " + ex.Message); }
        }
        catch (Exception ex) { Slider.Log("arayüz ölçeği: " + ex.Message); }
    }
}

// Artık dosya süpürücüsü: çökme / güç kesintisi yarım kalan geçici dosyaları bırakabiliyor (Windows bildirim veritabanı
// kopyaları ll-wpn-*, Google Lens sayfaları lunge-lens-*, atomik yazmanın *.tmp'leri). Açılıştan 2 dk sonra ve günde bir
// kez bir günden eski olanlar silinir. Bağlantı (junction) klasörlerin içine girilmez: yalnızca bağlantının kendisi gider.
static class TempSweep
{
    static readonly TimeSpan AGE = TimeSpan.FromDays(1);

    public static void Start()
    {
        new Thread(() =>
        {
            Thread.Sleep(120000);
            while (true)
            {
                try { Run(); } catch (Exception ex) { Slider.Log("geçici dosya süpürme: " + ex.Message); }
                Thread.Sleep(TimeSpan.FromHours(24));
            }
        }) { IsBackground = true, Priority = ThreadPriority.Lowest, Name = "temp-sweep" }.Start();
    }

    static bool Old(System.IO.FileSystemInfo f) { return DateTime.UtcNow - f.LastWriteTimeUtc > AGE; }

    static int OldFiles(string dir, string pattern)
    {
        int n = 0;
        if (!System.IO.Directory.Exists(dir)) return 0;
        foreach (var f in new System.IO.DirectoryInfo(dir).GetFiles(pattern))
            if (Old(f)) { try { f.Delete(); n++; } catch { } }
        return n;
    }

    public static void Run()
    {
        string temp = System.IO.Path.GetTempPath();
        int n = OldFiles(temp, "lunge-lens-*.html");
        foreach (var d in new System.IO.DirectoryInfo(temp).GetDirectories("ll-wpn-*"))
        {
            if (!Old(d)) continue;
            try
            {
                if ((d.Attributes & System.IO.FileAttributes.ReparsePoint) == 0)
                    foreach (var f in d.GetFiles()) f.Delete();
                d.Delete(false);
                n++;
            }
            catch { }
        }
        foreach (string dir in new[] { Paths.StateDir, Paths.ConfigDir, Paths.DataDir("cache") }) n += OldFiles(dir, "*.tmp");
        if (n > 0) Slider.Log("geçici dosya süpürme: " + n + " artık silindi");
    }
}

static class Files
{
    // Atomik yazma: okuyan (dosya izleyici, kabuk, native bar) yarım yazılmış dosya görmez
    public static bool WriteAtomic(string path, string text)
    {
        string tmp = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            System.IO.File.WriteAllText(tmp, text, new UTF8Encoding(false));
            for (int i = 0; i < 6; i++)
            {
                try
                {
                    if (System.IO.File.Exists(path)) System.IO.File.Replace(tmp, path, null);
                    else System.IO.File.Move(tmp, path);
                    return true;
                }
                catch (System.IO.IOException) { Thread.Sleep(50); } // başka bir süreç o an paylaşımsız okuyor
            }
        }
        catch { }
        try { System.IO.File.Delete(tmp); } catch { }
        return false;
    }
}

// Tepsi sabitlemeleri (state\tray-pins.json; anahtar = simge ipucunun ilk
// kelimesi). Değişince kabuğa ll:tray-pins gider.
// Sabitleme listesi (state klasöründe JSON dizi): tepsi simgeleri, Dock uygulamaları. Değişince kabuğa olay gider.
sealed class PinFile
{
    readonly object gate = new object();
    readonly string file, evt;
    readonly int max;
    public PinFile(string file, string evt, int max) { this.file = file; this.evt = evt; this.max = max; }
    string FilePath { get { return Paths.State(file); } }

    // JSON dizi; kayıt yoksa "null"
    public string Read()
    {
        try { return System.IO.File.Exists(FilePath) ? System.IO.File.ReadAllText(FilePath) : "null"; }
        catch { return "null"; }
    }

    static bool Valid(string key) { return !string.IsNullOrEmpty(key) && key.Length <= 64; }

    // null: dosya var ama okunamadı (o zaman üstüne yazılmaz: diğer sabitlemeler silinirdi)
    List<string> Keys()
    {
        var keys = new List<string>();
        if (!System.IO.File.Exists(FilePath)) return keys;
        try
        {
            var arr = new JavaScriptSerializer().Deserialize<object[]>(System.IO.File.ReadAllText(FilePath));
            if (arr != null) foreach (var o in arr) { var k = o as string; if (Valid(k)) keys.Add(k); }
            return keys;
        }
        catch { return null; }
    }

    public bool Contains(string key)
    {
        var keys = Keys();
        return keys != null && keys.Contains(key);
    }

    // ifMissing: yalnızca kayıt yoksa yaz (eski sürümün tarayıcı deposundan taşıma). false: geçersiz değer / yazılamadı
    public bool Write(string json, bool ifMissing)
    {
        object[] arr;
        try { arr = new JavaScriptSerializer().Deserialize<object[]>(json); } catch { return false; }
        if (arr == null || arr.Length > max) return false;
        var keys = new List<string>();
        foreach (var o in arr)
        {
            var k = o as string;
            if (!Valid(k)) return false;
            keys.Add(k);
        }
        return Save(keys, ifMissing);
    }

    // Tek anahtarı ekler / çıkarır (Dock'un ve Super menüsünün sağ tık menüsü)
    public bool Set(string key, bool on)
    {
        if (!Valid(key)) return false;
        lock (gate)
        {
            var keys = Keys();
            if (keys == null) return false;
            if (keys.Contains(key) == on) return true;
            if (on) { if (keys.Count >= max) return false; keys.Add(key); }
            else keys.Remove(key);
            return Save(keys, false);
        }
    }

    bool Save(List<string> keys, bool ifMissing)
    {
        string text = new JavaScriptSerializer().Serialize(keys);
        lock (gate)
        {
            if (ifMissing && System.IO.File.Exists(FilePath)) return true;
            if (text == Read()) return true;
            if (!Files.WriteAtomic(FilePath, text)) return false;
        }
        Toasts.Emit(evt);
        return true;
    }
}

static class Pins
{
    public static readonly PinFile Tray = new PinFile("tray-pins.json", "ll:tray-pins", 64);
    // Dock'ta tutulan uygulamalar: exe adı (küçük harf, .exe'siz); Dock çalışan pencereyi süreç adıyla eşler
    public static readonly PinFile Dock = new PinFile("dock-pins.json", "ll:dock-pins", 24);
}

// Çekirdeğin kendi çizdiği metinler (sağ tık menüsü) için ui\logical-lunge\i18n.json'dan birebir çeviri (native bar'daki
// tr() gibi): Türkçe arayüzde ya da çevirisi yoksa olduğu gibi
static class I18n
{
    static Dictionary<string, string> table;

    public static string T(string tr)
    {
        if (table == null) table = Load();
        string v;
        return table.TryGetValue(tr, out v) ? v : tr;
    }

    static Dictionary<string, string> Load()
    {
        var d = new Dictionary<string, string>();
        try
        {
            object lang;
            string code = Prefs.Read().TryGetValue("language", out lang) ? lang as string : null;
            if (string.IsNullOrEmpty(code) || code == "system") code = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
            if (code == "tr") return d;
            var j = new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.DeserializeObject(System.IO.File.ReadAllText(Paths.UiPack("i18n.json"))) as Dictionary<string, object>;
            if (j == null) return d;
            var langs = j["langs"] as object[];
            if (langs == null || Array.IndexOf(langs, code) < 0) code = "en";
            var keys = j["keys"] as object[];
            object valsObj;
            var vals = j.TryGetValue(code, out valsObj) ? valsObj as object[] : null;
            if (keys == null || vals == null) return d;
            for (int i = 0; i < keys.Length && i < vals.Length; i++)
            {
                var k = keys[i] as string; var v = vals[i] as string;
                if (k != null && v != null) d[k] = v;
            }
        }
        catch { }
        return d;
    }
}

// Ayarlar penceresinin (ui/settings.html) çekirdek komutları
static class Settings
{
    static readonly JavaScriptSerializer Js = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    public static string Cli(string[] a)
    {
        switch (a[0])
        {
            case "--settings-get": return Js.Serialize(Get());
            case "--set-focus-color": return Js.Serialize(SetFocusColor(a.Length > 1 ? a[1] : ""));
            case "--set-pref": return Js.Serialize(Result(a.Length > 2 && Prefs.Set(a[1], a[2])));
            case "--set-workspaces":
                var arr = new string[a.Length - 1];
                Array.Copy(a, 1, arr, 0, arr.Length);
                return Js.Serialize(SetWorkspaces(arr));
            case "--health": return Js.Serialize(Health());
            case "--edit-config": return Js.Serialize(Result(UserLaunch.Start(Keys2.CodeEditor, "\"" + Paths.ConfigFile + "\"", Paths.ConfigDir)));
            case "--wm":
                // yalnızca ayar yenileme / yeniden çizme (ayarlar penceresinin düğmeleri)
                string cmd = a.Length > 1 ? a[1] : "";
                if (cmd != "wm-reload-config" && cmd != "wm-redraw") return Js.Serialize(Result(false));
                new TilingClient().Command(cmd);
                return Js.Serialize(Result(true));
        }
        return Js.Serialize(Result(false));
    }

    static Dictionary<string, object> Result(bool ok) { return new Dictionary<string, object> { { "ok", ok } }; }

    static string Sha256(string text)
    {
        using (var sha = System.Security.Cryptography.SHA256.Create())
            return BitConverter.ToString(sha.ComputeHash(Encoding.UTF8.GetBytes(text))).Replace("-", "");
    }

    static bool PrefOn(Dictionary<string, object> p, string key)
    {
        object v;
        return !(p.TryGetValue(key, out v) && v is bool && !(bool)v);
    }

    static Dictionary<string, object> Get()
    {
        var p = Prefs.Read();
        string cfg = "";
        try { cfg = System.IO.File.ReadAllText(Paths.ConfigFile); } catch { }
        var m = System.Text.RegularExpressions.Regex.Match(cfg, @"(?m)^\s*active_color:\s*""(#[0-9a-fA-F]{6})");
        object lang, clock, anim, gest;
        // Bu komut ayrı bir süreçte çalışır: Prefs yüklenmemiştir (animasyonlar kapalıyken de "açık" görünüyordu)
        var workspaces = new List<Dictionary<string, object>>();
        List<WorkspaceConfigText.Entry> entries;
        string workspaceError;
        if (WorkspaceConfigText.TryRead(cfg, out entries, out workspaceError))
            foreach (var entry in entries) {
                var item = new Dictionary<string, object> { { "name", entry.Number.ToString() } };
                if (entry.Monitor.HasValue) item["bind_to_monitor"] = entry.Monitor.Value;
                workspaces.Add(item);
            }

        return new Dictionary<string, object>
        {
            { "focusColor", m.Success ? m.Groups[1].Value.ToLowerInvariant() : "#b69df8" },
            { "language", p.TryGetValue("language", out lang) ? lang : "system" },
            { "clock", p.TryGetValue("clock", out clock) ? clock : "24" },
            { "animations", !(p.TryGetValue("animations", out anim) && anim is bool && !(bool)anim) },
            { "gestures", !(p.TryGetValue("gestures", out gest) && gest is bool && !(bool)gest) },
            // açık/kapalı tercihler: yoksa açık (ayarlar penceresi anahtarın gerçek hâlini göstersin)
            { "winToasts", PrefOn(p, "winToasts") },
            { "takeover", PrefOn(p, "takeover") },
            { "toastInfo", Prefs.ToastSeconds(p, "toastInfo") },
            { "toastError", Prefs.ToastSeconds(p, "toastError") },
            { "touchpad", Touchpad.Present() },
            { "version", Updater.Installed() },
            { "configDir", Paths.ConfigDir },
            { "configFile", Paths.ConfigFile },
            { "logsDir", Paths.LogsDir },
            { "workspaces", workspaces },
            { "monitors", MonitorList() },
            { "workspaceError", workspaceError },
        };
    }

    static List<Dictionary<string, object>> MonitorList()
    {
        var list = new List<Dictionary<string, object>>();
        var monitors = new TilingClient().Monitors();
        var devices = new List<string>();
        foreach (var monitor in monitors) devices.Add(J.Str(monitor, "deviceName"));
        var friendly = MonitorFriendlyNames.Read(devices);
        for (int i = 0; i < monitors.Count; i++)
        {
            var m = monitors[i];
            string device = J.Str(m, "deviceName");
            string raw = device.StartsWith("\\\\.\\") ? device.Substring(4) : device;
            string model;
            if (!friendly.TryGetValue(device, out model)) model = raw;
            list.Add(new Dictionary<string, object> {
                { "index", i },
                { "name", model },
                { "deviceName", raw },
                { "w", J.Int(m, "width") },
                { "h", J.Int(m, "height") },
                { "x", J.Int(m, "x") },
                { "y", J.Int(m, "y") },
            });
        }
        return list;
    }

    // config.yaml'daki borders.active_color (saydamlık korunur) + pencere yöneticisine yeniden yükle. Kurulumdan beri
    // elle değiştirilmemiş config bu değişiklikten sonra da "bizim" sayılır: güncellemeler yeni sürümünü yazabilir, renk
    // tercihten (prefs.json focusColor) yeniden uygulanır.
    static readonly object colorGate = new object();
    static Dictionary<string, object> SetFocusColor(string hex)
    {
        lock (colorGate) {
        if (!System.Text.RegularExpressions.Regex.IsMatch(hex, "^#[0-9a-fA-F]{6}$")) return Result(false);
        hex = hex.ToLowerInvariant();
        string cfg = System.IO.File.ReadAllText(Paths.ConfigFile);
        string hashFile = Paths.State("config.sha256");
        bool ours = false;
        try { ours = System.IO.File.Exists(hashFile) && System.IO.File.ReadAllText(hashFile).Trim() == Sha256(cfg); } catch { }
        var re = new System.Text.RegularExpressions.Regex(@"(?m)^(\s*active_color:\s*"")#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?("")");
        if (!re.IsMatch(cfg)) return Result(false);
        string next = re.Replace(cfg, x => x.Groups[1].Value + hex + x.Groups[2].Value + x.Groups[3].Value, 1);
        if (!Files.WriteAtomic(Paths.ConfigFile, next)) return Result(false);
        if (!Prefs.Set("focusColor", hex)) { Files.WriteAtomic(Paths.ConfigFile, cfg); return Result(false); }
        if (ours && !Files.WriteAtomic(hashFile, Sha256(next))) Slider.Log("focus color: could not update config hash");
        try { new TilingClient().Command("wm-reload-config"); } catch { }
        Toasts.Emit("ll:theme-color");
        var r = Result(true);
        r["focusColor"] = hex;
        return r;
        }
    }

    static Dictionary<string, object> SetWorkspaces(string[] mappings)
    {
        lock (colorGate) {
        var map = new Dictionary<int, int>();
        int? count = null;
        int? first = null;
        foreach (var mapping in mappings) {
            var parts = mapping.Split(':');
            int number, monitor;
            if (parts.Length != 2 || !int.TryParse(parts[1], out monitor))
                return WorkspaceFailure("Geçersiz çalışma alanı ayarı.");
            if (parts[0] == "count") count = monitor;
            else if (parts[0] == "first") first = monitor;
            else if (int.TryParse(parts[0], out number) && !map.ContainsKey(number)) map[number] = monitor;
            else return WorkspaceFailure("Geçersiz veya tekrarlanan çalışma alanı numarası.");
        }
        var client = new TilingClient();
        var monitors = client.Monitors();
        if (monitors.Count == 0) return WorkspaceFailure("Pencere yöneticisine ulaşılamadı.");
        string cfg = System.IO.File.ReadAllText(Paths.ConfigFile);
        List<WorkspaceConfigText.Entry> current;
        string error;
        if (!WorkspaceConfigText.TryRead(cfg, out current, out error)) return WorkspaceFailure(error);
        if (count.HasValue && count.Value < current.Count) {
            foreach (var active in client.Workspaces()) {
                int number;
                if (int.TryParse(J.Str(active, "name"), out number) && number > count.Value)
                    return WorkspaceFailure("Önce kaldırılacak çalışma alanlarını kapatın veya daha küçük bir numaraya taşıyın.");
            }
        }
        string next;
        if (!WorkspaceConfigText.TryRewrite(cfg, count, first, map, monitors.Count, out next, out error))
            return WorkspaceFailure(error);
        if (next == cfg) return Result(true);
        string hashFile = Paths.State("config.sha256");
        bool managed = false;
        try { managed = System.IO.File.Exists(hashFile) && System.IO.File.ReadAllText(hashFile).Trim() == Sha256(cfg); } catch { }
        if (!Files.WriteAtomic(Paths.ConfigFile, next)) return WorkspaceFailure("config.yaml yazılamadı.");
        if (!client.TryCommand("wm-reload-config", out error)) {
            Files.WriteAtomic(Paths.ConfigFile, cfg);
            string ignored; client.TryCommand("wm-reload-config", out ignored);
            return WorkspaceFailure("Pencere yöneticisi ayarı kabul etmedi: " + error);
        }
        // The reload command is complete only when active workspaces have
        // actually moved to their configured monitors.
        var expected = new Dictionary<string, int>();
        foreach (var assignment in map) expected[assignment.Key.ToString()] = assignment.Value;
        var after = client.Monitors();
        bool verified = after.Count == monitors.Count;
        for (int i = 0; i < after.Count && verified; i++)
            foreach (Dictionary<string, object> workspace in J.Children(after[i])) {
                int target;
                string name = J.Str(workspace, "name");
                if (expected.TryGetValue(name, out target) && target != i) { verified = false; break; }
            }
        if (!verified) {
            Files.WriteAtomic(Paths.ConfigFile, cfg);
            string ignored; client.TryCommand("wm-reload-config", out ignored);
            return WorkspaceFailure("Monitör atamaları uygulanamadı; önceki ayar geri yüklendi.");
        }
        if (managed && !Files.WriteAtomic(hashFile, Sha256(next)))
            Slider.Log("workspace settings: could not update config hash");
        return Result(true);
        }
    }

    static Dictionary<string, object> WorkspaceFailure(string message)
    {
        var response = Result(false);
        response["error"] = message;
        return response;
    }

    static Dictionary<string, object> Part(string key, Process p)
    {
        var d = new Dictionary<string, object> { { "key", key }, { "running", p != null } };
        if (p == null) return d;
        try { d["pid"] = p.Id; } catch { }
        try { d["uptime"] = (int)(DateTime.Now - p.StartTime).TotalSeconds; } catch { }
        try { d["memMB"] = (int)(p.PrivateMemorySize64 / (1024 * 1024)); } catch { }
        try { d["handles"] = p.HandleCount; } catch { }
        return d;
    }

    static Process Oldest(string name)
    {
        Process best = null;
        foreach (var p in Process.GetProcessesByName(name))
        {
            try { if (best == null || p.StartTime < best.StartTime) { if (best != null) best.Dispose(); best = p; continue; } } catch { }
            p.Dispose();
        }
        return best;
    }

    // Parçaların durumu, son kara kutu kaydı ve nöbetçilerin son işleri (core.log'dan)
    static Dictionary<string, object> Health()
    {
        Process core = null;
        try { core = Process.GetProcessById(int.Parse(System.IO.File.ReadAllText(Supervisor.PidFile).Trim())); if (!core.ProcessName.Equals(Names.Core, StringComparison.OrdinalIgnoreCase)) { core.Dispose(); core = null; } } catch { core = null; }
        var parts = new List<object> { Part("core", core), Part("tiling", Oldest(Names.Tiling)), Part("shell", Oldest(Names.Shell)) };
        var events = new List<string>();
        string blackBox = null;
        try
        {
            string log = System.IO.Path.Combine(Paths.LogsDir, "core.log");
            var lines = new List<string>();
            using (var fs = new System.IO.FileStream(log, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite))
            {
                long start = Math.Max(0, fs.Length - 400000);
                fs.Seek(start, System.IO.SeekOrigin.Begin);
                using (var sr = new System.IO.StreamReader(fs, Encoding.UTF8)) { string l; while ((l = sr.ReadLine()) != null) lines.Add(l); }
            }
            foreach (var l in lines)
            {
                if (l.Contains("KARA KUTU")) blackBox = l;
                else if (l.Contains("nöbetçisi") || l.Contains("masaüstü") || l.Contains("CRASH")) events.Add(l);
            }
        }
        catch { }
        if (events.Count > 6) events = events.GetRange(events.Count - 6, 6);
        object elevated = null;
        try { var cj = Js.Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(Paths.State("core.json"))); cj.TryGetValue("elevated", out elevated); } catch { }
        return new Dictionary<string, object> { { "parts", parts }, { "elevated", elevated }, { "blackBox", blackBox }, { "events", events }, { "logsDir", Paths.LogsDir } };
    }
}

static class BugReports
{
    static readonly JavaScriptSerializer Js = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };
    const int MaxLogBytes = 192 * 1024;
    internal static Func<string, string> CaptureBlackbox = PerfGuard.DumpNow;

    public static string File(string kind)
    {
        var result = new Dictionary<string, object> { { "ok", false }, { "kind", kind } };
        try
        {
            string text;
            if (kind == "blackbox")
            {
                // The log writer is asynchronous and may rotate the file. Return the captured record itself;
                // reading the log immediately after enqueueing it lost this attachment nondeterministically.
                text = CaptureBlackbox("hata raporu istendi");
            }
            else
            {
                string filename;
                switch (kind)
                {
                    case "core": filename = "core.log"; break;
                    case "shell": filename = "shell.log"; break;
                    case "tiling": filename = "tiling.log"; break;
                    default: throw new ArgumentException("Bilinmeyen günlük türü.");
                }
                text = ReadFrom(System.IO.Path.Combine(Paths.LogsDir, filename), -1, MaxLogBytes);
            }
            if (string.IsNullOrWhiteSpace(text)) throw new System.IO.IOException("Günlük boş veya erişilemiyor.");
            result["ok"] = true;
            result["text"] = text;
            result["bytes"] = Encoding.UTF8.GetByteCount(text);
        }
        catch (Exception ex) { result["error"] = ex.GetBaseException().Message; }
        return Js.Serialize(result);
    }

    static string ReadFrom(string path, long start, int limit)
    {
        if (limit <= 0) throw new ArgumentOutOfRangeException("limit");
        using (var fs = new System.IO.FileStream(path, System.IO.FileMode.Open, System.IO.FileAccess.Read,
            System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
        {
            long end = fs.Length;
            long offset = start < 0 ? Math.Max(0, end - limit) : start;
            if (offset > end) throw new System.IO.IOException("Günlük toplama sırasında değişti.");
            fs.Seek(offset, System.IO.SeekOrigin.Begin);
            // Snapshot the length and cap bytes, even while a writer appends.
            var bytes = new byte[(int)Math.Min(limit, end - offset)];
            int count = 0, read;
            while (count < bytes.Length && (read = fs.Read(bytes, count, bytes.Length - count)) > 0) count += read;
            var chars = new char[Encoding.UTF8.GetMaxCharCount(count)];
            int usedBytes, usedChars; bool completed;
            // Do not flush a truncated final UTF-8 character into a replacement.
            Encoding.UTF8.GetDecoder().Convert(bytes, 0, count, chars, 0, chars.Length, false, out usedBytes, out usedChars, out completed);
            string text = new string(chars, 0, usedChars);
            if (offset == 0) text = text.TrimStart('\uFEFF');
            // A tail may begin midway through a UTF-8 line.
            if (start < 0 && offset > 0) { int line = text.IndexOf('\n'); text = line < 0 ? "" : text.Substring(line + 1); }
            return text;
        }
    }

    static string WmiName(string query)
    {
        try
        {
            using (var search = new System.Management.ManagementObjectSearcher(query))
            {
                var names = new List<string>();
                foreach (System.Management.ManagementObject item in search.Get())
                {
                    using (item)
                    {
                        string name = Convert.ToString(item["Name"]);
                        if (!string.IsNullOrWhiteSpace(name) && !names.Contains(name)) names.Add(name);
                    }
                }
                return string.Join(", ", names.ToArray());
            }
        }
        catch { return ""; }
    }

    public static string Device()
    {
        string os = Environment.OSVersion.VersionString;
        try
        {
            using (var key = Microsoft.Win32.Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows NT\CurrentVersion"))
            {
                if (key != null) os = Convert.ToString(key.GetValue("ProductName")) + " "
                    + Convert.ToString(key.GetValue("DisplayVersion")) + " (" + Convert.ToString(key.GetValue("CurrentBuild")) + ")";
            }
        }
        catch { }
        string ram = "";
        try
        {
            using (var search = new System.Management.ManagementObjectSearcher("SELECT TotalPhysicalMemory FROM Win32_ComputerSystem"))
                foreach (System.Management.ManagementObject item in search.Get())
                    using (item) { ram = Math.Round(Convert.ToDouble(item["TotalPhysicalMemory"]) / (1024 * 1024 * 1024), 1) + " GB"; break; }
        }
        catch { }
        return Js.Serialize(new Dictionary<string, object> {
            { "os", os }, { "cpu", WmiName("SELECT Name FROM Win32_Processor") },
            { "gpu", WmiName("SELECT Name FROM Win32_VideoController") }, { "ram", ram },
            { "appVersion", Updater.Installed() }
        });
    }
}

static class ConfigWatch
{
    static System.IO.FileSystemWatcher watcher;
    static System.Threading.Timer configTimer;

    public static void Start(Control ui)
    {
        Prefs.Load();
        Anims.Load();
        ThreadPool.QueueUserWorkItem(_ => UiScale.Apply(UiScale.Percent(Prefs.Read())));
        try
        {
            watcher = new System.IO.FileSystemWatcher(Paths.ConfigDir) { NotifyFilter = System.IO.NotifyFilters.LastWrite | System.IO.NotifyFilters.FileName | System.IO.NotifyFilters.Size };
            // Kaydederken dosya birkaç kez yazılır: son değişiklikten 300 ms sonra bir kez oku
            configTimer = new System.Threading.Timer(_ =>
            {
                BorderStyle.Load();
                Anims.Load();
                UiScale.Apply(UiScale.Percent(Prefs.Read()));
                try { ui.BeginInvoke((Action)Slider.RepaintRings); } catch { }
            }, null, Timeout.Infinite, Timeout.Infinite);
            System.IO.FileSystemEventHandler on = (s, e) =>
            {
                if (e.Name.Equals("prefs.json", StringComparison.OrdinalIgnoreCase))
                {
                    Prefs.Load();
                    configTimer.Change(300, Timeout.Infinite);
                }
                else if (e.Name.Equals("config.yaml", StringComparison.OrdinalIgnoreCase)) configTimer.Change(300, Timeout.Infinite);
            };
            watcher.Changed += on; watcher.Created += on;
            watcher.Renamed += (s, e) => on(s, e);
            watcher.EnableRaisingEvents = true;
        }
        catch (Exception ex) { Slider.Log("ayar izleme: " + ex.Message); }
    }
}

// ---------------- Kullanıcı olarak başlatma ----------------
// Çekirdek ve pencere yöneticisi yönetici haklarıyla çalışır (kısayollar ve pencere yönetimi yönetici pencerelerinde de
// çalışsın diye). Kullanıcının açtığı programlar ise (terminal, tarayıcı, uygulama kısayolları, kabuk) her zaman normal
// kullanıcı olarak açılmalı: bunları Windows Gezgini'nin masaüstü kabuğu başlatır (IShellDispatch2.ShellExecute; Microsoft'un
// önerdiği yol). Çekirdek yönetici değilse doğrudan başlatır.
static class UserLaunch
{
    [ComImport, Guid("85CB6900-4D95-11CF-960C-0080C7F4EE85"), InterfaceType(ComInterfaceType.InterfaceIsDual)]
    interface IShellWindows
    {
        void _Count(); void _Item(); void _NewEnum(); void _Register(); void _RegisterPending(); void _Revoke(); void _OnNavigate(); void _OnActivated();
        [return: MarshalAs(UnmanagedType.IDispatch)]
        object FindWindowSW([In] ref object pvarLoc, [In] ref object pvarLocRoot, int swClass, out int phwnd, int swfwOptions);
    }

    [ComImport, Guid("6D5140C1-7436-11CE-8034-00AA006009FA"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IServiceProvider
    {
        [PreserveSig] int QueryService(ref Guid guidService, ref Guid riid, [MarshalAs(UnmanagedType.Interface)] out object ppvObject);
    }

    [ComImport, Guid("000214E2-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellBrowser
    {
        void _GetWindow(); void _ContextSensitiveHelp(); void _InsertMenusSB(); void _SetMenuSB(); void _RemoveMenusSB(); void _SetStatusTextSB();
        void _EnableModelessSB(); void _TranslateAcceleratorSB(); void _BrowseObject(); void _GetViewStateStream(); void _GetControlWindow(); void _SendControlMsg();
        [PreserveSig] int QueryActiveShellView(out IShellView ppshv);
    }

    [ComImport, Guid("000214E3-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellView
    {
        void _GetWindow(); void _ContextSensitiveHelp(); void _TranslateAccelerator(); void _EnableModeless(); void _UIActivate(); void _Refresh();
        void _CreateViewWindow(); void _DestroyViewWindow(); void _GetCurrentInfo(); void _AddPropertySheetPages(); void _SaveViewState(); void _SelectItem();
        [PreserveSig] int GetItemObject(uint uItem, ref Guid riid, [MarshalAs(UnmanagedType.IDispatch)] out object ppv);
    }

    static readonly Guid CLSID_ShellWindows = new Guid("9BA05972-F6A8-11CF-A442-00A0C90A8F39");
    static readonly Guid SID_STopLevelBrowser = new Guid("4C96BE40-915C-11CF-99D3-00AA004AE837");
    static readonly Guid IID_IShellBrowser = new Guid("000214E2-0000-0000-C000-000000000046");
    static readonly Guid IID_IDispatch = new Guid("00020400-0000-0000-C000-000000000046");
    const int SWC_DESKTOP = 8, SWFO_NEEDDISPATCH = 1, SVGIO_BACKGROUND = 0;

    static readonly bool elevated = CheckElevated();
    public static bool Elevated { get { return elevated; } }

    static bool CheckElevated()
    {
        try
        {
            using (var id = System.Security.Principal.WindowsIdentity.GetCurrent())
                return new System.Security.Principal.WindowsPrincipal(id).IsInRole(System.Security.Principal.WindowsBuiltInRole.Administrator);
        }
        catch { return false; }
    }

    // hidden: pencere gizli açılır (terminal ön-ısıtması). false: masaüstü kabuğu yok (Gezgin açılmamış / çökmüş).
    public static bool Start(string file, string args, string dir, bool hidden = false, string verb = "")
    {
        // Açılamayacak bir şeyde Gezgin kendi kutusunu gösterirdi: önce denetlenir, sorun bizim kartımızla söylenir
        int problem = Launcher.Check(file);
        if (problem != Launcher.OK) { Launcher.Report(file, problem); return false; }
        if (!elevated)
        {
            try
            {
                // ErrorDialog false: ShellExecuteEx SEE_MASK_FLAG_NO_UI ile (hata istisna olarak gelir, kutu açılmaz). Yeni süreç
                // olmayan başlatmada (shell:AppsFolder, Store uygulaması, açık örneğe devir) Process null döner: using null'ı geçer
                // (.Dispose() burada NullReference atıyor, açılan şey "açılamadı" sayılıyordu).
                using (Process.Start(new ProcessStartInfo(file, args ?? "") { UseShellExecute = true, ErrorDialog = false, Verb = verb ?? "", WorkingDirectory = dir ?? Paths.Home, WindowStyle = hidden ? ProcessWindowStyle.Hidden : ProcessWindowStyle.Normal })) { }
                return true;
            }
            catch (System.ComponentModel.Win32Exception ex)
            {
                // 1223: kullanıcı UAC'ı reddetti (bir hata değil)
                if (ex.NativeErrorCode != 1223) Launcher.Report(file, ex.NativeErrorCode);
                return false;
            }
            catch (Exception ex) { Slider.Log("başlatılamadı: " + file + ": " + ex.Message); return false; }
        }
        // COM nesneleri tek iş parçacıklı (STA) bir thread ister
        bool ok = false;
        var t = new Thread(() => ok = ViaDesktopShell(file, args ?? "", dir ?? Paths.Home, hidden ? 0 : 1, verb ?? "")) { IsBackground = true, Name = "user-launch" };
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
        if (!t.Join(15000)) { Slider.Log("kullanıcı olarak başlatma zaman aşımı: " + file); return false; }
        return ok;
    }

    static bool ViaDesktopShell(string file, string args, string dir, int show, string verb)
    {
        object windows = null, desktop = null, view = null, folderView = null, app = null;
        try
        {
            windows = Activator.CreateInstance(Type.GetTypeFromCLSID(CLSID_ShellWindows));
            object loc = 0 /* CSIDL_DESKTOP */, root = Type.Missing;
            int hwnd;
            desktop = ((IShellWindows)windows).FindWindowSW(ref loc, ref root, SWC_DESKTOP, out hwnd, SWFO_NEEDDISPATCH);
            if (desktop == null) return false;
            object browserObj;
            Guid sid = SID_STopLevelBrowser, iid = IID_IShellBrowser;
            if (((IServiceProvider)desktop).QueryService(ref sid, ref iid, out browserObj) != 0 || browserObj == null) return false;
            IShellView shellView;
            if (((IShellBrowser)browserObj).QueryActiveShellView(out shellView) != 0 || shellView == null) return false;
            view = shellView;
            Guid disp = IID_IDispatch;
            if (shellView.GetItemObject(SVGIO_BACKGROUND, ref disp, out folderView) != 0 || folderView == null) return false;
            app = folderView.GetType().InvokeMember("Application", System.Reflection.BindingFlags.GetProperty, null, folderView, null);
            app.GetType().InvokeMember("ShellExecute", System.Reflection.BindingFlags.InvokeMethod, null, app, new object[] { file, args, dir, verb, show });
            return true;
        }
        catch (Exception ex) { Slider.Log("kullanıcı olarak başlatılamadı (" + file + "): " + ex.Message); return false; }
        finally
        {
            foreach (var o in new[] { app, folderView, view, desktop, windows })
                if (o != null && Marshal.IsComObject(o)) try { Marshal.ReleaseComObject(o); } catch { }
        }
    }

    // Masaüstü kabuğu hazır olana kadar bekleyerek (oturum açılışında Gezgin çekirdekten sonra gelebilir)
    public static bool StartWhenReady(string file, string args, string dir, int waitMs)
    {
        // açılamayacaksa bir kez söylenir (beklerken her saniye kart çıkmasın)
        int problem = Launcher.Check(file);
        if (problem != Launcher.OK) { Launcher.Report(file, problem); return false; }
        var sw = Stopwatch.StartNew();
        while (true)
        {
            if (Start(file, args, dir)) return true;
            if (sw.ElapsedMilliseconds > waitMs) return false;
            Thread.Sleep(1000);
        }
    }
}

// ---------------- Üstte duran widget pencereleri ----------------
// Bildirim, güncelleme kartı ve ekran klavyesi en üstte duran saydam pencereler. Boşken gerçekten gizlenirler: saydam da
// olsa açık bir pencere oyunun üstünde duruyor, Windows oyunu doğrudan ekrana veremiyor (DWM her kareyi birleştiriyor) ve
// içinde kalan bir animasyon saniyede 144 kez çizilmeye devam ediyordu. Kabuk açık kaldıkça biriken kasmanın nedeni buydu;
// masaüstünü yenilemek widget'ları sıfırdan açtığı için geçiriyordu. Göster / gizle odak çalmadan burada yapılır (widget'lar
// /widget?w=..&v=.. ile ister). Tam ekran oyun ya da sunum sürerken bildirim ve güncelleme kartı gösterilmez; bittiğinde
// hâlâ gösterilmesi isteniyorsa gelir.
static class WidgetWindows
{
    [DllImport("shell32.dll")] static extern int SHQueryUserNotificationState(out int state);

    static readonly Dictionary<string, string> Titles = new Dictionary<string, string> { { "toast", Names.Toast }, { "update", Names.Update }, { "osk", Names.Osk } };
    static readonly Dictionary<string, bool> wanted = new Dictionary<string, bool>();
    static readonly object gate = new object();
    static bool waiting;

    // Tam ekran uygulama / D3D oyun / sunum: Windows kendi bildirimlerini de göstermez
    public static bool Busy()
    {
        int st;
        return SHQueryUserNotificationState(out st) == 0 && (st == 2 || st == 3 || st == 4);
    }

    // Özel (exclusive) tam ekran Direct3D uygulaması önde: ekranı doğrudan o çizer, bizim pencerelerimiz üstünde görünmez
    public static bool ExclusiveFullscreen()
    {
        int st;
        return SHQueryUserNotificationState(out st) == 0 && st == 3; // QUNS_RUNNING_D3D_FULL_SCREEN
    }

    public static bool Set(string w, bool visible)
    {
        if (!Titles.ContainsKey(w)) return false;
        lock (gate) wanted[w] = visible;
        if (!Apply(w)) WaitForGameEnd();
        return true;
    }

    // false: gösterilmesi gerekiyor ama oyun sürüyor
    static bool Apply(string w)
    {
        bool v;
        lock (gate) v = wanted.ContainsKey(w) && wanted[w];
        IntPtr h = Native.FindWindow(null, Titles[w]);
        if (h == IntPtr.Zero) return true;
        bool hold = v && w != "osk" && Busy();   // ekran klavyesini kullanıcı açar: her zaman
        if (v && !hold) Native.SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010 | 0x0040); // HWND_TOPMOST; NOSIZE|NOMOVE|NOACTIVATE|SHOWWINDOW
        else if (Native.IsWindowVisible(h)) Native.ShowWindowAsync(h, 0); // SW_HIDE
        return !hold;
    }

    static void WaitForGameEnd()
    {
        lock (gate) { if (waiting) return; waiting = true; }
        new Thread(() =>
        {
            try
            {
                while (true)
                {
                    Thread.Sleep(1000);
                    bool pending = false;
                    foreach (var w in new[] { "toast", "update" })
                    {
                        bool v; lock (gate) v = wanted.ContainsKey(w) && wanted[w];
                        if (v && !Apply(w)) pending = true;
                    }
                    if (!pending) break;
                }
            }
            catch (Exception ex) { Slider.Log("widget pencereleri: " + ex.Message); }
            finally { lock (gate) waiting = false; }
        }) { IsBackground = true, Name = "widget-wait", Priority = ThreadPriority.BelowNormal }.Start();
    }
}

// ---------------- Kök süreç ----------------
// lunge.exe masaüstünün köküdür: oturum açılınca yalnızca o başlar (\LogicalLunge\Start görevi); açılış perdesini,
// tiling'i ve shell'i kendi alt süreçleri olarak açar, Görev Yöneticisi üçünü tek "lunge" altında gruplar. Sonradan
// çöken parçayı nöbetçiler yine buradan başlatır. tiling'den çıkılınca (kod 0) shell'i kapatır, Windows görev
// çubuğunu geri getirir ve kendisi de çıkar.
// Intentional restart is a two-phase handoff. The next core waits before taking the main mutex,
// writing state or starting any desktop parts. A successful scheduler API call is not readiness.
static class DesktopRestart
{
    const int LaunchTimeout = 10000, CommitTimeout = 30000;
    static string PipeName { get { return "LogicalLunge.Restart." + System.Security.Principal.WindowsIdentity.GetCurrent().User.Value + "." + Process.GetCurrentProcess().SessionId; } }

    public static bool Handoff(Func<bool> prepare, Action stop, Func<bool> commit, Action rollback)
    {
        bool stopping = false;
        try
        {
            if (!prepare()) return false;
            stopping = true;
            stop();
            if (commit()) return true;
        }
        catch (Exception ex) { Slider.Log("restart handoff: " + ex.GetBaseException().Message); }
        if (stopping)
        {
            try { rollback(); } catch (Exception ex) { Slider.Log("restart recovery: " + ex.GetBaseException().Message); }
        }
        return false;
    }

    // No runas/ShellExecute: retain the caller's token without inheriting shell sockets or pipe handles.
    // Break away from a parent job so exiting the old scheduled action cannot take the coordinator down.
    // A job that denies breakaway may use the verified own-token broker route below.
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO
    {
        public int cb; public string reserved, desktop, title;
        public int x, y, xSize, ySize, xChars, yChars, fill, flags;
        public short show, reservedSize; public IntPtr reservedPtr, input, output, error;
    }
    [StructLayout(LayoutKind.Sequential)] struct PROCESS_INFORMATION { public IntPtr process, thread; public uint pid, tid; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string exe, StringBuilder command, IntPtr processSecurity, IntPtr threadSecurity,
        bool inheritHandles, uint flags, IntPtr environment, string directory, ref STARTUPINFO startup, out PROCESS_INFORMATION info);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    static extern bool CreateProcessWithTokenW(IntPtr token, uint logonFlags, string exe, StringBuilder command,
        uint flags, IntPtr environment, string directory, ref STARTUPINFO startup, out PROCESS_INFORMATION info);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool ProcessIdToSessionId(uint pid, out uint session);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetTokenInformation(IntPtr token, int tokenInfoClass, out int tokenInfo, int tokenInfoLength, out int returnLength);
    [DllImport("kernel32.dll")] static extern bool GetNamedPipeClientProcessId(IntPtr pipe, out uint pid);
    [DllImport("kernel32.dll")] static extern bool GetNamedPipeServerProcessId(IntPtr pipe, out uint pid);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle, uint timeout);

    // Retain the opened process object: a recycled PID must never become a termination target.
    public interface StopProcess : IDisposable { bool Exit(int timeout); }
    public sealed class OwnedProcess : StopProcess
    {
        IntPtr handle;
        public OwnedProcess(IntPtr value) { handle = value; }
        public bool Exit(int timeout)
        {
            return WaitForSingleObject(handle, 0) == 0 ||
                (TerminateProcess(handle, 0) && WaitForSingleObject(handle, (uint)timeout) == 0);
        }
        public void Dispose() { if (handle != IntPtr.Zero) { CloseHandle(handle); handle = IntPtr.Zero; } }
    }

    public sealed class CoreStop : IDisposable
    {
        readonly Mutex mutex;
        StopProcess process;
        bool reserved;
        public bool Exited { get { return reserved; } }
        public CoreStop() : this("LogicalLunge.Core", OpenCore) { }
        public CoreStop(string name, Func<StopProcess> open)
        {
            mutex = new Mutex(false, name);
            try
            {
                if (Reserve(0)) return; // PID file is unnecessary only when the actual mutex is free.
                process = open();
                if (process == null) throw new InvalidOperationException("No verified termination target");
            }
            catch { Dispose(); throw; }
        }
        static StopProcess OpenCore()
        {
            int pid;
            if (!int.TryParse(System.IO.File.ReadAllText(Supervisor.PidFile).Trim(), out pid) ||
                pid == Process.GetCurrentProcess().Id || !Peer((uint)pid, false))
                throw new InvalidOperationException("Core mutex is busy but its owner cannot be verified");
            // PROCESS_TERMINATE | SYNCHRONIZE | QUERY_LIMITED_INFORMATION, before any desktop mutation.
            IntPtr handle = OpenProcess(0x101001, false, (uint)pid);
            if (handle == IntPtr.Zero) throw new InvalidOperationException("No termination access to the existing core");
            var target = new OwnedProcess(handle);
            if (!Peer((uint)pid, false)) { target.Dispose(); throw new InvalidOperationException("Core identity changed during preparation"); }
            return target;
        }
        bool Reserve(int timeout)
        {
            try { reserved = mutex.WaitOne(timeout); }
            catch (AbandonedMutexException) { reserved = true; }
            return reserved;
        }
        public void ExitAndReserve()
        {
            if (reserved) return;
            if (Reserve(0)) return;
            if (process == null || !process.Exit(3000) || !Reserve(3000))
                throw new InvalidOperationException("Old core exit and free mutex could not be verified");
        }
        public void Release()
        {
            if (reserved) { mutex.ReleaseMutex(); reserved = false; }
        }
        public void Dispose() { Release(); if (process != null) process.Dispose(); mutex.Dispose(); }
    }

    public static bool StartInherited(string exe, string args)
    {
        using (var process = StartOwned(exe, args)) return process != null;
    }
    public static OwnedProcess StartOwned(string exe, string args)
    {
        var startup = new STARTUPINFO { cb = Marshal.SizeOf(typeof(STARTUPINFO)), flags = 1, show = 0 };
        PROCESS_INFORMATION info;
        if (!CreateProcess(exe, new StringBuilder("\"" + exe + "\" " + (args ?? "")), IntPtr.Zero, IntPtr.Zero,
            false, 0x09000000, IntPtr.Zero, Paths.Home, ref startup, out info)) // NO_WINDOW | BREAKAWAY_FROM_JOB
        {
            int error = Marshal.GetLastWin32Error();
            if (!CanBrokerLaunch(error, UserLaunch.Elevated) || !StartOwnToken(exe, args, ref startup, out info))
            {
                Slider.Log("restart launch failed: " + error);
                return null;
            }
        }
        CloseHandle(info.thread);
        return new OwnedProcess(info.process);
    }

    public static bool CanBrokerLaunch(int error, bool elevated) { return error == 5 && elevated; }
    static bool StartOwnToken(string exe, string args, ref STARTUPINFO startup, out PROCESS_INFORMATION info)
    {
        info = new PROCESS_INFORMATION();
        string taskName = "Restart-" + Guid.NewGuid().ToString("N").Substring(0, 8);
        object service = null, folder = null, regTask = null, run = null;
        try
        {
            service = Activator.CreateInstance(Type.GetTypeFromProgID("Schedule.Service"));
            Com(service, "Connect", false, null, null, null, null);
            folder = Com(service, "GetFolder", false, @"\LogicalLunge");
            try
            {
                object tasks = Com(folder, "GetTasks", false, 1);
                int count = Convert.ToInt32(Com(tasks, "Count", true));
                for (int i = 1; i <= count; i++)
                {
                    object t = Com(tasks, "get_Item", false, i);
                    try
                    {
                        string name = (string)Com(t, "Name", true);
                        if (name.StartsWith("Restart-") && Convert.ToInt32(Com(t, "State", true)) != 4)
                            Com(folder, "DeleteTask", false, name, 0);
                    }
                    catch { }
                    finally { ReleaseCom(t); }
                }
                ReleaseCom(tasks);
            }
            catch { }
            using (var own = System.Security.Principal.WindowsIdentity.GetCurrent())
            {
                string xml = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>" +
                    "<Task version=\"1.3\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">" +
                    "<RegistrationInfo/><Principals><Principal id=\"A\">" +
                    "<UserId>" + own.User.Value + "</UserId>" +
                    "<LogonType>InteractiveToken</LogonType>" +
                    "<RunLevel>HighestAvailable</RunLevel>" +
                    "</Principal></Principals><Settings>" +
                    "<MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>" +
                    "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>" +
                    "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>" +
                    "<ExecutionTimeLimit>PT2M</ExecutionTimeLimit>" +
                    "<UseUnifiedSchedulingEngine>true</UseUnifiedSchedulingEngine>" +
                    "</Settings><Actions Context=\"A\"><Exec>" +
                    "<Command>" + System.Security.SecurityElement.Escape(exe) + "</Command>" +
                    "<Arguments>" + System.Security.SecurityElement.Escape(args ?? "") + "</Arguments>" +
                    "<WorkingDirectory>" + System.Security.SecurityElement.Escape(Paths.Home) + "</WorkingDirectory>" +
                    "</Exec></Actions></Task>";
                regTask = Com(folder, "RegisterTask", false, taskName, xml, 2, null, null, 3, null);
            }
            run = Com(regTask, "RunEx", false, null, 4, Process.GetCurrentProcess().SessionId, null);
            if (run != null)
            {
                ThreadPool.QueueUserWorkItem(_ =>
                {
                    try
                    {
                        Thread.Sleep(30000);
                        object s = Activator.CreateInstance(Type.GetTypeFromProgID("Schedule.Service"));
                        Com(s, "Connect", false, null, null, null, null);
                        object f = Com(s, "GetFolder", false, @"\LogicalLunge");
                        Com(f, "DeleteTask", false, taskName, 0);
                        ReleaseCom(f); ReleaseCom(s);
                    }
                    catch { }
                });
            }
            return run != null;
        }
        catch (Exception ex)
        {
            Slider.Log("restart temp-task: " + ex.GetBaseException().Message);
            try { if (folder != null) Com(folder, "DeleteTask", false, taskName, 0); } catch { }
            return false;
        }
        finally
        {
            ReleaseCom(run); ReleaseCom(regTask);
            ReleaseCom(folder); ReleaseCom(service);
        }
    }
    // Validate both pipe peers using OS identity, never a claimed PID or elevation in a message.
    static bool Peer(uint pid, bool requireElevated)
    {
        uint session;
        if (!ProcessIdToSessionId(pid, out session) || session != Process.GetCurrentProcess().SessionId) return false;
        string path = ProcInfo.Path(pid);
        if (!string.Equals(path, Paths.Core, StringComparison.OrdinalIgnoreCase)) return false;
        IntPtr process = OpenProcess(0x1000, false, pid), token = IntPtr.Zero;
        if (process == IntPtr.Zero) return false;
        try
        {
            if (!OpenProcessToken(process, 8, out token)) return false;
            using (var identity = new System.Security.Principal.WindowsIdentity(token))
            using (var own = System.Security.Principal.WindowsIdentity.GetCurrent())
            {
                if (identity.User != own.User) return false;
                if (!requireElevated) return true;
                int isElevated = 0, returnLength = 0;
                // TokenElevation = 20. Win32 TokenElevation works directly on primary tokens without throwing.
                return GetTokenInformation(token, 20, out isElevated, 4, out returnLength) && isElevated != 0;
            }
        }
        catch { return false; }
        finally { if (token != IntPtr.Zero) CloseHandle(token); CloseHandle(process); }
    }

    public static bool ReadSignal(System.IO.Stream stream, byte expected, int timeout)
    {
        var buffer = new byte[1];
        var read = stream.ReadAsync(buffer, 0, 1);
        return read.Wait(timeout) && read.Result == 1 && buffer[0] == expected;
    }
    public static bool WriteSignal(System.IO.Stream stream, byte value, int timeout)
    {
        return stream.WriteAsync(new byte[] { value }, 0, 1).Wait(timeout);
    }

    // A rejected client is disconnected, not allowed to consume the entire launch attempt.
    public static bool WaitCandidate(System.IO.Pipes.NamedPipeServerStream pipe, Func<bool> launch,
        Func<bool> peer, int timeout)
    {
        var clock = Stopwatch.StartNew();
        bool launched = false;
        while (clock.ElapsedMilliseconds < timeout)
        {
            // A wait that ends early (no successor in time, its launch failed) is cancelled and finished here. Left
            // behind with its event disposed, it completed later on an I/O thread when the pipe closed, and the
            // ObjectDisposedException there killed the core.
            using (var cancel = new CancellationTokenSource())
            {
                var connected = pipe.WaitForConnectionAsync(cancel.Token);
                bool ok = false;
                try
                {
                    if (!launched) { launched = true; if (!launch()) return false; }
                    ok = connected.Wait(Math.Max(0, timeout - (int)clock.ElapsedMilliseconds));
                }
                finally
                {
                    if (!ok)
                    {
                        cancel.Cancel();
                        try { connected.Wait(2000); } catch (AggregateException) { }
                    }
                }
                if (!ok) return false;
            }
            try
            {
                if (peer() && ReadSignal(pipe, (byte)'R', Math.Min(5000, Math.Max(0, timeout - (int)clock.ElapsedMilliseconds))))
                    return true;
            }
            catch (System.IO.IOException) { }
            catch (AggregateException ex) { if (!(ex.GetBaseException() is System.IO.IOException)) throw; }
            pipe.Disconnect();
        }
        return false;
    }

    public sealed class Candidate : IDisposable
    {
        readonly System.IO.Pipes.NamedPipeServerStream pipe;
        uint pid;
        public Candidate()
        {
            var security = new System.IO.Pipes.PipeSecurity();
            using (var own = System.Security.Principal.WindowsIdentity.GetCurrent())
                security.AddAccessRule(new System.IO.Pipes.PipeAccessRule(own.User, System.IO.Pipes.PipeAccessRights.FullControl,
                    System.Security.AccessControl.AccessControlType.Allow));
            pipe = new System.IO.Pipes.NamedPipeServerStream(PipeName, System.IO.Pipes.PipeDirection.InOut, 1,
                System.IO.Pipes.PipeTransmissionMode.Byte, System.IO.Pipes.PipeOptions.Asynchronous, 0, 0, security);
        }
        public bool Prepare(Func<bool> launch)
        {
            return WaitCandidate(pipe, launch, () =>
                GetNamedPipeClientProcessId(pipe.SafePipeHandle.DangerousGetHandle(), out pid) && Peer(pid, true), LaunchTimeout);
        }
        public bool Alive { get { return pipe.IsConnected && Peer(pid, true); } }
        public bool Commit()
        {
            return WriteSignal(pipe, (byte)'C', 3000) && ReadSignal(pipe, (byte)'S', CommitTimeout) &&
                WriteSignal(pipe, (byte)'A', 3000);
        }
        public void Dispose() { pipe.Dispose(); } // EOF aborts a candidate that has not received commit.
    }

    public static bool CanRunTask(string xml, string core, string home, string sid, string account, int running)
    {
        try
        {
            var doc = new System.Xml.XmlDocument { XmlResolver = null }; doc.LoadXml(xml);
            var ns = new System.Xml.XmlNamespaceManager(doc.NameTable); ns.AddNamespace("t", "http://schemas.microsoft.com/windows/2004/02/mit/task");
            Func<string, string> value = path => { var node = doc.SelectSingleNode("/t:Task/" + path, ns); return node == null ? "" : node.InnerText; };
            string user = value("t:Principals/t:Principal/t:UserId"), policy = value("t:Settings/t:MultipleInstancesPolicy");
            var principal = doc.SelectSingleNode("/t:Task/t:Principals/t:Principal", ns) as System.Xml.XmlElement;
            var actions = doc.SelectSingleNode("/t:Task/t:Actions", ns) as System.Xml.XmlElement;
            return doc.SelectNodes("/t:Task/t:Principals/t:Principal", ns).Count == 1 &&
                doc.SelectNodes("/t:Task/t:Actions/*", ns).Count == 1 &&
                principal != null && actions != null && principal.GetAttribute("id") == actions.GetAttribute("Context") &&
                (user == sid || string.Equals(user, account, StringComparison.OrdinalIgnoreCase)) &&
                value("t:Principals/t:Principal/t:LogonType") == "InteractiveToken" &&
                value("t:Principals/t:Principal/t:RunLevel") == "HighestAvailable" &&
                string.Equals(value("t:Actions/t:Exec/t:Command"), core, StringComparison.OrdinalIgnoreCase) &&
                string.Equals(value("t:Actions/t:Exec/t:WorkingDirectory"), home, StringComparison.OrdinalIgnoreCase) &&
                string.IsNullOrWhiteSpace(value("t:Actions/t:Exec/t:Arguments")) &&
                value("t:Settings/t:Enabled") != "false" && value("t:Settings/t:Enabled") != "0" &&
                value("t:Settings/t:AllowStartOnDemand") != "false" && value("t:Settings/t:AllowStartOnDemand") != "0" &&
                (policy == "Parallel" || ((policy == "IgnoreNew" || policy == "Queue" || policy == "") && running == 0));
        }
        catch { return false; }
    }

    internal static object Com(object obj, string member, bool property, params object[] args)
    {
        return obj.GetType().InvokeMember(member, property ? System.Reflection.BindingFlags.GetProperty : System.Reflection.BindingFlags.InvokeMethod,
            null, obj, args);
    }
    internal static void ReleaseCom(object obj) { if (obj != null && Marshal.IsComObject(obj)) Marshal.FinalReleaseComObject(obj); }
    static bool StartTask()
    {
        object service = null, folder = null, task = null, instances = null, run = null;
        try
        {
            service = Activator.CreateInstance(Type.GetTypeFromProgID("Schedule.Service"));
            Com(service, "Connect", false, null, null, null, null);
            folder = Com(service, "GetFolder", false, @"\LogicalLunge");
            task = Com(folder, "GetTask", false, "Start");
            instances = Com(task, "GetInstances", false, 0);
            using (var own = System.Security.Principal.WindowsIdentity.GetCurrent())
                if (!CanRunTask((string)Com(task, "Xml", true), Paths.Core, Paths.Home, own.User.Value, own.Name,
                    Convert.ToInt32(Com(instances, "Count", true)))) return false;
            // Use the registered highest principal in this interactive session. Never TASK_RUN_AS_SELF.
            run = Com(task, "RunEx", false, null, 4, Process.GetCurrentProcess().SessionId, null);
            return run != null;
        }
        catch (Exception ex) { Slider.Log("restart task unavailable: " + ex.GetBaseException().Message); return false; }
        finally { ReleaseCom(run); ReleaseCom(instances); ReleaseCom(task); ReleaseCom(folder); ReleaseCom(service); }
    }

    static Candidate TryPrepare(Func<bool> launch)
    {
        Candidate candidate = null;
        try
        {
            candidate = new Candidate();
            if (candidate.Prepare(launch)) return candidate;
        }
        catch (Exception ex) { Slider.Log("restart preparation: " + ex.GetBaseException().Message); }
        if (candidate != null) candidate.Dispose();
        return null;
    }
    public static bool PrepareRoutes(bool elevated, Func<bool> scheduled, Func<bool> inherited)
    {
        if (scheduled()) return true;
        // A running IgnoreNew task cannot launch a second instance. An already elevated coordinator can
        // preserve its token directly. A medium coordinator must abort rather than start a medium desktop.
        return elevated && inherited();
    }
    public static Candidate Prepare()
    {
        Candidate candidate = null;
        PrepareRoutes(UserLaunch.Elevated, () => { candidate = TryPrepare(StartTask); return candidate != null; },
            () => { candidate = TryPrepare(() => StartInherited(Paths.Core, "--restart-desktop-candidate")); return candidate != null; });
        return candidate;
    }

    // Arg-less scheduler action joins a pending restart; normal sign-in has no pipe and starts normally.
    public static bool Join(bool required, out System.IO.Pipes.NamedPipeClientStream committed)
    {
        committed = null;
        var pipe = new System.IO.Pipes.NamedPipeClientStream(".", PipeName, System.IO.Pipes.PipeDirection.InOut,
            System.IO.Pipes.PipeOptions.Asynchronous, System.Security.Principal.TokenImpersonationLevel.Identification);
        bool connected = false;
        try
        {
            pipe.Connect(required ? 3000 : 500); connected = true;
            uint server;
            if (!GetNamedPipeServerProcessId(pipe.SafePipeHandle.DangerousGetHandle(), out server) || !Peer(server, false) ||
                !UserLaunch.Elevated || !System.IO.File.Exists(Paths.Tiling) || !System.IO.File.Exists(Paths.Shell)) return false;
            // Prove that the existing main mutex is accessible before sending readiness.
            Mutex main; if (Mutex.TryOpenExisting("LogicalLunge.Core", out main)) main.Dispose();
            if (!WriteSignal(pipe, (byte)'R', 3000)) return false;
            if (!ReadSignal(pipe, (byte)'C', CommitTimeout)) return false;
            committed = pipe; return true;
        }
        catch (TimeoutException) { return !required && !connected; }
        catch (Exception ex) { Slider.Log("restart candidate: " + ex.GetBaseException().Message); return false; }
        finally { if (committed == null) pipe.Dispose(); }
    }
    public sealed class Startup : IDisposable
    {
        public int BringUpDone, UiReady, Initialized;
        int failed;
        public volatile bool Accepted;
        readonly System.Threading.Tasks.Task worker;
        public Startup(System.IO.Pipes.NamedPipeClientStream pipe)
        {
            worker = WatchStartup(pipe, () => Volatile.Read(ref BringUpDone) == 1 &&
                Volatile.Read(ref UiReady) == 1 && Volatile.Read(ref Initialized) == 1 &&
                Supervisor.RestartReady(), () => Volatile.Read(ref failed) != 0,
                () => { Accepted = true; SelfHeal.IsMain = true; }, () => Environment.Exit(3), CommitTimeout);
        }
        public void Fail() { Interlocked.Exchange(ref failed, 1); }
        public void Wait(int timeout) { try { worker.Wait(timeout); } catch { } }
        public void Dispose() { if (!Accepted) { Fail(); Wait(3500); } }
    }

    // Keep EOF/rollback observable throughout initialization, including a stuck UI or BringUp.
    public static System.Threading.Tasks.Task WatchStartup(System.IO.Stream pipe, Func<bool> ready,
        Func<bool> failed, Action accepted, Action abort, int timeout)
    {
        // Independent of HTTP/hook thread-pool load: the startup/rollback deadline must keep advancing.
        return System.Threading.Tasks.Task.Factory.StartNew(() =>
        {
            bool success = false;
            try
            {
                var clock = Stopwatch.StartNew();
                var signal = new byte[1];
                var reply = pipe.ReadAsync(signal, 0, 1);
                bool sent = false;
                while (clock.ElapsedMilliseconds < timeout && !failed())
                {
                    if (reply.IsCompleted)
                    {
                        success = sent && reply.Result == 1 && signal[0] == (byte)'A';
                        break;
                    }
                    if (!sent && ready()) { if (!WriteSignal(pipe, (byte)'S', 1000)) break; sent = true; }
                    Thread.Sleep(20);
                }
                if (!success) { try { WriteSignal(pipe, (byte)'F', 500); } catch { } }
            }
            catch (Exception ex) { Slider.Log("restart startup: " + ex.GetBaseException().Message); }
            finally
            {
                pipe.Dispose();
                if (success) accepted(); else abort();
            }
        }, CancellationToken.None, System.Threading.Tasks.TaskCreationOptions.LongRunning, System.Threading.Tasks.TaskScheduler.Default);
    }
}

static class Supervisor
{
    public static string PidFile { get { return Paths.State("core.pid"); } }

    // Eksik parçaları sırayla açar: perde (istenirse), tiling, IPC'si hazır olunca shell. Çalışanlara dokunmaz.
    public static void BringUp(bool splash)
    {
        if (!Maint.Running(Names.Tiling))
        {
            if (splash && !WarmTerminal.SplashActive()) Start(Paths.Core, "--splash");
            TilingWatchdog.StartTiling();
        }
        // shell tiling'e bağlanır: IPC açılınca başlat (açılmazsa da başlat; bar ve güvenli taraf yine gelsin)
        var sw = Stopwatch.StartNew();
        while (!TilingIpcUp() && sw.ElapsedMilliseconds < 20000) Thread.Sleep(150);
        if (!Maint.Running(Names.Shell)) ShellWatchdog.StartShell("açılış");
        try { LiveWallpaper.EnsureRunning(); } catch (Exception ex) { Slider.Log("canlı duvar kağıdı başlatılamadı: " + ex.Message); }
        // Masaüstü yeniden ayakta: bakım bitti. --stop-desktop'ın bıraktığı işaret kalınca odak bekçisi, parça nöbetçileri
        // ve kendini toparlama 10 dakika susuyordu (her "masaüstünü yenile" / durdur-başlat sonrasında; gizlenen overview
        // önde kalıyor, boş workspace'te tuşlar görünmeyen pencereye gidiyordu).
        Maint.Unmark();
    }

    internal static bool TilingIpcUp()
    {
        try { using (var c = new System.Net.Sockets.TcpClient()) return c.ConnectAsync("127.0.0.1", 6123).Wait(150) && c.Connected; }
        catch { return false; }
    }

    public static bool RestartReady()
    {
        return Toasts.Listening && Maint.Running(Names.Tiling) && Maint.Running(Names.Shell) && TilingIpcUp();
    }

    // ShellExecute: çekirdeğin tutamaçları alt sürece miras kalmasın
    static void Start(string exe, string args)
    {
        try { Process.Start(new ProcessStartInfo(exe, args ?? "") { UseShellExecute = true, WorkingDirectory = Paths.Home }); }
        catch (Exception ex) { Slider.Log("kök: " + System.IO.Path.GetFileName(exe) + " " + args + " başlatılamadı: " + ex.Message); }
    }

    static void Kill(string name, int exceptPid = 0)
    {
        foreach (var p in Process.GetProcessesByName(name))
        {
            try { if (p.Id != exceptPid) { p.Kill(); p.WaitForExit(3000); } }
            catch { }
            finally { p.Dispose(); }
        }
    }

    // tiling'den çıkıldı: shell'i kapat, Windows görev çubuğunu geri getir. exitSelf: çekirdek de çıkar.
    public static void Shutdown(string why, bool exitSelf)
    {
        Slider.Log("kapanış: " + why);
        Kill(Names.Shell);
        LiveWallpaper.Stop();
        ShellTakeover.ReleaseAll();
        if (exitSelf) Environment.Exit(0);
    }

    // Asıl çekirdek (kilidi tutan, argümansız başlayan): açılışta süreç kimliğini yazar
    static Process MainCore()
    {
        try
        {
            int pid = int.Parse(System.IO.File.ReadAllText(PidFile).Trim());
            var p = Process.GetProcessById(pid);
            if (p.Id != Process.GetCurrentProcess().Id && p.ProcessName.Equals(Names.Core, StringComparison.OrdinalIgnoreCase)) return p;
            p.Dispose();
        }
        catch { }
        return null;
    }

    // Masaüstünü kapatır (kurulum, güncelleme, "masaüstünü yenile"): bakım işareti, önce çekirdek (kapanan parçaları
    // yeniden başlatmasın), pencere yöneticisine nazik çıkış (gizli workspace'lerin pencerelerini geri getirir), kalanlar,
    // sonra görünmez kalmış pencereler. Bakım işareti kalır; kaldırmak çağıranın işi (en geç 10 dakikada geçersizleşir).
    public static void StopDesktop(bool requireCoreExit = false)
    {
        if (requireCoreExit)
        {
            using (var stop = new DesktopRestart.CoreStop()) { stop.ExitAndReserve(); StopDesktopParts(); }
            return;
        }
        Maint.Mark();
        LiveWallpaper.Stop();
        var core = MainCore();
        if (core != null)
        {
            try { core.Kill(); if (!core.WaitForExit(3000) && requireCoreExit) throw new InvalidOperationException("Old core did not exit"); }
            catch { if (requireCoreExit) throw; }
            finally { core.Dispose(); }
        }
        StopDesktopParts();
    }

    static void StopDesktopParts()
    {
        Maint.Mark();
        LiveWallpaper.Stop();
        if (Maint.Running(Names.Tiling))
        {
            try { new TilingClient().Command("wm-exit"); } catch { }
            var sw = Stopwatch.StartNew();
            while (Maint.Running(Names.Tiling) && sw.ElapsedMilliseconds < 6000) Thread.Sleep(100);
        }
        Kill(Names.Tiling);
        Kill(Names.Shell);
        // Bilerek kapatılan kabuk çökmüş sayılmasın: kabuğun çöküş döngüsü kaydı (son açılışları) silinir. Yoksa iki
        // dakikada üç "masaüstünü yenile" yerel barı bırakıp web barına düşürüyordu.
        try { System.IO.File.Delete(Paths.State("native-ui-starts")); } catch { }
        Kill(LiveWallpaper.Name);
        Thread.Sleep(300);
        // Windows'un devredilen parçaları (görev çubuğu, ayarlar, bildirim balonları) geri gelir; yeni çekirdek yeniden alır
        ShellTakeover.ReleaseAll();
        Slider.Log("masaüstü kapatıldı; geri getirilen pencere: " + Orphans.Uncloak());
    }

    // "Masaüstünü yenile" (oturum menüsü, Başlat kısayolu; lunge.exe --restart-desktop): bütün parçaları kapatıp temiz
    // baştan açar. Açık pencereler kapanmaz, yeniden başlayan pencere yöneticisi onları yerleştirir; geçişi perde örter.
    // Çalışan çekirdekten iste (yerel kanal /cmd?a=...): kabul ettiyse true. Kabuk ve kurulum normal kullanıcı olarak
    // çalışır; yönetici haklarıyla çalışan çekirdeği ve pencere yöneticisini ancak çekirdeğin kendisi kapatabilir.
    public static bool RequestFromCore(string act)
    {
        return PostToCore("/cmd?a=" + act, 2000) == 202;
    }

    // Komut satırından çalışan çekirdeğe istek (tek yer): HTTP durum kodu; çekirdek yoksa / cevap vermezse 0
    public static int PostToCore(string target, int timeoutMs)
    {
        try
        {
            var rq = (System.Net.HttpWebRequest)System.Net.WebRequest.Create("http://127.0.0.1:6131" + target);
            rq.Method = "POST"; rq.ContentLength = 0; rq.Timeout = timeoutMs; rq.Proxy = null;
            using (var rs = (System.Net.HttpWebResponse)rq.GetResponse()) return (int)rs.StatusCode;
        }
        catch (System.Net.WebException ex)
        {
            var rs = ex.Response as System.Net.HttpWebResponse;
            if (rs == null) return 0;
            using (rs) return (int)rs.StatusCode;
        }
        catch { return 0; }
    }

    // lunge.exe --stop-desktop: çalışan çekirdek her şeyi kapatıp çıkar (beklenir); kalanları (0.1.x parçaları, yanıt
    // vermeyen bir çekirdek) bu süreç kapatır
    public static void StopDesktopFromAnywhere()
    {
        var core = MainCore();
        if (core != null)
        {
            try { if (RequestFromCore("stop-desktop")) core.WaitForExit(30000); }
            catch { }
            finally { core.Dispose(); }
        }
        StopDesktop();
    }

    // Detached coordinator retains the main core's elevation and inherits no shell/socket handles.
    public static void RestartDesktopDetached()
    {
        DesktopRestart.StartInherited(Paths.Core, "--restart-desktop-now");
    }

    public static void RestartDesktop()
    {
        using (var gate = new Mutex(false, @"Local\LogicalLunge.Restart.Coordinator"))
        {
            bool owned; try { owned = gate.WaitOne(0); } catch (AbandonedMutexException) { owned = true; }
            if (!owned) return;
            DesktopRestart.Candidate next = null;
            DesktopRestart.OwnedProcess splash = null;
            DesktopRestart.CoreStop stop = null;
            bool changed = false;
            try
            {
                // A medium fallback without termination access must fail before task launch or any state change.
                stop = new DesktopRestart.CoreStop();
                bool ok = DesktopRestart.Handoff(() => { next = DesktopRestart.Prepare(); return next != null; }, () =>
                {
                    if (!next.Alive) throw new InvalidOperationException("Prepared core exited");
                    stop.ExitAndReserve();
                    changed = true;
                    StopDesktopParts();
                    // Old core is gone and the mutex is reserved. Only now create an owned, cancellable cover.
                    splash = DesktopRestart.StartOwned(Paths.Core, "--splash");
                    stop.Release();
                }, () => next.Commit(), () =>
                {
                    if (splash != null) { splash.Exit(3000); splash.Dispose(); splash = null; }
                    next.Dispose(); next = null;
                    if (!changed) return; // failed kill: leave wallpaper, maintenance and the running desktop intact
                    // EOF makes the failed candidate exit without SelfHeal. Reserve the mutex before cleanup.
                    stop.ExitAndReserve();
                    StopDesktopParts();
                    stop.Release();
                    Maint.Unmark();
                    using (var recovery = DesktopRestart.Prepare())
                        if (recovery == null || !recovery.Commit()) Slider.Log("restart recovery failed; Windows desktop restored");
                });
                if (!ok) Slider.Log("restart aborted or recovered; elevated handoff was not completed");
            }
            catch (Exception ex) { Slider.Log("restart refused: " + ex.GetBaseException().Message); }
            finally
            {
                if (next != null) next.Dispose();
                if (splash != null) splash.Dispose();
                if (stop != null) stop.Dispose();
                gate.ReleaseMutex();
            }
        }
    }
}

// ---------------- Kendini toparlama ----------------
// Bir OS gibi: bir parça çökerse ya da donarsa kullanıcı komut satırı, taskkill bilmeden masaüstü kendiliğinden
// toparlanır. Nöbetçiler birbirini korur (yeni süreç yok):
//   tiling çöker / donar      -> helper (TilingWatchdog) masaüstünü yeniden başlatır
//   shell çöker                -> helper (ShellWatchdog)
//   helper çöker (yönetilen hata) ya da arayüzü donar -> helper kendini yeniden başlatır (SelfHeal)
//   helper tamamen ölür        -> shell'in bildirim kopyası (lunge --toast-stream) onu başlatır
// Hepsi kasıtlı çıkışta (tiling 0 koduyla kapanır, kapanırken helper'ı ve shell'i da kapatır), oturum kapanırken
// ve bakım sırasında (kurulum, güncelleme, kaldırma, "masaüstünü yenile") hiçbir şey yapmaz. Çöküş döngüsüne
// girmesinler diye her biri 5 dakikada en fazla 3 kez dener.
static class Maint
{
    public static volatile bool SessionEnding;
    static string Dir { get { return Paths.StateDir; } }

    // Bakım işareti: kurulum / güncelleme / kaldırma / "masaüstünü yenile" bırakır. Yarım kalan bir işlem nöbetçileri
    // sonsuza dek susturmasın diye 10 dakikadan eskisi sayılmaz.
    public static bool Quiet()
    {
        if (SessionEnding) return true;
        try
        {
            var fi = new System.IO.FileInfo(System.IO.Path.Combine(Dir, "maintenance"));
            return fi.Exists && (DateTime.UtcNow - fi.LastWriteTimeUtc).TotalMinutes < 10;
        }
        catch { return false; }
    }

    // Son 5 dakikadaki denemeler (süreçler arası, dosyada): sınırı aşmadıysa bu denemeyi kaydeder ve true döner.
    public static void Mark()
    {
        try { System.IO.File.WriteAllText(System.IO.Path.Combine(Dir, "maintenance"), DateTime.UtcNow.ToString("o")); } catch { }
    }

    public static void Unmark()
    {
        try { System.IO.File.Delete(System.IO.Path.Combine(Dir, "maintenance")); } catch { }
    }

    public static bool Allow(string name)
    {
        string file = System.IO.Path.Combine(Dir, name);
        var now = DateTime.UtcNow;
        var recent = new List<long>();
        try
        {
            foreach (var line in System.IO.File.ReadAllLines(file))
            {
                long t;
                if (long.TryParse(line, out t) && (now - new DateTime(t, DateTimeKind.Utc)).TotalMinutes < 5) recent.Add(t);
            }
        }
        catch { }
        if (recent.Count >= 3) return false;
        recent.Add(now.Ticks);
        try
        {
            System.IO.Directory.CreateDirectory(Dir);
            System.IO.File.WriteAllLines(file, recent.ConvertAll(t => t.ToString()).ToArray());
        }
        catch { }
        return true;
    }

    public static string CoreExe { get { return Paths.Core; } }

    public static int RunHidden(string exe, string args, int waitMs)
    {
        try
        {
            using (var p = Process.Start(new ProcessStartInfo(exe, args) { UseShellExecute = false, CreateNoWindow = true }))
                return p.WaitForExit(waitMs) ? p.ExitCode : -1;
        }
        catch { return -1; }
    }

    public static bool Running(string name)
    {
        var ps = Process.GetProcessesByName(name);
        foreach (var p in ps) p.Dispose();
        return ps.Length > 0;
    }
}

// tiling'i izler. Çıkış kodu 0 değilse (çökme, zorla kapatılma, başlatma hatası) ya da 15 sn'den uzun IPC'ye yanıt
// vermezse masaüstünü yeniden başlatır. Kasıtlı çıkış 0 koduyla olur (ve helper'ı da kapatır). Yeniden başlatılan tiling
// açılamazsa (ör. çöken sürecin IPC portu, süreci tamamen kapanana kadar dolu kalabiliyor) port boşalınca yeniden denenir.
static class TilingWatchdog
{
    const int IPC_PORT = 6123;
    static int recoveringUntil;
    public static bool Recovering { get { return Environment.TickCount - Volatile.Read(ref recoveringUntil) < 0; } }

    public static void Start()
    {
        new Thread(Loop) { IsBackground = true, Priority = ThreadPriority.BelowNormal, Name = "wm-watchdog" }.Start();
    }

    // Pencere yöneticisi: en eski lunge-tiling süreci (kısa ömürlü CLI çağrıları da aynı adla
    // görünür; 5 sn'den genç olan sayılmaz)
    static Process FindWm()
    {
        Process best = null;
        foreach (var p in Process.GetProcessesByName(Names.Tiling))
        {
            try
            {
                if ((DateTime.Now - p.StartTime).TotalSeconds >= 5 && (best == null || p.StartTime < best.StartTime))
                {
                    if (best != null) best.Dispose();
                    best = p;
                    continue;
                }
            }
            catch { }
            p.Dispose();
        }
        return best;
    }

    static bool PortFree()
    {
        try
        {
            var l = new System.Net.Sockets.TcpListener(System.Net.IPAddress.Loopback, IPC_PORT);
            l.ExclusiveAddressUse = true;
            l.Start();
            l.Stop();
            return true;
        }
        catch { return false; }
    }

    static void Loop()
    {
        Thread.Sleep(10000); // oturum açılışı / helper yeni başladı: önce her şey yerine otursun
        var ipc = new TilingClient();
        Process wm = null;
        bool wanted = false;  // tiling çalışmalı mı: çalışırken görüldü ve kasıtlı kapanmadı
        int missing = 0, hung = 0, tick = 0, starts = 0, nextStartAt = 0;
        bool portNoted = false;
        while (true)
        {
            Thread.Sleep(2000);
            try
            {
                if (wm == null)
                {
                    wm = FindWm();
                    if (wm != null)
                    {
                        // Tutamaç şimdi açılır: yoksa süreç kapandıktan sonra çıkış kodu okunamıyor
                        try { var handle = wm.Handle; } catch { }
                        if (starts > 0)
                        {
                            Slider.Log("tiling nöbetçisi: tiling yeniden çalışıyor");
                            // Bildirim, shell geri gelip bildirim kanalına bağlanınca
                            new Thread(() =>
                            {
                                Thread.Sleep(9000);
                                Toasts.Send("info", "Masaüstü toparlandı", "Pencere yöneticisi beklenmedik biçimde kapanmıştı; yeniden başlatıldı.", "restart_alt");
                            }) { IsBackground = true }.Start();
                        }
                        wanted = true; hung = 0; missing = 0; starts = 0; portNoted = false;
                        continue;
                    }
                    if (Maint.Quiet() || Maint.Running(Names.Tiling)) { missing = 0; continue; } // bakım / açılıyor
                    if (!wanted)
                    {
                        // Helper tiling'siz başladı ya da tiling'i hiç görmedi. Kasıtlı çıkış helper'ı da kapattığı için
                        // helper yaşarken tiling ~20 sn yoksa masaüstü bozuktur: tiling çalışmalı.
                        if (++missing < 10) continue;
                        missing = 0; wanted = true; nextStartAt = Environment.TickCount;
                    }
                    if (Environment.TickCount - nextStartAt < 0) continue;
                    if (!PortFree())
                    {
                        if (!portNoted) { portNoted = true; Slider.Log("tiling nöbetçisi: IPC portu hâlâ eski süreçte; boşalınca başlatılacak"); }
                        continue;
                    }
                    starts++;
                    Slider.Log("tiling nöbetçisi: tiling çalışmıyor; başlatılıyor (deneme " + starts + ")");
                    Maint.RunHidden(Maint.CoreExe, "--uncloak-orphans", 15000);
                    Supervisor.BringUp(false);
                    nextStartAt = Environment.TickCount + Math.Min(120000, 15000 * starts);
                    if (starts == 3)
                        Toasts.Send("error", "Pencere yöneticisi açılamıyor",
                            "Denenmeye devam ediliyor. Sürerse oturum menüsünden \"Masaüstünü yenile\"yi seçin ya da oturumu kapatıp açın.", "error");
                    continue;
                }

                if (wm.HasExited)
                {
                    int code = -1;
                    try { code = wm.ExitCode; } catch { }
                    wm.Dispose(); wm = null;
                    if (code == 0)
                    {
                        wanted = false;
                        Slider.Log("tiling nöbetçisi: tiling kapandı (kasıtlı, kod 0)");
                        // Çıkış: masaüstünü Windows'a geri ver. Bakımda (yenile / güncelleme) parçaları o işlem yönetir.
                        if (!Maint.Quiet()) Supervisor.Shutdown("tiling'den çıkıldı", true);
                        continue;
                    }
                    Thread.Sleep(1500); // oturum kapanıyorsa bu arada bayrak kalkar
                    if (Maint.Quiet()) { wanted = false; Slider.Log("tiling nöbetçisi: tiling kapandı (kod " + code + "), bakım / oturum kapanışı: dokunulmadı"); continue; }
                    if (!Recover("Pencere yöneticisi beklenmedik biçimde kapandı (kod " + code + ")")) { wanted = false; continue; }
                    wanted = true; starts = 1; portNoted = false; nextStartAt = Environment.TickCount + 15000;
                    continue;
                }

                // Donma: 5 sn'de bir yokla; üst üste 3 başarısız yoklama (en az 15 sn) donmuş demektir. Ardışık sayım
                // uykudan dönüşte yanlış alarm vermez.
                if (++tick % 3 != 0) continue;
                if ((DateTime.Now - wm.StartTime).TotalSeconds < 30) { hung = 0; continue; }
                if (PingWithTimeout(ipc, 8000)) { hung = 0; continue; }
                if (++hung < 3) { Slider.Log("tiling nöbetçisi: tiling yanıt vermedi (" + hung + "/3)"); continue; }
                hung = 0;
                if (Maint.Quiet()) continue;
                Slider.Log("tiling nöbetçisi: tiling 15 sn'den uzun yanıt vermedi; kapatılıyor");
                try { wm.Kill(); wm.WaitForExit(5000); } catch { }
                // Bir sonraki turda çıkış kodu 0 olmadığı için masaüstü yeniden başlatılır
            }
            catch (Exception ex) { Slider.Log("tiling nöbetçisi: " + ex.Message); if (wm != null) { try { wm.Dispose(); } catch { } wm = null; } }
        }
    }

    // Monitor ile bekler: çekirdek nesnesi yok (her çağrıda bir olay nesnesi çöp toplayıcıya kalıyordu; kapatmak da olmazdı,
    // geç cevap veren iş parçacığı kapanmış nesneye yazar)
    static bool PingWithTimeout(TilingClient g, int ms)
    {
        var gate = new object();
        bool ok = false, done = false;
        ThreadPool.QueueUserWorkItem(_ =>
        {
            bool r = false;
            try { r = g.Ping(); } catch { }
            lock (gate) { ok = r; done = true; Monitor.Pulse(gate); }
        });
        lock (gate)
        {
            if (!done) Monitor.Wait(gate, ms);
            return done && ok;
        }
    }

    // tiling'i kurulum klasöründen, çekirdeğin alt süreci olarak başlatır (Görev Yöneticisi'nde tek uygulama).
    // ShellExecute: çekirdeğin tutamaçları tiling'e miras kalmasın.
    public static void StartTiling()
    {
        if (!System.IO.File.Exists(Paths.Tiling)) { Slider.Log("tiling nöbetçisi: " + Paths.Tiling + " bulunamadı"); return; }
        try { Process.Start(new ProcessStartInfo(Paths.Tiling) { UseShellExecute = true, WorkingDirectory = Paths.Home }); }
        catch (Exception ex) { Slider.Log("tiling nöbetçisi: tiling başlatılamadı: " + ex.Message); }
    }

    // Masaüstünü temiz baştan başlatır: kalanları kapat, gizli kalmış pencereleri geri getir, tiling'i başlat. shell'i
    // Shell nöbetçisi geri açar. Çöküş döngüsünde (5 dakikada 3) vazgeçer ve false döner.
    static bool Recover(string why)
    {
        if (!Maint.Allow("wm-restarts"))
        {
            Slider.Log("tiling nöbetçisi: " + why + "; son 5 dakikada 3 kez yeniden başlatıldı, bırakıldı");
            Toasts.Send("error", "Pencere yöneticisi tekrar tekrar kapanıyor",
                "Otomatik olarak yeniden başlatılmadı. Oturum menüsünden \"Masaüstünü yenile\"yi seçin ya da oturumu kapatıp açın.", "error");
            return false;
        }
        Volatile.Write(ref recoveringUntil, Environment.TickCount + 30000);
        Slider.Log("tiling nöbetçisi: " + why + "; masaüstü yeniden başlatılıyor");
        // Windows'un çökme kutusu yerine (kurulum bizim exe'lerimizi Hata Bildirimi'nden çıkarır): kabuk geri gelince görünür
        Toasts.SendLater("warn", "Pencere yöneticisi durdu, yeniden başlatıldı", why, "restart_alt");
        foreach (var name in new[] { Names.Shell, Names.Tiling })
            foreach (var p in Process.GetProcessesByName(name))
            {
                try { p.Kill(); p.WaitForExit(3000); } catch { }
                finally { p.Dispose(); }
            }
        Maint.RunHidden(Maint.CoreExe, "--uncloak-orphans", 15000);
        if (PortFree()) Supervisor.BringUp(false);
        else Slider.Log("tiling nöbetçisi: IPC portu hâlâ eski süreçte; boşalınca başlatılacak");
        return true;
    }
}

// Helper'ın kendisi: yönetilen bir hata onu kapatırsa ya da arayüz thread'i 30 sn'den uzun donarsa yeni bir kopya
// başlatılır (Super, animasyonlar, pano, bildirimler geri gelir). Yeni kopya (--respawn) eskisinin tek-kopya kilidini
// bekler.
static class SelfHeal
{
    public static volatile bool IsMain;
    static int started, answered, posted;

    public static void Respawn(string why)
    {
        if (!IsMain || Interlocked.Exchange(ref started, 1) == 1) return;
        try
        {
            if (Maint.Quiet()) { Slider.Log("kendini toparlama atlandı (" + why + "): bakım / oturum kapanışı"); return; }
            if (!Maint.Allow("helper-restarts")) { Slider.Log("kendini toparlama: son 5 dakikada 3 kez denendi, bırakıldı (" + why + ")"); ShellTakeover.ReleaseAll(); return; }
            Slider.Log("helper yeniden başlıyor: " + why);
            Process.Start(new ProcessStartInfo(Maint.CoreExe, "--respawn") { UseShellExecute = true, WorkingDirectory = AppDomain.CurrentDomain.BaseDirectory });
        }
        catch (Exception ex) { try { Slider.Log("helper yeniden başlatılamadı: " + ex.Message); } catch { } }
    }

    // Arayüz thread'i 5 sn'de bir yoklanır; aynı yoklama 6 tur (30 sn) cevapsız kalırsa donmuş sayılır. Ardışık sayım
    // uykudan dönüşte yanlış alarm vermez (bekleyen yoklama hemen işlenir).
    public static void WatchUi(Control ui)
    {
        new Thread(() =>
        {
            int misses = 0;
            while (true)
            {
                Thread.Sleep(5000);
                if (Volatile.Read(ref answered) != Volatile.Read(ref posted))
                {
                    if (++misses < 6) continue;
                    Respawn("arayüz 30 sn'den uzun yanıt vermedi");
                    Thread.Sleep(1000);
                    Process.GetCurrentProcess().Kill();
                    return;
                }
                misses = 0;
                int mine = Interlocked.Increment(ref posted);
                try { ui.BeginInvoke((Action)(() => Volatile.Write(ref answered, mine))); }
                catch { Volatile.Write(ref answered, mine); } // pencere kapanıyor
            }
        }) { IsBackground = true, Name = "ui-watchdog" }.Start();
    }
}

// ---------------- Bildirim kanalı (toast widget'ına) ----------------
// shell toast widget'ı http://127.0.0.1:6131/events adresine EventSource ile bağlanır; helper
// buradan bildirim gönderir (Windows hata pencereleri yerine). Yalnızca loopback dinlenir.
static class Toasts
{
    public static volatile bool Listening;
    const int PORT = 6131;
    static readonly List<System.Net.Sockets.NetworkStream> clients = new List<System.Net.Sockets.NetworkStream>();
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    public static void Start()
    {
        var t = new Thread(Serve) { IsBackground = true, Name = "core-http" };
        t.Start();
        var ping = new Thread(() => { while (true) { Thread.Sleep(20000); Write(": ping\n\n"); } }) { IsBackground = true };
        ping.Start();
    }

    // Bir bağlantının hatası (karşı taraf vazgeçti, kısa süreli kaynak darlığı) yalnızca onu düşürür; dinleyici bozulursa
    // beklenip yeniden açılır. Önceden tek bir try/catch bütün döngüyü sarıyordu: ilk hatada /cmd, olay akışı ve barın kalp
    // atışı bir daha hiç cevap vermiyordu.
    static void Serve()
    {
        int wait = 500;
        while (true)
        {
            System.Net.Sockets.TcpListener l = null;
            try
            {
                l = new System.Net.Sockets.TcpListener(System.Net.IPAddress.Loopback, PORT);
                l.Start();
                Listening = true;
                if (wait > 500) Slider.Log("toast server: yeniden dinliyor");
                wait = 500;
                int errors = 0;
                while (errors < 20)
                {
                    try
                    {
                        var c = l.AcceptTcpClient();
                        errors = 0;
                        ThreadPool.QueueUserWorkItem(_ => Accept(c));
                    }
                    catch (System.Net.Sockets.SocketException ex)
                    {
                        if (errors++ == 0) Slider.Log("toast server: bağlantı alınamadı (" + ex.SocketErrorCode + ")");
                        Thread.Sleep(100);
                    }
                }
                Slider.Log("toast server: üst üste hata; dinleyici yeniden açılıyor");
            }
            catch (Exception ex) { Slider.Log("toast server: " + ex.GetBaseException().Message); }
            finally { Listening = false; try { if (l != null) l.Stop(); } catch { } }
            Thread.Sleep(wait);
            wait = Math.Min(wait * 2, 30000);
        }
    }

    static void Accept(System.Net.Sockets.TcpClient c)
    {
        try
        {
            // yarım bırakılan ya da bitmeyen bir istek bir iş parçacığını tutmasın (olay akışı istekten sonra okumaz)
            c.ReceiveTimeout = 5000;
            var s = c.GetStream();
            var buf = new byte[4096]; var req = new StringBuilder();
            while (!req.ToString().Contains("\r\n\r\n"))
            {
                int n = s.Read(buf, 0, buf.Length);
                if (n <= 0 || req.Length > 65536) { c.Close(); return; }
                req.Append(Encoding.ASCII.GetString(buf, 0, n));
            }
            string cors = "Access-Control-Allow-Origin: " + SHELL_ORIGIN + "\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Allow-Headers: *\r\n";
            string reqs = req.ToString();
            // Yalnızca yerel ad: DNS yeniden bağlamayla (rebinding) 127.0.0.1'e yönlenen bir sitenin isteği Host'unda
            // kendi adını taşır; tarayıcı Host'u her zaman gönderir
            string host = Header(reqs, "Host");
            if (host != null && !LocalHost(host)) { Refuse(s); c.Close(); return; }
            // Widget'lar POST kullanır: shell'in service worker'ı başka adreslere giden GET'leri önbelleğe alıyordu (ilk
            // cevap hep tekrar geliyordu: Super hep pano modunu açıyor, bar tıklamaları helper'a ulaşmıyordu)
            string verbless = reqs.StartsWith("POST ") ? reqs.Substring(5) : reqs.StartsWith("GET ") ? reqs.Substring(4) : "";
            // Soru (Dialogs): cevap dakikalarca bekleyebilir, havuz thread'ini tutmasın
            if (verbless.StartsWith("/dialog?"))
            {
                var cc = c;
                new Thread(() => { try { Command(s, reqs); } catch { } finally { try { cc.Close(); } catch { } } }) { IsBackground = true, Name = "core-dialog" }.Start();
                return;
            }
            if (verbless.StartsWith("/dialog-") || verbless.StartsWith("/notify?") || verbless.StartsWith("/launch?") || verbless.StartsWith("/cmd?") || verbless.StartsWith("/overview-mode") || verbless.StartsWith("/overview-wait") || verbless.StartsWith("/overview-signal") || verbless.StartsWith("/bar-alive?") || verbless.StartsWith("/log?") || verbless.StartsWith("/widget?") || verbless.StartsWith("/apps.json") || verbless.StartsWith("/urgent.json") || verbless.StartsWith("/prefs.json") || verbless.StartsWith("/temps.json") || verbless.StartsWith("/desktop-ready") || verbless.StartsWith("/pref?") || verbless.StartsWith("/focus-color?") || verbless.StartsWith("/tray-pins") || verbless.StartsWith("/winicon?") || verbless.StartsWith("/notification") || verbless.StartsWith("/dock-pin") || verbless.StartsWith("/gamma") || verbless.StartsWith("/brightness?") || verbless.StartsWith("/qs/") || verbless.StartsWith("/widgets/") || verbless.StartsWith("/library-remove?")) { Command(s, reqs); c.Close(); return; }
            if (reqs.StartsWith("OPTIONS"))
            {
                var ok = Encoding.ASCII.GetBytes("HTTP/1.1 204 No Content\r\n" + cors + "Content-Length: 0\r\n\r\n");
                s.Write(ok, 0, ok.Length); c.Close(); return;
            }
            // Olay akışı yalnızca yerel istemcilere (kabuğun --toast-stream kopyası, native bar; ikisi de Origin göndermez).
            // Tarayıcı her zaman Origin gönderir: Windows bildirimlerinin metni (doğrulama kodları, mesajlar) bir siteye akmaz.
            if (Header(reqs, "Origin") != null) { Refuse(s); c.Close(); return; }
            // Takılan istemci (dolu boru) çekirdeği kilitlemesin: yazma 3 sn'de düşer, istemci listeden çıkar
            c.SendTimeout = 3000;
            var head = Encoding.ASCII.GetBytes("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n: hazır\n\n");
            s.Write(head, 0, head.Length);
            lock (clients) clients.Add(s);
            FlushLater();
        }
        catch { try { c.Close(); } catch { } }
    }

    // İstek başlığının değeri; yoksa null
    static string Header(string req, string name)
    {
        foreach (var line in req.Split(new[] { "\r\n" }, StringSplitOptions.None))
            if (line.Length > name.Length && line[name.Length] == ':' && line.StartsWith(name, StringComparison.OrdinalIgnoreCase))
                return line.Substring(name.Length + 1).Trim();
        return null;
    }

    static bool LocalHost(string host)
    {
        foreach (var name in new[] { "127.0.0.1", "localhost" })
            if (host.Equals(name, StringComparison.OrdinalIgnoreCase) || host.Equals(name + ":" + PORT, StringComparison.OrdinalIgnoreCase)) return true;
        return false;
    }

    static void Refuse(System.Net.Sockets.NetworkStream s)
    {
        var no = Encoding.ASCII.GetBytes("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        try { s.Write(no, 0, no.Length); } catch { }
    }

    // Bar ve overview'dan anında komut (her tıklamada yeni helper süreci başlatmak ~100-300 ms sürüyordu).
    // Yalnızca shell widget'larının kökeninden (yerel varlık sunucusu) ve yalnızca zararsız komutlar: başka bir sitenin
    // tarayıcıdan bu kanalı kullanması mümkün değil (tarayıcı Origin'i gönderir).
    const string SHELL_ORIGIN = "http://127.0.0.1:6124";
    static void Command(System.Net.Sockets.NetworkStream s, string req)
    {
        string origin = null;
        foreach (var line in req.Split(new[] { "\r\n" }, StringSplitOptions.None))
            if (line.StartsWith("Origin:", StringComparison.OrdinalIgnoreCase)) origin = line.Substring(7).Trim();
        string status = "403 Forbidden", body = "";
        if (origin == null || origin == SHELL_ORIGIN)
        {
            int sp1 = req.IndexOf(' '), sp2 = sp1 < 0 ? -1 : req.IndexOf(' ', sp1 + 1);
            string target = sp2 > sp1 ? req.Substring(sp1 + 1, sp2 - sp1 - 1) : ""; // "/cmd?a=ws-3"
            // Bir şey değiştiren istekler yalnızca POST: tarayıcı bir sitenin <img> / bağlantı GET'ine Origin koymaz, o istek
            // buraya yerel bir istemci gibi gelir (masaüstünü kapatabilir, tercih ve sabitleme yazabilirdi). Widget'lar,
            // native bar ve komut satırı zaten POST gönderir.
            bool writes = target.StartsWith("/cmd?") || target.StartsWith("/pref?") || target.StartsWith("/tray-pins?") || target.StartsWith("/dock-pins?")
                || target.StartsWith("/widget?") || target.StartsWith("/overview-") || target.StartsWith("/log?") || target.StartsWith("/bar-alive?")
                || target.StartsWith("/notification-open?") || target.StartsWith("/dialog") || target.StartsWith("/notify?") || target.StartsWith("/launch?") || target.StartsWith("/widgets/");
            if (writes && !req.StartsWith("POST ")) status = "405 Method Not Allowed";
            else if (target.StartsWith("/launch?"))
            {
                // Kabuğun başlattıkları (Super menüsü, Dock, ayarlar): tek yoldan, kullanıcı olarak, Windows kutusu yerine
                // bizim kartımızla. verb: "" / open / runas / explore
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/launch\?file=([^&\s]{1,4096})(?:&args=([^&\s]{0,8192}))?(?:&verb=(open|runas|explore))?$");
                if (!m.Success) status = "400 Bad Request";
                else
                {
                    string file = Uri.UnescapeDataString(m.Groups[1].Value);
                    string largs = m.Groups[2].Success ? Uri.UnescapeDataString(m.Groups[2].Value) : "";
                    string verb = m.Groups[3].Success ? m.Groups[3].Value : "";
                    bool started = UserLaunch.Start(file, largs, Paths.Home, false, verb == "open" ? "" : verb);
                    status = started ? "204 No Content" : "422 Unprocessable Entity";
                }
            }
            else if (target.StartsWith("/notify?"))
            {
                // Bizim parçaların (pencere yöneticisi, betikler) uyarı ve hataları: Windows kutusu yerine bildirim kartı
                string why;
                var n = Dialogs.ParseNotice(target.Substring(8), out why);
                if (n == null) status = "400 Bad Request";
                else
                {
                    Slider.Log("bildirim (" + n.Kind + "): " + n.Title + (n.Body.Length > 0 ? " - " + n.Body : ""));
                    Toasts.Send(n.Kind == "warning" ? "warn" : n.Kind, n.Title, n.Body, n.Kind);
                    status = "204 No Content";
                }
            }
            else if (target.StartsWith("/dialog?"))
            {
                string why;
                var spec = Dialogs.Parse(target.Substring(8), out why);
                if (spec == null) { status = "400 Bad Request"; body = Dialogs.AnswerJson(-1, false, "invalid: " + why); }
                else { body = Dialogs.Ask(spec); status = "200 OK"; }
            }
            else if (target.StartsWith("/dialog-shown?") || target.StartsWith("/dialog-answer?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/dialog-(shown|answer)\?id=(\d{1,18})(?:&b=(-?\d{1,2})&c=([01]))?$");
                if (!m.Success || (m.Groups[1].Value == "answer") != m.Groups[3].Success) status = "400 Bad Request";
                else
                {
                    long id = long.Parse(m.Groups[2].Value);
                    bool known = m.Groups[1].Value == "shown"
                        ? Dialogs.MarkShown(id)
                        : Dialogs.Answer(id, int.Parse(m.Groups[3].Value), m.Groups[4].Value == "1");
                    status = known ? "204 No Content" : "404 Not Found";
                }
            }
            else if (target.StartsWith("/overview-mode")) { body = Keys2.TakeOverviewMode(); status = "200 OK"; Slider.Log("overview modu okundu: '" + body + "'"); }
            else if (target.StartsWith("/overview-wait"))
            {
                // Uzun yoklama: istek overview gösterilene / gizlenene ya da 25 sn dolana kadar bekletilir
                int q = target.IndexOf("since="), since;
                if (q < 0 || !int.TryParse(target.Substring(q + 6).Split('&')[0], out since)) since = -1;
                body = Keys2.WaitOverviewSignal(since, 25000); status = "200 OK";
            }
            else if (target.StartsWith("/bar-alive?id="))
            {
                string id = Uri.UnescapeDataString(target.Substring(14).Split('&')[0]);
                if (id.Length > 0 && id.Length <= 64) { ShellWatchdog.BarAlive(id); status = "204 No Content"; }
                else status = "400 Bad Request";
            }
            else if (target.StartsWith("/overview-signal?w=show") || target.StartsWith("/overview-signal?w=hide"))
            {
                Keys2.OverviewSignal(target.EndsWith("hide") ? "hide" : "show"); status = "204 No Content";
            }
            else if (target.StartsWith("/log?m=")) { Slider.Log("widget: " + Uri.UnescapeDataString(target.Substring(7))); status = "204 No Content"; }
            // Arayüz tercihleri (dil, saat, animasyon): widget'lar sayfa çizilmeden önce okur
            else if (target.StartsWith("/widgets/")) { body = WidgetLocations.Handle(target); status = "200 OK"; }
            else if (target == "/prefs.json" || target.StartsWith("/prefs.json?")) { body = Prefs.Json(); status = "200 OK"; }
            // Kullanım menüsünün sıcaklıkları: süreç başlatmadan (eskiden her 2 sn'de bir lunge-temps --read)
            else if (target == "/temps.json") { body = TempsFile.Json(); status = "200 OK"; }
            // Açılış örtüsü: masaüstünün bütün parçaları geldi mi
            else if (target == "/desktop-ready") { body = DesktopReady.Json(); status = "200 OK"; }
            else if (target.StartsWith("/focus-color?v="))
            {
                if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                else try
                {
                    var hex = Uri.UnescapeDataString(target.Substring(15));
                    body = Settings.Cli(new string[] { "--set-focus-color", hex });
                    status = "200 OK";
                }
                catch (Exception ex)
                {
                    body = new JavaScriptSerializer().Serialize(new { ok = false, error = ex.GetBaseException().Message });
                    status = "500 Internal Server Error";
                }
            }
            // Tercih yaz (/pref?k=theme&v=light): bar / panel / ayarlar; değer Prefs.Set'te doğrulanır
            else if (target.StartsWith("/pref?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/pref\?k=([A-Za-z]{1,20})&v=([^&\s]{1,40})$");
                status = m.Success && Prefs.Set(m.Groups[1].Value, Uri.UnescapeDataString(m.Groups[2].Value)) ? "204 No Content" : "400 Bad Request";
            }
            // Sabitlemeler: /tray-pins | /dock-pins okur; ?v=[...] yazar; ?if-missing=1&v=[...] yalnızca kayıt yoksa
            else if (target == "/tray-pins" || target == "/dock-pins") { body = (target == "/dock-pins" ? Pins.Dock : Pins.Tray).Read(); status = "200 OK"; }
            else if (target.StartsWith("/tray-pins?") || target.StartsWith("/dock-pins?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/(tray|dock)-pins\?(if-missing=1&)?v=([^&\s]{2,6000})$");
                status = m.Success && (m.Groups[1].Value == "dock" ? Pins.Dock : Pins.Tray).Write(Uri.UnescapeDataString(m.Groups[3].Value), m.Groups[2].Success) ? "204 No Content" : "400 Bad Request";
            }
            // Tek uygulamayı Dock'a ekler / çıkarır: /dock-pin?id=<exe adı>&on=1|0 (Dock, Super menüsünün sağ tık menüsü)
            else if (target.StartsWith("/dock-pin?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/dock-pin\?id=([^&\s]{1,200})&on=([01])$");
                if (!m.Success) status = "400 Bad Request";
                else if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                else status = Pins.Dock.Set(Uri.UnescapeDataString(m.Groups[1].Value), m.Groups[2].Value == "1") ? "204 No Content" : "400 Bad Request";
            }
            // Parlaklık (bar tekerleği): /brightness?dev=\\.\DISPLAY1 okur -> {"value":N} ya da {"value":null} (ayarlanamıyor);
            // &v=0..100 ile POST yazar (arka planda, monitör başına son değer kazanır)
            else if (target.StartsWith("/brightness?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/brightness\?dev=([^&\s]{1,64})(?:&v=(\d{1,3}))?$");
                string dev = m.Success ? Uri.UnescapeDataString(m.Groups[1].Value) : "";
                if (!System.Text.RegularExpressions.Regex.IsMatch(dev, @"^\\\\\.\\DISPLAY\d{1,2}$")) status = "400 Bad Request";
                else if (m.Groups[2].Success)
                {
                    if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                    else { Brightness.Set(dev, int.Parse(m.Groups[2].Value)); status = "204 No Content"; }
                }
                else
                {
                    try
                    {
                        int? v = Brightness.Get(dev);
                        body = "{\"value\":" + (v.HasValue ? v.Value.ToString(System.Globalization.CultureInfo.InvariantCulture) : "null") + "}";
                        status = "200 OK";
                    }
                    catch (Exception ex) { body = "{\"value\":null}"; status = "500 Internal Server Error"; Slider.Log("parlaklık okunamadı: " + ex.Message); }
                }
            }
            // Sağ panelin hızlı ayarları (radyolar, Ethernet, Bluetooth, uyanık tut): okumalar da cihaz bilgisi verdiği için
            // yalnızca POST (aynı kökenden GET Origin göndermez)
            else if (target.StartsWith("/library-remove?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/library-remove\?kind=(wall|live|saver)&path=([^&\s]{1,2048})$");
                if (!m.Success) status = "400 Bad Request";
                else if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                else
                {
                    string err;
                    try { err = Library.Remove(m.Groups[1].Value, Uri.UnescapeDataString(m.Groups[2].Value)); }
                    catch (Exception ex) { err = "io"; Slider.Log("kütüphaneden silinemedi: " + ex.Message); }
                    status = err == null ? "204 No Content" : err == "missing" ? "404 Not Found" : err == "io" ? "500 Internal Server Error" : "400 Bad Request";
                }
            }
            else if (target.StartsWith("/qs/"))
            {
                if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                else if (!QuickSettings.Http(target, out status, out body)) status = "404 Not Found";
            }
            // Windows bildirimleri (sağ panelin listesi): {"items":[...], "icons":{...}}
            else if (target == "/notifications")
            {
                // Yalnızca POST: aynı kökenden GET Origin göndermez
                if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                else { body = WinNotifications.Json(); status = "200 OK"; }
            }
            else if (target.StartsWith("/notification-open?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/notification-open\?id=(\d{1,18})$");
                if (!m.Success) status = "400 Bad Request";
                else if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                else status = WinNotifications.Open(long.Parse(m.Groups[1].Value)) ? "204 No Content" : "404 Not Found";
            }
            // Gama (parlaklık 0'ın altında ekran başına yazılımsal karartma): /gamma?dev=\\.\DISPLAY1 okur -> {"gamma":N};
            // &v=0..100 ile POST yazar. Bar'lar her tekerlek adımında buraya gelir (eskiden her adım yeni bir lunge.exe süreci).
            else if (target.StartsWith("/gamma?"))
            {
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/gamma\?dev=([^&\s]{1,64})(?:&v=(\d{1,3}))?$");
                string dev = m.Success ? Uri.UnescapeDataString(m.Groups[1].Value) : "";
                if (!System.Text.RegularExpressions.Regex.IsMatch(dev, @"^\\\\\.\\DISPLAY\d{1,2}$")) status = "400 Bad Request";
                else if (m.Groups[2].Success)
                {
                    if (!req.StartsWith("POST ")) status = "405 Method Not Allowed";
                    else
                    {
                        try { status = NightLight.SetGamma(dev, Math.Min(100, int.Parse(m.Groups[2].Value))) ? "204 No Content" : "500 Internal Server Error"; }
                        catch (Exception ex) { status = "500 Internal Server Error"; Slider.Log("gama yazılamadı: " + ex.Message); }
                    }
                }
                else { body = "{\"gamma\":" + NightLight.Gamma(dev).ToString(System.Globalization.CultureInfo.InvariantCulture) + "}"; status = "200 OK"; }
            }
            // Dikkat isteyen pencereler (Urgent): bar workspace'lerini işaretler
            else if (target == "/urgent.json")
            {
                body = Urgent.Json(); status = "200 OK";
            }
            else if (target == "/apps.json" || target.StartsWith("/apps.json?"))
            {
                // Super menüsünün uygulama listesi (lunge.exe --build-apps kullanıcının veri klasörüne yazar)
                try { body = System.IO.File.ReadAllText(Paths.AppsJson); status = "200 OK"; }
                catch { body = "[]"; status = "200 OK"; }
            }
            // Pencerenin kendi simgesi (uygulama listesinde karşılığı yoksa): /winicon?h=<pencere>
            else if (target.StartsWith("/winicon?h="))
            {
                long hv;
                if (long.TryParse(target.Substring(11).Split('&')[0], out hv) && hv > 0)
                {
                    body = WinIcons.For(new IntPtr(hv)) ?? "";
                    status = body.Length > 0 ? "200 OK" : "204 No Content";
                }
                else status = "400 Bad Request";
            }
            else if (target.StartsWith("/widget?"))
            {
                // /widget?w=toast|update|osk&v=0|1: üstte duran widget penceresini odak çalmadan göster / gizle
                var m = System.Text.RegularExpressions.Regex.Match(target, @"^/widget\?w=(toast|update|osk)&v=([01])$");
                status = m.Success && WidgetWindows.Set(m.Groups[1].Value, m.Groups[2].Value == "1") ? "204 No Content" : "400 Bad Request";
            }
            else
            {
                int q = target.IndexOf("a=");
                string act = q < 0 ? "" : Uri.UnescapeDataString(target.Substring(q + 2).Split('&')[0]);
                if (System.Text.RegularExpressions.Regex.IsMatch(act, @"^ws-(\d{1,2}|next|prev)$") && Keys2.Instance != null)
                {
                    Keys2.Instance.Dispatch(act);
                    status = "204 No Content";
                }
                // Test: dokunmatik yüzey olmadan parmak hareketi (/cmd?a=gesture&f=3&dx=-45&dy=0&ms=260; mm)
                else if (act == "gesture" && SelfHeal.IsMain && Slider.Ui != null)
                {
                    var g = System.Text.RegularExpressions.Regex.Match(target, @"[?&]f=([3-5])&dx=(-?\d{1,3})&dy=(-?\d{1,3})&ms=(\d{2,4})");
                    if (g.Success)
                    {
                        Touchpad.Simulate(Slider.Ui, int.Parse(g.Groups[1].Value), int.Parse(g.Groups[2].Value), int.Parse(g.Groups[3].Value), Math.Min(3000, int.Parse(g.Groups[4].Value)));
                        status = "202 Accepted";
                    }
                    else status = "400 Bad Request";
                }
                // Masaüstünü yenile / kapat: kabuk ve kurulum normal kullanıcı olarak çalışır, yönetici haklarıyla
                // çalışan parçaları kapatamaz; işi çalışan çekirdek yapar (bkz. Supervisor.RequestFromCore)
                else if (act == "restart-desktop" && SelfHeal.IsMain)
                {
                    ThreadPool.QueueUserWorkItem(_ => Supervisor.RestartDesktopDetached());
                    status = "202 Accepted";
                }
                // Ayarlar penceresi: dil / saat biçimi / animasyon tercihi widget'lar yeniden açılınca uygulanır
                else if (act == "restart-shell" && SelfHeal.IsMain)
                {
                    ThreadPool.QueueUserWorkItem(_ => ShellWatchdog.Restart("tercihler değişti"));
                    status = "202 Accepted";
                }
                // Canlı duvar kağıdı ayarı değişti (lunge.exe --live-*): oynatıcı başlar, ayarı yeniden okur ya da kapanır
                else if (act == "live-wallpaper" && SelfHeal.IsMain)
                {
                    ThreadPool.QueueUserWorkItem(_ =>
                    {
                        try { LiveWallpaper.Sync(); } catch (Exception ex) { Slider.Log("canlı duvar kağıdı: " + ex.Message); }
                    });
                    status = "202 Accepted";
                }
                else if (act == "stop-desktop" && SelfHeal.IsMain)
                {
                    ThreadPool.QueueUserWorkItem(_ =>
                    {
                        try { Supervisor.StopDesktop(); } catch (Exception ex) { Slider.Log("masaüstü kapatılamadı: " + ex.Message); }
                        Environment.Exit(0);
                    });
                    status = "202 Accepted";
                }
                else status = "400 Bad Request";
            }
        }
        var bytes = Encoding.UTF8.GetBytes(body);
        var head = Encoding.ASCII.GetBytes("HTTP/1.1 " + status + "\r\nAccess-Control-Allow-Origin: " + SHELL_ORIGIN + "\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: " + bytes.Length + "\r\nConnection: close\r\n\r\n");
        s.Write(head, 0, head.Length);
        if (bytes.Length > 0) s.Write(bytes, 0, bytes.Length);
        s.Flush();
    }

    // Olaylar kuyruğa girer, tek bir yazıcı thread sırayla gönderir: Emit'i çağıran (UI thread'i, klavye kancasının
    // işleri) takılan bir istemcinin 3 sn'lik yazma süresini beklemez; liste kilidi yazarken tutulmaz (yeni bağlantı
    // eklenebilir). Kuyruk sınırlı: akış tıkanırsa en eski olaylar düşer, bellek büyümez.
    static readonly Queue<byte[]> outbox = new Queue<byte[]>();
    const int OUTBOX_MAX = 512;
    static Thread writer;

    static void Write(string text)
    {
        var bytes = Encoding.UTF8.GetBytes(text);
        lock (outbox)
        {
            if (outbox.Count >= OUTBOX_MAX) outbox.Dequeue();
            outbox.Enqueue(bytes);
            if (writer == null)
            {
                writer = new Thread(WriteLoop) { IsBackground = true, Name = "core-events" };
                writer.Start();
            }
            Monitor.Pulse(outbox);
        }
    }

    static void WriteLoop()
    {
        while (true)
        {
            byte[] bytes;
            lock (outbox)
            {
                while (outbox.Count == 0) Monitor.Wait(outbox);
                bytes = outbox.Dequeue();
            }
            List<System.Net.Sockets.NetworkStream> now;
            lock (clients) now = new List<System.Net.Sockets.NetworkStream>(clients);
            List<System.Net.Sockets.NetworkStream> dead = null;
            foreach (var s in now)
            {
                try { s.Write(bytes, 0, bytes.Length); s.Flush(); }
                catch
                {
                    // kopan / takılan istemcinin soketi hemen kapanır (çöp toplayıcıyı beklemez)
                    try { s.Dispose(); } catch { }
                    (dead ?? (dead = new List<System.Net.Sockets.NetworkStream>())).Add(s);
                }
            }
            if (dead != null) lock (clients) clients.RemoveAll(dead.Contains);
        }
    }

    // Kabuğa olay: bildirim widget'ı akıştaki {"emit": ...} satırlarını Tauri olayı olarak yayınlar (ör. sağ panel)
    public static void Emit(string evt)
    {
        Write("data: " + json.Serialize(new Dictionary<string, object> { { "emit", evt } }) + "\n\n");
    }

    // Kabuk kapalıyken söylenecek kart (ör. "kabuk durdu, yeniden başlatıldı"): akışa ilk bağlanan istemciye gider
    static readonly List<Dictionary<string, object>> later = new List<Dictionary<string, object>>();

    public static void SendLater(string kind, string title, string body, string icon)
    {
        lock (later)
        {
            if (later.Count >= 8) later.RemoveAt(0);
            later.Add(new Dictionary<string, object> { { "kind", kind }, { "title", title }, { "body", body }, { "icon", icon } });
        }
        lock (clients) if (clients.Count == 0) return;
        FlushLater();
    }

    static void FlushLater()
    {
        List<Dictionary<string, object>> now;
        lock (later) { if (later.Count == 0) return; now = new List<Dictionary<string, object>>(later); later.Clear(); }
        foreach (var c in now) Card(c);
    }

    public static void Send(string kind, string title, string body, string icon)
    {
        Card(new Dictionary<string, object> { { "kind", kind }, { "title", title }, { "body", body }, { "icon", icon } });
    }

    // Kartın tüm alanları (toast.html show(): app, image, actions, open, timeout ...)
    public static void Card(Dictionary<string, object> card)
    {
        Write("data: " + json.Serialize(card) + "\n\n");
    }
}



// ---------------- Yuvarlak köşeler ----------------
class Rounder
{
    const int RADIUS = 14; // kenarlık motoru border_radius ile aynı
    readonly Dictionary<IntPtr, long> applied = new Dictionary<IntPtr, long>();
    readonly Dictionary<IntPtr, List<long>> resets = new Dictionary<IntPtr, List<long>>();
    readonly HashSet<IntPtr> giveUp = new HashSet<IntPtr>();
    Native.WinEventDelegate cb;
    const int WS_EX_LAYERED = 0x00080000, WS_EX_NOREDIRECTIONBITMAP = 0x00200000;
    [DllImport("user32.dll")] static extern bool GetLayeredWindowAttributes(IntPtr h, out uint key, out byte alpha, out uint flags);
    static readonly HashSet<string> skipProcs = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { Names.Shell, Names.Tiling, Names.Core, "explorer" }; // Başlat / arama / kilit ekranı gibi kabuk yüzeyleri başlıksız
                                                                  // ya da araç penceresi: aşağıdaki stil kuralları onları zaten dışarıda bırakır
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Native.RECT r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref Native.POINT p);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
    [StructLayout(LayoutKind.Sequential)] struct MONITORINFO { public int cbSize; public Native.RECT rcMonitor, rcWork; public uint dwFlags; }
    [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr mon, ref MONITORINFO mi);
    // Pencerenin monitörü (Screen.FromHandle her çağrıda nesne ayırıyordu; bu her konum değişikliği olayında çalışır)
    static Native.RECT MonitorOf(IntPtr h)
    {
        var mi = new MONITORINFO { cbSize = Marshal.SizeOf(typeof(MONITORINFO)) };
        GetMonitorInfo(MonitorFromWindow(h, 2 /*MONITOR_DEFAULTTONEAREST*/), ref mi);
        return mi.rcMonitor;
    }

    // Başlığını Windows mu çiziyor: görünen üst kenar ile çizim alanının üstü arasındaki fark başlık (ve menü) çubuğudur.
    // Tarayıcılar, Electron, terminaller, Qt pencereleri başlığı kendileri çizer (0-1 px); Görev Yöneticisi, Not Defteri,
    // ayar pencereleri Windows'a çizdirir (30+ px). Bölge verilen pencerede Windows başlığı ve düğmeleri yönetmeyi bırakır:
    // öyle pencerelerde içi boş kaldı, X tıklamayı almadı. Onlara hiç bölge verilmez (köşeleri düz kalır).
    static bool SystemCaption(IntPtr h, Native.RECT frame)
    {
        var p = new Native.POINT();
        Native.RECT c;
        if (!GetClientRect(h, out c) || !ClientToScreen(h, ref p)) return true; // bilinmiyor: dokunma
        return p.Y - frame.Top > 4;
    }
    // pid -> (ad, okunma anı). pid'ler yeniden kullanılır: kayıt 30 sn geçerli, sözlük sınırlı
    static readonly Dictionary<uint, KeyValuePair<string, int>> procCache = new Dictionary<uint, KeyValuePair<string, int>>();
    int ticks;

    public void Start()
    {
        cb = Callback.Guard("köşe olayı", OnEvent);
        Native.SetWinEventHook(Native.EVENT_OBJECT_SHOW, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, cb, 0, 0, 0x0002);
        Native.SetWinEventHook(Native.EVENT_OBJECT_LOCATIONCHANGE, Native.EVENT_OBJECT_LOCATIONCHANGE, IntPtr.Zero, cb, 0, 0, 0x0002);
        Native.SetWinEventHook(Native.EVENT_SYSTEM_FOREGROUND, Native.EVENT_SYSTEM_FOREGROUND, IntPtr.Zero, cb, 0, 0, 0x0002);
        Native.SetWinEventHook(0x8001, 0x8001, IntPtr.Zero, cb, 0, 0, 0x0002); // EVENT_OBJECT_DESTROY: kapanan pencereyi unut
        Native.SetWinEventHook(0x8018, 0x8018, IntPtr.Zero, cb, 0, 0, 0x0002); // EVENT_OBJECT_UNCLOAKED: workspace geçişinde görünen
        Native.EnumWindows(delegate (IntPtr h, IntPtr l) { Apply(h); return true; }, IntPtr.Zero);

        // Aynı thread'de (mesaj döngüsü var) periyodik kontrol. Uygulamanın kendisi sıfırladığı bölge yalnızca köşesi
        // yuvarlanmış pencerelerde olur: onlara 0,7 sn'de bir bakılır. Önceden her seferinde tüm üst düzey pencereler
        // (yüzlerce, her birinde ~10 sistem çağrısı ve başlık okuma) geziliyordu. Tam tarama 5 sn'de bir güvenlik ağı;
        // kapanmış pencerelerin kayıtları da o sırada silinir.
        var timer = new System.Windows.Forms.Timer { Interval = 700 };
        timer.Tick += (s, e) =>
        {
            if (++ticks % 7 == 0)
            {
                Prune();
                Native.EnumWindows(delegate (IntPtr h, IntPtr l) { Apply(h); return true; }, IntPtr.Zero);
            }
            else foreach (var h in new List<IntPtr>(applied.Keys)) Apply(h);
            MarkOurWindows();
        };
        timer.Start();
        MarkOurWindows();
    }

    // Kapanan pencerenin kayıtları (önceden hiç silinmiyordu)
    readonly Dictionary<IntPtr, int> clipRepairAt = new Dictionary<IntPtr, int>();
    static bool ClipRepairDue(int now, int last) { return unchecked(now - last) >= 16; }
    static bool RegionMatches(int kind, Native.RECT actual, int expectedKind, Native.RECT expected)
    {
        return kind > 1 && kind == expectedKind && actual.Left == expected.Left && actual.Top == expected.Top
            && actual.Right == expected.Right && actual.Bottom == expected.Bottom;
    }
    // Pencereye konan bölge, hep buradan. Köşesi yuvarlanamayacak kadar küçük pencerenin yuvarlak bölgesi düz ya da boş
    // çıkar (boş bölge pencereyi görünmez bırakır): o köşeli kesilir.
    internal static IntPtr MakeRegion(bool square, int l, int t, int r, int b)
    {
        if (!square)
        {
            IntPtr round = Native.CreateRoundRectRgn(l, t, r + 1, b + 1, RADIUS * 2, RADIUS * 2);
            Native.RECT box;
            if (round != IntPtr.Zero && GetRgnBox(round, out box) == 3 /*COMPLEXREGION*/) return round;
            if (round != IntPtr.Zero) Native.DeleteObject(round);
        }
        return Native.CreateRectRgn(l, t, r + 1, b + 1);
    }
    // O bölgenin Windows'un bildireceği türü (GetRgnBox dönüşü) ve kutusu. Tahmin edilmez: yuvarlak bölgenin kutusu
    // köşelerinin dikdörtgeninden bir piksel küçük, küçük pencerede türü de değişir. Tahmin hiç tutmayınca bölge her olayda
    // yeniden konuyordu; SetWindowRgn de yeni bir konum olayı doğurduğundan pencere başına saniyede binlerce tur (boşta
    // bir çekirdek).
    static int RegionShape(bool square, int l, int t, int r, int b, out Native.RECT box)
    {
        box = new Native.RECT();
        IntPtr rgn = MakeRegion(square, l, t, r, b);
        if (rgn == IntPtr.Zero) return 0;
        int kind = GetRgnBox(rgn, out box);
        Native.DeleteObject(rgn);
        return kind;
    }
    [DllImport("gdi32.dll")] static extern int GetRgnBox(IntPtr rgn, out Native.RECT box);
    void Forget(IntPtr h) { applied.Remove(h); resets.Remove(h); giveUp.Remove(h); clipRepairAt.Remove(h); overflowSince.Remove(h); }

    // An app's own resize past its tile is undone by the window manager within a frame or two: Chromium in its
    // fullscreen mode (Discord's voice channel, Chrome) sets the monitor's size at every focus change. A clip set
    // meanwhile lands after the window is back and, cut for the old position, hides it for a frame; recorded at every
    // one of Discord's focus changes (it covered the top bar for one frame, then vanished for one). So the clip waits
    // CLIP_GRACE_MS: a window still past its tile then is clipped.
    const int CLIP_GRACE_MS = 60;
    readonly Dictionary<IntPtr, int> overflowSince = new Dictionary<IntPtr, int>();
    System.Windows.Forms.Timer grace;
    void CheckAfterGrace()
    {
        if (grace == null)
        {
            grace = new System.Windows.Forms.Timer { Interval = CLIP_GRACE_MS + 10 };
            grace.Tick += (s, e) => { grace.Stop(); foreach (var w in new List<IntPtr>(overflowSince.Keys)) Apply(w); };
        }
        if (!grace.Enabled) grace.Start();
    }
    void Prune()
    {
        foreach (var h in new List<IntPtr>(applied.Keys)) if (!Native.IsWindow(h)) Forget(h);
        foreach (var h in new List<IntPtr>(resets.Keys)) if (!Native.IsWindow(h)) resets.Remove(h);
        giveUp.RemoveWhere(h => !Native.IsWindow(h));
        foreach (var h in new List<IntPtr>(clipRepairAt.Keys)) if (!Native.IsWindow(h)) clipRepairAt.Remove(h);
        foreach (var h in new List<IntPtr>(overflowSince.Keys)) if (!Native.IsWindow(h)) overflowSince.Remove(h);
    }

    // Bar, bildirim ve ekran klavyesi pencereleri (başlıklarıyla, tüm pencereleri gezmeden)
    static void MarkOurWindows()
    {
        foreach (var t in new[] { Names.Bar, Names.Toast, Names.Osk })
        {
            IntPtr h = IntPtr.Zero;
            while ((h = Native.FindWindowEx(IntPtr.Zero, h, null, t)) != IntPtr.Zero) MarkNoActivate(h);
        }
    }

    // Bar ve bildirim penceresi odak almasın: "fareyle üzerine gelince etkinleştir" açıkken
    // bar'ın üstüne gelmek klavyeyi uygulamadan çalmasın. Tık ve tekerlek yine çalışır.
    static void MarkNoActivate(IntPtr h)
    {
        var sb = new StringBuilder(64);
        if (Native.GetWindowText(h, sb, 64) == 0) return;
        string t = sb.ToString();
        if (t != Names.Bar && t != Names.Toast && t != Names.Osk) return;
        int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        if ((ex & Native.WS_EX_NOACTIVATE) == 0) Native.SetWindowLong(h, Native.GWL_EXSTYLE, ex | Native.WS_EX_NOACTIVATE);
    }

    void OnEvent(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        EventLag.Note("köşe", time);
        if (idObject != 0 || hwnd == IntPtr.Zero) return; // OBJID_WINDOW
        if (ev == 0x8001) { Forget(hwnd); return; }
        if (ev == Native.EVENT_OBJECT_LOCATIONCHANGE) Dwindle.WindowMoved(hwnd);
        Apply(hwnd);
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetProp(IntPtr h, string name);

    // İki koordinat tek değerde; her yarı sıfır okunmasın diye kaydırılmış (tiling: pack_slot)
    static bool Slot(IntPtr h, out Native.RECT slot)
    {
        slot = new Native.RECT();
        long lt = GetProp(h, "LungeSlotLT").ToInt64(), rb = GetProp(h, "LungeSlotRB").ToInt64();
        if (lt == 0 || rb == 0) return false;
        unchecked
        {
            slot.Left = (int)((uint)(lt >> 32) - 0x40000000u); slot.Top = (int)((uint)lt - 0x40000000u);
            slot.Right = (int)((uint)(rb >> 32) - 0x40000000u); slot.Bottom = (int)((uint)rb - 0x40000000u);
        }
        return slot.Right > slot.Left && slot.Bottom > slot.Top;
    }

    static string ProcName(IntPtr h)
    {
        uint pid; Native.GetWindowThreadProcessId(h, out pid);
        KeyValuePair<string, int> e;
        int now = Environment.TickCount;
        if (procCache.TryGetValue(pid, out e) && now - e.Value < 30000) return e.Key;
        string name = ProcInfo.Name(pid);
        if (procCache.Count > 512) procCache.Clear();
        procCache[pid] = new KeyValuePair<string, int>(name, now);
        return name;
    }

    // Decorative corner repair can stop when an app repeatedly removes it.
    // A managed tile's overflow constraint remains necessary independently.
    static bool NeedsTileClip(Native.RECT frame, Native.RECT slot, bool tiledSlot)
    {
        return tiledSlot && (frame.Left < slot.Left || frame.Top < slot.Top || frame.Right > slot.Right || frame.Bottom > slot.Bottom);
    }

    void Apply(IntPtr h)
    {
        if (Native.GetAncestor(h, 2) != h) return; // GA_ROOT: yalnızca üst düzey pencereler
        if (!Native.IsWindowVisible(h)) return;
        if (Native.IsHungAppWindow(h)) return; // askıdaki pencerede SetWindowRgn bekler
        // Gizli workspace'teki (cloak edilmiş) pencereye dokunma: workspace geçişinde tüm pencereler
        // gizlenip açılırken hepsine yeniden bölge uygulamak geçişi uygulama sayısıyla yavaşlatıyordu.
        int cloaked;
        if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cloaked, 4) == 0 && cloaked != 0) return;
        int style = Native.GetWindowLong(h, Native.GWL_STYLE);
        int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        if ((style & Native.WS_CHILD) != 0 || (ex & Native.WS_EX_TOOLWINDOW) != 0) return;
        if (skipProcs.Contains(ProcName(h))) return;
        // A per-pixel layered window (its picture handed over whole, UpdateLayeredWindow) or one drawn straight by DirectComposition
        // without a redirection surface (custom-drawn launchers and clients) takes no window region: DWM keeps composing
        // the window's last picture where the region used to clip it, so moving it or toggling its fullscreen left a copy
        // of it on screen. Such a window keeps its own shape; a region of ours left from before is taken off.
        uint lwKey; byte lwAlpha; uint lwFlags;
        bool perPixel = (ex & WS_EX_LAYERED) != 0 && !GetLayeredWindowAttributes(h, out lwKey, out lwAlpha, out lwFlags); // a whole-window alpha (opacity effect) still has its surface
        if (perPixel || (ex & WS_EX_NOREDIRECTIONBITMAP) != 0)
        {
            if (applied.Remove(h)) Native.SetWindowRgn(h, IntPtr.Zero, true);
            return;
        }

        Native.RECT wr, fr;
        if (!Native.GetWindowRect(h, out wr)) return;
        if (Native.DwmGetWindowAttribute(h, Native.DWMWA_EXTENDED_FRAME_BOUNDS, out fr, Marshal.SizeOf(typeof(Native.RECT))) != 0) fr = wr;

        // Tam ekran / maximize: köşe yok (Hyprland'de de fullscreen'de rounding kalkar)
        var wp = new Native.WINDOWPLACEMENT { length = Marshal.SizeOf(typeof(Native.WINDOWPLACEMENT)) };
        Native.GetWindowPlacement(h, ref wp);
        var screen = MonitorOf(h);
        bool full = wp.showCmd == 3 || (fr.Left <= screen.Left && fr.Top <= screen.Top && fr.Right >= screen.Right && fr.Bottom >= screen.Bottom);

        // A managed tile keeps its screen-space slot even when an application
        // briefly expands its own window to the monitor for video fullscreen.
        // Clip that transition to the tile; explicit WM fullscreen clears the
        // slot and remains a genuine monitor-wide window.
        Native.RECT slot;
        bool tiledSlot = Slot(h, out slot) && slot.Right > screen.Left && slot.Left < screen.Right
            && slot.Bottom > screen.Top && slot.Top < screen.Bottom
            && (slot.Left > screen.Left || slot.Top > screen.Top || slot.Right < screen.Right || slot.Bottom < screen.Bottom);
        // Borderless video in a tile is clipped square. Ordinary captioned
        // windows keep their rounded corners.
        bool sysCaption = SystemCaption(h, fr);
        bool borderless = (style & Native.WS_CAPTION) != Native.WS_CAPTION && (style & 0x00040000) == 0;
        if (((full || borderless) && !tiledSlot) || sysCaption)
        {
            // bizim koyduğumuz; başlığı Windows'un çizdiği pencerede önceki çekirdeğin koyup bıraktığı bölge de kalkar
            Native.RECT rb;
            if (applied.Remove(h) || (sysCaption && Native.GetWindowRgnBox(h, out rb) != 0)) Native.SetWindowRgn(h, IntPtr.Zero, true);
            return;
        }

        // Pencere yöneticisi döşenmiş pencerenin yuvasını pencere özelliği olarak yazar (LungeSlotLT/RB). Yuvasından
        // büyük kalan pencere (en küçük boyutu yuvaya sığmıyor) yuvaya kesilir: komşusunun üstüne binmez, taşan
        // yere tıklama komşuya gider. Kenarlık da aynı kesilmiş alana çizilir.
        Native.RECT vis = fr;
        if (tiledSlot)
        {
            var c = new Native.RECT { Left = Math.Max(fr.Left, slot.Left), Top = Math.Max(fr.Top, slot.Top), Right = Math.Min(fr.Right, slot.Right), Bottom = Math.Min(fr.Bottom, slot.Bottom) };
            if (c.Right > c.Left && c.Bottom > c.Top) vis = c;
        }
        long key;
        unchecked { key = (((((long)(fr.Right - fr.Left) * 31 + (fr.Bottom - fr.Top)) * 31 + (vis.Left - fr.Left)) * 31 + (vis.Top - fr.Top)) * 31 + (fr.Right - vis.Right)) * 31 + (fr.Bottom - vis.Bottom); }
        long prev;
        Native.RECT box;
        int regionKind = Native.GetWindowRgnBox(h, out box);
        bool hasRgn = regionKind > 1;
        bool clipRequired = NeedsTileClip(fr, slot, tiledSlot);
        if (!clipRequired) overflowSince.Remove(h);
        int l = vis.Left - wr.Left, t = vis.Top - wr.Top;
        int r = l + (vis.Right - vis.Left), b = t + (vis.Bottom - vis.Top);
        bool square = full || borderless || giveUp.Contains(h);
        // Bazı uygulamalar (Terminal, Firefox/Zen) bölgeyi kendileri sıfırlıyor: yoksa yeniden uygula
        Native.RECT ours;
        if (applied.TryGetValue(h, out prev) && prev == key && RegionMatches(regionKind, box, RegionShape(square, l, t, r, b, out ours), ours)) return;
        if (giveUp.Contains(h) && !clipRequired)
        {
            // A previous fullscreen clip is relative to the old HWND bounds.
            // Remove only our region once the real window fits its tile again.
            if (applied.Remove(h) && hasRgn) Native.SetWindowRgn(h, IntPtr.Zero, true);
            return;
        }
        if (prev == key)
        {
            // Aynı boyutta bölge silinmiş ya da uygulamanınkiyle değişmiş -> uygulama kendisi sıfırlıyor. Kısa sürede çok
            // tekrarlarsa kavga etme (titreme + CPU): o pencereyi köşesiz bırak.
            long now = Environment.TickCount;
            List<long> hits;
            if (!resets.TryGetValue(h, out hits)) resets[h] = hits = new List<long>();
            hits.Add(now);
            hits.RemoveAll(x => unchecked((int)(now - x)) > 3000);
            // Vazgeçerken uygulamanın kendi bölgesine dokunulmaz; yalnızca boş bölge (pencereyi görünmez bırakır) kalkar
            if (hits.Count > 4)
            {
                if (giveUp.Add(h)) Slider.Log("gave up rounding " + ProcName(h) + " hwnd=" + h.ToInt64() + "; tile clipping retained");
                if (!clipRequired) { applied.Remove(h); if (regionKind == 1) Native.SetWindowRgn(h, IntPtr.Zero, true); return; }
            }
        }
        if (clipRequired)
        {
            int since, last, now = Environment.TickCount;
            if (!overflowSince.TryGetValue(h, out since)) overflowSince[h] = since = now;
            if (unchecked(now - since) < CLIP_GRACE_MS) { CheckAfterGrace(); return; }
            if (clipRepairAt.TryGetValue(h, out last) && !ClipRepairDue(now, last)) return;
            clipRepairAt[h] = now;
        }
        IntPtr rgn = MakeRegion(square || giveUp.Contains(h), l, t, r, b);
        if (Native.SetWindowRgn(h, rgn, true) == 0)
        {
            Slider.Log("SetWindowRgn failed " + ProcName(h) + " err=" + Marshal.GetLastWin32Error());
            Native.DeleteObject(rgn); // başarılıysa sistem sahiplenir
            applied.Remove(h);
        }
        else applied[h] = key;
    }
}

// ---------------- Kısayollar (~\.config\logical-lunge\keybinds.json) ----------------
// Helper'ın işlediği tüm kısayollar burada tanımlı; sağ paneldeki kısayol düzenleyicisi dosyayı yazar,
// helper dosyayı izleyip anında yeniden yükler. Dosyada olmayan eylem varsayılanını kullanır; boş dize
// ("") o eylemi kapatır. Biçim: "Super+Ctrl+Shift+Alt+Tuş" (Tuş: Left/Right/Up/Down, Enter, Space,
// Tab, Print, F1..F24, A..Z, 0..9).
static class Binds
{
    public const int SUPER = 1, CTRL = 2, SHIFT = 4, ALT = 8;

    // Sıra = düzenleyicideki sıra
    public static readonly string[,] Defaults = {
        { "focus-left", "Super+Left" }, { "focus-right", "Super+Right" }, { "focus-up", "Super+Up" }, { "focus-down", "Super+Down" },
        { "move-left", "Super+Shift+Left" }, { "move-right", "Super+Shift+Right" }, { "move-up", "Super+Shift+Up" }, { "move-down", "Super+Shift+Down" },
        { "ws-prev", "Super+Ctrl+Left" }, { "ws-next", "Super+Ctrl+Right" },
        { "ws-move-prev", "Super+Ctrl+Shift+Left" }, { "ws-move-next", "Super+Ctrl+Shift+Right" },
        { "ws-1", "Super+1" }, { "ws-2", "Super+2" }, { "ws-3", "Super+3" }, { "ws-4", "Super+4" }, { "ws-5", "Super+5" },
        { "ws-6", "Super+6" }, { "ws-7", "Super+7" }, { "ws-8", "Super+8" }, { "ws-9", "Super+9" }, { "ws-10", "Super+0" },
        { "terminal", "Super+Enter" }, { "terminal-alt", "Super+T" },
        { "browser", "Super+W" }, { "files", "Super+E" }, { "code", "Super+C" }, { "editor", "Super+X" },
        { "close", "Alt+F4" }, { "screenshot", "Print" }, { "screenshot-screen", "Ctrl+Print" }, { "clipboard", "Super+V" },
        { "file-search", "Super+S" },
        // Windows'un kendi kabuğunun kombinasyonları da bizim: hiçbiri Windows'a ulaşmaz (Başlat, Arama, Bildirim
        // merkezi açılmasın), karşılıkları Logical Lunge'da
        { "overview-alt", "Ctrl+Escape" }, { "run", "Super+R" }, { "search", "Super+Q" }, { "workspaces", "Super+Tab" },
        { "settings", "Super+I" }, { "sidebar", "Super+A" }, { "notifications", "Super+N" },
        { "screenshot-alt", "Super+Shift+S" }, { "task-manager", "Ctrl+Shift+Escape" },
        { "focus-urgent-or-last", "Super+U" },
    };

    // Uygulama açan kısayollar (düzenleyicide "Uygulamalar": kaldırılabilir, kullanıcı yenilerini ekleyebilir)
    static readonly HashSet<string> appIds = new HashSet<string> { "terminal", "terminal-alt", "browser", "files", "code", "editor" };
    public static bool IsApp(string id) { return appIds.Contains(id) || id.StartsWith("app:"); }
    public static bool IsDefault(string id)
    {
        for (int i = 0; i < Defaults.GetLength(0); i++) if (Defaults[i, 0] == id) return true;
        return false;
    }

    // "super+shift+s" -> "Super+Shift+S"; tanınmayan kombinasyon: null
    public static string Canonical(string combo)
    {
        int m, vk;
        return Parse(combo, out m, out vk) ? Combo(m, vk) : null;
    }

    // Kullanıcının eklediği uygulama kısayolu (keybinds.json > "$apps")
    public sealed class CustomApp { public string Id = "", Name = "", Path = "", Combo = ""; }

    public sealed class UserState
    {
        public Dictionary<string, object> Raw = new Dictionary<string, object>();
        public List<CustomApp> Apps = new List<CustomApp>();
        public HashSet<string> Removed = new HashSet<string>();
    }

    static readonly System.Text.RegularExpressions.Regex appId = new System.Text.RegularExpressions.Regex("^app:[a-z0-9-]{1,40}$");
    static readonly string[] launchable = { ".exe", ".lnk", ".url", ".appref-ms", ".bat", ".cmd" };

    // null: geçerli; değilse neden
    public static string ValidateApp(CustomApp a)
    {
        if (!appId.IsMatch(a.Id)) return "app id";
        if (a.Name.Length == 0 || a.Name.Length > 120) return "app name";
        if (a.Combo != "" && Canonical(a.Combo) == null) return "combo " + a.Combo;
        string p = a.Path;
        // uygulama listesindeki (Super menüsü) mağaza uygulamaları: shell:AppsFolder\<AUMID>
        if (p.StartsWith(@"shell:AppsFolder\", StringComparison.OrdinalIgnoreCase) && p.Length > 17 && p.IndexOfAny(new[] { '"', '\r', '\n' }, 17) < 0) return null;
        try
        {
            if (!System.IO.Path.IsPathRooted(p) || !System.IO.File.Exists(p)) return "app path";
            string ext = System.IO.Path.GetExtension(p).ToLowerInvariant();
            if (Array.IndexOf(launchable, ext) < 0) return "app type";
        }
        catch { return "app path"; }
        return null;
    }

    public static UserState ReadUser()
    {
        var u = new UserState();
        try
        {
            if (!System.IO.File.Exists(FilePath)) return u;
            u.Raw = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(FilePath)) ?? new Dictionary<string, object>();
        }
        catch (Exception ex) { Slider.Log("keybinds: " + ex.Message); return u; }
        object v;
        if (u.Raw.TryGetValue("$apps", out v) && v is System.Collections.ArrayList)
            foreach (var o in (System.Collections.ArrayList)v)
            {
                var d = o as Dictionary<string, object>;
                if (d == null) continue;
                Func<string, string> str = k => { object x; return d.TryGetValue(k, out x) && x is string ? (string)x : ""; };
                var a = new CustomApp { Id = str("id"), Name = str("name"), Path = str("path"), Combo = str("combo") };
                if (ValidateApp(a) == null) u.Apps.Add(a);
            }
        if (u.Raw.TryGetValue("$removed", out v) && v is System.Collections.ArrayList)
            foreach (var o in (System.Collections.ArrayList)v) if (o is string && IsApp((string)o)) u.Removed.Add((string)o);
        return u;
    }

    // Kancadan çağrılır: dosyayı okumaz, Load'un listesine bakar
    static Dictionary<string, string> appPaths = new Dictionary<string, string>();
    public static string AppPath(string id)
    {
        string p;
        lock (gate) return appPaths.TryGetValue(id, out p) ? p : null;
    }

    // Düzenleyicinin kaydı: değişen çekirdek kısayolları, uygulama listesinin tamamı ve kaldırılan varsayılan
    // uygulamalar. reset: önceki kullanıcı değerleri silinir. Okunamayan dosyanın üstüne yazılmaz (kısayollar silinirdi).
    public static bool Write(Dictionary<string, string> core, List<CustomApp> apps, HashSet<string> removed, bool reset = false)
    {
        Dictionary<string, object> d = new Dictionary<string, object>();
        if (!reset && !SettingsFile.TryReadForUpdate(FilePath,
                text => new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(text) ?? new Dictionary<string, object>(),
                () => new Dictionary<string, object>(), out d))
            return false;
        foreach (var kv in core)
        {
            if (kv.Key.StartsWith("app:")) { foreach (var a in apps) if (a.Id == kv.Key) a.Combo = kv.Value; continue; }
            string def = null;
            for (int i = 0; i < Defaults.GetLength(0); i++) if (Defaults[i, 0] == kv.Key) def = Defaults[i, 1];
            if (def == null) return false;
            if (kv.Value == def) d.Remove(kv.Key); else d[kv.Key] = kv.Value;
        }
        d["$apps"] = System.Linq.Enumerable.ToList(System.Linq.Enumerable.Select(apps, a => new Dictionary<string, object> { { "id", a.Id }, { "name", a.Name }, { "path", a.Path }, { "combo", a.Combo } }));
        d["$removed"] = new List<string>(removed);
        if (apps.Count == 0) d.Remove("$apps");
        if (removed.Count == 0) d.Remove("$removed");
        if (!Files.WriteAtomic(FilePath, new JavaScriptSerializer().Serialize(d))) return false;
        Load();
        return true;
    }

    static readonly object gate = new object();
    static Dictionary<long, string> table = new Dictionary<long, string>();
    static System.IO.FileSystemWatcher watcher, captureWatcher, wmWatcher;

    public static string FilePath
    {
        get
        {
            return System.IO.Path.Combine(Paths.ConfigDir, "keybinds.json");
        }
    }

    public static bool Parse(string combo, out int mods, out int vk)
    {
        mods = 0; vk = 0;
        if (string.IsNullOrEmpty(combo)) return false;
        foreach (var raw in combo.Split('+'))
        {
            string p = raw.Trim().ToLowerInvariant();
            if (p == "") continue;
            if (p == "super" || p == "win" || p == "lwin") mods |= SUPER;
            else if (p == "ctrl" || p == "control") mods |= CTRL;
            else if (p == "shift") mods |= SHIFT;
            else if (p == "alt") mods |= ALT;
            else vk = KeyCode(p);
        }
        return vk != 0;
    }

    static int KeyCode(string p)
    {
        switch (p)
        {
            case "left": return 0x25; case "up": return 0x26; case "right": return 0x27; case "down": return 0x28;
            case "enter": case "return": return 0x0D; case "space": return 0x20; case "tab": return 0x09;
            case "escape": case "esc": return 0x1B; case "backspace": return 0x08; case "delete": case "del": return 0x2E;
            case "insert": return 0x2D; case "home": return 0x24; case "end": return 0x23;
            case "pageup": return 0x21; case "pagedown": return 0x22; case "print": case "printscreen": return 0x2C;
            case "minus": return 0xBD; case "plus": case "equal": return 0xBB; case "comma": return 0xBC; case "period": return 0xBE;
            case ";": case "semicolon": return 0xBA; case "'": case "quote": return 0xDE;
        }
        if (p.Length == 1 && ((p[0] >= 'a' && p[0] <= 'z') || (p[0] >= '0' && p[0] <= '9'))) return char.ToUpperInvariant(p[0]);
        int f;
        if (p.Length >= 2 && p[0] == 'f' && int.TryParse(p.Substring(1), out f) && f >= 1 && f <= 24) return 0x70 + f - 1;
        return 0;
    }

    public static Dictionary<string, string> Effective()
    {
        var d = new Dictionary<string, string>();
        for (int i = 0; i < Defaults.GetLength(0); i++) d[Defaults[i, 0]] = Defaults[i, 1];
        try
        {
            if (System.IO.File.Exists(FilePath))
            {
                var user = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(FilePath));
                if (user != null) foreach (var kv in user) if (d.ContainsKey(kv.Key) && !(kv.Value is System.Collections.ArrayList)) d[kv.Key] = kv.Value == null ? "" : kv.Value.ToString();
            }
        }
        catch (Exception ex) { Slider.Log("keybinds: " + ex.Message); }
        return d;
    }

    public static void Load()
    {
        var t = new Dictionary<long, string>();
        var user = ReadUser();
        var all = Effective();
        var paths = new Dictionary<string, string>();
        foreach (var id in user.Removed) all.Remove(id);
        foreach (var a in user.Apps) { all[a.Id] = a.Combo; paths[a.Id] = a.Path; }
        foreach (var kv in all)
        {
            int m, vk;
            if (!Parse(kv.Value, out m, out vk)) continue;
            long key = ((long)m << 16) | (uint)vk;
            if (!t.ContainsKey(key)) t[key] = kv.Key; // çakışmada listedeki ilk eylem kazanır
        }
        lock (gate) { table = t; appPaths = paths; }
        Slider.Log("keybinds: " + t.Count + " kısayol");
    }

    public static string Lookup(int mods, int vk)
    {
        string a;
        lock (gate) return table.TryGetValue(((long)mods << 16) | (uint)vk, out a) ? a : null;
    }

    public static void Watch()
    {
        Load();
        WmBinds.Load();
        try
        {
            var wm = new System.IO.FileSystemWatcher(System.IO.Path.GetDirectoryName(Paths.ConfigFile), System.IO.Path.GetFileName(Paths.ConfigFile));
            System.IO.FileSystemEventHandler wmReload = (s, e) => { Thread.Sleep(80); WmBinds.Load(); };
            wm.Changed += wmReload; wm.Created += wmReload;
            wm.Renamed += (s, e) => { Thread.Sleep(80); WmBinds.Load(); };
            wm.EnableRaisingEvents = true;
            wmWatcher = wm;
            watcher = new System.IO.FileSystemWatcher(System.IO.Path.GetDirectoryName(FilePath), "keybinds.json");
            System.IO.FileSystemEventHandler reload = (s, e) => { Thread.Sleep(80); Load(); };
            watcher.Changed += reload; watcher.Created += reload; watcher.Deleted += reload;
            watcher.Renamed += (s, e) => { Thread.Sleep(80); Load(); };
            watcher.EnableRaisingEvents = true;
            var cw = new System.IO.FileSystemWatcher(CaptureDir, "capture.req");
            cw.Created += (s, e) => { Capturing = true; try { System.IO.File.Delete(e.FullPath); } catch { } };
            cw.EnableRaisingEvents = true;
            captureWatcher = cw;
        }
        catch (Exception ex) { Slider.Log("keybinds watch: " + ex.Message); }
    }

    // ---- Düzenleyici için tuş yakalama: panel "capture.req" yazar, ana helper sonraki kombinasyonu
    // "capture.res"e yazar (Super'i helper yuttuğu için tarayıcı penceresi onu hiç göremiyordu).
    public static volatile bool Capturing;
    public static string CaptureDir { get { return System.IO.Path.GetDirectoryName(FilePath); } }

    public static string KeyName(int vk)
    {
        switch (vk)
        {
            case 0x25: return "Left"; case 0x26: return "Up"; case 0x27: return "Right"; case 0x28: return "Down";
            case 0x0D: return "Enter"; case 0x20: return "Space"; case 0x09: return "Tab"; case 0x1B: return "Escape";
            case 0x08: return "Backspace"; case 0x2E: return "Delete"; case 0x2D: return "Insert"; case 0x24: return "Home";
            case 0x23: return "End"; case 0x21: return "PageUp"; case 0x22: return "PageDown"; case 0x2C: return "Print";
            case 0xBD: return "Minus"; case 0xBB: return "Plus"; case 0xBC: return "Comma"; case 0xBE: return "Period";
            case 0xBA: return ";"; case 0xDE: return "'";
        }
        if ((vk >= 0x41 && vk <= 0x5A) || (vk >= 0x30 && vk <= 0x39)) return ((char)vk).ToString();
        if (vk >= 0x70 && vk <= 0x87) return "F" + (vk - 0x70 + 1);
        return null;
    }
    public static string Combo(int mods, int vk)
    {
        string k = KeyName(vk);
        if (k == null) return null;
        var sb = new StringBuilder();
        if ((mods & SUPER) != 0) sb.Append("Super+");
        if ((mods & CTRL) != 0) sb.Append("Ctrl+");
        if ((mods & SHIFT) != 0) sb.Append("Shift+");
        if ((mods & ALT) != 0) sb.Append("Alt+");
        return sb.Append(k).ToString();
    }
    public static void FinishCapture(string combo)
    {
        Capturing = false;
        try { System.IO.File.WriteAllText(System.IO.Path.Combine(CaptureDir, "capture.res"), combo ?? ""); } catch { }
    }

    // Kullanıcı değerini yaz (varsayılana eşitse dosyadan çıkar); id "" ise hepsini sıfırla. false: yazılmadı
    // (bilinmeyen kısayol, dosya okunamadı ya da yazılamadı); okunamayan dosyanın üstüne yazmak diğer özel kısayolları silerdi.
    public static bool Set(string id, string combo)
    {
        var d = new Dictionary<string, object>();
        if (id != "")
        {
            string def = null;
            for (int i = 0; i < Defaults.GetLength(0); i++) if (Defaults[i, 0] == id) def = Defaults[i, 1];
            if (def == null) return false;
            if (!SettingsFile.TryReadForUpdate(FilePath,
                    text => new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(text) ?? new Dictionary<string, object>(),
                    () => new Dictionary<string, object>(), out d))
                return false;
            if (combo == def) d.Remove(id); else d[id] = combo;
        }
        if (!Files.WriteAtomic(FilePath, new JavaScriptSerializer().Serialize(d))) return false;
        Load();
        return true;
    }

    // lunge.exe --keybinds -> [{"id","combo","default"}] (düzenleyici için)
    public static string ListJson()
    {
        var eff = Effective();
        var list = new List<Dictionary<string, object>>();
        for (int i = 0; i < Defaults.GetLength(0); i++)
            list.Add(new Dictionary<string, object> { { "id", Defaults[i, 0] }, { "combo", eff[Defaults[i, 0]] }, { "default", Defaults[i, 1] } });
        return new JavaScriptSerializer().Serialize(list);
    }
}

// ---------------- Klavye ----------------
class Keys2
{
    readonly DesktopMenuKeyState desktopMenuKeys = new DesktopMenuKeyState();
    const int VK_LWIN = 0x5B, VK_RWIN = 0x5C, VK_CONTROL = 0x11, VK_SHIFT = 0x10, VK_MENU = 0x12;
    const int VK_LEFT = 0x25, VK_UP = 0x26, VK_RIGHT = 0x27, VK_DOWN = 0x28;
    const byte VK_DUMMY = 0xE8; // atanmamış tuş: Başlat menüsünü bastırmak için

    readonly Control ui;
    readonly Slider slider;
    Native.LowLevelKeyboardProc proc;
    bool winDown, otherKeyWhileWin, modifierWhileWin, dockChord, dockMasked;
    int winVk = VK_LWIN, lastWinEvent;

    public static Keys2 Instance;
    public Keys2(Control ui, Slider slider) { this.ui = ui; this.slider = slider; Instance = this; }
    // Bar'dan (yerel HTTP) gelen workspace komutu: klavyedeki kısayolla aynı yol
    public bool Dispatch(string act) { return RunAction(act); }
    static int lastMoveAction = Environment.TickCount - 100000;

    // Test kanalı (yalnızca LL_TEST=1 ortam değişkeniyle başlatılınca açılır): \\.\pipe\lunge-test'e yazılan her
    // satır (ws-3, move-left, ws-move-next ...) klavyenin çağırdığı RunAction'a gider. Kanca enjekte tuşları bilerek yok
    // saydığı için otomatik animasyon testinin tek yolu.
    public void StartTestPipe()
    {
        if (Environment.GetEnvironmentVariable("LL_TEST") != "1") return;
        new Thread(() =>
        {
            while (true)
            {
                try
                {
                    using (var pipe = new System.IO.Pipes.NamedPipeServerStream("lunge-test", System.IO.Pipes.PipeDirection.In))
                    {
                        pipe.WaitForConnection();
                        using (var rd = new System.IO.StreamReader(pipe))
                        {
                            string line;
                            while ((line = rd.ReadLine()) != null)
                            {
                                line = line.Trim();
                                if (line.Length == 0) continue;
                                Slider.Log("test: " + line);
                                try { RunAction(line); } catch (Exception ex) { Slider.Log("test hata: " + ex.Message); }
                            }
                        }
                    }
                }
                catch (Exception ex) { Slider.Log("test kanalı: " + ex.Message); Thread.Sleep(500); }
            }
        }) { IsBackground = true }.Start();
    }

    IntPtr hookHandle = IntPtr.Zero;

    public void Start()
    {
        proc = Callback.Guard("klavye kancası", (Native.LowLevelKeyboardProc)Hook);
        hookHandle = Native.SetWindowsHookEx(Native.WH_KEYBOARD_LL, proc, Native.GetModuleHandle(null), 0);
    }

    public void Reinstall() { Reinstall(false); }

    // force: kanca Windows tarafından sökülmüş (tuşlar bize gelmiyor): basılı Win bilgisi de artık geçersiz
    public void Reinstall(bool force)
    {
        // Tuş basılıyken değiştirme (durum karışmasın)
        if (winDown && !force) return;
        if (force) ForgetKeys();
        IntPtr fresh = Native.SetWindowsHookEx(Native.WH_KEYBOARD_LL, proc, Native.GetModuleHandle(null), 0);
        if (fresh == IntPtr.Zero) return;
        IntPtr old = hookHandle;
        hookHandle = fresh;
        if (old != IntPtr.Zero) Native.UnhookWindowsHookEx(old);
    }

    static bool Down(int vk) { return (Native.GetAsyncKeyState(vk) & 0x8000) != 0; }

    // Bırakmaları bize gelmeyen basılı tuş bilgisini unut (kanca söküldü, masaüstü değişti)
    void ForgetKeys()
    {
        winDown = false; dockChord = false; dockMasked = false; held.Clear();
        desktopMenuKeys.Clear();
    }

    // Kilit ekranı / UAC / güvenli masaüstü: o sırada basılan ve bırakılan tuşlar bize gelmez. Kanca thread'inde çalışır.
    Native.WinEventDelegate desktopCb;
    public void WatchDesktopSwitch()
    {
        desktopCb = Callback.Guard("masaüstü değişimi", (h, ev, hwnd, obj, child, thread, time) => { ForgetKeys(); Unstick(); });
        Native.SetWinEventHook(0x0020, 0x0020, IntPtr.Zero, desktopCb, 0, 0, 0x0000); // EVENT_SYSTEM_DESKTOPSWITCH, OUTOFCONTEXT
    }

    // Windows Win'i basılı sanıyor ama biz onu ne basılı tutuyoruz ne de enjekte ettik: kanca zamanında cevap
    // veremeyince basış sisteme geçmiş, bırakışı biz yutmuşuz. Böyle kalırsa Q arama, A bildirimler, Ctrl Başlat
    // açar ve hiçbir yere yazı yazılamaz. Bırakış enjekte edilir (sahte tuş: Başlat menüsü açılmasın).
    // Kanca thread'inde çağrılır (kancayla yarışmaz).
    public void Unstick()
    {
        foreach (int w in new[] { VK_LWIN, VK_RWIN })
        {
            if (!Down(w)) continue;
            SuppressStart();
            Native.keybd_event((byte)w, 0, 0x2 | 0x1, Native.LL_MARK);
            ThreadPool.QueueUserWorkItem(_ => Slider.Log("Win tuşu Windows'ta basılı kalmıştı: bırakıldı (0x" + w.ToString("X") + ")"));
        }
    }

    // Hızlı art arda workspace geçişleri sıraya girip her biri animasyonunu beklemesin:
    // süren animasyon hemen biter, biriken istekler komut olarak uygulanır, yalnızca SONUNCUSU kayar.
    readonly object pendLock = new object();
    readonly object inWsLock = new object(); // odak/taşıma istekleri sırayla, ama UI thread'ini (slide) beklemeden
    readonly List<object[]> pend = new List<object[]>();

    // Whether window manager commands switch the shown workspace (the last one focuses a workspace): the
    // slide's direction (+1 next, -1 previous, 0 from the names) and, for a named workspace, its name
    internal static bool SwitchesWorkspace(string[] cmds, out int dir, out string target)
    {
        dir = 0; target = null;
        if (cmds == null || cmds.Length == 0 || cmds.Length > 2) return false;
        string last = cmds[cmds.Length - 1];
        if (cmds.Length == 2 && !(cmds[0].StartsWith("move --") && last.StartsWith("focus --"))) return false;
        if (last == "focus --next-workspace" || last == "focus --next-active-workspace") { dir = 1; return true; }
        if (last == "focus --prev-workspace" || last == "focus --prev-active-workspace") { dir = -1; return true; }
        if (last == "focus --recent-workspace") return true;
        if (last.StartsWith("focus --workspace ")) { target = last.Substring("focus --workspace ".Length).Trim(); return target.Length > 0; }
        return false;
    }

    void Post(string[] cmds, int dir, string target)
    {
        lock (pendLock) pend.Add(new object[] { cmds, dir, target });
        slider.Interrupt = true; // önceki animasyon varsa hemen bitir
        ui.BeginInvoke((Action)DrainSlides);
    }

    void DrainSlides()
    {
        List<object[]> batch;
        lock (pendLock)
        {
            if (pend.Count == 0) return;
            batch = new List<object[]>(pend);
            pend.Clear();
        }
        try
        {
            for (int i = 0; i < batch.Count - 1; i++) slider.Commands((string[])batch[i][0]);
            var last = batch[batch.Count - 1];
            slider.Run((string[])last[0], (int)last[1], (string)last[2]);
        }
        catch (Exception ex) { Slider.Log("slide: " + ex.Message); }
    }

    static void SuppressStart() { Native.keybd_event(VK_DUMMY, 0, 0, Native.LL_MARK); Native.keybd_event(VK_DUMMY, 0, 2, Native.LL_MARK); }

    // Super+Alt (Dock): Win yutulurken Alt odaktaki uygulamaya gider; uygulama bunu tek başına bir Alt basışı sanıp
    // menüsünü açar (Zen/Firefox menü çubuğu, Gezgin'in kısayol harfleri). Alt'ın basılışı ile bırakılışı arasına
    // atanmamış bir tuş girince Windows menüyü açmaz (Başlat menüsünü bastıran hileyle aynı). Kombinasyon başına bir kez:
    // Alt'ın otomatik tekrarı yeniden enjekte etmesin.
    void MaskAltMenu()
    {
        if (dockMasked) return;
        dockMasked = true;
        SuppressStart();
    }

    public static volatile int LastHookTick = Environment.TickCount;

    // Süre ölçümü: kanca Windows'un sınırını (~300 ms) aşarsa tuş işlenmeden uygulamaya gider, tekrarlarsa kanca sessizce
    // sökülür. Yavaş çağrılar log'a yazılır (hangi tuşta).
    IntPtr Hook(int nCode, IntPtr wParam, IntPtr lParam)
    {
        LastHookTick = Environment.TickCount;
        var mark = InputLatency.Start();
        IntPtr r = HookInner(nCode, wParam, lParam);
        string slow = InputLatency.Slow(mark);
        if (slow != null)
        {
            int vk = nCode >= 0 ? Marshal.ReadInt32(lParam) : -1;
            ThreadPool.QueueUserWorkItem(_ => Slider.Log("klavye kancası yavaş: " + slow + " (tuş 0x" + vk.ToString("X") + ")"));
        }
        return r;
    }

    IntPtr HookInner(int nCode, IntPtr wParam, IntPtr lParam)
    {
        if (nCode < 0) return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
        var k = (Native.KBDLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(Native.KBDLLHOOKSTRUCT));
        if ((k.flags & Native.LLKHF_INJECTED) != 0 && (UIntPtr)(ulong)k.extra.ToInt64() == Native.LL_MARK) return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
        // Kabuk (bar) çökmüş / açılamamışsa tuşlar olduğu gibi Windows'a: Win tuşu Başlat menüsünü açar (ShellState).
        // Basılı bir Win ya da açık değiştirici varsa önce o biter.
        if (!ShellState.Up && !winDown && !Switcher.Active && !held.Contains((int)k.vkCode) && !desktopMenuKeys.Contains((int)k.vkCode)) return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);

        int msg = wParam.ToInt32();
        bool isDown = msg == Native.WM_KEYDOWN || msg == Native.WM_SYSKEYDOWN;
        bool isUp = msg == Native.WM_KEYUP || msg == Native.WM_SYSKEYUP;
        int vk = (int)k.vkCode;
        // Alt+Tab pencere değiştirici (Switcher): Windows'un kendisi hiç açılmaz
        bool altHeld = Down(VK_MENU) || (k.flags & 0x20) != 0; // LLKHF_ALTDOWN
        if (Switcher.Active || (isDown && vk == 0x09 && altHeld && !winDown))
        {
            if (Switcher.HandleKey(vk, isDown, isUp, Down(VK_SHIFT), altHeld, Down(VK_CONTROL) || winDown)) return (IntPtr)1;
        }

        // Masaüstü öndeyken menü tuşu / Shift+F10: Explorer'ın menüsü yerine barınki (fare sağ tıkıyla aynı menü, seçili
        // simgenin yerinde). Basış da bırakış da yutulur.
        bool desktopMenuOpen;
        bool desktopMenuEligible = isDown && ShellState.Up && !Binds.Capturing
            && (vk == 0x5D || (vk == 0x79 && Down(VK_SHIFT))) && !winDown
            && DesktopClick.IsDesktopWindow(Native.GetForegroundWindow());
        if (desktopMenuKeys.Handle(vk, isDown, isUp, desktopMenuEligible, out desktopMenuOpen))
        {
            if (desktopMenuOpen) ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:desktop-menu-key"));
            return (IntPtr)1;
        }

        // Masaüstü öndeyken Enter: seçili simgeleri Explorer değil biz açarız (çift tıklamayla aynı yol). Alt+Enter
        // (Özellikler), Ctrl'li kombinasyonlar ve yeniden adlandırma kutusundaki Enter Explorer'ın kalır.
        if (isDown && vk == 0x0D && !winDown && !Down(VK_CONTROL) && !Down(VK_MENU))
        {
            IntPtr fg = Native.GetForegroundWindow();
            if (DesktopClick.IsDesktopWindow(fg) && !DesktopClick.FocusIsEdit(fg))
            {
                if (!held.Contains(vk)) ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:desktop-open-key"));
                held.Add(vk);
                return (IntPtr)1;
            }
        }

        // Gerçek Win tuşu Windows'a HİÇ iletilmez ve hiç enjekte edilmez: Windows bir Win basışı görmediği için Başlat
        // menüsü, Arama, Bildirim merkezi, Win+X hiçbir tuş sırasıyla açılamaz. Super'li her kombinasyon bizim:
        // kısayol tablosu, Windows'un kilidi (Reserved), pencere yöneticisinin tablosu (WmBinds); hiçbirinde yoksa yutulur.
        if (vk == VK_LWIN || vk == VK_RWIN)
        {
            // Win+L gibi durumlarda bırakma olayı hiç gelmeyebilir: otomatik tekrar ~30 ms'de bir gelir,
            // uzun bir aradan sonraki basış yeni basıştır.
            int now = Environment.TickCount;
            bool fresh = !winDown || now - lastWinEvent > 700;
            lastWinEvent = now;
            if (isDown && fresh)
            {
                winDown = true; winVk = vk;
                otherKeyWhileWin = false;
                // Win'den önce basılı tutulan Ctrl/Shift/Alt da "kombinasyon" sayılır
                modifierWhileWin = Down(VK_CONTROL) || Down(VK_SHIFT) || Down(VK_MENU);
                dockChord = Down(VK_MENU) && !Down(VK_CONTROL) && !Down(VK_SHIFT);
                dockMasked = false;
                if (dockChord) MaskAltMenu();
            }
            if (isUp)
            {
                bool toggleDock = dockChord && !otherKeyWhileWin && !Binds.Capturing;
                winDown = false;
                dockChord = false;
                dockMasked = false;
                // Basış sisteme geçmişse (kanca geç kaldı) Windows Win'i hâlâ basılı sanıyor: bırakış, araya sahte bir
                // tuş girerek gönderilir (tek başına Win bırakışı Başlat'ı açardı)
                if (Down(vk))
                {
                    SuppressStart();
                    Native.keybd_event((byte)vk, 0, 0x2 | 0x1, Native.LL_MARK); // KEYUP | EXTENDEDKEY
                }
                if (toggleDock) ui.BeginInvoke((Action)(() => Toasts.Emit("ll:dock-toggle")));
                else if (!otherKeyWhileWin && !modifierWhileWin && !Binds.Capturing) ui.BeginInvoke((Action)ToggleOverview);
            }
            return (IntPtr)1; // basış, otomatik tekrar ve bırakma: hepsi yutulur
        }

        // Kısayol düzenleyicisi tuş bekliyor: ilk kombinasyonu ona ver, hiçbir eylemi çalıştırma
        if (Binds.Capturing && isDown && !(vk == VK_CONTROL || vk == VK_SHIFT || vk == VK_MENU || (vk >= 0xA0 && vk <= 0xA5)))
        {
            int cm = (winDown ? Binds.SUPER : 0) | (Down(VK_CONTROL) ? Binds.CTRL : 0) | (Down(VK_SHIFT) ? Binds.SHIFT : 0) | (Down(VK_MENU) ? Binds.ALT : 0);
            string combo = vk == 0x1B && cm == 0 ? "" : Binds.Combo(cm, vk); // tek başına Esc: iptal
            if (combo != null) { Binds.FinishCapture(combo); held.Add(vk); if (winDown) otherKeyWhileWin = true; return (IntPtr)1; }
        }

        if (winDown && isDown)
        {
            bool isModifier = vk == VK_CONTROL || vk == VK_SHIFT || vk == VK_MENU || (vk >= 0xA0 && vk <= 0xA5);
            if (isModifier)
            {
                modifierWhileWin = true;
                if ((vk == VK_MENU || vk == 0xA4 || vk == 0xA5) && !otherKeyWhileWin && !Down(VK_CONTROL) && !Down(VK_SHIFT)) { dockChord = true; MaskAltMenu(); }
                if (vk == VK_CONTROL || vk == VK_SHIFT || vk == 0xA2 || vk == 0xA3 || vk == 0xA0 || vk == 0xA1) dockChord = false;
            }
            else { otherKeyWhileWin = true; dockChord = false; }
        }

        // Kısayol tablosu (keybinds.json): Super / Ctrl / Shift / Alt + tuş -> eylem
        bool modKey = vk == VK_CONTROL || vk == VK_SHIFT || vk == VK_MENU || (vk >= 0xA0 && vk <= 0xA5);
        if (isDown && !modKey)
        {
            int mods = (winDown ? Binds.SUPER : 0) | (Down(VK_CONTROL) ? Binds.CTRL : 0) | (Down(VK_SHIFT) ? Binds.SHIFT : 0) | (Down(VK_MENU) ? Binds.ALT : 0);
            string act = Binds.Lookup(mods, vk);
            if (act != null)
            {
                bool repeat = held.Contains(vk);
                // Basılı tutunca tekrar eden eylemler: odak/taşıma/workspace. Uygulama açma, kapatma, ekran alıntısı bir kez.
                bool repeatable = act.StartsWith("focus-") || act.StartsWith("move-") || act == "ws-prev" || act == "ws-next" || act.StartsWith("ws-move-");
                if (repeat && !repeatable) return (IntPtr)1;
                if (repeat || RunAction(act))
                {
                    held.Add(vk);
                    if (winDown)                    // Ctrl+Super (+Shift) ile gezinme: bar noktaların yerine numaraları kısa süre gösterir (kanca beklemesin)
                    if (!repeat && (act == "ws-prev" || act == "ws-next" || act.StartsWith("ws-move-")))
                        ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:ws-numbers"));
                    return (IntPtr)1;
                }
            }
        }
        if (isUp && held.Remove(vk)) return (IntPtr)1;
        if (winDown && isDown && !modKey)
        {
            int mods = Binds.SUPER | (Down(VK_CONTROL) ? Binds.CTRL : 0) | (Down(VK_SHIFT) ? Binds.SHIFT : 0) | (Down(VK_MENU) ? Binds.ALT : 0);
            held.Add(vk);
            // Windows'un kilidi (Super+L): Win Windows'a ulaşmadığı için kilidi çekirdek ister
            string reserved = Reserved.Action(mods, vk);
            if (reserved != null) { if (reserved.Length > 0) RunAction(reserved); return (IntPtr)1; }
            // Pencere yöneticisinin Super'li kısayolu: komutları IPC ile (pencere yöneticisi Win'i hiç görmez)
            string[] wm = WmBinds.Lookup(mods, vk);
            int slideDir; string slideTarget;
            if (wm != null && SwitchesWorkspace(wm, out slideDir, out slideTarget))
            {
                // the window manager's own workspace keys (Super+PageUp/PageDown, Super+Ctrl+Alt+←/→, their
                // Shift forms) slide like ours, through the same queue
                lastMoveAction = Environment.TickCount;
                Post(wm, slideDir, slideTarget);
                return (IntPtr)1;
            }
            if (wm != null)
            {
                // state changes (fullscreen, floating) animate through a freeze, as a layout change does
                ThreadPool.QueueUserWorkItem(_ => { lock (inWsLock) { try { if (!Dwindle.AnimateState(wm)) slider.Commands(wm); } catch (Exception ex) { Slider.Log("kısayol: " + ex.Message); } } });
                return (IntPtr)1;
            }
            return (IntPtr)1; // hiçbir tabloda yok: Windows'a da gitmez
        }

        return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
    }

    readonly HashSet<int> held = new HashSet<int>();

    // Kısayol eylemleri. false: işlenmedi (tuş normal yoluna devam eder)
    // Aynı işi gören kısayollar (Windows'un kendi kombinasyonlarının karşılıkları): tablo, eylem kodu değil
    static readonly Dictionary<string, string> aliases = new Dictionary<string, string> {
        { "overview-alt", "overview" }, { "search", "overview" }, { "workspaces", "overview" },
        { "notifications", "sidebar" }, { "screenshot-alt", "screenshot" },
    };

    [DllImport("user32.dll")] static extern bool LockWorkStation();

    bool RunAction(string act)
    {
        string alias;
        if (aliases.TryGetValue(act, out alias)) act = alias;
        // Pencere hareketi / odak / workspace kısayolları overview'u (arama, pano) kapatır. Kanca thread'i başka bir
        // sürecin penceresini gizlerken beklemesin (ShowWindow karşı tarafı bekler): arayüz thread'inde.
        if (act.StartsWith("move-") || act.StartsWith("focus-") || act.StartsWith("ws-"))
        {
            lastMoveAction = Environment.TickCount;
            ui.BeginInvoke((Action)(() =>
            {
                IntPtr ov = Native.FindWindow(null, "lunge-overview");
                if (ov != IntPtr.Zero && Native.IsWindowVisible(ov)) HideOverview(ov);
            }));
        }
        if (act == "settings") { ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:settings-toggle")); return true; }
        if (act == "lock") { LockWorkStation(); return true; }
        if (act == "task-manager") { LaunchQueue.Enqueue("taskmgr.exe"); return true; }
        // Win+R: Windows'un Çalıştır penceresi, uygulamalar gibi kullanıcı olarak (yönetici değil) açılır
        if (act == "run") { LaunchQueue.Enqueue(AppIndex.RunDialog); return true; }
        if (act.StartsWith("app:"))
        {
            string path = Binds.AppPath(act);
            if (path == null) return false;
            LaunchQueue.Enqueue(path);
            return true;
        }
        if (act == "clipboard") { ui.BeginInvoke((Action)ToggleClipboard); return true; }
        if (act == "file-search") { ui.BeginInvoke((Action)ToggleFileSearch); return true; }
        // Parmak hareketleri (dokunmatik yüzey): overview ve sağ panel (panel kabukta: olay bildirim akışıyla gider)
        if (act == "overview") { ui.BeginInvoke((Action)ToggleOverview); return true; }
        if (act == "sidebar") { ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:sidebar-right-toggle")); return true; }
        string[] dirs = { "left", "right", "up", "down" };
        foreach (var d0 in dirs)
        {
            string d = d0;
            if (act == "focus-" + d || act == "move-" + d)
            {
                bool mv = act.StartsWith("move-");
                ThreadPool.QueueUserWorkItem(_ =>
                {
                    lock (inWsLock)
                    {
                        try { if (mv) slider.MoveInWorkspace(d); else slider.FocusInWorkspace(d); }
                        catch (Exception ex) { Debug.WriteLine(ex); }
                    }
                });
                return true;
            }
        }
        if (act == "ws-prev" || act == "ws-next" || act == "ws-move-prev" || act == "ws-move-next")
        {
            int dir = act.EndsWith("next") ? 1 : -1;
            string d = dir > 0 ? "next" : "prev";
            if (act.StartsWith("ws-move-")) Post(new[] { "move --" + d + "-workspace", "focus --" + d + "-workspace" }, dir, null);
            else Post(new[] { "focus --" + d + "-workspace" }, dir, null);
            return true;
        }
        // Hyprland focusurgentorlast: en son dikkat isteyen pencere (workspace'i başkaysa kayarak), yoksa bir önceki workspace
        if (act == "focus-urgent-or-last")
        {
            ThreadPool.QueueUserWorkItem(_ =>
            {
                try
                {
                    long h = Urgent.Latest();
                    string id, ws; bool shown;
                    if (h == 0) Post(new[] { "focus --recent-workspace" }, 0, null);
                    else if (!slider.FindHandle(h, out id, out ws, out shown)) FocusSink.Give(new IntPtr(h));
                    else if (shown) slider.Commands(new[] { "focus --container-id " + id });
                    else Post(new[] { "focus --container-id " + id }, 0, ws);
                }
                catch (Exception ex) { Slider.Log("urgent: " + ex.Message); }
            });
            return true;
        }
        if (act.StartsWith("ws-"))
        {
            string n = act.Substring(3);
            Post(new[] { "focus --workspace " + n }, 0, n);
            return true;
        }
        if (act == "terminal" || act == "terminal-alt") { LaunchQueue.Enqueue(Terminal); return true; }
        string app;
        if (Apps.TryGetValue(act, out app)) { ui.BeginInvoke((Action)(() => Launch(app))); return true; }
        if (act == "screenshot")
        {
            // Kanca thread'inde süreç başlatma (ShellExecute 50-200 ms): o sırada tüm klavye beklerdi
            ThreadPool.QueueUserWorkItem(_ => { try { Process.Start(new ProcessStartInfo(Application.ExecutablePath, "--snip") { UseShellExecute = true }); } catch { } });
            return true;
        }
        if (act == "screenshot-screen")
        {
            ThreadPool.QueueUserWorkItem(_ => { try { Process.Start(new ProcessStartInfo(Application.ExecutablePath, "--snip-screen") { UseShellExecute = true }); } catch { } });
            return true;
        }
        if (act == "close")
        {
            // Hyprland killactive: odaktaki pencereye kapat komutunu doğrudan gönder (konsol/PowerShell
            // tuşu kendisi yutup kapanmıyordu). Masaüstü ve kabuk pencerelerinde Windows'un davranışı kalır.
            IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
            var cls = new StringBuilder(64); Native.GetClassName(fg, cls, 64);
            var title = new StringBuilder(128); Native.GetWindowText(fg, title, 128);
            string c = cls.ToString(), t = title.ToString();
            bool shell = fg == IntPtr.Zero || c == "Progman" || c == "WorkerW" || c == "Shell_TrayWnd" || t.StartsWith(Names.TitlePrefix) || t.StartsWith("lunge-");
            if (shell) return false;
            Native.PostMessage(fg, 0x0112, (IntPtr)0xF060, IntPtr.Zero); // WM_SYSCOMMAND SC_CLOSE
            return true;
        }
        return false;
    }
    public static string TerminalPath { get { return Terminal; } }
    // Hyprland $terminal (kitty) karşılığı: logical-lunge içindeki WezTerm; yoksa Windows Terminal
    static readonly string Terminal = System.IO.File.Exists(Paths.Tool(@"wezterm\wezterm-gui.exe"))
        ? Paths.Tool(@"wezterm\wezterm-gui.exe") : "wt.exe";

    // Hyprland keybinds.lua: Super+W tarayıcı, E dosya yöneticisi, C kod editörü, X metin editörü.
    // tiling'in shell-exec'i boşluklu tırnaklı yolları ayrıştıramıyordu ("doesn't have an ending").
    // Use user preferences and Windows associations instead of vendor/path lists.
    static readonly Dictionary<string, string> Apps = new Dictionary<string, string>
    {
        { "browser", DefaultBrowser() },
        { "files", "explorer.exe" },
        { "code", DefaultEditor() },
        { "editor", "notepad.exe" },
    };
    // "Edit config": VISUAL/EDITOR executable, registered YAML editor, then Windows text editor.
    public static string CodeEditor { get { return Apps["code"]; } }
    static string DefaultEditor()
    {
        foreach (string key in new[] { "VISUAL", "EDITOR" })
        {
            string value = Environment.GetEnvironmentVariable(key);
            if (string.IsNullOrWhiteSpace(value)) continue;
            string exe = Environment.ExpandEnvironmentVariables(value.Trim().Trim('"'));
            if (System.IO.File.Exists(exe)) return exe;
        }
        foreach (string ext in new[] { ".yaml", ".yml", ".txt" })
        {
            try
            {
                string prog = Convert.ToString(Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\" + ext + @"\UserChoice", "ProgId", null));
                if (string.IsNullOrEmpty(prog)) prog = Convert.ToString(Microsoft.Win32.Registry.GetValue(@"HKEY_CLASSES_ROOT\" + ext, "", null));
                foreach (string verb in new[] { "edit", "open" })
                {
                    string cmd = Convert.ToString(Microsoft.Win32.Registry.GetValue(@"HKEY_CLASSES_ROOT\" + prog + @"\shell\" + verb + @"\command", "", null));
                    if (string.IsNullOrWhiteSpace(cmd)) continue;
                    string exe = cmd.StartsWith("\"") ? cmd.Substring(1, cmd.IndexOf('"', 1) - 1) : cmd.Split(' ')[0];
                    exe = Environment.ExpandEnvironmentVariables(exe);
                    if (System.IO.File.Exists(exe)) return exe;
                }
            }
            catch { }
        }
        return "notepad.exe";
    }
    static string DefaultBrowser()
    {
        try
        {
            var prog = (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice", "ProgId", null);
            var cmd = prog == null ? null : (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CLASSES_ROOT\" + prog + @"\shell\open\command", "", null);
            if (!string.IsNullOrEmpty(cmd))
            {
                string exe = cmd.StartsWith("\"") ? cmd.Substring(1, cmd.IndexOf('"', 1) - 1) : cmd.Split(' ')[0];
                if (System.IO.File.Exists(exe)) return exe;
            }
        }
        catch { }
        // No registered browser: let the user choose one through Windows.
        return "ms-settings:defaultapps";
    }

    void Launch(string path) { LaunchQueue.Enqueue(path); }

    // Overview'u önce saydam göster; widget helper'ın bıraktığı mod bayrağını okuyup arayüzü kurunca (bayrak silinir)
    // görünür yap. Aksi halde önce düz arama, sonra ";" pano modu görünüyordu.
    // Overview modu bayrağı: bir kez okunur ve silinir ("" ya da ";" = pano, "#" = dosya araması)
    public static string TakeOverviewMode()
    {
        string f = Paths.State(@"overview-mode.txt");
        string m = "";
        if (System.IO.File.Exists(f)) { try { m = System.IO.File.ReadAllText(f).Trim(); System.IO.File.Delete(f); } catch { } }
        return m;
    }

    // Overview (Super / Super+V) kapanınca odak boşta kalıyordu (elle tıklamak gerekiyordu): açılmadan önceki pencereye
    // geri ver. Kapanırken başka bir pencere odak aldıysa (overview'dan uygulama açıldı, başka yere tıklandı) ya da bir
    // workspace / taşıma kısayoluyla kapandıysa (odağı tiling yönetir) dokunma.
    static int overviewGen;
    static bool ShellLike(IntPtr w)
    {
        if (w == IntPtr.Zero) return true;
        var c = new StringBuilder(64); Native.GetClassName(w, c, 64);
        var t = new StringBuilder(128); Native.GetWindowText(w, t, 128);
        string cs = c.ToString(), ts = t.ToString();
        return cs == "Progman" || cs == "WorkerW" || cs == "Shell_TrayWnd" || ts.StartsWith(Names.TitlePrefix) || ts.StartsWith("lunge-");
    }
    static void RestoreFocusAfterOverview(IntPtr ov, IntPtr prev, int gen)
    {
        var sw = Stopwatch.StartNew();
        while (!Native.IsWindowVisible(ov) && sw.ElapsedMilliseconds < 1500) Thread.Sleep(15);
        while (Native.IsWindowVisible(ov)) { if (gen != overviewGen || sw.Elapsed.TotalMinutes > 30) return; Thread.Sleep(15); }
        if (gen != overviewGen) return;
        Thread.Sleep(60); // yeni açılan pencere / tiling odağı alsın
        if (gen != overviewGen || Environment.TickCount - lastMoveAction < 700) return;
        IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
        if (fg != ov && !ShellLike(fg)) return;
        int cl;
        if (!Native.IsWindow(prev) || !Native.IsWindowVisible(prev) || Native.IsIconic(prev)) return;
        if (Native.DwmGetWindowAttribute(prev, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0) return; // başka workspace'te
        Native.keybd_event(VK_DUMMY, 0, 0, Native.LL_MARK); Native.keybd_event(VK_DUMMY, 0, 2, Native.LL_MARK);
        Native.SetForegroundWindow(prev);
    }

    // Overview widget'ı helper'ın Win32 ile gösterip gizlediğini görünürlüğü 40 ms'de bir sorarak anlıyordu (gün boyu
    // saniyede 25 IPC: boştaki shell'in başlıca işi). Artık helper haber verir; widget /overview-wait uzun yoklamasıyla
    // bekler. Sıra numarası iki istek arasındaki olayı kaçırmamak için; aradaki birden fazla olaydan sonuncusu yeter.
    static readonly object ovSignalLock = new object();
    static int ovSignalSeq;
    static string ovSignalWhat = "";
    public static void OverviewSignal(string what)
    {
        lock (ovSignalLock) { ovSignalSeq++; ovSignalWhat = what; Monitor.PulseAll(ovSignalLock); }
    }
    // "sıra olay": since < 0 yalnızca eşitler (olay yok); sıra since'ten farklıysa hemen döner (helper yeniden başladıysa
    // sıra sıfırdan başlar, widget eşitlenir); aynıysa bir olay ya da zaman aşımı beklenir (olay boş).
    public static string WaitOverviewSignal(int since, int timeoutMs)
    {
        lock (ovSignalLock)
        {
            if (since < 0) return ovSignalSeq + " ";
            var sw = Stopwatch.StartNew();
            while (ovSignalSeq == since)
            {
                int left = timeoutMs - (int)sw.ElapsedMilliseconds;
                if (left <= 0) break;
                Monitor.Wait(ovSignalLock, left);
            }
            return ovSignalSeq + " " + (ovSignalSeq == since ? "" : ovSignalWhat);
        }
    }
    // Overview açılmadan önce öndeki pencere (kapanınca odak ona döner)
    static volatile IntPtr overviewPrev;

    public static void HideOverview(IntPtr h)
    {
        Native.ShowWindow(h, 0);
        // Gizlenen overview ön plan penceresi olarak kalıyordu: sayfa kapanırken odak ona geri dönüyor, sayfa bunu "açıldım"
        // sanıp kendini yeniden açıyordu (Super ile kapatınca kapanıp geri açılma; 10 denemede 7). Odak hemen, sayfa
        // tepki vermeden önceki pencereye verilir; o yoksa odak bekçisi (boş workspace: odak penceresi) karar verir.
        IntPtr prev = overviewPrev;
        IntPtr fg = Native.GetForegroundWindow();
        if ((fg == h || fg == IntPtr.Zero) && Usable(prev, h)) FocusSink.Give(prev);
        OverviewSignal("hide");
        FocusGuard.Kick(); // önceki pencere yoksa / verilemediyse
    }

    static bool Usable(IntPtr w, IntPtr ov)
    {
        if (w == IntPtr.Zero || w == ov || !Native.IsWindow(w) || !Native.IsWindowVisible(w) || Native.IsIconic(w) || ShellLike(w)) return false;
        int cl;
        return !(Native.DwmGetWindowAttribute(w, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0); // başka workspace'te değil
    }

    public static void ShowOverviewInMode(IntPtr h, string mode)
    {
        IntPtr prevFg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
        if (prevFg != h) overviewPrev = prevFg;
        int gen = Interlocked.Increment(ref overviewGen);
        if (prevFg != h && !ShellLike(prevFg)) ThreadPool.QueueUserWorkItem(_ => RestoreFocusAfterOverview(h, prevFg, gen));
        string d = Paths.StateDir;
        string flag = System.IO.Path.Combine(d, "overview-mode.txt");
        try { System.IO.Directory.CreateDirectory(d); System.IO.File.WriteAllText(flag, mode); } catch { }
        int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        Native.SetWindowLong(h, Native.GWL_EXSTYLE, ex | 0x00080000); // WS_EX_LAYERED
        Native.SetLayeredWindowAttributes(h, 0, 0, 0x2);              // tamamen saydam
        Native.ShowWindow(h, 5);
        OverviewSignal("show");
        Native.keybd_event(VK_DUMMY, 0, 0, Native.LL_MARK); Native.keybd_event(VK_DUMMY, 0, 2, Native.LL_MARK);
        Native.SetForegroundWindow(h);
        // Kendi thread'inde: havuz uzun yoklamalarla doluyken iş yarım saniye bekleyebiliyordu (menü saydam kalıyordu)
        new Thread(() =>
        {
            var sw = Stopwatch.StartNew();
            while (sw.ElapsedMilliseconds < 800 && System.IO.File.Exists(flag)) Thread.Sleep(6);
            Thread.Sleep(70); // widget'ın çizimi tamamlaması
            Native.SetLayeredWindowAttributes(h, 0, 255, 0x2);
            int e2 = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
            Native.SetWindowLong(h, Native.GWL_EXSTYLE, e2 & ~0x00080000);
        }) { IsBackground = true, Name = "overview-reveal" }.Start();
    }

    static string WinDesc(IntPtr w)
    {
        if (w == IntPtr.Zero) return "yok";
        uint pid; Native.GetWindowThreadProcessId(w, out pid);
        var t = new StringBuilder(64); Native.GetWindowText(w, t, 64);
        return ProcInfo.Name(pid) + " '" + t + "'";
    }

    // Super+V: overview'u pano modunda (";" öneki) aç; açıkken tekrar basınca kapat
    static void ToggleClipboard() { ToggleInMode(";"); }

    // Super+S: Super menüsü dosya aramasıyla açılır (# öneki); öneki bilmeyen de dosya aramasını bulsun
    static void ToggleFileSearch() { ToggleInMode("#"); }

    // Menü bu kısayolla açıkken aynı kısayol kapatır; değilse menü o modda açılır
    static void ToggleInMode(string mode)
    {
        IntPtr h = Native.FindWindow(null, "lunge-overview");
        if (h == IntPtr.Zero) return;
        if (Native.IsWindowVisible(h) && Native.GetForegroundWindow() == h) { HideOverview(h); return; }
        ShowOverviewInMode(h, mode);
    }

    static void ToggleOverview()
    {
        IntPtr h = Native.FindWindow(null, "lunge-overview");
        if (h == IntPtr.Zero) return;
        bool vis = Native.IsWindowVisible(h);
        IntPtr fg = Native.GetForegroundWindow();
        Slider.Log("super: overview görünür=" + (vis ? 1 : 0) + " önde=" + (fg == h ? 1 : 0) + " -> " + (vis && fg == h ? "kapat" : "aç") + (vis && fg != h ? " (ön plan: " + WinDesc(fg) + ")" : ""));
        if (vis && fg == h) { HideOverview(h); return; }
        // Mod bayrağı: "s" = düz arama (pano modunun bayrağı ";"); widget taze açılmış gibi davransın
        ShowOverviewInMode(h, "s");
    }
}

// ---------------- Dokunmatik yüzey hareketleri (Hyprland gestures) ----------------
// Hassas dokunmatik yüzeyin (Windows Precision Touchpad) ham HID raporları okunur (Raw Input; çekirdek odakta olmasa
// da gelir):
//   - 3 parmak sağa / sola: workspace parmakla birlikte kayar (Hyprland workspace_swipe, bkz. Slider.SwipeBegin)
//   - 3 parmak yukarı: overview, aşağı: sağ panel
//   - 4 parmak: odaktaki pencereyi o yöne taşır (Super+Shift+ok)
// Windows'un kendi 3/4 parmak hareketleri kurulumda kapatılır (ikisi birden çalışmasın). Dokunmatik yüzey yoksa ya da
// raporları çözülemezse hiçbir şey yapmaz (nedeni log'a yazılır). Her şey UI thread'inde: girdi mesajı da orada gelir.
static class Touchpad
{
    [StructLayout(LayoutKind.Sequential)] struct RAWINPUTDEVICE { public ushort UsagePage, Usage; public uint Flags; public IntPtr Target; }
    [StructLayout(LayoutKind.Sequential)] struct RAWINPUTHEADER { public uint Type, Size; public IntPtr Device, WParam; }
    [StructLayout(LayoutKind.Sequential)] struct RAWINPUTDEVICELIST { public IntPtr Device; public uint Type; }
    [StructLayout(LayoutKind.Sequential)]
    struct HIDP_CAPS
    {
        public ushort Usage, UsagePage, InputReportByteLength, OutputReportByteLength, FeatureReportByteLength;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = 17)] public ushort[] Reserved;
        public ushort NumberLinkCollectionNodes, NumberInputButtonCaps, NumberInputValueCaps, NumberInputDataIndices,
            NumberOutputButtonCaps, NumberOutputValueCaps, NumberOutputDataIndices, NumberFeatureButtonCaps,
            NumberFeatureValueCaps, NumberFeatureDataIndices;
    }
    // HIDP_VALUE_CAPS ve HIDP_BUTTON_CAPS: 72 bayt; yalnızca kullanılan alanlar (NotRange.Usage / Range.UsageMin 56'da)
    [StructLayout(LayoutKind.Explicit, Size = 72)]
    struct HIDP_CAPS72
    {
        [FieldOffset(0)] public ushort UsagePage;
        [FieldOffset(6)] public ushort LinkCollection;
        [FieldOffset(12)] public byte IsRange;
        [FieldOffset(32)] public uint UnitsExp;
        [FieldOffset(36)] public uint Units;
        [FieldOffset(40)] public int LogicalMin;
        [FieldOffset(44)] public int LogicalMax;
        [FieldOffset(48)] public int PhysicalMin;
        [FieldOffset(52)] public int PhysicalMax;
        [FieldOffset(56)] public ushort Usage;
        [FieldOffset(58)] public ushort UsageMax;
    }
    [DllImport("user32.dll", SetLastError = true)] static extern bool RegisterRawInputDevices(RAWINPUTDEVICE[] devices, uint count, uint size);
    [DllImport("user32.dll")] static extern uint GetRawInputData(IntPtr raw, uint command, IntPtr data, ref uint size, uint headerSize);
    [DllImport("user32.dll")] static extern uint GetRawInputDeviceInfo(IntPtr device, uint command, IntPtr data, ref uint size);
    [DllImport("user32.dll")] static extern uint GetRawInputDeviceList([Out] RAWINPUTDEVICELIST[] list, ref uint count, uint size);
    [DllImport("hid.dll")] static extern int HidP_GetCaps(IntPtr preparsed, out HIDP_CAPS caps);
    [DllImport("hid.dll")] static extern int HidP_GetValueCaps(int reportType, [Out] HIDP_CAPS72[] caps, ref ushort length, IntPtr preparsed);
    [DllImport("hid.dll")] static extern int HidP_GetButtonCaps(int reportType, [Out] HIDP_CAPS72[] caps, ref ushort length, IntPtr preparsed);
    [DllImport("hid.dll")] static extern int HidP_GetUsageValue(int reportType, ushort usagePage, ushort link, ushort usage, out uint value, IntPtr preparsed, byte[] report, uint length);
    [DllImport("hid.dll")] static extern int HidP_GetUsages(int reportType, ushort usagePage, ushort link, [Out] ushort[] usages, ref uint length, IntPtr preparsed, byte[] report, uint reportLength);
    const int HIDP_OK = 0x00110000, WM_INPUT = 0x00FF;
    const uint RIDEV_INPUTSINK = 0x100, RID_INPUT = 0x10000003, RIDI_PREPARSEDDATA = 0x20000005, RIDI_DEVICENAME = 0x20000007, RIDI_DEVICEINFO = 0x2000000b, RIM_TYPEHID = 2;
    const ushort PAGE_DIGITIZER = 0x0D, PAGE_DESKTOP = 0x01, USAGE_TOUCHPAD = 0x05, USAGE_TIP = 0x42, USAGE_COUNT = 0x54, USAGE_X = 0x30, USAGE_Y = 0x31;

    // Bir dokunmatik yüzey: parmak başına bir "link collection" (X, Y, değme anahtarı), kare başında parmak sayısı
    sealed class Pad
    {
        public IntPtr Preparsed;
        public readonly List<ushort> Links = new List<ushort>();
        public bool HasCount, HasTip, Ok;
        public ushort CountLink;
        public int XMin, YMin;
        public double MmPerX, MmPerY;
        // Hibrit raporlama: parmaklar birkaç rapora bölünebilir; sayı yalnızca karenin ilk raporunda gelir
        public int Expected, Slots;
        public double SumX, SumY;
        public int Touching;
    }
    static readonly Dictionary<IntPtr, Pad> pads = new Dictionary<IntPtr, Pad>();

    // Uykudan dönüş, yeniden bağlanma ya da dock değişimi cihaza yeni bir tutamaç verir: eskisinin kaydı ve HID tanımı
    // (AllocHGlobal) gün boyu birikiyordu. Yeni bir cihaz gelince Windows'un artık tanımadığı tutamaçlar bırakılır.
    static void ForgetGonePads()
    {
        foreach (var h in new List<IntPtr>(pads.Keys))
        {
            uint size = 0;
            if (GetRawInputDeviceInfo(h, RIDI_DEVICENAME, IntPtr.Zero, ref size) != unchecked((uint)-1)) continue;
            var gone = pads[h];
            if (gone.Preparsed != IntPtr.Zero) Marshal.FreeHGlobal(gone.Preparsed);
            pads.Remove(h);
        }
    }
    static readonly ushort[] usageBuf = new ushort[64];

    sealed class Sink : NativeWindow
    {
        protected override void WndProc(ref Message m)
        {
            if (m.Msg == WM_INPUT) { try { OnInput(m.LParam); } catch (Exception ex) { Fail(ex); } }
            base.WndProc(ref m);
        }
    }
    static Sink sink;
    static Slider slider;
    static System.Windows.Forms.Timer lift;
    static int lastReport;
    static int failures;

    static void Fail(Exception ex)
    {
        if (++failures <= 5) Slider.Log("dokunmatik yüzey: " + ex.GetBaseException().Message);
    }

    // Sistemde hassas dokunmatik yüzey var mı (ayarlar penceresi anahtarı yalnızca o zaman gösterir)
    public static bool Present()
    {
        try
        {
            uint n = 0, sz = (uint)Marshal.SizeOf(typeof(RAWINPUTDEVICELIST));
            GetRawInputDeviceList(null, ref n, sz);
            if (n == 0) return false;
            var list = new RAWINPUTDEVICELIST[n];
            if (GetRawInputDeviceList(list, ref n, sz) == unchecked((uint)-1)) return false;
            // RID_DEVICE_INFO: cbSize, dwType, sonra hid: dwVendorId, dwProductId, dwVersionNumber, usUsagePage, usUsage
            IntPtr info = Marshal.AllocHGlobal(32);
            try
            {
                foreach (var d in list)
                {
                    if (d.Type != RIM_TYPEHID) continue;
                    Marshal.WriteInt32(info, 0, 32);
                    uint size = 32;
                    if (GetRawInputDeviceInfo(d.Device, RIDI_DEVICEINFO, info, ref size) == unchecked((uint)-1)) continue;
                    if ((ushort)Marshal.ReadInt16(info, 20) == PAGE_DIGITIZER && (ushort)Marshal.ReadInt16(info, 22) == USAGE_TOUCHPAD) return true;
                }
            }
            finally { Marshal.FreeHGlobal(info); }
        }
        catch { }
        return false;
    }

    public static void Start(Slider s)
    {
        slider = s;
        try
        {
            sink = new Sink();
            sink.CreateHandle(new CreateParams { Parent = new IntPtr(-3) }); // HWND_MESSAGE: görünmez, yalnızca mesaj alır
            var dev = new[] { new RAWINPUTDEVICE { UsagePage = PAGE_DIGITIZER, Usage = USAGE_TOUCHPAD, Flags = RIDEV_INPUTSINK, Target = sink.Handle } };
            if (!RegisterRawInputDevices(dev, 1, (uint)Marshal.SizeOf(typeof(RAWINPUTDEVICE))))
            {
                Slider.Log("dokunmatik yüzey: kayıt olmadı (" + Marshal.GetLastWin32Error() + ")");
                return;
            }
            // Parmaklar kalkarken son rapor gelmezse (cihaz sessizce keser) hareket takılı kalmasın: 180 ms rapor yoksa kalktı
            lift = new System.Windows.Forms.Timer { Interval = 90 };
            lift.Tick += (o, e) =>
            {
                if (unchecked(Environment.TickCount - lastReport) < 180) return;
                lift.Stop();
                Frame(0, 0, 0, Environment.TickCount);
            };
            Slider.Log("dokunmatik yüzey: " + (Present() ? "bulundu, hareketler hazır" : "yok (takılırsa hareketler çalışır)"));
        }
        catch (Exception ex) { Slider.Log("dokunmatik yüzey: " + ex.Message); }
    }

    static void OnInput(IntPtr raw)
    {
        uint size = 0, hs = (uint)Marshal.SizeOf(typeof(RAWINPUTHEADER));
        GetRawInputData(raw, RID_INPUT, IntPtr.Zero, ref size, hs);
        if (size == 0 || size > 65536) return;
        IntPtr buf = Marshal.AllocHGlobal((int)size);
        try
        {
            if (GetRawInputData(raw, RID_INPUT, buf, ref size, hs) != size) return;
            var head = (RAWINPUTHEADER)Marshal.PtrToStructure(buf, typeof(RAWINPUTHEADER));
            if (head.Type != RIM_TYPEHID) return;
            var pad = PadFor(head.Device);
            if (pad == null || !pad.Ok) return;
            int sizeHid = Marshal.ReadInt32(buf, (int)hs), count = Marshal.ReadInt32(buf, (int)hs + 4);
            if (sizeHid <= 0 || count <= 0 || hs + 8 + (long)sizeHid * count > size) return;
            var report = new byte[sizeHid];
            for (int i = 0; i < count; i++)
            {
                Marshal.Copy(IntPtr.Add(buf, (int)hs + 8 + i * sizeHid), report, 0, sizeHid);
                Report(pad, report);
            }
        }
        finally { Marshal.FreeHGlobal(buf); }
    }

    static Pad PadFor(IntPtr device)
    {
        Pad pad;
        if (pads.TryGetValue(device, out pad)) return pad;
        ForgetGonePads();
        pad = new Pad();
        pads[device] = pad;
        uint size = 0;
        GetRawInputDeviceInfo(device, RIDI_PREPARSEDDATA, IntPtr.Zero, ref size);
        if (size == 0 || size > 1 << 20) { Slider.Log("dokunmatik yüzey: HID tanımı okunamadı"); return pad; }
        pad.Preparsed = Marshal.AllocHGlobal((int)size); // cihaz yaşadıkça kullanılır
        if (GetRawInputDeviceInfo(device, RIDI_PREPARSEDDATA, pad.Preparsed, ref size) == unchecked((uint)-1)) { Slider.Log("dokunmatik yüzey: HID tanımı okunamadı"); return pad; }
        HIDP_CAPS caps;
        if (HidP_GetCaps(pad.Preparsed, out caps) != HIDP_OK) { Slider.Log("dokunmatik yüzey: HidP_GetCaps"); return pad; }

        ushort nv = caps.NumberInputValueCaps;
        var vc = new HIDP_CAPS72[nv];
        if (nv == 0 || HidP_GetValueCaps(0, vc, ref nv, pad.Preparsed) != HIDP_OK) { Slider.Log("dokunmatik yüzey: değer tanımları yok"); return pad; }
        var xs = new Dictionary<ushort, HIDP_CAPS72>();
        var ys = new Dictionary<ushort, HIDP_CAPS72>();
        for (int i = 0; i < nv; i++)
        {
            var c = vc[i];
            if (c.IsRange != 0) continue;
            if (c.UsagePage == PAGE_DIGITIZER && c.Usage == USAGE_COUNT) { pad.HasCount = true; pad.CountLink = c.LinkCollection; }
            else if (c.UsagePage == PAGE_DESKTOP && c.Usage == USAGE_X) xs[c.LinkCollection] = c;
            else if (c.UsagePage == PAGE_DESKTOP && c.Usage == USAGE_Y) ys[c.LinkCollection] = c;
        }
        foreach (var l in xs.Keys) if (ys.ContainsKey(l)) pad.Links.Add(l);
        pad.Links.Sort();

        ushort nb = caps.NumberInputButtonCaps;
        if (nb > 0)
        {
            var bc = new HIDP_CAPS72[nb];
            if (HidP_GetButtonCaps(0, bc, ref nb, pad.Preparsed) == HIDP_OK)
                for (int i = 0; i < nb; i++)
                    if (bc[i].UsagePage == PAGE_DIGITIZER && (bc[i].IsRange != 0 ? bc[i].Usage <= USAGE_TIP && USAGE_TIP <= bc[i].UsageMax : bc[i].Usage == USAGE_TIP))
                        pad.HasTip = true;
        }

        if (pad.Links.Count > 0)
        {
            var x = xs[pad.Links[0]]; var y = ys[pad.Links[0]];
            pad.XMin = x.LogicalMin; pad.YMin = y.LogicalMin;
            double mx = Mm(x), my = Mm(y);
            // Birim bilinmiyorsa yüzey ~100 mm genişlikte sayılır; Y, X ile aynı ölçekte
            pad.MmPerX = mx > 0 ? mx : (x.LogicalMax > x.LogicalMin ? 100.0 / (x.LogicalMax - x.LogicalMin) : 0);
            pad.MmPerY = my > 0 ? my : pad.MmPerX;
        }
        pad.Ok = pad.Links.Count > 0 && pad.MmPerX > 0;
        Slider.Log("dokunmatik yüzey: " + pad.Links.Count + " parmak yuvası" + (pad.HasCount ? "" : ", parmak sayısı alanı yok")
            + (pad.HasTip ? "" : ", değme anahtarı yok") + ", " + (pad.MmPerX * 1000).ToString("0.0") + " µm/birim"
            + (pad.Ok ? "" : " — çözülemedi, hareketler kapalı"));
        return pad;
    }

    // HID fiziksel birimi -> birim başına mm (cm ya da inç, üs ile)
    static double Mm(HIDP_CAPS72 c)
    {
        int logical = c.LogicalMax - c.LogicalMin, phys = c.PhysicalMax - c.PhysicalMin;
        if (logical <= 0 || phys <= 0) return 0;
        uint system = c.Units & 0xF, length = (c.Units >> 4) & 0xF;
        int exp = (int)(c.UnitsExp & 0xF);
        if (exp > 7) exp -= 16;
        double unitMm = system == 1 ? 10 : system == 3 ? 25.4 : 0;
        if (unitMm == 0 || length != 1) return 0;
        return phys * Math.Pow(10, exp) * unitMm / logical;
    }

    static void Report(Pad pad, byte[] r)
    {
        uint n = 0;
        if (pad.HasCount && HidP_GetUsageValue(0, PAGE_DIGITIZER, pad.CountLink, USAGE_COUNT, out n, pad.Preparsed, r, (uint)r.Length) == HIDP_OK && n > 0)
        {
            pad.Expected = (int)Math.Min(n, 10); pad.Slots = 0; pad.Touching = 0; pad.SumX = pad.SumY = 0;
        }
        else if (!pad.HasCount)
        {
            pad.Expected = pad.Links.Count; pad.Slots = 0; pad.Touching = 0; pad.SumX = pad.SumY = 0;
        }
        if (pad.Expected <= 0) return;
        foreach (var link in pad.Links)
        {
            if (pad.Slots >= pad.Expected) break;
            uint x, y;
            if (HidP_GetUsageValue(0, PAGE_DESKTOP, link, USAGE_X, out x, pad.Preparsed, r, (uint)r.Length) != HIDP_OK) continue;
            if (HidP_GetUsageValue(0, PAGE_DESKTOP, link, USAGE_Y, out y, pad.Preparsed, r, (uint)r.Length) != HIDP_OK) continue;
            bool tip = !pad.HasTip;
            if (pad.HasTip)
            {
                uint len = (uint)usageBuf.Length;
                if (HidP_GetUsages(0, PAGE_DIGITIZER, link, usageBuf, ref len, pad.Preparsed, r, (uint)r.Length) == HIDP_OK)
                    for (int i = 0; i < len; i++) if (usageBuf[i] == USAGE_TIP) { tip = true; break; }
            }
            pad.Slots++;
            if (!tip) continue;
            pad.Touching++;
            pad.SumX += ((int)x - pad.XMin) * pad.MmPerX;
            pad.SumY += ((int)y - pad.YMin) * pad.MmPerY;
        }
        if (pad.Slots < pad.Expected) return; // karenin geri kalanı sonraki raporda
        int t = pad.Touching;
        Frame(t, t > 0 ? pad.SumX / t : 0, t > 0 ? pad.SumY / t : 0, Environment.TickCount);
        pad.Expected = 0;
    }

    // ---- Hareket tanıma (parmak sayısı ve ağırlık merkezi, mm) ----
    enum St { Idle, Pending, Swipe, Discrete, Done }
    static St st;
    static int fingers;
    static bool horizontal;
    static double x0, y0, lastX, lastY;
    static readonly List<KeyValuePair<long, double>> trail = new List<KeyValuePair<long, double>>();
    const double LOCK_MM = 4, FIRE_MM = 12;
    // Bir workspace boyu kaydırma için parmak yolu (dokunmatik yüzeyin yarısından biraz fazlası)
    const double SWIPE_MM = 60;

    public static void Frame(int n, double cx, double cy, long t)
    {
        // Zamanlayıcı parmaklar değdiği sürece çalışır; her raporda durdurup başlatmak saniyede ~100 kez gizli pencere
        // açıp kapatıyordu
        if (lift != null)
        {
            if (n > 0) { lastReport = Environment.TickCount; if (!lift.Enabled) lift.Start(); }
            else if (lift.Enabled) lift.Stop();
        }
        if (n > 0) { lastX = cx; lastY = cy; }
        if (!Prefs.Gestures && st != St.Swipe) { st = n == 0 ? St.Idle : St.Done; return; }
        switch (st)
        {
            case St.Idle:
                if (n >= 3) { st = St.Pending; fingers = n; x0 = cx; y0 = cy; }
                return;
            case St.Pending:
                if (n < 3) { st = n == 0 ? St.Idle : St.Done; return; }
                // Parmaklar peş peşe değdi (3 -> 4): hareket yeni parmak sayısıyla baştan
                if (n != fingers) { fingers = n; x0 = cx; y0 = cy; return; }
                double dx = cx - x0, dy = cy - y0;
                if (Math.Abs(dx) < LOCK_MM && Math.Abs(dy) < LOCK_MM) return;
                horizontal = Math.Abs(dx) >= Math.Abs(dy);
                // Animasyonlar kapalıysa kaydırma parmağı izlemez: yeterince gidince workspace doğrudan değişir
                if (fingers == 3 && horizontal && slider != null && slider.SwipeBegin())
                {
                    st = St.Swipe;
                    trail.Clear();
                    Swipe(cx, t);
                }
                else st = fingers <= 4 ? St.Discrete : St.Done; // 5 parmak: hareket yok
                return;
            case St.Discrete:
                if (n < fingers) { st = n == 0 ? St.Idle : St.Done; return; }
                double d = horizontal ? cx - x0 : cy - y0;
                if (Math.Abs(d) >= FIRE_MM) { Fire(fingers, horizontal, d); st = St.Done; }
                return;
            case St.Swipe:
                if (n < 3) { EndSwipe(); st = n == 0 ? St.Idle : St.Done; return; }
                Swipe(cx, t);
                return;
            default: // Done: hepsi kalkana kadar yeni hareket yok
                if (n == 0) st = St.Idle;
                return;
        }
    }

    // Parmaklar sola -> içerik sola -> sağdaki (sonraki) workspace (dokunmatik ekrandaki gibi içerik parmağı izler)
    static void Swipe(double cx, long t)
    {
        trail.Add(new KeyValuePair<long, double>(t, cx));
        while (trail.Count > 2 && t - trail[0].Key > 80) trail.RemoveAt(0);
        slider.SwipeUpdate(-(cx - x0) / SWIPE_MM);
    }

    static void EndSwipe()
    {
        double v = 0; // ilerleme / ms
        if (trail.Count >= 2)
        {
            var a = trail[0]; var b = trail[trail.Count - 1];
            long dt = b.Key - a.Key;
            if (dt > 0) v = -(b.Value - a.Value) / SWIPE_MM / dt;
        }
        trail.Clear();
        slider.SwipeEnd(v);
    }

    static void Fire(int n, bool isHorizontal, double d)
    {
        var k = Keys2.Instance;
        if (k == null) return;
        string act = n == 3
            ? (isHorizontal ? (d < 0 ? "ws-next" : "ws-prev") : (d < 0 ? "overview" : "sidebar"))
            : "move-" + (isHorizontal ? (d < 0 ? "left" : "right") : (d < 0 ? "up" : "down"));
        Slider.Log("parmak hareketi: " + n + " parmak -> " + act);
        k.Dispatch(act);
    }

    // Test: gerçek dokunmatik yüzey olmadan aynı yoldan yapay hareket (kareler 8 ms arayla UI thread'ine)
    //   /cmd?a=gesture&f=3&dx=-45&dy=0&ms=260   (mm)
    public static void Simulate(Control ui, int f, double dx, double dy, int ms)
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            const double X = 50, Y = 35;
            Action<int, double, double> frame = (n, x, y) => { try { ui.Invoke((Action)(() => Frame(n, x, y, Environment.TickCount))); } catch { } };
            for (int i = 0; i < 3; i++) { frame(f, X, Y); Thread.Sleep(8); }
            int steps = Math.Max(2, ms / 8);
            for (int i = 1; i <= steps; i++) { frame(f, X + dx * i / steps, Y + dy * i / steps); Thread.Sleep(8); }
            frame(0, X + dx, Y + dy);
        });
    }
}

// ---------------- Pencere simgesi (bar / overview) ----------------
// Uygulama listesinde (apps.json) karşılığı olmayan pencere (Git Bash, oyun istemcileri, kurulumlar) bar'da noktayla
// kalıyordu: simge pencerenin kendisinden (WM_GETICON, sınıf simgesi), yoksa exe'sinden alınır ve PNG veri adresi
// olarak verilir. Exe başına önbellekte; askıdaki pencerede 150 ms'den fazla beklenmez.
static class WinIcons
{
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "GetClassLongPtrW")] static extern IntPtr GetClassLongPtr(IntPtr h, int index);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool QueryFullProcessImageName(IntPtr p, uint flags, StringBuilder name, ref uint size);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    static readonly Dictionary<string, string> cache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);

    public static string For(IntPtr h)
    {
        if (!Native.IsWindow(h)) return null;
        uint pid; Native.GetWindowThreadProcessId(h, out pid);
        string exe = ExePath(pid);
        // Uygulamanın kimliği (AppUserModelID) exe'den önce: bütün Store uygulamaları ApplicationFrameHost.exe'de çalışır
        // ve exe'ye göre önbellekte ilk hangisi alındıysa (Hesap Makinesi) sonrakiler (Roblox) onun simgesiyle kalıyordu;
        // Steam'in pencereleri, web uygulamaları da exe paylaşır. Simge Başlat menüsünün kullandığı yerden gelir.
        string app = AppId(h);
        string key = app != null ? "app:" + app : exe ?? ("pid:" + pid);
        lock (cache) { string c; if (cache.TryGetValue(key, out c)) return c; }
        string data = app != null ? AppIcon(app) : null;
        if (data == null) try
        {
            IntPtr hi = IntPtr.Zero, r;
            if (SendMessageTimeout(h, 0x7F /*WM_GETICON*/, (IntPtr)1 /*ICON_BIG*/, IntPtr.Zero, 0x2 /*SMTO_ABORTIFHUNG*/, 150, out r) != IntPtr.Zero) hi = r;
            if (hi == IntPtr.Zero && SendMessageTimeout(h, 0x7F, (IntPtr)2 /*ICON_SMALL2*/, IntPtr.Zero, 0x2, 150, out r) != IntPtr.Zero) hi = r;
            if (hi == IntPtr.Zero) hi = GetClassLongPtr(h, -14 /*GCLP_HICON*/);
            // FromHandle tanıtıcıyı sahiplenmez: pencerenin simgesi yok edilmez
            if (hi != IntPtr.Zero) { using (var ic = Icon.FromHandle(hi)) data = Png(ic); }
            else if (exe != null) { using (var ic = Icon.ExtractAssociatedIcon(exe)) if (ic != null) data = Png(ic); }
        }
        catch { }
        // Boş sonuç önbelleğe girmez: pencere simgesini açıldıktan biraz sonra koyabiliyor
        if (data != null) lock (cache) { if (cache.Count > 300) cache.Clear(); cache[key] = data; }
        return data;
    }

    static string Png(Icon ic)
    {
        using (var bmp = ic.ToBitmap())
        using (var ms = new System.IO.MemoryStream())
        {
            bmp.Save(ms, System.Drawing.Imaging.ImageFormat.Png);
            return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray());
        }
    }

    [StructLayout(LayoutKind.Sequential)] struct PROPERTYKEY { public Guid fmtid; public uint pid; }
    [StructLayout(LayoutKind.Sequential)] sealed class PROPVARIANT { public ushort vt; public ushort r1, r2, r3; public IntPtr p; public IntPtr p2; }
    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPropertyStore
    {
        [PreserveSig] int GetCount(out uint count);
        [PreserveSig] int GetAt(uint index, out PROPERTYKEY key);
        [PreserveSig] int GetValue(ref PROPERTYKEY key, [Out] PROPVARIANT value);
        [PreserveSig] int SetValue(ref PROPERTYKEY key, [In] PROPVARIANT value);
        [PreserveSig] int Commit();
    }
    [ComImport, Guid("BCC18B79-BA16-442F-80C4-8A59C30C463B"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellItemImageFactory { [PreserveSig] int GetImage(Size size, int flags, out IntPtr bitmap); }
    [DllImport("shell32.dll")] static extern int SHGetPropertyStoreForWindow(IntPtr h, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IPropertyStore store);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)] static extern int SHCreateItemFromParsingName(string path, IntPtr ctx, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);
    [DllImport("ole32.dll")] static extern int PropVariantClear(PROPVARIANT v);
    [DllImport("gdi32.dll")] static extern int GetDIBits(IntPtr dc, IntPtr bmp, uint start, uint lines, byte[] bits, ref BITMAPINFOHEADER info, uint usage);
    [DllImport("gdi32.dll")] static extern int GetObject(IntPtr obj, int size, ref BITMAP bmp);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [StructLayout(LayoutKind.Sequential)] struct BITMAP { public int type, width, height, widthBytes; public ushort planes, bitsPixel; public IntPtr bits; }
    [StructLayout(LayoutKind.Sequential)] struct BITMAPINFOHEADER { public uint size; public int width, height; public ushort planes, bitCount; public uint compression, sizeImage; public int xppm, yppm; public uint clrUsed, clrImportant; }
    static readonly Guid PropertyStoreIid = new Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99");
    static readonly Guid ImageFactoryIid = new Guid("BCC18B79-BA16-442F-80C4-8A59C30C463B");
    static PROPERTYKEY AppIdKey() { return new PROPERTYKEY { fmtid = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), pid = 5 }; }

    // Pencerenin uygulama kimliği; yoksa null
    internal static string AppId(IntPtr h)
    {
        IPropertyStore store = null;
        var v = new PROPVARIANT();
        try
        {
            var iid = PropertyStoreIid;
            if (SHGetPropertyStoreForWindow(h, ref iid, out store) != 0 || store == null) return null;
            var key = AppIdKey();
            if (store.GetValue(ref key, v) != 0 || v.vt != 31 /*VT_LPWSTR*/ || v.p == IntPtr.Zero) return null;
            string id = Marshal.PtrToStringUni(v.p);
            return string.IsNullOrWhiteSpace(id) ? null : id;
        }
        catch { return null; }
        finally { PropVariantClear(v); if (store != null) Marshal.ReleaseComObject(store); }
    }

    // Başlat menüsünün simgesi (shell:AppsFolder), saydamlığıyla PNG; yoksa null
    internal static string AppIcon(string app)
    {
        IShellItemImageFactory item = null;
        IntPtr hbmp = IntPtr.Zero;
        try
        {
            var iid = ImageFactoryIid;
            if (SHCreateItemFromParsingName(@"shell:AppsFolder\" + app, IntPtr.Zero, ref iid, out item) != 0 || item == null) return null;
            if (item.GetImage(new Size(64, 64), 0x4 /*SIIGBF_ICONONLY*/, out hbmp) != 0 || hbmp == IntPtr.Zero) return null;
            var bm = new BITMAP();
            if (GetObject(hbmp, Marshal.SizeOf(typeof(BITMAP)), ref bm) == 0 || bm.width <= 0 || bm.height <= 0) return null;
            // 32 bit, yukarıdan aşağıya: kanalı ön-çarpılmış ARGB (kabuk resimleri öyle)
            var info = new BITMAPINFOHEADER { size = (uint)Marshal.SizeOf(typeof(BITMAPINFOHEADER)), width = bm.width, height = -bm.height, planes = 1, bitCount = 32 };
            var bits = new byte[bm.width * bm.height * 4];
            IntPtr dc = GetDC(IntPtr.Zero);
            try { if (GetDIBits(dc, hbmp, 0, (uint)bm.height, bits, ref info, 0) == 0) return null; }
            finally { ReleaseDC(IntPtr.Zero, dc); }
            bool any = false;
            for (int i = 3; i < bits.Length; i += 4) if (bits[i] != 0) { any = true; break; }
            if (!any) return null;
            using (var bmp = new Bitmap(bm.width, bm.height, System.Drawing.Imaging.PixelFormat.Format32bppPArgb))
            {
                var lk = bmp.LockBits(new Rectangle(0, 0, bm.width, bm.height), System.Drawing.Imaging.ImageLockMode.WriteOnly, System.Drawing.Imaging.PixelFormat.Format32bppPArgb);
                for (int y = 0; y < bm.height; y++) Marshal.Copy(bits, y * bm.width * 4, lk.Scan0 + y * lk.Stride, bm.width * 4);
                bmp.UnlockBits(lk);
                using (var ms = new System.IO.MemoryStream())
                {
                    bmp.Save(ms, System.Drawing.Imaging.ImageFormat.Png);
                    return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray());
                }
            }
        }
        catch { return null; }
        finally { if (hbmp != IntPtr.Zero) Native.DeleteObject(hbmp); if (item != null) Marshal.ReleaseComObject(item); }
    }

    // Yönetici haklarıyla çalışan süreçte de çalışır (PROCESS_QUERY_LIMITED_INFORMATION)
    static string ExePath(uint pid)
    {
        IntPtr p = OpenProcess(0x1000, false, pid);
        if (p == IntPtr.Zero) return null;
        try
        {
            var sb = new StringBuilder(1024);
            uint n = (uint)sb.Capacity;
            return QueryFullProcessImageName(p, 0, sb, ref n) ? sb.ToString() : null;
        }
        finally { CloseHandle(p); }
    }
}

// ---------------- Mikrofon (Windows Core Audio) ----------------
// shell'in setMute'u yalnızca varsayılan kayıt cihazını susturuyordu; Discord gibi uygulamalar
// "iletişim" cihazını ya da başka bir mikrofonu kullanınca ses gitmeye devam ediyordu.
// Burada TÜM etkin kayıt cihazları birlikte susturulur/açılır.
static class Mic
{
    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumerator { }
    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceEnumerator
    {
        int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
    }
    [ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceCollection
    {
        int GetCount(out int count);
        int Item(int index, out IMMDevice device);
    }
    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDevice
    {
        int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
    }
    [ComImport, Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioEndpointVolume
    {
        int RegisterControlChangeNotify(IntPtr n); int UnregisterControlChangeNotify(IntPtr n);
        int GetChannelCount(out int c);
        int SetMasterVolumeLevel(float l, ref Guid ctx); int SetMasterVolumeLevelScalar(float l, ref Guid ctx);
        int GetMasterVolumeLevel(out float l); int GetMasterVolumeLevelScalar(out float l);
        int SetChannelVolumeLevel(int ch, float l, ref Guid ctx); int SetChannelVolumeLevelScalar(int ch, float l, ref Guid ctx);
        int GetChannelVolumeLevel(int ch, out float l); int GetChannelVolumeLevelScalar(int ch, out float l);
        int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, ref Guid ctx);
        int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
    }

    static IAudioEndpointVolume Vol(IMMDevice d)
    {
        var iid = typeof(IAudioEndpointVolume).GUID; object o;
        d.Activate(ref iid, 23 /*CLSCTX_ALL*/, IntPtr.Zero, out o);
        return (IAudioEndpointVolume)o;
    }

    public static bool IsMuted()
    {
        var en = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDevice d;
        if (en.GetDefaultAudioEndpoint(1 /*eCapture*/, 0, out d) != 0 || d == null) return true;
        bool m; Vol(d).GetMute(out m); return m;
    }

    public static void SetAll(bool mute)
    {
        var en = (IMMDeviceEnumerator)new MMDeviceEnumerator();
        IMMDeviceCollection col;
        en.EnumAudioEndpoints(1 /*eCapture*/, 1 /*ACTIVE*/, out col);
        int n; col.GetCount(out n);
        var ctx = Guid.Empty;
        for (int i = 0; i < n; i++)
        {
            IMMDevice d; col.Item(i, out d);
            try { Vol(d).SetMute(mute, ref ctx); } catch { }
        }
    }
}

// ---------------- Ekran klavyesi girişi ----------------
// lunge.exe --osk : stdin'den satır okur ("tap <vk>", "down <vk>", "up <vk>", "text <karakterler>")
// ve SendInput ile odaktaki pencereye yollar. shell'deki ii tarzı ekran klavyesi bunu kullanır.
static class Osk
{
    static Native.INPUT Key(ushort vk, ushort scan, uint flags)
    {
        var i = new Native.INPUT { type = 1 };
        i.ki = new Native.KEYBDINPUT { wVk = vk, wScan = scan, dwFlags = flags };
        return i;
    }

    public static void Run()
    {
        var sz = Marshal.SizeOf(typeof(Native.INPUT));
        string line;
        var input = new System.IO.StreamReader(Console.OpenStandardInput(), Encoding.UTF8);
        while ((line = input.ReadLine()) != null)
        {
            var parts = line.Split(new[] { ' ' }, 2);
            if (parts.Length < 2) continue;
            try
            {
                if (parts[0] == "text")
                {
                    var list = new List<Native.INPUT>();
                    foreach (char ch in parts[1])
                    {
                        list.Add(Key(0, ch, 0x4));        // KEYEVENTF_UNICODE
                        list.Add(Key(0, ch, 0x4 | 0x2));
                    }
                    Native.SendInput((uint)list.Count, list.ToArray(), sz);
                    continue;
                }
                ushort vk = ushort.Parse(parts[1]);
                uint ext = (vk >= 0x21 && vk <= 0x2E) || vk == 0x5B ? 0x1u : 0u; // ok tuşları, Home/End, Win
                if (parts[0] == "down" || parts[0] == "tap") Native.SendInput(1, new[] { Key(vk, 0, ext) }, sz);
                if (parts[0] == "up" || parts[0] == "tap") Native.SendInput(1, new[] { Key(vk, 0, ext | 0x2) }, sz);
            }
            catch { }
        }
    }
}

// ---------------- Gece ışığı (ii: hyprsunset, gama tabanlı) ----------------
// Windows'un kendi gece ışığı yerine ekranın gama eğrisini sıcak renge çeker; ii de böyle yapar.
// Durum %LOCALAPPDATA%\LogicalLunge\state\nightlight dosyasında; açıkken ana helper birkaç sn'de bir
// yeniden uygular (Windows mod değişiminde / uykudan dönüşte gamayı sıfırlayabiliyor).
static class NightLight
{
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateDC(string driver, string device, string output, IntPtr init);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern bool SetDeviceGammaRamp(IntPtr dc, ushort[] ramp);

    // Seviye %0..100 -> renk sıcaklığı 6500K..1900K (ii Intensity kaydırıcısı 6500 -> 1200K).
    // %35 civarı ii varsayılanı 5000K'ye denk gelir.
    static void Rgb(int level, out double r, out double g, out double b)
    {
        double k = (6500 - (6500 - 1900) * Math.Max(0, Math.Min(100, level)) / 100.0) / 100.0;
        // Tanner Helland yaklaşımı, 6500K = beyaz olacak şekilde normalize
        Func<double, double> clamp = v => Math.Max(0, Math.Min(255, v)) / 255.0;
        r = k <= 66 ? 1.0 : clamp(329.698727446 * Math.Pow(k - 60, -0.1332047592));
        g = k <= 66 ? clamp(99.4708025861 * Math.Log(k) - 161.1195681661) : clamp(288.1221695283 * Math.Pow(k - 60, -0.0755148492));
        b = k >= 66 ? 1.0 : k <= 19 ? 0 : clamp(138.5177312231 * Math.Log(k - 10) - 305.0447927307);
        double wr = 1.0, wg = clamp(99.4708025861 * Math.Log(65) - 161.1195681661), wb = clamp(138.5177312231 * Math.Log(55) - 305.0447927307);
        r = Math.Min(1, r / wr); g = Math.Min(1, g / wg); b = Math.Min(1, b / wb);
    }

    // Ayarlar: on=1, level=50, mode=manual|after|range, from=20:00, to=07:00
    //   manual: açıkken hep; after: "from" saatinden sabah 07:00'ye kadar; range: from–to aralığında
    public static Dictionary<string, string> Settings()
    {
        // Gösterim ve uygulama için hoşgörülü okuma: okunamazsa varsayılanlar (Set bunların üstüne yazmaz)
        try { return System.IO.File.Exists(StateFile) ? Parse(System.IO.File.ReadAllText(StateFile)) : Defaults(); }
        catch { return Defaults(); }
    }
    static Dictionary<string, string> Defaults()
    {
        return new Dictionary<string, string> { { "on", "0" }, { "level", "50" }, { "mode", "manual" }, { "from", "20:00" }, { "to", "07:00" } };
    }
    static Dictionary<string, string> Parse(string text)
    {
        var d = Defaults();
        foreach (var line in text.Split('\n'))
        {
            var t = line.Trim();
            if (t == "1" || t == "0") { d["on"] = t; continue; } // eski biçim
            int eq = t.IndexOf('=');
            if (eq > 0) d[t.Substring(0, eq)] = t.Substring(eq + 1);
        }
        return d;
    }
    // false: yazılmadı. Okunamayan durum dosyasının üstüne varsayılanlarla yazmak zamanlamayı ve yoğunluğu sıfırlardı.
    public static bool Set(string key, string value)
    {
        Dictionary<string, string> d;
        if (!SettingsFile.TryReadForUpdate(StateFile, Parse, Defaults, out d)) return false;
        d[key] = value;
        var lines = new List<string>(); foreach (var kv in d) lines.Add(kv.Key + "=" + kv.Value);
        // Yerinde yazma: ana çekirdeğin izleyicisi Changed/Created dinliyor; atomik değiştirme Renamed üretirdi
        try { System.IO.File.WriteAllLines(StateFile, lines.ToArray()); } catch { return false; }
        // Ana helper çalışıyorsa dosyadaki değişikliği görüp yumuşak geçişle uygular (burada anında uygulamak "bam" diye
        // değiştiriyordu); yoksa hemen uygula.
        if (!MainRunning()) Apply(Active);
        return true;
    }
    static bool MainRunning()
    {
        Mutex m;
        if (!Mutex.TryOpenExisting("LogicalLunge.Core", out m)) return false;
        m.Dispose();
        return true;
    }
    static int Minutes(string hhmm, int def)
    {
        var p = (hhmm ?? "").Split(':'); int h, m;
        return p.Length == 2 && int.TryParse(p[0], out h) && int.TryParse(p[1], out m) ? (h * 60 + m) % 1440 : def;
    }
    public static bool Active
    {
        get
        {
            var d = Settings();
            if (d["on"] != "1") return false;
            if (d["mode"] == "manual") return true;
            int now = DateTime.Now.Hour * 60 + DateTime.Now.Minute;
            int from = Minutes(d["from"], 1200), to = d["mode"] == "after" ? 7 * 60 : Minutes(d["to"], 420);
            return from <= to ? (now >= from && now < to) : (now >= from || now < to); // gece yarısını aşan aralık
        }
    }
    public static string StatusJson()
    {
        var d = Settings();
        return "{\"on\":" + (d["on"] == "1" ? "true" : "false") + ",\"active\":" + (Active ? "true" : "false") +
               ",\"level\":" + d["level"] + ",\"mode\":\"" + d["mode"] + "\",\"from\":\"" + d["from"] + "\",\"to\":\"" + d["to"] + "\"}";
    }

    static string StateFile
    {
        get
        {
            string dir = Paths.StateDir;
            System.IO.Directory.CreateDirectory(dir);
            return System.IO.Path.Combine(dir, "nightlight");
        }
    }

    public static bool Enabled
    {
        get { return Settings()["on"] == "1"; }
        set { Set("on", value ? "1" : "0"); }
    }

    // Hyprland'deki gibi parlaklık 0'ın altına inilince gama 100 -> 0 (yazılımsal karartma), ekran başına.
    // %LOCALAPPDATA%\LogicalLunge\state\gamma: "\\.\DISPLAY1=60" satırları; yeniden başlatınca da korunur.
    // Gama 0 simsiyah olmasın: gerçek çarpan %20..%100.
    static string GammaFile { get { return System.IO.Path.Combine(System.IO.Path.GetDirectoryName(StateFile), "gamma"); } }
    public static Dictionary<string, int> Gammas()
    {
        var d = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        try
        {
            foreach (var line in System.IO.File.ReadAllLines(GammaFile))
            {
                int eq = line.LastIndexOf('='); int v;
                if (eq > 0 && int.TryParse(line.Substring(eq + 1), out v)) d[line.Substring(0, eq)] = Math.Max(0, Math.Min(100, v));
            }
        }
        catch { }
        return d;
    }
    public static int Gamma(string dev) { int v; return Gammas().TryGetValue(dev, out v) ? v : 100; }
    static readonly object gammaGate = new object();
    // Bar'ların /gamma isteği (tekerlek adımı): yazar ve bekçiyi dosya izleyicisini beklemeden uyandırır
    public static bool SetGamma(string dev, int v)
    {
        lock (gammaGate)
        {
            var d = Gammas(); d[dev] = Math.Max(0, Math.Min(100, v));
            var lines = new List<string>(); foreach (var kv in d) if (kv.Value < 100) lines.Add(kv.Key + "=" + kv.Value);
            System.IO.File.WriteAllLines(GammaFile, lines.ToArray());
        }
        Interlocked.Increment(ref animGen);
        keeperWake.Set();
        return MainRunning() || Apply(Active);
    }
    public static bool AnyActive() { if (Active) return true; foreach (var v in Gammas().Values) if (v < 100) return true; return false; }

    [DllImport("gdi32.dll")] static extern bool GetDeviceGammaRamp(IntPtr dc, ushort[] ramp);

    // Ekranın hedef rampası: gece ışığı rengi x yazılımsal karartma
    static ushort[] TargetRamp(string dev, bool on, Dictionary<string, int> gammas, int lv)
    {
        int g; if (!gammas.TryGetValue(dev, out g)) g = 100;
        double k = 0.2 + 0.8 * g / 100.0;
        double R, G, B; Rgb(lv, out R, out G, out B);
        var ramp = new ushort[256 * 3];
        for (int i = 0; i < 256; i++)
        {
            ramp[i] = (ushort)(i * 257 * k * (on ? R : 1));
            ramp[256 + i] = (ushort)(i * 257 * k * (on ? G : 1));
            ramp[512 + i] = (ushort)(i * 257 * k * (on ? B : 1));
        }
        return ramp;
    }
    static int Level() { int lv; return int.TryParse(Settings()["level"], out lv) ? lv : 50; }

    public static bool Apply(bool on)
    {
        var gammas = Gammas();
        int lv = Level();
        bool ok = true;
        foreach (var s in Screen.AllScreens)
        {
            IntPtr dc = CreateDC(null, s.DeviceName, null, IntPtr.Zero);
            if (dc == IntPtr.Zero) continue;
            if (!SetDeviceGammaRamp(dc, TargetRamp(s.DeviceName, on, gammas, lv))) ok = false;
            DeleteDC(dc);
        }
        return ok;
    }

    // Yumuşak geçiş: her ekranda o an gerçekten uygulanan rampadan hedefe (ease-in-out). Hep gerçek rampadan başladığı
    // için yarıda kesilen bir geçişin ya da başka bir sürecin uyguladığı değerin üstünden atlamadan devam eder.
    // Yeni bir ayar değişikliği (animGen) süren geçişi keser.
    static int animGen;
    // false: yeni bir değişiklik geçişi yarıda kesti
    static bool Animate(int ms)
    {
        int gen = Volatile.Read(ref animGen);
        bool on = Active;
        var gammas = Gammas();
        int lv = Level();
        var devs = new List<string>(); var from = new List<ushort[]>(); var to = new List<ushort[]>();
        foreach (var s in Screen.AllScreens)
        {
            var target = TargetRamp(s.DeviceName, on, gammas, lv);
            var cur = new ushort[256 * 3];
            IntPtr dc = CreateDC(null, s.DeviceName, null, IntPtr.Zero);
            if (dc == IntPtr.Zero) continue;
            bool read = GetDeviceGammaRamp(dc, cur);
            DeleteDC(dc);
            devs.Add(s.DeviceName); from.Add(read ? cur : target); to.Add(target);
        }
        var sw = Stopwatch.StartNew();
        var frame = new ushort[256 * 3];
        while (true)
        {
            double t = ms <= 0 ? 1 : Math.Min(1, sw.ElapsedMilliseconds / (double)ms);
            double e = t < 0.5 ? 2 * t * t : 1 - Math.Pow(-2 * t + 2, 2) / 2;
            for (int d = 0; d < devs.Count; d++)
            {
                for (int i = 0; i < frame.Length; i++) frame[i] = (ushort)(from[d][i] + (to[d][i] - from[d][i]) * e);
                IntPtr dc = CreateDC(null, devs[d], null, IntPtr.Zero);
                if (dc == IntPtr.Zero) continue;
                SetDeviceGammaRamp(dc, frame);
                DeleteDC(dc);
            }
            if (t >= 1) return true;
            if (gen != Volatile.Read(ref animGen)) return false;
            Thread.Sleep(16);
        }
    }

    // Ayar dosyalarının anlık hali: değiştiyse geçiş yapılır
    static string Snapshot()
    {
        var d = Settings();
        var g = new List<string>(); foreach (var kv in Gammas()) g.Add(kv.Key + "=" + kv.Value);
        g.Sort();
        return d["on"] + "|" + d["level"] + "|" + d["mode"] + "|" + d["from"] + "|" + d["to"] + "|" + (Active ? "A" : "-") + "|" + string.Join(",", g);
    }

    // Bekçiyi uyandırır: ayar dosyası değişti (izleyici) ya da gama çekirdeğin içinden yazıldı (SetGamma)
    static readonly AutoResetEvent keeperWake = new AutoResetEvent(false);
    public static void StartKeeper()
    {
        string last = Snapshot();
        Apply(Active);
        // Kenar çubuğundaki düğme / kaydırıcı ve bar'daki karartma ayrı bir helper süreciyle dosyaya yazar: değişikliği
        // hemen fark et (yedek: 5 sn'lik yoklama, zamanlı açılıp kapanma da orada yakalanır).
        var changed = keeperWake;
        try
        {
            var fsw = new System.IO.FileSystemWatcher(System.IO.Path.GetDirectoryName(StateFile))
            {
                NotifyFilter = System.IO.NotifyFilters.LastWrite | System.IO.NotifyFilters.FileName | System.IO.NotifyFilters.Size,
            };
            System.IO.FileSystemEventHandler h = (s, e) =>
            {
                if (e.Name == "nightlight" || e.Name == "gamma") { Interlocked.Increment(ref animGen); changed.Set(); }
            };
            fsw.Changed += h; fsw.Created += h;
            fsw.EnableRaisingEvents = true;
            GC.KeepAlive(fsw);
            keepWatcher = fsw;
        }
        catch (Exception ex) { Slider.Log("gece ışığı izleyici: " + ex.Message); }
        var t = new Thread(() =>
        {
            bool resume = false; // önceki geçiş yarıda kesildi: ayar eski haline dönmüş olsa da hedefe geçişle git
            while (true)
            {
                changed.WaitOne(5000);
                try
                {
                    string now = Snapshot();
                    if (now != last || resume)
                    {
                        var a = last.Split('|'); var b = now.Split('|');
                        // Açıp kapama 1 sn; saatle açılıp kapanma 3 sn (gün batımı gibi); yoğunluk / karartma kaydırıcıyı
                        // takip etsin diye kısa
                        int ms = a[0] != b[0] || a[2] != b[2] || a[3] != b[3] || a[4] != b[4] ? 1000
                               : a[5] != b[5] ? 3000
                               : a[1] != b[1] ? 300
                               : a[6] != b[6] ? 150 : 600;
                        last = now;
                        resume = !Animate(ms);
                    }
                    else if (AnyActive()) Apply(Active); // başka bir uygulama / ekran değişimi rampayı sıfırladıysa
                }
                catch (Exception ex) { Slider.Log("gece ışığı: " + ex.GetBaseException().Message); }
            }
        }) { IsBackground = true, Name = "nightlight" };
        t.Start();
    }
    static System.IO.FileSystemWatcher keepWatcher;
}

// ---------------- Bölge seçici + Google Lens (ii modules/ii/regionSelector) ----------------
// Ekranın donmuş görüntüsü karartılır, sürükleyerek seçilen alan aydınlık kalır; Esc / sağ tık iptal.
// Seçilen alan Google Lens'e tarayıcıdan yüklenir: görüntü üçüncü bir sunucuya konmaz, yerel bir sayfa
// dosyayı doğrudan lens.google.com'a POST eder.
static class RegionSearch
{
    class Picker : Form
    {
        readonly Bitmap shot;
        Point? a; Point b;
        public Rectangle Result = Rectangle.Empty;
        static readonly Color Accent = Color.FromArgb(208, 188, 255);

        public Picker(Bitmap shot, Rectangle bounds)
        {
            this.shot = shot;
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true; KeyPreview = true;
            StartPosition = FormStartPosition.Manual; Bounds = bounds;
            DoubleBuffered = true; Cursor = Cursors.Cross;
        }
        protected override CreateParams CreateParams { get { var p = base.CreateParams; p.ExStyle |= 0x80; return p; } }
        Rectangle Sel()
        {
            if (!a.HasValue) return Rectangle.Empty;
            return Rectangle.FromLTRB(Math.Min(a.Value.X, b.X), Math.Min(a.Value.Y, b.Y), Math.Max(a.Value.X, b.X), Math.Max(a.Value.Y, b.Y));
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            try { PaintBody(e); } catch (Exception ex) { PaintErrors.Report("alan seçimi", ex); }
        }
        void PaintBody(PaintEventArgs e)
        {
            var g = e.Graphics;
            g.DrawImageUnscaled(shot, 0, 0);
            var r = Sel();
            using (var dim = new SolidBrush(Color.FromArgb(120, 0, 0, 0)))
            using (var reg = new System.Drawing.Region(ClientRectangle))
            {
                if (r.Width > 0 && r.Height > 0) reg.Exclude(r);
                g.FillRegion(dim, reg);
            }
            if (r.Width > 1 && r.Height > 1)
            {
                g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
                using (var pen = new Pen(Accent, 2)) g.DrawRectangle(pen, r);
                string label = r.Width + " × " + r.Height;
                using (var f = new Font("Segoe UI", 10f, FontStyle.Bold))
                {
                    var sz = g.MeasureString(label, f);
                    float lx = r.Left, ly = r.Bottom + 6 + sz.Height > Height ? r.Top - sz.Height - 10 : r.Bottom + 6;
                    using (var bg = new SolidBrush(Accent)) g.FillRectangle(bg, lx, ly, sz.Width + 10, sz.Height + 4);
                    using (var fg = new SolidBrush(Color.FromArgb(56, 30, 114))) g.DrawString(label, f, fg, lx + 5, ly + 2);
                }
            }
            else
            {
                string hint = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr"
                    ? "Google Lens: aramak istediğin alanı seç  •  Esc: iptal"
                    : "Google Lens: select the area to search  •  Esc: cancel";
                using (var f = new Font("Segoe UI", 12f))
                {
                    var sz = g.MeasureString(hint, f);
                    var scr = Screen.FromPoint(Cursor.Position).Bounds; scr.Offset(-Bounds.Left, -Bounds.Top);
                    float x = scr.Left + (scr.Width - sz.Width) / 2, y = scr.Top + 70;
                    using (var bg = new SolidBrush(Color.FromArgb(230, 20, 18, 24))) g.FillRectangle(bg, x - 16, y - 8, sz.Width + 32, sz.Height + 16);
                    using (var fg = new SolidBrush(Color.FromArgb(230, 224, 233))) g.DrawString(hint, f, fg, x, y);
                }
            }
        }
        protected override void OnMouseDown(MouseEventArgs e)
        {
            if (e.Button == MouseButtons.Right) { Close(); return; }
            a = e.Location; b = e.Location; Invalidate();
        }
        protected override void OnMouseMove(MouseEventArgs e) { if (a.HasValue) { b = e.Location; Invalidate(); } }
        protected override void OnMouseUp(MouseEventArgs e)
        {
            if (!a.HasValue) return;
            b = e.Location;
            var r = Sel();
            if (r.Width >= 8 && r.Height >= 8) { Result = r; Close(); }
            else { a = null; Invalidate(); }
        }
        protected override void OnKeyDown(KeyEventArgs e) { if (e.KeyCode == Keys.Escape) Close(); }
        System.Windows.Forms.Timer keepTop;
        protected override void OnShown(EventArgs e)
        {
            base.OnShown(e); Activate();
            // Ekran alıntısı HER ŞEYİN üstünde kalmalı: odak değişince pencereler (ve shell penceresi) kendini
            // en üst katmana alıp donmuş görüntünün üstüne çıkabiliyordu. Kapanana kadar sık sık yeniden en üste al.
            keepTop = new System.Windows.Forms.Timer { Interval = 30 };
            keepTop.Tick += (o, ev) => Native.SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // TOPMOST, NOSIZE|NOMOVE|NOACTIVATE
            keepTop.Start();
        }
        protected override void OnFormClosed(FormClosedEventArgs e) { if (keepTop != null) keepTop.Stop(); base.OnFormClosed(e); }
    }

    // Varsayılan tarayıcının açma komutu (http ilişkilendirmesi)
    static string BrowserCommand()
    {
        try
        {
            var prog = (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice", "ProgId", null);
            if (!string.IsNullOrEmpty(prog))
                return (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CLASSES_ROOT\" + prog + @"\shell\open\command", "", null);
        }
        catch { }
        return null;
    }

    public static void Lens()
    {
        var vs = SystemInformation.VirtualScreen;
        Bitmap shot = new Bitmap(vs.Width, vs.Height);
        using (var g = Graphics.FromImage(shot)) g.CopyFromScreen(vs.Left, vs.Top, 0, 0, vs.Size);
        var pk = new Picker(shot, vs);
        Application.Run(pk);
        var r = pk.Result;
        if (r.IsEmpty) return;

        string b64;
        using (var crop = shot.Clone(r, shot.PixelFormat))
        using (var ms = new System.IO.MemoryStream())
        {
            crop.Save(ms, System.Drawing.Imaging.ImageFormat.Png);
            b64 = Convert.ToBase64String(ms.ToArray());
        }
        string lang = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
        long ts = (long)(DateTime.UtcNow - new DateTime(1970, 1, 1)).TotalMilliseconds;
        string html = "<!doctype html><meta charset=utf-8><title>Google Lens</title>" +
            "<body style=\"background:#141218;color:#e6e0e9;font:16px 'Segoe UI';display:grid;place-items:center;height:100vh;margin:0\">Google Lens…" +
            "<form id=f method=POST enctype=multipart/form-data action=\"https://lens.google.com/v3/upload?hl=" + lang + "&re=df&stcs=" + ts + "&ep=subb\">" +
            "<input type=file name=encoded_image id=i hidden></form><script>" +
            "const s=atob('" + b64 + "'),a=new Uint8Array(s.length);for(let k=0;k<s.length;k++)a[k]=s.charCodeAt(k);" +
            "const dt=new DataTransfer();dt.items.add(new File([a],'image.png',{type:'image/png'}));" +
            "document.getElementById('i').files=dt.files;document.getElementById('f').submit();</script>";
        string path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "lunge-lens-" + ts + ".html");
        System.IO.File.WriteAllText(path, html);
        string url = new Uri(path).AbsoluteUri;

        string cmd = BrowserCommand();
        bool launched = false;
        try
        {
            // Tarayıcı kullanıcı olarak açılır (çekirdek yönetici olsa da)
            if (!string.IsNullOrEmpty(cmd))
            {
                // "C:\...\zen.exe" -osint -url "%1"  ->  exe + argümanlar
                string exe, rest;
                if (cmd.StartsWith("\"")) { int q = cmd.IndexOf('"', 1); exe = cmd.Substring(1, q - 1); rest = cmd.Substring(q + 1); }
                else { int sp = cmd.IndexOf(' '); exe = sp < 0 ? cmd : cmd.Substring(0, sp); rest = sp < 0 ? "" : cmd.Substring(sp); }
                rest = rest.Contains("%1") ? rest.Replace("%1", url) : rest + " \"" + url + "\"";
                launched = UserLaunch.Start(exe, rest.Trim(), Paths.Home);
            }
            else launched = UserLaunch.Start(url, "", Paths.Home);
        }
        catch { }
        if (!launched) UserLaunch.Start(url, "", Paths.Home);
        // Geçici sayfa: tarayıcı yükledikten bir dakika sonra kaldırılır
        Thread.Sleep(60000);
        try { System.IO.File.Delete(path); } catch { }
    }
}

// ---------------- Ekran alıntısı (Hyprland: Print -> grim + slurp + swappy) ----------------
// Alan seçilir, sonra seçimin çevresinde dairesel araçlar çıkar: kopyala, farklı kaydet, kalem, çember,
// dikdörtgen ve renk. Seçim yeterince genişse araçlar yalnızca altta; değilse alt -> sağ -> üst -> sol
// sırasıyla, birbirinin üstüne binmeden dağılır. Enter/Ctrl+C kopyalar, Ctrl+S kaydeder, Ctrl+Z geri alır.
static class SnipTool
{
    enum Tool { None, Pen, Ellipse, Rect }
    class Mark { public Tool Kind; public Color Col; public List<Point> Pts = new List<Point>(); public Point A, B; }
    class Btn { public string Id; public Rectangle R; }

    static readonly Color[] Palette = {
        Color.FromArgb(255, 82, 82), Color.FromArgb(255, 171, 64), Color.FromArgb(255, 235, 59),
        Color.FromArgb(105, 240, 174), Color.FromArgb(64, 196, 255), Color.FromArgb(208, 188, 255),
        Color.FromArgb(255, 128, 171), Color.White, Color.FromArgb(28, 27, 31) };
    static readonly Color Accent = Color.FromArgb(208, 188, 255);
    static readonly Color OnAccent = Color.FromArgb(56, 30, 114);
    static readonly Color Surface = Color.FromArgb(240, 43, 41, 48);
    static readonly Color SurfaceHover = Color.FromArgb(250, 61, 58, 68);
    static bool Tr { get { return System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr"; } }

    // Alıntı süreci kapanınca Windows odağı sıradaki pencereye (çoğu zaman masaüstüne ya da bar'a) veriyordu ve klavye
    // boşta kalıyordu: alıntıdan önce odakta olan pencereye geri ver. Bu arada başka bir pencere öne geldiyse (ör. kaydedilen
    // dosyayı gösteren Gezgin) dokunma.
    public static void GiveFocusBack(IntPtr prev)
    {
        if (prev == IntPtr.Zero || !Native.IsWindow(prev) || !Native.IsWindowVisible(prev) || Native.IsIconic(prev)) return;
        IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
        if (fg == prev) return;
        if (fg != IntPtr.Zero)
        {
            uint pid; Native.GetWindowThreadProcessId(fg, out pid);
            var cls = new StringBuilder(64); Native.GetClassName(fg, cls, 64);
            var t = new StringBuilder(64); Native.GetWindowText(fg, t, 64);
            string c = cls.ToString();
            bool emptyFocus = pid == (uint)Process.GetCurrentProcess().Id || c == "Progman" || c == "WorkerW" || c == "Shell_TrayWnd"
                || t.ToString().StartsWith(Names.Bar) || !Native.IsWindowVisible(fg);
            if (!emptyFocus) return;
        }
        Native.keybd_event(0xE8, 0, 0, Native.LL_MARK); Native.keybd_event(0xE8, 0, 2, Native.LL_MARK); // odak kilidi
        Native.SetForegroundWindow(prev);
    }

    class SnipForm : Form
    {
        readonly Bitmap shot;
        readonly Rectangle vs;
        Point? a; Point b;
        public Rectangle Sel = Rectangle.Empty;
        bool editing;
        Tool tool = Tool.None;
        Color col = Palette[0];
        public readonly List<Mark> Marks = new List<Mark>();
        Mark drawing;
        readonly List<Btn> btns = new List<Btn>();
        readonly List<Btn> swatches = new List<Btn>();
        string hover;
        public string Action;
        readonly Font glyph = new Font("Segoe MDL2 Assets", 14f);
        readonly Font small = new Font("Segoe UI", 9.5f, FontStyle.Bold);
        readonly Font hintFont = new Font("Segoe UI", 12f);

        public SnipForm(Bitmap shot, Rectangle vs)
        {
            this.shot = shot; this.vs = vs;
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true; KeyPreview = true;
            StartPosition = FormStartPosition.Manual; Bounds = vs;
            DoubleBuffered = true; Cursor = Cursors.Cross;
        }
        protected override CreateParams CreateParams { get { var p = base.CreateParams; p.ExStyle |= 0x80; return p; } }
        // Odak başka bir uygulamadayken Windows yeni sürecin öne gelmesini engelliyor: form odaktaki pencerenin (ve PiP gibi
        // hep üstte duran pencerelerin) altında kalıyor, o pencere karartılmıyor ve tıklanana kadar alıntı başlamıyordu.
        // Odak kilidini aş (atanmamış tuş) ve kapanana kadar kendini en üstte tut.
        System.Windows.Forms.Timer keepTop;
        int focusTries;
        void TakeForeground()
        {
            Native.keybd_event(0xE8, 0, 0, Native.LL_MARK); Native.keybd_event(0xE8, 0, 2, Native.LL_MARK);
            Native.SetForegroundWindow(Handle);
            Activate();
        }
        protected override void OnShown(EventArgs e)
        {
            base.OnShown(e);
            Native.SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002); // TOPMOST, en üste
            TakeForeground();
            keepTop = new System.Windows.Forms.Timer { Interval = 30 };
            keepTop.Tick += (o, ev) =>
            {
                Native.SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // TOPMOST, NOSIZE|NOMOVE|NOACTIVATE
                if (Native.GetForegroundWindow() != Handle && focusTries++ < 10) TakeForeground();
            };
            keepTop.Start();
        }
        protected override void OnFormClosed(FormClosedEventArgs e) { if (keepTop != null) keepTop.Stop(); base.OnFormClosed(e); }

        Rectangle Dragged()
        {
            if (!a.HasValue) return Rectangle.Empty;
            return Rectangle.FromLTRB(Math.Min(a.Value.X, b.X), Math.Min(a.Value.Y, b.Y), Math.Max(a.Value.X, b.X), Math.Max(a.Value.Y, b.Y));
        }
        Rectangle MonitorOf(Rectangle r)
        {
            var m = Screen.FromRectangle(new Rectangle(r.X + vs.X, r.Y + vs.Y, Math.Max(1, r.Width), Math.Max(1, r.Height))).Bounds;
            m.Offset(-vs.X, -vs.Y);
            return m;
        }

        // ---- Düğme yerleşimi: önce alt; sığmazsa alt, sağ, üst, sol kenarlar boyunca ----
        const int D = 44, G = 10, OFF = 14;
        void PlaceButtons()
        {
            btns.Clear(); swatches.Clear();
            var ids = new[] { "copy", "save", "pen", "ellipse", "rect", "color" };
            var mon = MonitorOf(Sel);
            int total = ids.Length * D + (ids.Length - 1) * G;
            var slots = new List<Rectangle>();
            bool below = Sel.Bottom + OFF + D <= mon.Bottom - 6;
            bool above = Sel.Top - OFF - D >= mon.Top + 6;
            bool right = Sel.Right + OFF + D <= mon.Right - 6;
            bool left = Sel.Left - OFF - D >= mon.Left + 6;

            Func<int, int, List<Rectangle>> row = (y, cap) =>
            {
                var list = new List<Rectangle>();
                int n = Math.Min(cap, ids.Length);
                int w = n * D + (n - 1) * G, x0 = Sel.Left + (Sel.Width - w) / 2;
                x0 = Math.Max(mon.Left + 6, Math.Min(mon.Right - 6 - w, x0));
                for (int i = 0; i < n; i++) list.Add(new Rectangle(x0 + i * (D + G), y, D, D));
                return list;
            };
            Func<int, int, List<Rectangle>> column = (x, cap) =>
            {
                var list = new List<Rectangle>();
                int n = Math.Min(cap, ids.Length);
                int h = n * D + (n - 1) * G, y0 = Sel.Top + (Sel.Height - h) / 2;
                y0 = Math.Max(mon.Top + 6, Math.Min(mon.Bottom - 6 - h, y0));
                for (int i = 0; i < n; i++) list.Add(new Rectangle(x, y0 + i * (D + G), D, D));
                return list;
            };

            int capW = Math.Max(1, (Sel.Width + G) / (D + G)), capH = Math.Max(1, (Sel.Height + G) / (D + G));
            if (below && Sel.Width >= total) slots.AddRange(row(Sel.Bottom + OFF, ids.Length));
            else if (!below && above && Sel.Width >= total) slots.AddRange(row(Sel.Top - OFF - D, ids.Length));
            else
            {
                if (below) slots.AddRange(row(Sel.Bottom + OFF, capW));
                if (slots.Count < ids.Length && right) slots.AddRange(column(Sel.Right + OFF, Math.Min(capH, ids.Length - slots.Count)));
                if (slots.Count < ids.Length && above) slots.AddRange(row(Sel.Top - OFF - D, Math.Min(capW, ids.Length - slots.Count)));
                if (slots.Count < ids.Length && left) slots.AddRange(column(Sel.Left - OFF - D, Math.Min(capH, ids.Length - slots.Count)));
                // Hâlâ yer kalmadıysa (çok küçük seçim): bir halka dışarıda, alta ortalanmış sıra
                if (slots.Count < ids.Length)
                {
                    int y = below ? Sel.Bottom + OFF + D + G : Math.Max(mon.Top + 6, Sel.Top - OFF - 2 * D - G);
                    int rest = ids.Length - slots.Count;
                    int w = rest * D + (rest - 1) * G, x0 = Math.Max(mon.Left + 6, Math.Min(mon.Right - 6 - w, Sel.Left + (Sel.Width - w) / 2));
                    for (int i = 0; i < rest; i++) slots.Add(new Rectangle(x0 + i * (D + G), y, D, D));
                }
            }
            for (int i = 0; i < ids.Length; i++) btns.Add(new Btn { Id = ids[i], R = slots[i] });
        }

        void ShowSwatches(Rectangle anchor)
        {
            swatches.Clear();
            var mon = MonitorOf(anchor);
            const int S = 30, SG = 8;
            int n = Palette.Length, w = n * S + (n - 1) * SG;
            int x0 = anchor.Left + anchor.Width / 2 - w / 2;
            x0 = Math.Max(mon.Left + 6, Math.Min(mon.Right - 6 - w, x0));
            int y = anchor.Bottom + 10;
            // Seçimin ya da diğer düğmelerin üstüne binmesin: altta yer yoksa üstte
            var probe = new Rectangle(x0, y, w, S);
            bool clash = y + S > mon.Bottom - 6 || probe.IntersectsWith(Sel);
            foreach (var bt in btns) if (bt.Id != "color" && probe.IntersectsWith(bt.R)) clash = true;
            if (clash) y = anchor.Top - 10 - S;
            probe = new Rectangle(x0, y, w, S);
            if (probe.IntersectsWith(Sel) || y < mon.Top + 6)
            {
                // Yan tarafa dikey
                int x = anchor.Right + 10 + S <= mon.Right - 6 ? anchor.Right + 10 : anchor.Left - 10 - S;
                int h = n * S + (n - 1) * SG, y0 = Math.Max(mon.Top + 6, Math.Min(mon.Bottom - 6 - h, anchor.Top + anchor.Height / 2 - h / 2));
                for (int i = 0; i < n; i++) swatches.Add(new Btn { Id = "c" + i, R = new Rectangle(x, y0 + i * (S + SG), S, S) });
                return;
            }
            for (int i = 0; i < n; i++) swatches.Add(new Btn { Id = "c" + i, R = new Rectangle(x0 + i * (S + SG), y, S, S) });
        }

        Btn HitBtn(Point p)
        {
            foreach (var s in swatches) if (Inside(s.R, p)) return s;
            foreach (var bt in btns) if (Inside(bt.R, p)) return bt;
            return null;
        }
        static bool Inside(Rectangle r, Point p)
        {
            double cx = r.X + r.Width / 2.0, cy = r.Y + r.Height / 2.0, rr = r.Width / 2.0 + 2;
            return (p.X - cx) * (p.X - cx) + (p.Y - cy) * (p.Y - cy) <= rr * rr;
        }

        // ---- Çizim ----
        public static void DrawMark(Graphics g, Mark m)
        {
            using (var pen = new Pen(m.Col, m.Kind == Tool.Pen ? 4f : 3.5f) { StartCap = System.Drawing.Drawing2D.LineCap.Round, EndCap = System.Drawing.Drawing2D.LineCap.Round, LineJoin = System.Drawing.Drawing2D.LineJoin.Round })
            {
                if (m.Kind == Tool.Pen && m.Pts.Count > 1) g.DrawLines(pen, m.Pts.ToArray());
                else if (m.Kind == Tool.Pen && m.Pts.Count == 1) using (var br = new SolidBrush(m.Col)) g.FillEllipse(br, m.Pts[0].X - 2, m.Pts[0].Y - 2, 4, 4);
                else
                {
                    var r = Rectangle.FromLTRB(Math.Min(m.A.X, m.B.X), Math.Min(m.A.Y, m.B.Y), Math.Max(m.A.X, m.B.X), Math.Max(m.A.Y, m.B.Y));
                    if (m.Kind == Tool.Ellipse) g.DrawEllipse(pen, r);
                    else if (m.Kind == Tool.Rect) g.DrawRectangle(pen, r);
                }
            }
        }

        void DrawButton(Graphics g, Btn bt)
        {
            bool active = (bt.Id == "pen" && tool == Tool.Pen) || (bt.Id == "ellipse" && tool == Tool.Ellipse) || (bt.Id == "rect" && tool == Tool.Rect) || (bt.Id == "color" && swatches.Count > 0);
            var r = bt.R;
            using (var sh = new SolidBrush(Color.FromArgb(70, 0, 0, 0))) g.FillEllipse(sh, r.X, r.Y + 2, r.Width, r.Height);
            using (var bg = new SolidBrush(active ? Accent : (hover == bt.Id ? SurfaceHover : Surface))) g.FillEllipse(bg, r);
            Color fg = active ? OnAccent : Color.FromArgb(230, 224, 233);
            var c = new Point(r.X + r.Width / 2, r.Y + r.Height / 2);
            using (var pen = new Pen(fg, 2f))
            {
                switch (bt.Id)
                {
                    case "copy": DrawGlyph(g, "", fg, r); break;
                    case "save": DrawGlyph(g, "", fg, r); break;
                    case "pen": DrawGlyph(g, "", fg, r); break;
                    case "ellipse": g.DrawEllipse(pen, c.X - 9, c.Y - 9, 18, 18); break;
                    case "rect": g.DrawRectangle(pen, c.X - 9, c.Y - 8, 18, 16); break;
                    case "color":
                        using (var br = new SolidBrush(col)) g.FillEllipse(br, c.X - 11, c.Y - 11, 22, 22);
                        using (var ring = new Pen(Color.FromArgb(200, 255, 255, 255), 2f)) g.DrawEllipse(ring, c.X - 11, c.Y - 11, 22, 22);
                        break;
                }
            }
        }
        void DrawGlyph(Graphics g, string s, Color fg, Rectangle r)
        {
            using (var br = new SolidBrush(fg))
            using (var sf = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
                g.DrawString(s, glyph, br, new RectangleF(r.X, r.Y + 1, r.Width, r.Height), sf);
        }

        string Tip(string id)
        {
            switch (id)
            {
                case "copy": return Tr ? "Kopyala (Enter)" : "Copy (Enter)";
                case "save": return Tr ? "Farklı kaydet (Ctrl+S)" : "Save as (Ctrl+S)";
                case "pen": return Tr ? "Kalem" : "Pen";
                case "ellipse": return Tr ? "Çember" : "Circle";
                case "rect": return Tr ? "Dikdörtgen" : "Rectangle";
                case "color": return Tr ? "Renk" : "Color";
            }
            return null;
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            try { PaintBody(e); } catch (Exception ex) { PaintErrors.Report("ekran alıntısı", ex); }
        }
        void PaintBody(PaintEventArgs e)
        {
            var g = e.Graphics;
            g.DrawImageUnscaled(shot, 0, 0);
            var r = editing ? Sel : Dragged();
            using (var dim = new SolidBrush(Color.FromArgb(120, 0, 0, 0)))
            using (var reg = new System.Drawing.Region(ClientRectangle))
            {
                if (r.Width > 0 && r.Height > 0) reg.Exclude(r);
                g.FillRegion(dim, reg);
            }
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            if (r.Width > 1 && r.Height > 1)
            {
                var clip = g.Clip;
                g.SetClip(r);
                foreach (var m in Marks) DrawMark(g, m);
                if (drawing != null) DrawMark(g, drawing);
                g.Clip = clip;
                using (var pen = new Pen(Accent, 2)) g.DrawRectangle(pen, r);
                if (!editing)
                {
                    string label = r.Width + " × " + r.Height;
                    var sz = g.MeasureString(label, small);
                    float lx = r.Left, ly = r.Bottom + 6 + sz.Height > Height ? r.Top - sz.Height - 10 : r.Bottom + 6;
                    using (var bg = new SolidBrush(Accent)) g.FillRectangle(bg, lx, ly, sz.Width + 10, sz.Height + 4);
                    using (var fg = new SolidBrush(OnAccent)) g.DrawString(label, small, fg, lx + 5, ly + 2);
                }
            }
            else
            {
                string hint = Tr ? "Ekran alıntısı: alanı seç  •  Esc: iptal" : "Screenshot: select an area  •  Esc: cancel";
                var sz = g.MeasureString(hint, hintFont);
                var scr = Screen.FromPoint(Cursor.Position).Bounds; scr.Offset(-vs.Left, -vs.Top);
                float x = scr.Left + (scr.Width - sz.Width) / 2, y = scr.Top + 70;
                using (var bg = new SolidBrush(Color.FromArgb(230, 20, 18, 24))) g.FillRectangle(bg, x - 16, y - 8, sz.Width + 32, sz.Height + 16);
                using (var fg = new SolidBrush(Color.FromArgb(230, 224, 233))) g.DrawString(hint, hintFont, fg, x, y);
            }
            if (editing)
            {
                foreach (var bt in btns) DrawButton(g, bt);
                for (int i = 0; i < swatches.Count; i++)
                {
                    var s = swatches[i].R;
                    using (var sh = new SolidBrush(Color.FromArgb(70, 0, 0, 0))) g.FillEllipse(sh, s.X, s.Y + 2, s.Width, s.Height);
                    using (var br = new SolidBrush(Palette[i])) g.FillEllipse(br, s);
                    bool on = Palette[i].ToArgb() == col.ToArgb();
                    using (var ring = new Pen(on ? Accent : Color.FromArgb(160, 255, 255, 255), on ? 3f : 1.5f)) g.DrawEllipse(ring, s);
                }
                // İpucu balonu
                Btn hb = null; foreach (var bt in btns) if (bt.Id == hover) hb = bt;
                if (hb != null)
                {
                    string t = Tip(hb.Id);
                    var sz = g.MeasureString(t, small);
                    float tx = hb.R.X + hb.R.Width / 2f - sz.Width / 2f - 6, ty = hb.R.Bottom + 6;
                    if (ty + sz.Height + 6 > Height) ty = hb.R.Top - sz.Height - 12;
                    using (var bg = new SolidBrush(Color.FromArgb(235, 20, 18, 24))) g.FillRectangle(bg, tx, ty, sz.Width + 12, sz.Height + 6);
                    using (var fg = new SolidBrush(Color.FromArgb(230, 224, 233))) g.DrawString(t, small, fg, tx + 6, ty + 3);
                }
            }
        }

        // ---- Fare ----
        protected override void OnMouseDown(MouseEventArgs e)
        {
            if (e.Button == MouseButtons.Right) { Close(); return; }
            if (editing)
            {
                var hit = HitBtn(e.Location);
                if (hit != null) { Press(hit); return; }
                if (swatches.Count > 0) { swatches.Clear(); Invalidate(); }
                if (tool != Tool.None && Sel.Contains(e.Location))
                {
                    drawing = new Mark { Kind = tool, Col = col, A = e.Location, B = e.Location };
                    if (tool == Tool.Pen) drawing.Pts.Add(e.Location);
                    return;
                }
                if (Sel.Contains(e.Location)) return;
                // Seçim dışına tıklandı: yeniden seç
                editing = false; Marks.Clear(); btns.Clear(); Sel = Rectangle.Empty; tool = Tool.None; Cursor = Cursors.Cross;
            }
            a = e.Location; b = e.Location; Invalidate();
        }
        protected override void OnMouseMove(MouseEventArgs e)
        {
            if (drawing != null)
            {
                var p = new Point(Math.Max(Sel.Left, Math.Min(Sel.Right, e.X)), Math.Max(Sel.Top, Math.Min(Sel.Bottom, e.Y)));
                if (drawing.Kind == Tool.Pen) drawing.Pts.Add(p); else drawing.B = p;
                Invalidate(Sel); return;
            }
            if (!editing && a.HasValue) { b = e.Location; Invalidate(); return; }
            if (editing)
            {
                var hit = HitBtn(e.Location);
                string h = hit == null ? null : hit.Id;
                Cursor = hit != null ? Cursors.Hand : (tool != Tool.None && Sel.Contains(e.Location) ? Cursors.Cross : Cursors.Default);
                if (h != hover) { hover = h; Invalidate(); }
            }
        }
        protected override void OnMouseUp(MouseEventArgs e)
        {
            if (drawing != null) { Marks.Add(drawing); drawing = null; Invalidate(); return; }
            if (editing || !a.HasValue) return;
            b = e.Location;
            var r = Dragged();
            a = null;
            if (r.Width >= 8 && r.Height >= 8) { Sel = r; editing = true; PlaceButtons(); Cursor = Cursors.Default; }
            Invalidate();
        }

        void Press(Btn bt)
        {
            if (bt.Id.StartsWith("c") && bt.Id.Length <= 2 && char.IsDigit(bt.Id[1])) { col = Palette[bt.Id[1] - '0']; swatches.Clear(); if (tool == Tool.None) tool = Tool.Pen; Invalidate(); return; }
            switch (bt.Id)
            {
                case "copy": Action = "copy"; Close(); return;
                case "save": Action = "save"; Close(); return;
                case "pen": tool = tool == Tool.Pen ? Tool.None : Tool.Pen; break;
                case "ellipse": tool = tool == Tool.Ellipse ? Tool.None : Tool.Ellipse; break;
                case "rect": tool = tool == Tool.Rect ? Tool.None : Tool.Rect; break;
                case "color": if (swatches.Count > 0) swatches.Clear(); else ShowSwatches(bt.R); break;
            }
            Invalidate();
        }

        protected override void OnKeyDown(KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Escape) { if (swatches.Count > 0) { swatches.Clear(); Invalidate(); } else Close(); return; }
            if (!editing) return;
            if (e.KeyCode == Keys.Enter || (e.Control && e.KeyCode == Keys.C)) { Action = "copy"; Close(); }
            else if (e.Control && e.KeyCode == Keys.S) { Action = "save"; Close(); }
            else if (e.Control && e.KeyCode == Keys.Z && Marks.Count > 0) { Marks.RemoveAt(Marks.Count - 1); Invalidate(); }
        }
    }

    // Ctrl+Print: farenin bulunduğu monitörün tamamı; hiçbir şey sormadan panoya kopyalanır ve Resimler\Screenshots'a kaydedilir.
    public static void RunScreen()
    {
        var mon = Screen.FromPoint(Cursor.Position).Bounds;
        var bmp = new Bitmap(mon.Width, mon.Height);
        using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(mon.Left, mon.Top, 0, 0, mon.Size);
        try { Clipboard.SetDataObject(bmp, true, 5, 100); } catch { }
        try
        {
            string dir = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyPictures), "Screenshots");
            System.IO.Directory.CreateDirectory(dir);
            bmp.Save(System.IO.Path.Combine(dir, "Screenshot_" + DateTime.Now.ToString("yyyy-MM-dd_HH-mm-ss") + ".png"), System.Drawing.Imaging.ImageFormat.Png);
        }
        catch { }
        // Deklanşör hissi: monitör bir an beyaza döner ve sönerek geri gelir
        var flash = new Form { FormBorderStyle = FormBorderStyle.None, ShowInTaskbar = false, TopMost = true, StartPosition = FormStartPosition.Manual, Bounds = mon, BackColor = Color.White, Opacity = 0.55 };
        flash.Shown += (o, e) =>
        {
            var t = new System.Windows.Forms.Timer { Interval = 15 };
            int t0 = Environment.TickCount;
            t.Tick += (o2, e2) =>
            {
                double k = Math.Min(1, (Environment.TickCount - t0) / 260.0);
                flash.Opacity = 0.55 * (1 - k) * (1 - k);
                if (k >= 1) { t.Stop(); flash.Close(); }
            };
            t.Start();
        };
        Application.Run(flash);
    }

    public static void Run()
    {
        var vs = SystemInformation.VirtualScreen;
        var shot = new Bitmap(vs.Width, vs.Height);
        using (var g = Graphics.FromImage(shot)) g.CopyFromScreen(vs.Left, vs.Top, 0, 0, vs.Size);
        var f = new SnipForm(shot, vs);
        Application.Run(f);
        var sel = f.Sel;
        if (f.Action == null || sel.IsEmpty) return;

        var outBmp = new Bitmap(sel.Width, sel.Height);
        using (var g = Graphics.FromImage(outBmp))
        {
            g.DrawImage(shot, new Rectangle(0, 0, sel.Width, sel.Height), sel, GraphicsUnit.Pixel);
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            g.TranslateTransform(-sel.X, -sel.Y);
            foreach (var m in f.Marks) SnipForm.DrawMark(g, m);
        }
        if (f.Action == "copy")
        {
            // true: helper kapansa da panoda kalsın
            try { Clipboard.SetDataObject(outBmp, true, 10, 100); Slider.Log("alıntı panoya kopyalandı: " + outBmp.Width + "x" + outBmp.Height); }
            catch (Exception ex) { Slider.Log("alıntı panoya kopyalanamadı: " + ex.Message); }
            return;
        }
        string dir = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyPictures), "Screenshots");
        System.IO.Directory.CreateDirectory(dir);
        string name = "Screenshot_" + DateTime.Now.ToString("yyyy-MM-dd_HH-mm-ss") + ".png";
        // Kaydetme penceresi ayrı thread'de: kabuk eklentisi vb. yüzünden hiç açılamazsa (bir kez oldu: süreç sonsuza dek
        // bekledi ve yeni alıntıları kilitledi) 3 sn sonra doğrudan Screenshots klasörüne kaydedip klasörü göster.
        string chosen = null; bool done = false;
        // Kutu, seçimin yapıldığı monitörde açılsın: sahipsiz kutuyu Windows hep ana monitöre koyuyordu
        var selMon = Screen.FromRectangle(new Rectangle(sel.X + vs.X, sel.Y + vs.Y, Math.Max(1, sel.Width), Math.Max(1, sel.Height))).WorkingArea;
        var dt = new Thread(() =>
        {
            try
            {
                using (var owner = new Form
                {
                    FormBorderStyle = FormBorderStyle.None, ShowInTaskbar = false, StartPosition = FormStartPosition.Manual,
                    Bounds = new Rectangle(selMon.X + selMon.Width / 2, selMon.Y + selMon.Height / 2, 1, 1), Opacity = 0, TopMost = true,
                })
                using (var dlg = new SaveFileDialog
                {
                    Title = Tr ? "Ekran alıntısını kaydet" : "Save screenshot",
                    InitialDirectory = dir, FileName = name,
                    Filter = "PNG (*.png)|*.png|JPEG (*.jpg)|*.jpg", AddExtension = true,
                })
                {
                    owner.Show();
                    if (dlg.ShowDialog(owner) == DialogResult.OK) chosen = dlg.FileName;
                }
            }
            catch { }
            done = true;
        }) { IsBackground = true };
        dt.SetApartmentState(ApartmentState.STA);
        dt.Start();
        // Kutunun ilk açılışı (her alıntı yeni bir süreç) kabuk eklentileri ve sürücüler yüzünden birkaç saniye sürebiliyor:
        // 3 sn'lik sınırda kutu hiç görünmeden doğrudan kaydediliyordu. Yedek yol yalnızca kutu gerçekten açılamazsa.
        var sw = Stopwatch.StartNew();
        while (!done && sw.ElapsedMilliseconds < 20000 && !HasVisibleDialog()) Thread.Sleep(50);
        if (!done && !HasVisibleDialog())
        {
            string path = System.IO.Path.Combine(dir, name);
            outBmp.Save(path, System.Drawing.Imaging.ImageFormat.Png);
            Slider.Log("kaydetme kutusu 20 sn'de açılmadı; doğrudan kaydedildi: " + path);
            try { Process.Start("explorer.exe", "/select,\"" + path + "\""); } catch { }
            return; // takılı thread arka planda; süreç kapanır
        }
        Slider.Log("kaydetme kutusu " + sw.ElapsedMilliseconds + " ms'de açıldı");
        dt.Join();
        if (chosen == null) return;
        var fmt = chosen.EndsWith(".jpg", StringComparison.OrdinalIgnoreCase) ? System.Drawing.Imaging.ImageFormat.Jpeg : System.Drawing.Imaging.ImageFormat.Png;
        outBmp.Save(chosen, fmt);
    }

    static bool HasVisibleDialog()
    {
        uint me = (uint)Process.GetCurrentProcess().Id; bool found = false;
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            uint pid; Native.GetWindowThreadProcessId(h, out pid);
            if (pid == me && Native.IsWindowVisible(h))
            {
                Native.RECT r; Native.GetWindowRect(h, out r);
                if (r.Right - r.Left > 50 && r.Bottom - r.Top > 50) { found = true; return false; } // 1x1 sahip pencere değil
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static string PidFile { get { return System.IO.Path.Combine(System.IO.Path.GetTempPath(), "lunge-snip.pid"); } }

    // Önceki alıntı süreci hâlâ çalışıyor ama hiç görünür penceresi yoksa takılıdır: kapat
    public static bool KillStale()
    {
        try
        {
            int pid = int.Parse(System.IO.File.ReadAllText(PidFile).Trim());
            if (!ProcInfo.Name((uint)pid).Equals(Names.Core, StringComparison.OrdinalIgnoreCase)) return false;
            bool visible = false;
            Native.EnumWindows(delegate (IntPtr h, IntPtr l)
            {
                uint wp; Native.GetWindowThreadProcessId(h, out wp);
                if (wp == (uint)pid && Native.IsWindowVisible(h))
                {
                    Native.RECT r; Native.GetWindowRect(h, out r);
                    if (r.Right - r.Left > 50 && r.Bottom - r.Top > 50) { visible = true; return false; }
                }
                return true;
            }, IntPtr.Zero);
            if (visible) return false;
            using (var pr = Process.GetProcessById(pid)) { pr.Kill(); pr.WaitForExit(1500); }
            return true;
        }
        catch { return false; }
    }
}

// ---------------- Pano geçmişi (Super+V: ii "overviewClipboardToggle" / cliphist) ----------------
// Çalışan helper panoyu dinler (WM_CLIPBOARDUPDATE); metinler ve görüntüler %LOCALAPPDATA%\LogicalLunge\state\clipboard'a
// yazılır (en çok 100 kayıt). Overview'da ";" öneki bu listeyi gösterir. Parola yöneticileri gibi geçmişe eklenmesini
// istemeyen uygulamalar (ExcludeClipboardContentFromMonitorProcessing / CanIncludeInClipboardHistory) atlanır.
//   --clip-list        -> [{"id","kind":"text|image","text","lines","thumb","time"}]  (en yeni önce)
//   --clip-set <id>    -> kaydı panoya koyar
//   --clip-del <id> | --clip-clear
static class ClipHistory
{
    const int MAX = 100, MAX_TEXT = 200000;
    // tek görüntü ve bütün görüntüler: 100 büyük ekran görüntüsü gigabaytlar tutuyordu
    const long MAX_IMAGE = 25L << 20, MAX_IMAGES = 300L << 20;
    static readonly object gate = new object();
    static readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    static string Dir
    {
        get
        {
            string d = Paths.DataDir("clipboard");
            System.IO.Directory.CreateDirectory(d);
            return d;
        }
    }
    static string DbPath { get { return System.IO.Path.Combine(Dir, "history.json"); } }

    class Item { public long id; public string kind, text, file; public long time; }

    static List<Item> Load()
    {
        var l = new List<Item>();
        try
        {
            if (!System.IO.File.Exists(DbPath)) return l;
            var arr = json.DeserializeObject(System.IO.File.ReadAllText(DbPath, Encoding.UTF8)) as System.Collections.IEnumerable;
            if (arr == null) return l;
            foreach (Dictionary<string, object> d in arr)
                l.Add(new Item
                {
                    id = Convert.ToInt64(d["id"]), kind = Convert.ToString(d["kind"]), time = Convert.ToInt64(d["time"]),
                    text = d.ContainsKey("text") ? Convert.ToString(d["text"]) : null, file = d.ContainsKey("file") ? Convert.ToString(d["file"]) : null
                });
        }
        catch { }
        return l;
    }

    static void Save(List<Item> l)
    {
        try
        {
            var rows = new List<Dictionary<string, object>>();
            foreach (var i in l) rows.Add(new Dictionary<string, object> { { "id", i.id }, { "kind", i.kind }, { "text", i.text }, { "file", i.file }, { "time", i.time } });
            string tmp = DbPath + ".tmp";
            System.IO.File.WriteAllText(tmp, json.Serialize(rows), new UTF8Encoding(false));
            if (System.IO.File.Exists(DbPath)) System.IO.File.Delete(DbPath);
            System.IO.File.Move(tmp, DbPath);
        }
        catch { }
    }

    static void DropFile(Item i)
    {
        if (i != null && i.file != null) { try { System.IO.File.Delete(System.IO.Path.Combine(Dir, i.file)); } catch { } }
    }

    // ---- dinleyici (ana helper) ----
    class Listener : NativeWindow
    {
        [DllImport("user32.dll", SetLastError = true)] static extern bool AddClipboardFormatListener(IntPtr h);
        public Listener()
        {
            CreateHandle(new CreateParams { Parent = new IntPtr(-3) }); // HWND_MESSAGE
            AddClipboardFormatListener(Handle);
        }
        protected override void WndProc(ref Message m)
        {
            if (m.Msg == 0x031D) { try { Changed(); } catch (Exception ex) { Slider.Log("clip: " + ex.Message); } }
            base.WndProc(ref m);
        }
    }

    // Dinleyiciye güçlü referans şart: NativeWindow yalnızca zayıf referansla izlenir; referanssız kalınca ilk çöp
    // toplamada silinir ve pano değişiklikleri artık işlenmez (Super+V geçmişi bir süre sonra hiç kayıt almıyordu).
    static Listener listener;

    public static void StartListener()
    {
        var t = new Thread(() => { listener = new Listener(); Application.Run(); });
        t.SetApartmentState(ApartmentState.STA);
        t.IsBackground = true;
        t.Start();
    }

    static T Retry<T>(Func<T> f, T fallback)
    {
        for (int i = 0; i < 8; i++)
        {
            try { return f(); }
            catch { Thread.Sleep(40); }
        }
        return fallback;
    }

    // Tarayıcıda "Resmi kopyala": görüntünün yanında resmin bağlantısı (ya da dosya yolu) metin olarak da gelir; eskiden
    // metin önce kaydedilip çıkıldığı için resimler geçmişe hiç girmiyordu. Metin tek satırlık bir bağlantı / yol ise
    // görüntü öncelikli; Excel / Word gibi hem metin hem görüntü koyanlarda metin.
    static bool LinkLike(string t)
    {
        t = t.Trim();
        if (t.Length == 0) return true;
        if (t.IndexOf('\n') >= 0) return false;
        return t.StartsWith("http://", StringComparison.OrdinalIgnoreCase) || t.StartsWith("https://", StringComparison.OrdinalIgnoreCase)
            || t.StartsWith("data:image", StringComparison.OrdinalIgnoreCase) || t.StartsWith("file:", StringComparison.OrdinalIgnoreCase)
            || (t.Length > 3 && t[1] == ':' && (t[2] == '\\' || t[2] == '/'));
    }

    static byte[] ClipboardPng()
    {
        // Önce hazır PNG (Chrome / Firefox / Zen bunu da koyar; saydamlığı korur), yoksa DIB -> PNG
        var obj = Retry(() => Clipboard.ContainsData("PNG") ? Clipboard.GetData("PNG") : null, (object)null);
        var ms0 = obj as System.IO.MemoryStream;
        if (ms0 != null && ms0.Length > 0) return ms0.ToArray();
        Image img = Retry(() => Clipboard.ContainsImage() ? Clipboard.GetImage() : null, (Image)null);
        if (img == null) return null;
        using (img)
        using (var ms = new System.IO.MemoryStream()) { img.Save(ms, System.Drawing.Imaging.ImageFormat.Png); return ms.ToArray(); }
    }

    static void AddText(string text)
    {
        if (text.Length > MAX_TEXT) text = text.Substring(0, MAX_TEXT);
        lock (gate)
        {
            var l = Load();
            // en üstteki zaten bu: bütün dosyayı yeniden yazmaya gerek yok (panoyu sık yazan araçlar)
            if (l.Count > 0 && l[0].kind == "text" && l[0].text == text) return;
            var same = l.FindAll(x => x.kind == "text" && x.text == text);
            foreach (var s in same) l.Remove(s);
            l.Insert(0, new Item { id = DateTime.UtcNow.Ticks, kind = "text", text = text, time = DateTimeOffset.UtcNow.ToUnixTimeSeconds() });
            Trim(l);
            Save(l);
        }
    }

    static string Formats()
    {
        try { var d = Clipboard.GetDataObject(); return d == null ? "(boş)" : string.Join(", ", d.GetFormats(false)); }
        catch (Exception ex) { return "(okunamadı: " + ex.Message + ")"; }
    }

    static void Changed()
    {
        if (Retry(() => Clipboard.ContainsData("ExcludeClipboardContentFromMonitorProcessing"), false)) { Slider.Log("pano: geçmişe alınmaması istendi"); return; }
        string text = Retry(() => Clipboard.ContainsText() ? Clipboard.GetText() : null, (string)null);
        bool hasText = !string.IsNullOrEmpty(text) && text.Trim().Length > 0;
        bool hasImage = Retry(() => Clipboard.ContainsImage() || Clipboard.ContainsData("PNG"), false);
        if (hasText && (!hasImage || !LinkLike(text))) { AddText(text); return; }
        byte[] png = hasImage ? ClipboardPng() : null;
        if (png == null) { Slider.Log("pano: görüntü okunamadı, biçimler=[" + Formats() + "]"); if (hasText) AddText(text); return; }
        if (png.Length > MAX_IMAGE) { Slider.Log("pano: görüntü geçmişe alınmadı (" + (png.Length >> 20) + " MB)"); if (hasText) AddText(text); return; }
        string hash;
        using (var sha = System.Security.Cryptography.SHA1.Create()) hash = BitConverter.ToString(sha.ComputeHash(png)).Replace("-", "").Substring(0, 16);
        string file = "img-" + hash + ".png";
        lock (gate)
        {
            var l = Load();
            if (l.Count > 0 && l[0].kind == "image" && l[0].file == file) return;
            var same = l.FindAll(x => x.kind == "image" && x.file == file);
            foreach (var s in same) l.Remove(s);
            if (!System.IO.File.Exists(System.IO.Path.Combine(Dir, file))) System.IO.File.WriteAllBytes(System.IO.Path.Combine(Dir, file), png);
            l.Insert(0, new Item { id = DateTime.UtcNow.Ticks, kind = "image", file = file, time = DateTimeOffset.UtcNow.ToUnixTimeSeconds() });
            Trim(l);
            Save(l);
        }
    }

    static void Trim(List<Item> l)
    {
        while (l.Count > MAX) { var last = l[l.Count - 1]; l.RemoveAt(l.Count - 1); DropFile(last); }
        // görüntülerin toplamı sınırı aşarsa en eskileri gider
        long total = 0;
        for (int i = 0; i < l.Count; i++)
        {
            if (l[i].kind != "image") continue;
            try { total += new System.IO.FileInfo(System.IO.Path.Combine(Dir, l[i].file)).Length; } catch { }
            if (total > MAX_IMAGES) { DropFile(l[i]); l.RemoveAt(i); i--; }
        }
    }

    // ---- CLI ----
    static string Thumb(string file)
    {
        try
        {
            using (var src = Image.FromFile(System.IO.Path.Combine(Dir, file)))
            {
                int h = 56, w = Math.Max(1, (int)((double)src.Width * h / src.Height));
                if (w > 160) { w = 160; h = Math.Max(1, (int)((double)src.Height * w / src.Width)); }
                using (var bmp = new Bitmap(w, h))
                {
                    using (var g = Graphics.FromImage(bmp)) { g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic; g.DrawImage(src, 0, 0, w, h); }
                    using (var ms = new System.IO.MemoryStream()) { bmp.Save(ms, System.Drawing.Imaging.ImageFormat.Png); return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray()); }
                }
            }
        }
        catch { return ""; }
    }

    public static string List()
    {
        List<Item> l;
        lock (gate) l = Load();
        var rows = new List<Dictionary<string, object>>();
        foreach (var i in l)
        {
            string text = i.text ?? "";
            int lines = text.Length == 0 ? 0 : text.Split('\n').Length;
            string preview = text.Length > 300 ? text.Substring(0, 300) : text;
            rows.Add(new Dictionary<string, object>
            {
                { "id", i.id.ToString() }, { "kind", i.kind }, { "text", preview }, { "lines", lines },
                { "thumb", i.kind == "image" && rows.Count < 30 ? Thumb(i.file) : "" }, { "time", i.time }
            });
        }
        return json.Serialize(rows);
    }

    public static string Set(string id)
    {
        Item it;
        lock (gate) it = Load().Find(x => x.id.ToString() == id);
        if (it == null) return "{\"ok\":false,\"error\":\"kayıt bulunamadı\"}";
        try
        {
            if (it.kind == "text") Clipboard.SetDataObject(it.text, true, 10, 50);
            else using (var img = Image.FromFile(System.IO.Path.Combine(Dir, it.file))) Clipboard.SetDataObject(new Bitmap(img), true, 10, 50);
        }
        catch (Exception ex) { return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", ex.Message } }); }
        return "{\"ok\":true}";
    }

    public static string Delete(string id)
    {
        lock (gate)
        {
            var l = Load();
            var it = l.Find(x => x.id.ToString() == id);
            if (it != null) { l.Remove(it); DropFile(it); Save(l); }
        }
        return "{\"ok\":true}";
    }

    public static string Clear()
    {
        lock (gate) { foreach (var i in Load()) DropFile(i); Save(new List<Item>()); }
        return "{\"ok\":true}";
    }
}

// ---------------- Güncelleme (sağ panel > güncelle düğmesi + update widget'ı) ----------------
// --update-check    -> {"current","latest","tag","available","downloaded","size","notes","error"}
// --update-download -> sürümü indirir (ilerleme: update\status.json), sağlamasını (SHA-256) doğrular
// --update-status   -> update\status.json (idle | downloading | ready | installing | error)
// --update-install  -> indirilen paketi kurar (UAC bir kez sorulur); kurulum bitince masaüstü kendiliğinden açılır
static class Updater
{
    const string Repo = "KaanAlper/logical-lunge";
    static readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    static string Home { get { return Environment.GetFolderPath(Environment.SpecialFolder.UserProfile); } }
    static string Dir
    {
        get
        {
            string d = Paths.DataDir("update");
            System.IO.Directory.CreateDirectory(d);
            return d;
        }
    }
    static string StatusPath { get { return System.IO.Path.Combine(Dir, "status.json"); } }

    public static string Installed()
    {
        try { string f = Paths.Version; if (System.IO.File.Exists(f)) return System.IO.File.ReadAllText(f).Trim(); } catch { }
        try
        {
            var v = Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\LogicalLunge", "DisplayVersion", null) as string;
            if (!string.IsNullOrEmpty(v)) return v.Trim();
        }
        catch { }
        return "0.0.0";
    }

    static Version Ver(string s)
    {
        Version v;
        if (s == null) return new Version(0, 0, 0);
        s = s.Trim().TrimStart('v', 'V');
        int i = s.IndexOfAny(new[] { '-', '+', ' ' });
        if (i > 0) s = s.Substring(0, i);
        return Version.TryParse(s, out v) ? v : new Version(0, 0, 0);
    }

    class Rel { public string Tag, ZipName, ZipUrl, ShaUrl, Notes; public long Size; }

    static string Edition()
    {
        try
        {
            string edition = System.IO.File.ReadAllText(Paths.In("EDITION")).Trim();
            if (edition == "native-ui" || edition == "web-ui") return edition;
        }
        catch { }
        // Upgrade from packages made before EDITION existed. Only the web
        // edition contains a bar widget; prefs.bar alone can be stale.
        try
        {
            var pack = json.DeserializeObject(System.IO.File.ReadAllText(Paths.UiPack("zpack.json"))) as Dictionary<string, object>;
            foreach (Dictionary<string, object> widget in (object[])pack["widgets"])
                if (Convert.ToString(widget["name"]) == "bar") return "web-ui";
        }
        catch { }
        return "native-ui";
    }

    static Rel SelectRelease(System.Collections.IEnumerable releases, string edition)
    {
        if (edition != "native-ui" && edition != "web-ui") throw new ArgumentException("Invalid edition");
        Rel best = null;
        var pattern = new System.Text.RegularExpressions.Regex("^v([0-9]+\\.[0-9]+\\.[0-9]+)-" + edition + "$");
        foreach (var item in releases)
        {
            var r = item as Dictionary<string, object>;
            if (r == null || !r.ContainsKey("tag_name") || !r.ContainsKey("assets")) continue;
            if ((r.ContainsKey("draft") && true.Equals(r["draft"])) || (r.ContainsKey("prerelease") && true.Equals(r["prerelease"]))) continue;
            string tag = Convert.ToString(r["tag_name"]);
            var m = pattern.Match(tag);
            if (!m.Success) continue;
            string name = "LogicalLunge-" + edition + "-" + m.Groups[1].Value + ".zip";
            var rel = new Rel { Tag = tag, ZipName = name, Notes = r.ContainsKey("body") ? Convert.ToString(r["body"]) : "" };
            foreach (Dictionary<string, object> a in (System.Collections.IEnumerable)r["assets"])
            {
                string n = Convert.ToString(a["name"]), u = Convert.ToString(a["browser_download_url"]);
                if (n == name) { rel.ZipUrl = u; rel.Size = Convert.ToInt64(a["size"]); }
                if (n == name + ".sha256") rel.ShaUrl = u;
            }
            if (rel.ZipUrl != null && rel.ShaUrl != null && (best == null || Ver(tag) > Ver(best.Tag))) best = rel;
        }
        return best;
    }

    static System.Net.WebClient Client()
    {
        System.Net.ServicePointManager.SecurityProtocol = (System.Net.SecurityProtocolType)3072; // TLS 1.2
        var c = new System.Net.WebClient();
        c.Headers[System.Net.HttpRequestHeader.UserAgent] = "LogicalLunge-Updater/1.0";
        c.Headers[System.Net.HttpRequestHeader.Accept] = "application/vnd.github+json";
        c.Encoding = Encoding.UTF8;
        return c;
    }

    // Yayınlanmış en son sürüm; henüz sürüm yoksa (404) null ve err = "none"
    static Rel Latest(out string err)
    {
        err = null;
        try
        {
            // LL_UPDATE_API: sınama için başka bir sürüm adresi (varsayılan GitHub)
            string api = Environment.GetEnvironmentVariable("LL_UPDATE_API");
            var releases = new List<object>();
            int page = 1;
            while (true)
            {
                string url = string.IsNullOrEmpty(api) ? "https://api.github.com/repos/" + Repo + "/releases?per_page=100&page=" + page : api;
                object data;
                using (var c = Client()) data = json.DeserializeObject(c.DownloadString(url));
                var batch = data as object[];
                if (batch == null) { releases.Add(data); break; } // single fixture via LL_UPDATE_API
                releases.AddRange(batch);
                if (batch.Length < 100 || !string.IsNullOrEmpty(api)) break;
                page++;
            }
            var rel = SelectRelease(releases, Edition());
            if (rel == null) { err = "none"; return null; }
            return rel;
        }
        catch (System.Net.WebException ex)
        {
            var resp = ex.Response as System.Net.HttpWebResponse;
            err = resp != null && resp.StatusCode == System.Net.HttpStatusCode.NotFound ? "none" : ex.Message;
            return null;
        }
        catch (Exception ex) { err = ex.Message; return null; }
    }

    static string ZipPath(Rel r) { return System.IO.Path.Combine(Dir, r.ZipName); }
    static bool IsReady(Rel r) { return System.IO.File.Exists(ZipPath(r)) && System.IO.File.Exists(ZipPath(r) + ".ok"); }

    static string Clean(string notes)
    {
        if (string.IsNullOrEmpty(notes)) return "";
        var sb = new StringBuilder();
        foreach (var line in notes.Replace("\r", "").Split('\n'))
        {
            string l = line.Trim().TrimStart('#', '*', '-', '>', ' ').Replace("**", "").Replace("`", "");
            if (l.Length == 0) continue;
            if (sb.Length > 0) sb.Append('\n');
            sb.Append(l);
            if (sb.Length > 400) break;
        }
        return sb.ToString();
    }

    public static string Check()
    {
        string cur = Installed(), err;
        var r = Latest(out err);
        var d = new Dictionary<string, object> { { "current", cur } };
        if (r == null)
        {
            d["latest"] = cur; d["available"] = false; d["downloaded"] = false; d["error"] = err == "none" ? "" : err;
            return json.Serialize(d);
        }
        bool avail = Ver(r.Tag) > Ver(cur);
        d["latest"] = Ver(r.Tag).ToString(3); d["tag"] = r.Tag; d["available"] = avail;
        d["downloaded"] = avail && IsReady(r); d["size"] = r.Size; d["notes"] = Clean(r.Notes); d["error"] = "";
        return json.Serialize(d);
    }

    static void SetStatus(string state, string version, long bytes, long total, string error)
    {
        var d = new Dictionary<string, object> { { "state", state }, { "version", version ?? "" }, { "bytes", bytes }, { "total", total }, { "error", error ?? "" } };
        string txt = json.Serialize(d);
        for (int i = 0; i < 5; i++)
        {
            try { System.IO.File.WriteAllText(StatusPath, txt, new UTF8Encoding(false)); return; }
            catch { Thread.Sleep(20); }
        }
    }

    public static string Status()
    {
        try
        {
            if (System.IO.File.Exists(StatusPath))
                using (var s = new System.IO.FileStream(StatusPath, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite))
                using (var rd = new System.IO.StreamReader(s, Encoding.UTF8)) return rd.ReadToEnd();
        }
        catch { }
        return "{\"state\":\"idle\"}";
    }

    public static string Download()
    {
        string err;
        var r = Latest(out err);
        if (r == null || Ver(r.Tag) <= Ver(Installed())) { SetStatus("idle", "", 0, 0, err == "none" ? "" : err); return Status(); }
        string ver = Ver(r.Tag).ToString(3), zip = ZipPath(r), part = zip + ".part";
        if (IsReady(r)) { SetStatus("ready", ver, r.Size, r.Size, ""); return Status(); }
        try
        {
            // older downloads go; a half-finished one of this version stays so that it can continue
            foreach (var f in System.IO.Directory.GetFiles(Dir, "LogicalLunge-*")) { if (f != part) try { System.IO.File.Delete(f); } catch { } }
            System.Net.ServicePointManager.SecurityProtocol = (System.Net.SecurityProtocolType)3072;
            long got = 0, total = r.Size;
            SetStatus("downloading", ver, 0, total, "");
            // A dropped connection continues where it stopped (HTTP Range); five tries with a growing pause
            string failed = null;
            for (int attempt = 1; attempt <= 5; attempt++)
            {
                try
                {
                    got = System.IO.File.Exists(part) ? new System.IO.FileInfo(part).Length : 0;
                    var req = (System.Net.HttpWebRequest)System.Net.WebRequest.Create(r.ZipUrl);
                    req.UserAgent = "LogicalLunge-Updater/1.0";
                    req.AllowAutoRedirect = true;
                    req.Timeout = 30000; req.ReadWriteTimeout = 60000;
                    if (got > 0) req.AddRange(got);
                    using (var resp = (System.Net.HttpWebResponse)req.GetResponse())
                    {
                        if (resp.StatusCode != System.Net.HttpStatusCode.PartialContent) got = 0;
                        if (resp.ContentLength > 0) total = got + resp.ContentLength;
                        using (var s = resp.GetResponseStream())
                        using (var f = new System.IO.FileStream(part, got > 0 ? System.IO.FileMode.Append : System.IO.FileMode.Create))
                        {
                            var buf = new byte[81920];
                            int n;
                            var sw = Stopwatch.StartNew();
                            long lastWrite = 0;
                            while ((n = s.Read(buf, 0, buf.Length)) > 0)
                            {
                                f.Write(buf, 0, n); got += n;
                                if (sw.ElapsedMilliseconds - lastWrite > 150) { lastWrite = sw.ElapsedMilliseconds; SetStatus("downloading", ver, got, total, ""); }
                            }
                        }
                    }
                    if (total > 0 && got < total) throw new System.IO.IOException("the connection closed at " + got + " of " + total + " bytes");
                    failed = null;
                    break;
                }
                catch (Exception ex)
                {
                    failed = ex.GetBaseException().Message;
                    var wex = ex as System.Net.WebException;
                    var code = wex != null && wex.Response is System.Net.HttpWebResponse ? (int)((System.Net.HttpWebResponse)wex.Response).StatusCode : 0;
                    if (code == 416) { try { System.IO.File.Delete(part); } catch { } }
                    if (code == 403 || code == 404 || code == 410 || attempt == 5) break;
                    SetStatus("downloading", ver, got, total, "");
                    System.Threading.Thread.Sleep(2000 * attempt);
                }
            }
            if (failed != null) throw new Exception(failed);
            if (r.ShaUrl != null)
            {
                string raw = Client().DownloadString(r.ShaUrl);
                var parts = raw.Split(new[] { ' ', '\t', '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries);
                string expected = parts.Length > 0 ? parts[0].ToUpperInvariant() : "";
                string actual;
                using (var sha = System.Security.Cryptography.SHA256.Create())
                using (var fs = System.IO.File.OpenRead(part)) actual = BitConverter.ToString(sha.ComputeHash(fs)).Replace("-", "");
                if (expected.Length != 64 || expected != actual) { try { System.IO.File.Delete(part); } catch { } throw new Exception("Sağlama toplamı uyuşmuyor, indirme geçersiz."); }
            }
            System.IO.File.Move(part, zip);
            System.IO.File.WriteAllText(zip + ".ok", r.Tag);
            SetStatus("ready", ver, total, total, "");
        }
        catch (Exception ex) { SetStatus("error", ver, 0, 0, ex.GetBaseException().Message); }
        return Status();
    }

    public static string Install()
    {
        try
        {
            string zip = null;
            foreach (var f in System.IO.Directory.GetFiles(Dir, "LogicalLunge-*.zip"))
                if (System.IO.File.Exists(f + ".ok") && (zip == null || System.IO.File.GetLastWriteTime(f) > System.IO.File.GetLastWriteTime(zip))) zip = f;
            if (zip == null) throw new Exception("İndirilmiş güncelleme bulunamadı.");
            string script = Paths.Script("update-install.ps1");
            if (!System.IO.File.Exists(script)) throw new Exception("update-install.ps1 bulunamadı.");
            // Kurulum betik klasörünü değiştirir: kendi kopyasından çalıştır
            string copy = System.IO.Path.Combine(Dir, "update-install.ps1");
            System.IO.File.Copy(script, copy, true);
            SetStatus("installing", System.IO.Path.GetFileNameWithoutExtension(zip).Replace("LogicalLunge-", ""), 0, 0, "");
            // ShellExecute: kabuğun başlattığı bu süreç onun soketlerini miras almış olabilir; dakikalarca süren kurulum
            // betiği onları taşırsa yeni kabuk sunucusunu açamıyor
            Process.Start(new ProcessStartInfo("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"" + copy + "\" -Zip \"" + zip + "\"")
                { UseShellExecute = true, WindowStyle = ProcessWindowStyle.Hidden, WorkingDirectory = Paths.Home });
        }
        catch (Exception ex) { SetStatus("error", "", 0, 0, ex.GetBaseException().Message); }
        return Status();
    }
}

// ---------------- Alt+Tab pencere değiştirici ----------------
// Super arama menüsü ve workspace önizlemesi gibi ii görünümünde: koyu yuvarlak panel, canlı DWM önizlemeli kartlar,
// seçili kart mor vurgulu. Tüm workspace'lerdeki pencereler (tiling) son kullanıma göre sıralı; Alt basılı tutulup
// Tab ile ilerlenir (Shift+Tab geri, ok tuşları, Enter, Esc iptal, fare ile tık), Alt bırakılınca seçilen pencere açılır.
// Arayüz lunge içinde çizilir (WebView yok): yük altında bile anında açılır.
class Switcher : Form
{
    public static volatile bool Active;
    static bool demo; // sınama: Alt basılı tutulmaz
    static Switcher inst;
    static Control ui;
    static readonly List<long> fgOrder = new List<long>(); // en yeni önde (Windows'un ön plan değişimlerinden)
    static Native.WinEventDelegate fgCb;
    const int VK_MENU = 0x12, VK_TAB = 0x09;
    const byte VK_DUMMY = 0xE8;

    class Card
    {
        public string Id, Title, Proc, Ws; public IntPtr H; public IntPtr Thumb; public Rectangle R, ThumbR; public Image Icon; public bool Min, OtherWs;
    }

    readonly List<Card> cards = new List<Card>();
    int sel;
    RectangleF hi, hiTarget;
    readonly TilingClient tiling = new TilingClient();
    readonly System.Windows.Forms.Timer anim = new System.Windows.Forms.Timer { Interval = 15 };
    readonly System.Windows.Forms.Timer altWatch = new System.Windows.Forms.Timer { Interval = 40 };
    int animStart, fadeStart;
    bool committed;
    string curWs;
    static readonly Dictionary<string, Image> iconCache = new Dictionary<string, Image>();

    const int CW = 232, CH = 170, GAP = 12, PAD = 20, TH_W = 212, TH_H = 118, RADIUS = 24;
    static readonly Color Surface = Color.FromArgb(20, 18, 24), Border = Color.FromArgb(58, 56, 66),
        SelFill = Color.FromArgb(79, 55, 139), SelBorder = Color.FromArgb(208, 188, 255), TxtColor = Color.FromArgb(230, 224, 233),
        SubColor = Color.FromArgb(202, 196, 208), CardFill = Color.FromArgb(34, 32, 40);
    readonly Font titleFont = new Font("Segoe UI", 9.5f, FontStyle.Regular), badgeFont = new Font("Segoe UI", 8.5f, FontStyle.Bold);

    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRoundRectRgn(int l, int t, int r, int b, int w, int hh);

    Switcher()
    {
        FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true; StartPosition = FormStartPosition.Manual;
        BackColor = Surface; DoubleBuffered = true; Opacity = 0; Text = "lunge-switcher";
        anim.Tick += (o, e) => Tick();
        altWatch.Tick += (o, e) =>
        {
            // Alt bırakıldı ama kanca olayını kaçırdıysa yine de seçimi uygula
            if (!demo && Active && (Native.GetAsyncKeyState(VK_MENU) & 0x8000) == 0) { Slider.Log("switcher: alt bırakılmış görüldü (40 ms yoklama) -> seçileni aç"); CommitCurrent(); }
        };
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override CreateParams CreateParams
    {
        get { var p = base.CreateParams; p.ExStyle |= 0x80 | 0x08000000 | 0x8; return p; } // TOOLWINDOW | NOACTIVATE | TOPMOST
    }

    // ---- kurulum: UI thread'inde bir kez ----
    public static void Init(Control uiCtl)
    {
        ui = uiCtl;
        ui.BeginInvoke((Action)(() =>
        {
            inst = new Switcher();
            inst.CreateControl(); var h = inst.Handle;
            fgCb = Callback.Guard("alt-tab olayı", OnForeground);
            Native.SetWinEventHook(Native.EVENT_SYSTEM_FOREGROUND, Native.EVENT_SYSTEM_FOREGROUND, IntPtr.Zero, fgCb, 0, 0, 0x0002);
            IntPtr cur = Native.GetAncestor(Native.GetForegroundWindow(), 2);
            if (cur != IntPtr.Zero) fgOrder.Add(cur.ToInt64());
        }));
    }

    static void OnForeground(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        EventLag.Note("alt-tab", time);
        try
        {
            IntPtr root = Native.GetAncestor(hwnd, 2);
            if (root == IntPtr.Zero || (inst != null && root == inst.Handle)) return;
            long k = root.ToInt64();
            fgOrder.Remove(k); fgOrder.Insert(0, k);
            if (fgOrder.Count > 200) fgOrder.RemoveRange(200, fgOrder.Count - 200);
        }
        catch { }
    }

    // ---- kanca (Keys2.Hook, kendi thread'inde) ----
    // true: tuş yutulur. Yalnızca bayrak ve kuyruk işi yapar; ağır iş UI thread'inde.
    public static bool HandleKey(int vk, bool isDown, bool isUp, bool shift, bool altDown, bool winOrCtrl)
    {
        if (inst == null) return false;
        if (!Active)
        {
            if (isDown && vk == VK_TAB && altDown && !winOrCtrl)
            {
                // Özel tam ekran oyun önde: değiştirici oyunun arkasında açılıp görünmüyordu, Alt bırakılınca oyun ancak o
                // zaman tam ekrandan çıkıyordu. Windows'un Alt+Tab'ı oyunu kendi yoluyla küçültür ve seçiciyi gösterir.
                if (WidgetWindows.ExclusiveFullscreen()) { ThreadPool.QueueUserWorkItem(_ => Slider.Log("switcher: özel tam ekran uygulama önde, Alt+Tab Windows'a bırakıldı")); return false; }
                Active = true;
                bool rev = shift;
                ui.BeginInvoke((Action)(() => inst.Open(rev)));
                return true;
            }
            return false;
        }
        // Alt bırakıldı: seçimi uygula (Alt olayı sisteme geçer; sahte tuş menü çubuğunu etkinleştirmesin diye araya girer)
        if (isUp && (vk == VK_MENU || vk == 0xA4 || vk == 0xA5))
        {
            ThreadPool.QueueUserWorkItem(_ => Slider.Log("switcher: alt bırakıldı -> seçileni aç"));
            Native.keybd_event(VK_DUMMY, 0, 0, Native.LL_MARK); Native.keybd_event(VK_DUMMY, 0, 2, Native.LL_MARK);
            ui.BeginInvoke((Action)(() => inst.CommitCurrent()));
            return false;
        }
        // Alt basılı değilken menü açık kalmış olamaz (kaçırılan bırakma olayı): kilitlenme olmasın, tuşu geçir
        if (!altDown && !demo)
        {
            ThreadPool.QueueUserWorkItem(_ => Slider.Log("switcher: tuş 0x" + vk.ToString("X") + " geldi ama alt basılı görünmüyor -> kapandı"));
            Active = false; ui.BeginInvoke((Action)(() => inst.CloseOnly())); return false;
        }
        if (isDown)
        {
            if (vk == VK_TAB) { bool rev = shift; ui.BeginInvoke((Action)(() => inst.Move(rev ? -1 : 1, 0))); return true; }
            if (vk == 0x27) { ui.BeginInvoke((Action)(() => inst.Move(1, 0))); return true; }   // sağ
            if (vk == 0x25) { ui.BeginInvoke((Action)(() => inst.Move(-1, 0))); return true; }  // sol
            if (vk == 0x28) { ui.BeginInvoke((Action)(() => inst.Move(0, 1))); return true; }   // aşağı
            if (vk == 0x26) { ui.BeginInvoke((Action)(() => inst.Move(0, -1))); return true; }  // yukarı
            if (vk == 0x0D) { ui.BeginInvoke((Action)(() => inst.CommitCurrent())); return true; }
            if (vk == 0x1B) { ui.BeginInvoke((Action)(() => inst.CloseOnly())); return true; }
            return true; // menü açıkken diğer tuşlar uygulamaya gitmesin
        }
        return isUp && (vk == VK_TAB || vk == 0x27 || vk == 0x25 || vk == 0x28 || vk == 0x26 || vk == 0x0D || vk == 0x1B);
    }

    // ---- pencere listesi ----
    List<Card> Collect()
    {
        var list = new List<Card>();
        try
        {
            foreach (var m in tiling.Monitors())
                foreach (Dictionary<string, object> ws in J.Children(m))
                {
                    string wsName = J.Str(ws, "name");
                    bool shown = J.Bool(ws, "isDisplayed");
                    if (J.Bool(ws, "hasFocus")) curWs = wsName;
                    var wins = new List<Dictionary<string, object>>();
                    J.WindowNodes(ws, wins);
                    foreach (var w in wins)
                    {
                        object hv; if (!w.TryGetValue("handle", out hv) || hv == null) continue;
                        var h = new IntPtr(Convert.ToInt64(hv));
                        if (!Native.IsWindow(h)) continue;
                        var c = new Card { Id = J.Str(w, "id"), H = h, Title = J.Str(w, "title"), Proc = J.Str(w, "processName"), Ws = wsName, OtherWs = !shown };
                        c.Min = IsIconic(h);
                        if (string.IsNullOrEmpty(c.Title)) c.Title = c.Proc;
                        list.Add(c);
                    }
                }
        }
        catch (Exception ex) { Slider.Log("switcher list: " + ex.Message); }
        // son kullanılan önde (Windows'un Alt+Tab sırası)
        list.Sort((a, b) =>
        {
            int ia = fgOrder.IndexOf(a.H.ToInt64()), ib = fgOrder.IndexOf(b.H.ToInt64());
            if (ia < 0) ia = int.MaxValue; if (ib < 0) ib = int.MaxValue;
            return ia.CompareTo(ib);
        });
        return list;
    }

    // Simge önbelleği uygulama yoluna göre; gün boyu açılan her yeni uygulama bir girdi ekliyordu. Sınırı aşınca
    // (kartlar bırakılmışken, açılışın başında) tamamen boşalır, gösterilenler yeniden okunur.
    const int ICON_CACHE_MAX = 96;
    static void TrimIcons()
    {
        if (iconCache.Count <= ICON_CACHE_MAX) return;
        foreach (var img in iconCache.Values) { try { img.Dispose(); } catch { } }
        iconCache.Clear();
    }

    static Image IconFor(IntPtr h)
    {
        try
        {
            uint pid; Native.GetWindowThreadProcessId(h, out pid);
            string path = ProcInfo.Path(pid);
            if (path == null) return null;
            Image img;
            if (iconCache.TryGetValue(path, out img)) return img;
            using (var ic = Icon.ExtractAssociatedIcon(path)) using (var bm = ic.ToBitmap()) img = new Bitmap(bm, new Size(22, 22));
            iconCache[path] = img;
            return img;
        }
        catch { return null; }
    }

    // ---- açma / kapama (UI thread) ----
    void Open(bool reverse)
    {
        try
        {
            committed = false;
            Release();
            cards.Clear();
            TrimIcons();
            cards.AddRange(Collect());
            Slider.Log("switcher: " + cards.Count + " pencere" + (reverse ? " (geri)" : ""));
            if (cards.Count == 0) { Active = false; return; }
            foreach (var c in cards) c.Icon = IconFor(c.H);
            sel = cards.Count > 1 ? (reverse ? cards.Count - 1 : 1) : 0;
            lastMouse = Cursor.Position; // açılırken imlecin altındaki kart seçilmez

            var mon = Screen.FromPoint(Cursor.Position).Bounds;
            int perRow = Math.Max(1, Math.Min(cards.Count, (int)((mon.Width * 0.9 - 2 * PAD + GAP) / (CW + GAP))));
            int rows = (cards.Count + perRow - 1) / perRow;
            int w = perRow * (CW + GAP) - GAP + 2 * PAD, h = rows * (CH + GAP) - GAP + 2 * PAD;
            Bounds = new Rectangle(mon.X + (mon.Width - w) / 2, mon.Y + (mon.Height - h) / 2, w, h);
            var rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, RADIUS * 2, RADIUS * 2);
            Native.SetWindowRgn(Handle, rgn, false);
            for (int i = 0; i < cards.Count; i++)
            {
                int cx = PAD + (i % perRow) * (CW + GAP), cy = PAD + (i / perRow) * (CH + GAP);
                cards[i].R = new Rectangle(cx, cy, CW, CH);
            }
            Show();
            Native.SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
            foreach (var c in cards) RegisterThumb(c);
            hi = hiTarget = drawHi = Inflate(cards[sel].R);
            fadeStart = animStart = Environment.TickCount;
            Opacity = 0;
            anim.Start(); altWatch.Start();
            Invalidate();
        }
        catch (Exception ex) { Slider.Log("switcher open: " + ex.Message); Active = false; }
    }

    static RectangleF Inflate(Rectangle r) { return new RectangleF(r.X - 3, r.Y - 3, r.Width + 6, r.Height + 6); }

    void RegisterThumb(Card c)
    {
        var thumbArea = new Rectangle(c.R.X + (CW - TH_W) / 2, c.R.Y + 12, TH_W, TH_H);
        c.ThumbR = thumbArea;
        if (c.Min) return; // küçültülmüş pencerenin yüzeyi yok: büyük simge çizilir
        IntPtr id;
        if (Native.DwmRegisterThumbnail(Handle, c.H, out id) != 0) return;
        Native.SIZE src;
        if (Native.DwmQueryThumbnailSourceSize(id, out src) != 0 || src.cx <= 0 || src.cy <= 0) { Native.DwmUnregisterThumbnail(id); return; }
        double k = Math.Min((double)TH_W / src.cx, (double)TH_H / src.cy);
        int tw = Math.Max(1, (int)(src.cx * k)), th = Math.Max(1, (int)(src.cy * k));
        var d = new Native.RECT { Left = thumbArea.X + (TH_W - tw) / 2, Top = thumbArea.Y + (TH_H - th) / 2 };
        d.Right = d.Left + tw; d.Bottom = d.Top + th;
        var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_VISIBLE | Native.DWM_TNP_OPACITY, rcDestination = d, opacity = 255, fVisible = true };
        Native.DwmUpdateThumbnailProperties(id, ref pr);
        c.Thumb = id;
    }

    void Release()
    {
        foreach (var c in cards) if (c.Thumb != IntPtr.Zero) { Native.DwmUnregisterThumbnail(c.Thumb); c.Thumb = IntPtr.Zero; }
    }

    void Move(int dx, int dy)
    {
        if (!Visible || cards.Count == 0) return;
        int perRow = Math.Max(1, (ClientSize.Width - 2 * PAD + GAP) / (CW + GAP));
        int n = cards.Count;
        int from = sel;
        if (dy != 0) { int t = sel + dy * perRow; if (t >= 0 && t < n) sel = t; }
        else sel = ((sel + dx) % n + n) % n;
        Slider.Log("switcher: ilerle " + (dx != 0 ? dx : dy * perRow) + ": " + from + " -> " + sel + " / " + n);
        hi = (Environment.TickCount - animStart < 170) ? drawHi : hiTarget;
        hiTarget = Inflate(cards[sel].R);
        animStart = Environment.TickCount;
        Invalidate();
    }

    void Tick()
    {
        int now = Environment.TickCount;
        double f = Math.Min(1.0, (now - fadeStart) / 140.0);
        if (!committed) Opacity = 0.97 * (1 - Math.Pow(1 - f, 3));
        double t = Math.Min(1.0, (now - animStart) / 170.0), e = 1 - Math.Pow(1 - t, 3);
        var cur = new RectangleF(hi.X + (hiTarget.X - hi.X) * (float)e, hi.Y + (hiTarget.Y - hi.Y) * (float)e, hi.Width + (hiTarget.Width - hi.Width) * (float)e, hi.Height + (hiTarget.Height - hi.Height) * (float)e);
        if (t >= 1.0) hi = hiTarget; else drawHi = cur;
        if (t < 1.0) Invalidate();
    }
    RectangleF drawHi;

    public void CloseOnly()
    {
        committed = true; Active = false;
        anim.Stop(); altWatch.Stop();
        Release();
        Hide();
        Opacity = 0;
    }

    void CommitCurrent()
    {
        if (committed) return;
        var target = sel >= 0 && sel < cards.Count ? cards[sel] : null;
        string ws = curWs;
        CloseOnly();
        if (target == null) return;
        Activate(target, ws);
    }

    void Activate(Card c, string fromWs)
    {
        try
        {
            if (c.Min) Native.ShowWindow(c.H, 9); // SW_RESTORE
            if (c.Ws != null && c.Ws != fromWs && !string.IsNullOrEmpty(c.Ws))
            {
                // başka workspace: önce animasyonlu geçiş, sonra pencereyi odakla
                var k = Slider.Ui;
                tiling.Command("focus --workspace " + c.Ws);
                ThreadPool.QueueUserWorkItem(_ => { Thread.Sleep(120); try { tiling.Command("focus --container-id " + c.Id); } catch { } });
            }
            else tiling.Command("focus --container-id " + c.Id);
        }
        catch (Exception ex) { Slider.Log("switcher activate: " + ex.Message); }
    }

    // ---- çizim ----
    // Yuvarlak dikdörtgen: yol her çizimde serbest bırakılır (animasyonda karede birkaç GDI+ nesnesi birikiyordu)
    static void FillRound(Graphics g, Brush b, RectangleF r, float rad) { using (var rp = new GraphicsPathHelper(r, rad)) g.FillPath(b, rp.Path); }
    static void DrawRound(Graphics g, Pen p, RectangleF r, float rad) { using (var rp = new GraphicsPathHelper(r, rad)) g.DrawPath(p, rp.Path); }

    protected override void OnPaint(PaintEventArgs e)
    {
        try { PaintBody(e); } catch (Exception ex) { PaintErrors.Report("alt-tab", ex); }
    }
    void PaintBody(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.ClearTypeGridFit;
        g.Clear(Surface);
        using (var bp = new Pen(Border, 1.5f)) DrawRound(g, bp, new RectangleF(0.75f, 0.75f, Width - 2f, Height - 2f), RADIUS);
        var hl = (Environment.TickCount - animStart < 170) ? drawHi : hiTarget;
        if (hl.Width > 0 && cards.Count > 0)
        {
            using (var b = new SolidBrush(SelFill)) FillRound(g, b, hl, 18);
            using (var p = new Pen(SelBorder, 2f)) DrawRound(g, p, hl, 18);
        }
        for (int i = 0; i < cards.Count; i++)
        {
            var c = cards[i];
            var inner = new RectangleF(c.R.X, c.R.Y, c.R.Width, c.R.Height);
            if (i != sel) using (var b = new SolidBrush(CardFill)) FillRound(g, b, inner, 16);
            // önizleme yuvası
            var slot = new RectangleF(c.ThumbR.X, c.ThumbR.Y, c.ThumbR.Width, c.ThumbR.Height);
            using (var b = new SolidBrush(Color.FromArgb(24, 22, 28))) FillRound(g, b, slot, 10);
            if (c.Min || c.Thumb == IntPtr.Zero)
            {
                if (c.Icon != null) g.DrawImage(c.Icon, slot.X + slot.Width / 2 - 20, slot.Y + slot.Height / 2 - 20, 40, 40);
            }
            // başlık satırı: simge + ad
            int ty = c.R.Y + 12 + TH_H + 10;
            int tx = c.R.X + 14;
            if (c.Icon != null) { g.DrawImage(c.Icon, tx, ty, 20, 20); tx += 26; }
            var rect = new RectangleF(tx, ty, c.R.Right - 12 - tx, 22);
            using (var sf = new StringFormat { Trimming = StringTrimming.EllipsisCharacter, FormatFlags = StringFormatFlags.NoWrap, LineAlignment = StringAlignment.Center })
            using (var br = new SolidBrush(i == sel ? Color.White : TxtColor)) g.DrawString(c.Title, titleFont, br, rect, sf);
            // başka workspace'teki pencere: köşede numara rozeti
            if (c.OtherWs && !string.IsNullOrEmpty(c.Ws))
            {
                var bd = new RectangleF(c.R.Right - 36, c.R.Y + 16, 24, 24);
                using (var b = new SolidBrush(Color.FromArgb(208, 188, 255))) g.FillEllipse(b, bd);
                using (var sf = new StringFormat { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center })
                using (var br = new SolidBrush(Color.FromArgb(56, 30, 114))) g.DrawString(c.Ws, badgeFont, br, bd, sf);
            }
        }
    }

    // Seçim yalnızca fare gerçekten hareket edince değişir. Windows fare kıpırdamadan da WM_MOUSEMOVE gönderir (pencere
    // belirince, z-sırası ya da altındaki pencereler değişince): imleç bir kartın üstünde duruyorsa seçim her Tab'dan sonra
    // o karta geri çekiliyordu ("ilerle 3 -> 4" tekrar tekrar).
    Point lastMouse;

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        Point p = Cursor.Position;
        if (p == lastMouse) return;
        lastMouse = p;
        for (int i = 0; i < cards.Count; i++)
            if (cards[i].R.Contains(e.Location) && i != sel) { sel = i; hi = (Environment.TickCount - animStart < 170) ? drawHi : hiTarget; hiTarget = Inflate(cards[i].R); animStart = Environment.TickCount; Invalidate(); break; }
    }
    protected override void OnMouseUp(MouseEventArgs e)
    {
        base.OnMouseUp(e);
        for (int i = 0; i < cards.Count; i++)
            if (cards[i].R.Contains(e.Location)) { sel = i; CommitCurrent(); return; }
    }

    // sınama: --switcher-demo
    public static void Demo()
    {
        var f = new Form { ShowInTaskbar = false, WindowState = FormWindowState.Minimized, FormBorderStyle = FormBorderStyle.None, Opacity = 0 };
        f.Load += (s, e) => f.Hide();
        var h = f.Handle;
        demo = true;
        Init(f);
        var t = new System.Windows.Forms.Timer { Interval = 300 };
        t.Tick += (s, e) => { t.Stop(); Active = true; inst.Open(false); };
        t.Start();
        var end = new System.Windows.Forms.Timer { Interval = 6000 };
        end.Tick += (s, e) => { inst.CloseOnly(); Application.ExitThread(); };
        end.Start();
        Application.Run(f);
    }
}

// Yuvarlak köşeli dikdörtgen yolu (Graphics.FillPath için)
// Çizim hatası: pencere bozuk işaretlenmesin (WinForms, OnPaint'ten çıkan istisnadan sonra süreç kapanana kadar içerik
// yerine beyaz zemin ve kırmızı X çizer). Hata yazılır (aynı yer için dakikada en fazla bir kez), sonraki çizim yeniden dener.
static class PaintErrors
{
    static readonly Dictionary<string, int> last = new Dictionary<string, int>();
    public static void Report(string who, Exception ex)
    {
        lock (last)
        {
            int t;
            if (last.TryGetValue(who, out t) && Environment.TickCount - t < 60000) return;
            last[who] = Environment.TickCount;
        }
        Slider.Log("çizim hatası (" + who + "): " + ex.GetType().Name + ": " + ex.Message);
    }
}

class GraphicsPathHelper : IDisposable
{
    public System.Drawing.Drawing2D.GraphicsPath Path = new System.Drawing.Drawing2D.GraphicsPath();
    public GraphicsPathHelper(RectangleF r, float rad)
    {
        float d = Math.Min(rad * 2, Math.Min(r.Width, r.Height));
        // Boyutu 0 (henüz yerleşmemiş kart) ya da köşesiz: GDI+ 0 çaplı yayda hata fırlatıyordu, hata çizimden dışarı
        // çıkınca WinForms pencereyi kalıcı olarak "bozuk" (beyaz zemin, kırmızı X) çiziyordu
        if (r.Width <= 0 || r.Height <= 0) return;
        if (d < 1) { Path.AddRectangle(r); return; }
        Path.AddArc(r.X, r.Y, d, d, 180, 90);
        Path.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        Path.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        Path.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
        Path.CloseFigure();
    }
    public void Dispose() { Path.Dispose(); }
}

// ---------------- Duvar kağıdı (sağ panel > Duvar kağıtları) ----------------
// Arama menüsündeki bir uygulamanın Windows sağ tık menüsü (Başlat menüsündekiyle aynı: dosya konumunu aç, yönetici
// olarak çalıştır, sabitle, kaldır ...): lunge.exe --shell-menu <ayrıştırma adı, ör. shell:AppsFolder\kimlik>.
// Kabuktan yetkisiz başlatılır: menüden açılanlar da yetkisiz açılsın (çekirdek yönetici haklarıyla çalışır). Fare
// imlecinin yerinde, LL temasının renginde açılır; Shift basılıysa genişletilmiş komutlarla (Explorer'daki gibi).
// Sonucu hemen stdout'a tek satır yazar ({"invoked":true|false}); iptal edilirse odağı menüden önceki pencereye
// (arama menüsü) geri verir. Seçilen komut bu süreçte pencere açtıysa (Özellikler) o kapanana dek süreç yaşar.
static class ShellMenu
{
    [ComImport, Guid("43826d1e-e718-42ee-bc55-a1e261c37bfe"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellItem
    {
        [PreserveSig] int BindToHandler(IntPtr pbc, [In] ref Guid bhid, [In] ref Guid riid, out IntPtr ppv);
        void GetParent(out IShellItem ppsi);
        void GetDisplayName(uint sigdnName, out IntPtr ppszName);
        void GetAttributes(uint sfgaoMask, out uint psfgaoAttribs);
        void Compare(IShellItem psi, uint hint, out int piOrder);
    }

    [ComImport, Guid("000214e4-0000-0000-c000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IContextMenu
    {
        [PreserveSig] int QueryContextMenu(IntPtr hmenu, uint indexMenu, uint idCmdFirst, uint idCmdLast, uint uFlags);
        [PreserveSig] int InvokeCommand(ref CMINVOKECOMMANDINFOEX pici);
        [PreserveSig] int GetCommandString(UIntPtr idCmd, uint uType, IntPtr reserved, IntPtr pszName, uint cchMax);
    }

    [ComImport, Guid("000214f4-0000-0000-c000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IContextMenu2
    {
        [PreserveSig] int QueryContextMenu(IntPtr hmenu, uint indexMenu, uint idCmdFirst, uint idCmdLast, uint uFlags);
        [PreserveSig] int InvokeCommand(ref CMINVOKECOMMANDINFOEX pici);
        [PreserveSig] int GetCommandString(UIntPtr idCmd, uint uType, IntPtr reserved, IntPtr pszName, uint cchMax);
        [PreserveSig] int HandleMenuMsg(uint uMsg, IntPtr wParam, IntPtr lParam);
    }

    [ComImport, Guid("bcfce0a0-ec17-11d0-8d10-00a0c90f2719"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IContextMenu3
    {
        [PreserveSig] int QueryContextMenu(IntPtr hmenu, uint indexMenu, uint idCmdFirst, uint idCmdLast, uint uFlags);
        [PreserveSig] int InvokeCommand(ref CMINVOKECOMMANDINFOEX pici);
        [PreserveSig] int GetCommandString(UIntPtr idCmd, uint uType, IntPtr reserved, IntPtr pszName, uint cchMax);
        [PreserveSig] int HandleMenuMsg(uint uMsg, IntPtr wParam, IntPtr lParam);
        [PreserveSig] int HandleMenuMsg2(uint uMsg, IntPtr wParam, IntPtr lParam, out IntPtr plResult);
    }

    [StructLayout(LayoutKind.Sequential)]
    struct CMINVOKECOMMANDINFOEX
    {
        public int cbSize; public uint fMask; public IntPtr hwnd; public IntPtr lpVerb; public IntPtr lpParameters; public IntPtr lpDirectory;
        public int nShow; public uint dwHotKey; public IntPtr hIcon; public IntPtr lpTitle; public IntPtr lpVerbW; public IntPtr lpParametersW;
        public IntPtr lpDirectoryW; public IntPtr lpTitleW; public Native.POINT ptInvoke;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern int SHCreateItemFromParsingName(string path, IntPtr pbc, [In] ref Guid riid, out IShellItem item);
    [DllImport("user32.dll")] static extern IntPtr CreatePopupMenu();
    [DllImport("user32.dll")] static extern bool DestroyMenu(IntPtr h);
    [DllImport("user32.dll")] static extern uint TrackPopupMenuEx(IntPtr hmenu, uint flags, int x, int y, IntPtr hwnd, IntPtr tpm);
    // Menülerin koyu / aydınlık çizimi (uxtheme, 1903+; adı yok, sıra numarasıyla)
    [DllImport("uxtheme.dll", EntryPoint = "#135")] static extern int SetPreferredAppMode(int mode);
    [DllImport("uxtheme.dll", EntryPoint = "#136")] static extern void FlushMenuThemes();

    static readonly Guid BHID_SFUIObject = new Guid("3981e225-f559-11d3-8e3a-00c04f6837d5");
    const uint First = 1, Last = 0x7fff;
    // Kabuk menüsünün kimlik aralığının dışında: Super menüsünden Dock'a ekleme
    const uint DockItem = 0x8001;
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool AppendMenu(IntPtr h, uint flags, UIntPtr id, string text);

    // Alt menüler (Birlikte aç, Gönder) içeriklerini sahip pencereye gelen bu iletilerle doldurur ve çizer
    sealed class Owner : NativeWindow
    {
        public IContextMenu2 Cm2;
        public IContextMenu3 Cm3;
        protected override void WndProc(ref Message m)
        {
            if (m.Msg == 0x117 || m.Msg == 0x2c || m.Msg == 0x2b || m.Msg == 0x120) // INITMENUPOPUP, MEASUREITEM, DRAWITEM, MENUCHAR
            {
                try
                {
                    IntPtr res;
                    if (Cm3 != null && Cm3.HandleMenuMsg2((uint)m.Msg, m.WParam, m.LParam, out res) == 0) { m.Result = res; return; }
                    if (Cm2 != null && Cm2.HandleMenuMsg((uint)m.Msg, m.WParam, m.LParam) == 0) { m.Result = IntPtr.Zero; return; }
                }
                catch { }
            }
            base.WndProc(ref m);
        }
    }

    // dock: uygulamanın exe adı (Super menüsü apps.json'dan verir); varsa menünün başında "Dock'ta tut / Dock'tan kaldır"
    public static void Run(string path, string dock)
    {
        var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false)) { AutoFlush = true };
        try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
        IntPtr prev = Native.GetForegroundWindow();
        bool invoked = false, dockOnly = false;
        if (dock != null && (dock.Length > 64 || dock.IndexOfAny(new[] { '\\', '/', ':', '"' }) >= 0)) dock = null;
        Owner owner = null;
        IntPtr menu = IntPtr.Zero;
        IContextMenu cm = null;
        try
        {
            object th;
            try { SetPreferredAppMode(Prefs.Read().TryGetValue("theme", out th) && "light".Equals(th) ? 3 : 2); FlushMenuThemes(); } catch { }
            Guid iidItem = typeof(IShellItem).GUID, bhid = BHID_SFUIObject, iidCm = typeof(IContextMenu).GUID;
            IShellItem item;
            if (SHCreateItemFromParsingName(path, IntPtr.Zero, ref iidItem, out item) != 0 || item == null) return;
            IntPtr ppv;
            if (item.BindToHandler(IntPtr.Zero, ref bhid, ref iidCm, out ppv) != 0 || ppv == IntPtr.Zero) return;
            try { cm = (IContextMenu)Marshal.GetObjectForIUnknown(ppv); } finally { Marshal.Release(ppv); }
            owner = new Owner { Cm2 = cm as IContextMenu2, Cm3 = cm as IContextMenu3 };
            owner.CreateHandle(new CreateParams { Caption = "lunge-shell-menu", Style = unchecked((int)0x80000000), ExStyle = 0x80 }); // WS_POPUP, TOOLWINDOW; görünmez
            menu = CreatePopupMenu();
            uint at = 0;
            if (dock != null)
            {
                AppendMenu(menu, 0 /*MF_STRING*/, new UIntPtr(DockItem), I18n.T(Pins.Dock.Contains(dock) ? "Dock’tan kaldır" : "Dock’ta tut"));
                AppendMenu(menu, 0x800 /*MF_SEPARATOR*/, UIntPtr.Zero, null);
                at = 2;
            }
            uint flags = (Control.ModifierKeys & Keys.Shift) != 0 ? 0x100u : 0u; // CMF_EXTENDEDVERBS
            if (cm.QueryContextMenu(menu, at, First, Last, flags) < 0) return;
            var pt = Cursor.Position;
            // Menü dışına tıklanınca kapanması için sahip pencere ön planda olmalı; sonra WM_NULL (TrackPopupMenu belgesi)
            Native.SetForegroundWindow(owner.Handle);
            uint cmd = TrackPopupMenuEx(menu, 0x100 | 0x2, pt.X, pt.Y, owner.Handle, IntPtr.Zero); // RETURNCMD, RIGHTBUTTON
            Native.PostMessage(owner.Handle, 0, IntPtr.Zero, IntPtr.Zero);
            if (cmd == DockItem)
            {
                invoked = true;
                dockOnly = true;
                so.WriteLine("{\"invoked\":true}");
                bool on = !Pins.Dock.Contains(dock);
                // Çalışan çekirdek yazar ve Dock'a haber verir (ll:dock-pins); çekirdek yoksa dosyaya doğrudan
                if (Supervisor.PostToCore("/dock-pin?id=" + Uri.EscapeDataString(dock) + "&on=" + (on ? "1" : "0"), 2000) != 204) Pins.Dock.Set(dock, on);
            }
            else if (cmd >= First && cmd <= Last)
            {
                invoked = true;
                so.WriteLine("{\"invoked\":true}");
                var ci = new CMINVOKECOMMANDINFOEX
                {
                    cbSize = Marshal.SizeOf(typeof(CMINVOKECOMMANDINFOEX)),
                    fMask = 0x4000 | 0x20000000 | 0x100, // UNICODE, PTINVOKE, NOASYNC (süreç komut bitmeden çıkmasın)
                    hwnd = owner.Handle,
                    lpVerb = new IntPtr(cmd - First),
                    lpVerbW = new IntPtr(cmd - First),
                    nShow = 1, // SW_SHOWNORMAL
                    ptInvoke = new Native.POINT { X = pt.X, Y = pt.Y },
                };
                int hr = cm.InvokeCommand(ref ci);
                if (hr < 0) Slider.Log("sağ tık menüsü: komut çalışmadı (0x" + hr.ToString("x8") + "): " + path);
            }
        }
        catch (Exception ex) { Slider.Log("sağ tık menüsü: " + ex.GetBaseException().Message + ": " + path); }
        finally
        {
            if (!invoked)
            {
                try { so.WriteLine("{\"invoked\":false}"); } catch { }
                if (prev != IntPtr.Zero) Native.SetForegroundWindow(prev);
            }
        }
        try { if (invoked && !dockOnly) WaitForOwnWindows(owner == null ? IntPtr.Zero : owner.Handle); } catch { }
        if (menu != IntPtr.Zero) DestroyMenu(menu);
        if (owner != null) owner.DestroyHandle();
        if (cm != null) Marshal.ReleaseComObject(cm);
    }

    // Komutun bu süreçte açtığı pencereler (Özellikler) kapanana dek bekle; 3 sn içinde hiç açılmadıysa çık
    static void WaitForOwnWindows(IntPtr owner)
    {
        uint me = (uint)Process.GetCurrentProcess().Id;
        var start = DateTime.UtcNow;
        bool seen = false;
        while (true)
        {
            bool any = false;
            Native.EnumWindows((h, l) =>
            {
                uint pid;
                Native.GetWindowThreadProcessId(h, out pid);
                if (pid == me && h != owner && Native.IsWindowVisible(h)) { any = true; return false; }
                return true;
            }, IntPtr.Zero);
            if (any) seen = true;
            else if (seen || (DateTime.UtcNow - start).TotalSeconds > 3) return;
            Application.DoEvents();
            Thread.Sleep(100);
        }
    }
}

// Windows'un IDesktopWallpaper API'si: monitör başına ayrı resim ya da tüm masaüstüne yayılan tek resim
// (Superpaper'ın "span" modu). Hazır öneriler Wallhaven'ın herkese açık API'sinden, yalnızca SFW.
[ComImport, Guid("B92B56A9-8B55-4E14-9A89-0199BBB6F93B"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IDesktopWallpaper
{
    void SetWallpaper([MarshalAs(UnmanagedType.LPWStr)] string monitorID, [MarshalAs(UnmanagedType.LPWStr)] string wallpaper);
    [return: MarshalAs(UnmanagedType.LPWStr)] string GetWallpaper([MarshalAs(UnmanagedType.LPWStr)] string monitorID);
    [return: MarshalAs(UnmanagedType.LPWStr)] string GetMonitorDevicePathAt(uint monitorIndex);
    uint GetMonitorDevicePathCount();
    Native.RECT GetMonitorRECT([MarshalAs(UnmanagedType.LPWStr)] string monitorID);
    void SetBackgroundColor(uint color);
    uint GetBackgroundColor();
    void SetPosition(int position);
    int GetPosition();
    void SetSlideshow(IntPtr items);
    IntPtr GetSlideshow();
    void SetSlideshowOptions(int options, uint slideshowTick);
    void GetSlideshowOptions(out int options, out uint slideshowTick);
    void AdvanceSlideshow([MarshalAs(UnmanagedType.LPWStr)] string monitorID, int direction);
    int GetStatus();
    void Enable([MarshalAs(UnmanagedType.Bool)] bool enable);
}
[ComImport, Guid("C2CF3110-460E-4fc1-B9D0-8A1C0C9CC4BD")] class DesktopWallpaperCoClass { }

static class Wallpaper
{
    const int FILL = 4, SPAN = 5;
    static IDesktopWallpaper Api() { return (IDesktopWallpaper)new DesktopWallpaperCoClass(); }
    static readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    public static string Dir
    {
        get
        {
            string d = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyPictures), "Wallpapers", "Logical Lunge");
            System.IO.Directory.CreateDirectory(d);
            return d;
        }
    }

    // --wall-info -> {"span":false,"monitors":[{"id","x","y","w","h","path"}]}
    public static string Info()
    {
        var w = Api();
        var mons = new List<Dictionary<string, object>>();
        uint n = w.GetMonitorDevicePathCount();
        for (uint i = 0; i < n; i++)
        {
            string id = w.GetMonitorDevicePathAt(i);
            Native.RECT r;
            try { r = w.GetMonitorRECT(id); } catch { continue; } // bağlı olmayan monitör
            string path = "";
            try { path = w.GetWallpaper(id) ?? ""; } catch { }
            mons.Add(new Dictionary<string, object> { { "id", id }, { "x", r.Left }, { "y", r.Top }, { "w", r.Right - r.Left }, { "h", r.Bottom - r.Top }, { "path", path }, { "live", LiveWallpaper.For(id) } });
        }
        return json.Serialize(new Dictionary<string, object> { { "span", w.GetPosition() == SPAN }, { "monitors", mons }, { "dir", Dir }, { "liveOptions", LiveWallpaper.Options() } });
    }

    static void SetRaw(string path, string mode)
    {
        var w = Api();
        if (mode == "span") { w.SetPosition(SPAN); w.SetWallpaper(null, path); }
        else
        {
            w.SetPosition(FILL);
            w.SetWallpaper(mode == "all" ? null : mode, path);
        }
    }

    // Seçilen duvar kağıdı kalıcıdır: başka araçlar (ör. Superpaper) açılışta kendi resmini uygularsa, birkaç dakika
    // boyunca bizim seçimimiz geri yüklenir. Kayıt: satır başına "mod<TAB>yol".
    static string StatePath { get { return Paths.State(@"wallpaper.txt"); } }

    static void SaveState(string path, string mode)
    {
        try
        {
            var d = new Dictionary<string, string>();
            if (mode != "all" && mode != "span" && System.IO.File.Exists(StatePath))
                foreach (var l in System.IO.File.ReadAllLines(StatePath)) { var a = l.Split('\t'); if (a.Length == 2 && a[0] != "all" && a[0] != "span") d[a[0]] = a[1]; }
            d[mode] = path; // "tümü"/"span" seçimi öncekilerin hepsinin yerine geçer
            System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(StatePath));
            var lines = new List<string>(); foreach (var kv in d) lines.Add(kv.Key + "\t" + kv.Value);
            System.IO.File.WriteAllLines(StatePath, lines.ToArray());
        }
        catch { }
    }

    [DllImport("advapi32.dll")]
    static extern int RegNotifyChangeKeyValue(IntPtr hKey, bool watchSubtree, uint filter, IntPtr hEvent, bool async);

    // Açılıştan sonraki ~3 dk: masaüstü ayarları (HKCU\Control Panel\Desktop) değiştiği an kontrol edilir; başka bir
    // aracın resmi 15 sn görünmesin. Bildirim kurulamazsa 5 sn'lik yoklama aynı işi görür.
    public static void StartKeeper()
    {
        var t = new Thread(() =>
        {
            int fixes = 0;
            Microsoft.Win32.RegistryKey key = null;
            var changed = new AutoResetEvent(false);
            try { key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(@"Control Panel\Desktop"); } catch { }
            var until = DateTime.UtcNow.AddMinutes(3);
            for (int i = 0; DateTime.UtcNow < until && fixes < 3; i++)
            {
                if (i > 0)
                {
                    bool watching = false;
                    try { watching = key != null && RegNotifyChangeKeyValue(key.Handle.DangerousGetHandle(), false, 4 /* LAST_SET */, changed.SafeWaitHandle.DangerousGetHandle(), true) == 0; } catch { }
                    changed.WaitOne(5000);
                    if (watching) Thread.Sleep(300); // aracın ardışık yazmaları bitsin
                }
                try
                {
                    if (!System.IO.File.Exists(StatePath)) return;
                    var w = Api();
                    bool fixedOne = false;
                    foreach (var l in System.IO.File.ReadAllLines(StatePath))
                    {
                        var a = l.Split('\t');
                        if (a.Length != 2 || !System.IO.File.Exists(a[1])) continue;
                        string cur = "";
                        try { cur = (a[0] == "all" || a[0] == "span") ? (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Control Panel\Desktop", "WallPaper", null) : w.GetWallpaper(a[0]); } catch { }
                        if (string.Equals(System.IO.Path.GetFullPath(cur ?? "x"), a[1], StringComparison.OrdinalIgnoreCase)) continue;
                        // Windows span/all'da kayıt defterine kopya yazabilir: dosya adı aynıysa yeniden uygulama
                        if (System.IO.Path.GetFileName(cur ?? "") == System.IO.Path.GetFileName(a[1])) continue;
                        SetRaw(a[1], a[0]); fixedOne = true;
                    }
                    if (fixedOne) { fixes++; Slider.Log("duvar kağıdı başka bir araçça değiştirilmişti, seçim geri yüklendi"); }
                }
                catch { }
            }
            if (key != null) key.Close();
        }) { IsBackground = true };
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
    }

    // mode: "all" | "span" | monitör kimliği
    // keepLive: canlı duvar kağıdının altındaki kare; değilse seçilen resim o monitörlerin canlı duvar kağıdını kapatır
    // (önce resim konur: kapanan videonun yerinde eski resim bir an görünmesin)
    public static void Apply(string path, string mode, bool keepLive = false)
    {
        path = System.IO.Path.GetFullPath(path);
        SetRaw(path, mode);
        SaveState(path, mode);
        // resim yerinde: canlı duvar kağıdı kapanamazsa (ayar dosyası o an kilitli) komut yine başarılı sayılır
        if (!keepLive)
        {
            try { LiveWallpaper.Clear(mode); }
            catch (Exception ex) { Slider.Log("canlı duvar kağıdı kapatılamadı: " + ex.Message); }
        }
        // ii switchwall.sh gibi terminal renklerini yeni duvar kağıdından üret (varsa)
        try
        {
            string exe = Paths.Tool(@"termcolors\lunge-termcolors.exe");
            string tc = Paths.Tool(@"termcolors\wezterm-colors.py");
            string py = Paths.Tool(@"songrec\venv\Scripts\pythonw.exe");
            if (System.IO.File.Exists(exe))
                Process.Start(new ProcessStartInfo(exe, "--path \"" + path + "\"") { UseShellExecute = false, CreateNoWindow = true });
            else if (System.IO.File.Exists(tc) && System.IO.File.Exists(py))
                Process.Start(new ProcessStartInfo(py, "\"" + tc + "\" --path \"" + path + "\"") { UseShellExecute = true, WindowStyle = ProcessWindowStyle.Hidden });
        }
        catch { }
    }

    static System.Net.WebClient Client()
    {
        System.Net.ServicePointManager.SecurityProtocol = (System.Net.SecurityProtocolType)3072; // TLS 1.2
        var c = new System.Net.WebClient { Proxy = null };
        c.Headers[System.Net.HttpRequestHeader.UserAgent] = "LogicalLunge/1.0 (wallpaper picker)";
        c.Encoding = Encoding.UTF8;
        return c;
    }

    // --wall-browse <kind> [page] -> [{"id","thumb","full","res","ratio"}]
    // kind: anime | nature | space | city | minimal | span (çift monitör, ultra geniş)
    public static string Browse(string kind, int page)
    {
        string q;
        switch (kind)
        {
            case "anime": q = "categories=010&q="; break;
            case "nature": q = "categories=100&q=nature"; break;
            case "space": q = "categories=100&q=space"; break;
            case "city": q = "categories=100&q=city"; break;
            case "minimal": q = "categories=100&q=minimalism"; break;
            case "span": q = "categories=110&q=&ratios=32x9,48x9&atleast=3840x1080"; break;
            default: q = "categories=110&q="; break;
        }
        string extra = kind == "span" ? "" : "&ratios=16x9,16x10&atleast=1920x1080";
        string url = "https://wallhaven.cc/api/v1/search?" + q + "&purity=100&sorting=toplist&topRange=1y&page=" + Math.Max(1, page) + extra;
        using (var c = Client())
        {
            var res = json.Deserialize<Dictionary<string, object>>(c.DownloadString(url));
            var list = new List<Dictionary<string, object>>();
            var data = res.ContainsKey("data") ? res["data"] as System.Collections.ArrayList : null;
            if (data != null)
                foreach (Dictionary<string, object> it in data)
                {
                    var thumbs = it["thumbs"] as Dictionary<string, object>;
                    list.Add(new Dictionary<string, object> {
                        { "id", it["id"] }, { "thumb", thumbs != null ? thumbs["small"] : "" }, { "large", thumbs != null ? thumbs["large"] : "" },
                        { "full", it["path"] }, { "res", it["resolution"] }, { "ratio", it["ratio"] } });
                }
            return json.Serialize(list);
        }
    }

    // --wall-get <url> <mode>: indir (önbellekte varsa yeniden indirme) ve uygula
    public static string Download(string url)
    {
        if (!System.Text.RegularExpressions.Regex.IsMatch(url, @"^https://w\.wallhaven\.cc/full/[0-9a-z]{2}/wallhaven-[0-9a-z]+\.(jpg|png)$"))
            throw new ArgumentException("url");
        string file = System.IO.Path.Combine(Dir, System.IO.Path.GetFileName(new Uri(url).AbsolutePath));
        if (!System.IO.File.Exists(file))
        {
            string tmp = file + ".part";
            using (var c = Client()) c.DownloadFile(url, tmp);
            System.IO.File.Move(tmp, file);
        }
        return file;
    }

    // --wall-local -> indirilmiş/eklenmiş duvar kağıtları (en yeni önce)
    public static string Local()
    {
        var files = new List<Dictionary<string, object>>();
        foreach (var f in new System.IO.DirectoryInfo(Dir).GetFiles())
        {
            string e = f.Extension.ToLowerInvariant();
            if (e != ".jpg" && e != ".jpeg" && e != ".png" && e != ".bmp" && e != ".webp") continue;
            files.Add(new Dictionary<string, object> { { "path", f.FullName }, { "name", f.Name }, { "time", (long)(f.LastWriteTimeUtc - new DateTime(1970, 1, 1)).TotalMilliseconds } });
        }
        files.Sort((a, b) => ((long)b["time"]).CompareTo((long)a["time"]));
        return json.Serialize(files);
    }

    // --wall-thumb <path> -> küçük JPEG data URL (yerel dosyalar tarayıcıda doğrudan açılamıyor)
    public static string Thumb(string path)
    {
        using (var src = Image.FromFile(path))
        {
            int w = 320, h = Math.Max(1, (int)(src.Height * 320.0 / src.Width));
            using (var bmp = new Bitmap(w, h))
            {
                using (var g = Graphics.FromImage(bmp)) { g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic; g.DrawImage(src, 0, 0, w, h); }
                using (var ms = new System.IO.MemoryStream())
                {
                    var enc = Array.Find(System.Drawing.Imaging.ImageCodecInfo.GetImageEncoders(), x => x.MimeType == "image/jpeg");
                    var ps = new System.Drawing.Imaging.EncoderParameters(1);
                    ps.Param[0] = new System.Drawing.Imaging.EncoderParameter(System.Drawing.Imaging.Encoder.Quality, 78L);
                    bmp.Save(ms, enc, ps);
                    return "data:image/jpeg;base64," + Convert.ToBase64String(ms.ToArray());
                }
            }
        }
    }

    // --wall-pick <mode>: dosya seçtir, kütüphaneye kopyala, uygula
    public static string Pick(string mode)
    {
        using (var d = new OpenFileDialog
        {
            Title = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr" ? "Duvar kağıdı seç" : "Choose wallpaper",
            Filter = "Resimler|*.jpg;*.jpeg;*.png;*.bmp;*.webp",
            InitialDirectory = Environment.GetFolderPath(Environment.SpecialFolder.MyPictures),
        })
        {
            if (d.ShowDialog() != DialogResult.OK) return "";
            string dst = System.IO.Path.Combine(Dir, System.IO.Path.GetFileName(d.FileName));
            if (!string.Equals(System.IO.Path.GetFullPath(d.FileName), dst, StringComparison.OrdinalIgnoreCase)) System.IO.File.Copy(d.FileName, dst, true);
            System.IO.File.SetLastWriteTimeUtc(dst, DateTime.UtcNow);
            Apply(dst, mode);
            return dst;
        }
    }
}

// ---------------- Canlı duvar kağıdı (lunge-wallpaper.exe) ----------------
// Videolar masaüstü simgelerinin arkasında oynar: her monitöre bir pencere, çözme ekran kartında (Media Foundation).
// Ayar: state\live-wallpaper.json {"wallpapers":[{"monitor":"*" | monitör kimliği,"file":video}],"pauseFullscreen","pauseOnBattery"}
// (boş "file": herkese bir video varken o monitör kapalı). Videonun bir karesi statik duvar kağıdı olur: tema renkleri
// ona uyar, video başlamadan ya da durunca masaüstünde aynı resim görünür.
// Mağaza: Sucrose Store (github.com/Taiizor/Store, MIT); yalnızca video türü ve yetişkin olmayan içerik.
// ---------------- Ekran koruyucu kütüphanesi ----------------
// İçe aktarılan .scr dosyaları %LOCALAPPDATA%\LogicalLunge\screensavers altında durur; kabuk onları Windows'unkilerle
// birlikte listeler. Bir .zip içindeki ekran koruyucular yanlarındaki dosyalarla (dll, veri) kendi klasörüne açılır.
static class ScreenSavers
{
    public static string Dir { get { return Paths.DataDir("screensavers"); } }
    const long MAX_PACK = 512L << 20;

    // --saver-pick -> {"added":[yollar]} (birden çok dosya seçilebilir)
    public static string Pick()
    {
        bool tr = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr";
        using (var d = new OpenFileDialog
        {
            Title = tr ? "Ekran koruyucu seç" : "Choose screen savers",
            Filter = (tr ? "Ekran koruyucu" : "Screen saver") + "|*.scr;*.zip",
            Multiselect = true,
        })
        {
            var added = new List<string>();
            if (d.ShowDialog() == DialogResult.OK)
                foreach (var f in d.FileNames) added.AddRange(Import(f));
            return new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "added", added } });
        }
    }

    static List<string> Import(string picked)
    {
        var added = new List<string>();
        if (System.IO.Path.GetExtension(picked).Equals(".scr", StringComparison.OrdinalIgnoreCase))
        {
            string dst = System.IO.Path.Combine(Dir, System.IO.Path.GetFileName(picked));
            if (!string.Equals(System.IO.Path.GetFullPath(picked), dst, StringComparison.OrdinalIgnoreCase)) System.IO.File.Copy(picked, dst, true);
            added.Add(dst);
            return added;
        }
        using (var zip = System.IO.Compression.ZipFile.OpenRead(picked))
        {
            bool any = false;
            long total = 0;
            foreach (var e in zip.Entries) { total += e.Length; if (e.FullName.EndsWith(".scr", StringComparison.OrdinalIgnoreCase)) any = true; }
            if (!any) throw new NotSupportedException("noscr");
            if (total > MAX_PACK) throw new NotSupportedException("size");
            string root = System.IO.Path.GetFullPath(System.IO.Path.Combine(Dir, System.IO.Path.GetFileNameWithoutExtension(picked))) + System.IO.Path.DirectorySeparatorChar;
            foreach (var e in zip.Entries)
            {
                if (e.FullName.EndsWith("/") || e.FullName.EndsWith("\\")) continue;
                string dst = System.IO.Path.GetFullPath(System.IO.Path.Combine(root, e.FullName));
                if (!dst.StartsWith(root, StringComparison.OrdinalIgnoreCase)) continue; // paketin dışına yazmaz
                System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(dst));
                using (var src = e.Open())
                using (var outf = System.IO.File.Create(dst)) src.CopyTo(outf);
                if (dst.EndsWith(".scr", StringComparison.OrdinalIgnoreCase)) added.Add(dst);
            }
        }
        return added;
    }

    // --saver-icons -> {"yol (küçük harf)": "data:image/png;base64,..."}: Windows'un ve kütüphanenin ekran koruyucuları
    public static string Icons()
    {
        var map = new Dictionary<string, object>();
        string win = Environment.GetFolderPath(Environment.SpecialFolder.Windows);
        var dirs = new[] { System.IO.Path.Combine(win, "System32"), System.IO.Path.Combine(win, "SysWOW64"), Dir };
        foreach (var dir in dirs)
        {
            if (!System.IO.Directory.Exists(dir)) continue;
            var opt = dir == Dir ? System.IO.SearchOption.AllDirectories : System.IO.SearchOption.TopDirectoryOnly;
            IEnumerable<string> found;
            try { found = System.IO.Directory.EnumerateFiles(dir, "*.scr", opt); } catch { continue; }
            foreach (var f in found)
            {
                try
                {
                    using (var icon = Icon.ExtractAssociatedIcon(f))
                    using (var bmp = icon.ToBitmap())
                    using (var ms = new System.IO.MemoryStream())
                    {
                        bmp.Save(ms, System.Drawing.Imaging.ImageFormat.Png);
                        map[f.ToLowerInvariant()] = "data:image/png;base64," + Convert.ToBase64String(ms.ToArray());
                    }
                }
                catch { } // simgesi okunamayan ekran koruyucu kartta genel simgeyle görünür
            }
        }
        return new JavaScriptSerializer().Serialize(map);
    }
}

// ---------------- Video ekran koruyucusu (LogicalLunge.scr) ----------------
// lunge-wallpaper.exe'nin .scr adlı kopyası (paket yapar). Oynattığı videolar state\screensaver-video.json'da:
// {"videos":[yollar],"shuffle":bool} (birden çoksa her açılışta biri). Videolar canlı duvar kağıdı kütüphanesinden gelir
// (içe aktarılan ya da mağazadan inen); kütüphane dışındaki bir dosya seçilmez. Seçmek Windows'un ekran koruyucusunu
// LogicalLunge.scr yapar (HKCU, kullanıcının kendi ayarı; bekleme süresi ve kilit ayarına dokunulmaz).
static class SaverVideo
{
    [DllImport("user32.dll", SetLastError = true)] static extern bool SystemParametersInfo(uint action, uint param, IntPtr pv, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern uint GetShortPathName(string longPath, StringBuilder shortPath, uint size);
    const uint SPI_SETSCREENSAVEACTIVE = 0x0011, SPIF_UPDATEINIFILE = 0x1, SPIF_SENDCHANGE = 0x2;
    const string DESKTOP = @"HKEY_CURRENT_USER\Control Panel\Desktop";
    static readonly string[] VIDEO = { ".mp4", ".m4v", ".mov", ".wmv", ".webm", ".mkv" };
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    static string StatePath { get { return Paths.State("screensaver-video.json"); } }
    public static string Scr { get { return Paths.In("LogicalLunge.scr"); } }

    sealed class Settings { public List<string> Videos = new List<string>(); public bool Shuffle = true; }

    static Settings Load()
    {
        var s = new Settings();
        try
        {
            if (!System.IO.File.Exists(StatePath)) return s;
            var d = json.Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(StatePath));
            object v;
            if (d.TryGetValue("videos", out v) && v is System.Collections.ArrayList)
                foreach (var o in (System.Collections.ArrayList)v) if (o is string && ((string)o).Length > 0) s.Videos.Add((string)o);
            if (d.TryGetValue("shuffle", out v) && v is bool) s.Shuffle = (bool)v;
        }
        catch (Exception ex) { Slider.Log("ekran koruyucu videosu ayarı okunamadı: " + ex.Message); }
        return s;
    }

    static void Save(Settings s)
    {
        string tmp = StatePath + ".tmp";
        System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(StatePath));
        System.IO.File.WriteAllText(tmp, json.Serialize(new Dictionary<string, object> { { "videos", s.Videos }, { "shuffle", s.Shuffle } }), new UTF8Encoding(false));
        if (System.IO.File.Exists(StatePath)) System.IO.File.Replace(tmp, StatePath, null);
        else System.IO.File.Move(tmp, StatePath);
    }

    // Yalnızca kütüphanedeki bir video (ekran koruyucu, gösterdiği dosyayı kullanıcının seçtiği yerden okur)
    public static string InLibrary(string video)
    {
        if (string.IsNullOrEmpty(video)) throw new ArgumentException("video");
        string full = System.IO.Path.GetFullPath(video);
        string root = System.IO.Path.GetFullPath(LiveWallpaper.Dir).TrimEnd('\\') + "\\";
        if (!full.StartsWith(root, StringComparison.OrdinalIgnoreCase)) throw new ArgumentException("library");
        if (Array.IndexOf(VIDEO, System.IO.Path.GetExtension(full).ToLowerInvariant()) < 0 || !System.IO.File.Exists(full)) throw new ArgumentException("video");
        return full;
    }

    // Windows SCRNSAVE.EXE'de boşluklu yolu her yerde okuyamıyor (Program Files): kısa yol yazılır
    static string ShortPath(string path)
    {
        var sb = new StringBuilder(520);
        uint n = GetShortPathName(path, sb, (uint)sb.Capacity);
        return n > 0 && n < sb.Capacity ? sb.ToString() : path;
    }

    static bool Selected()
    {
        string cur = Microsoft.Win32.Registry.GetValue(DESKTOP, "SCRNSAVE.EXE", "") as string;
        if (string.IsNullOrEmpty(cur) || !System.IO.File.Exists(Scr)) return false;
        return string.Equals(cur, Scr, StringComparison.OrdinalIgnoreCase) || string.Equals(cur, ShortPath(Scr), StringComparison.OrdinalIgnoreCase);
    }

    static void Activate(bool on)
    {
        if (on)
        {
            if (!System.IO.File.Exists(Scr)) throw new InvalidOperationException("scr");
            Microsoft.Win32.Registry.SetValue(DESKTOP, "SCRNSAVE.EXE", ShortPath(Scr));
        }
        if (on || Selected()) SystemParametersInfo(SPI_SETSCREENSAVEACTIVE, on ? 1u : 0u, IntPtr.Zero, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE);
    }

    // --saver-videos -> {"videos":[var olanlar],"shuffle":bool,"scr":yol,"installed":bool,"active":bool}
    public static string List()
    {
        var s = Load();
        return json.Serialize(new Dictionary<string, object>
        {
            { "videos", s.Videos.FindAll(System.IO.File.Exists) }, { "shuffle", s.Shuffle },
            { "scr", Scr }, { "installed", System.IO.File.Exists(Scr) }, { "active", Selected() },
        });
    }

    // --saver-video <set|add|remove> <video>: set tek video yapar ve LogicalLunge.scr'yi seçer, add listeye ekler
    // (karışık oynatılır), remove çıkarır (liste boşalırsa ekran koruyucumuz seçiliyse kapatılır)
    public static string Change(string op, string video)
    {
        var s = Load();
        if (op == "remove")
        {
            string full = System.IO.Path.GetFullPath(video);
            s.Videos.RemoveAll(v => string.Equals(v, full, StringComparison.OrdinalIgnoreCase));
            Save(s);
            if (s.Videos.Count == 0) Activate(false);
            return List();
        }
        string path = InLibrary(video);
        if (op == "set") s.Videos = new List<string> { path };
        else if (op == "add") { if (!s.Videos.Exists(v => string.Equals(v, path, StringComparison.OrdinalIgnoreCase))) s.Videos.Add(path); }
        else throw new ArgumentException("op");
        Save(s);
        Activate(true);
        return List();
    }

    // --saver-shuffle <1|0>
    public static string Shuffle(bool on) { var s = Load(); s.Shuffle = on; Save(s); return List(); }

    // --saver-store-get <kategori> <ad>: canlı duvar kağıdı mağazasından indir (duvar kağıdı yapmadan), ekran koruyucu yap
    public static string StoreGet(string category, string id)
    {
        var got = json.Deserialize<Dictionary<string, object>>(LiveWallpaper.Get(category, id, "none"));
        object p;
        if (!got.TryGetValue("path", out p) || !(p is string)) throw new InvalidOperationException("store");
        return Change("set", (string)p);
    }

    // --saver-video-run: şimdi göster (önizleme düğmesi)
    public static void Run()
    {
        if (!System.IO.File.Exists(Scr)) throw new InvalidOperationException("scr");
        Process.Start(new ProcessStartInfo(Scr, "/s") { UseShellExecute = false });
    }
}

static class Library
{
    static string Root(string dir) { return System.IO.Path.GetFullPath(dir).TrimEnd('\\'); }

    static bool Under(string root, string full)
    {
        return full.StartsWith(root + "\\", StringComparison.OrdinalIgnoreCase);
    }

    // Lexical containment alone is insufficient: junctions can point outside
    // the library. Never follow a reparse point on the path or in a package.
    static bool PlainPath(string full)
    {
        for (string p = full; !string.IsNullOrEmpty(p); p = System.IO.Path.GetDirectoryName(p))
            if ((System.IO.File.Exists(p) || System.IO.Directory.Exists(p))
                && (System.IO.File.GetAttributes(p) & System.IO.FileAttributes.ReparsePoint) != 0) return false;
        return true;
    }

    static bool PlainTree(string folder)
    {
        foreach (string entry in System.IO.Directory.GetFileSystemEntries(folder))
        {
            var attr = System.IO.File.GetAttributes(entry);
            if ((attr & System.IO.FileAttributes.ReparsePoint) != 0) return false;
            if ((attr & System.IO.FileAttributes.Directory) != 0 && !PlainTree(entry)) return false;
        }
        return true;
    }

    // null: silindi; "path" (kütüphane dışı / geçersiz), "missing", "kind"
    public static string Remove(string kind, string path)
    {
        if (string.IsNullOrEmpty(path) || path.IndexOfAny(System.IO.Path.GetInvalidPathChars()) >= 0 || !System.IO.Path.IsPathRooted(path)) return "path";
        string full;
        try { full = System.IO.Path.GetFullPath(path); } catch { return "path"; }
        if (full.IndexOf(':', 2) >= 0 || !PlainPath(full)) return "path";
        switch (kind)
        {
            case "wall":
            {
                string root = Root(Wallpaper.Dir);
                if (!string.Equals(System.IO.Path.GetDirectoryName(full), root, StringComparison.OrdinalIgnoreCase)) return "path";
                if (!System.IO.File.Exists(full)) return "missing";
                System.IO.File.Delete(full);
                return null;
            }
            case "live":
            {
                string root = Root(LiveWallpaper.Dir);
                string folder = System.IO.Path.GetDirectoryName(full);
                if (folder == null || !Under(root, full) || !string.Equals(System.IO.Path.GetDirectoryName(folder), root, StringComparison.OrdinalIgnoreCase)) return "path";
                if (!System.IO.File.Exists(full)) return "missing";
                if (!PlainTree(folder)) return "path";
                LiveWallpaper.Forget(full);
                System.IO.Directory.Delete(folder, true);
                return null;
            }
            case "saver":
            {
                string root = Root(ScreenSavers.Dir);
                if (!Under(root, full) || !full.EndsWith(".scr", StringComparison.OrdinalIgnoreCase)) return "path";
                if (!System.IO.File.Exists(full)) return "missing";
                string folder = System.IO.Path.GetDirectoryName(full);
                if (!string.Equals(folder, root, StringComparison.OrdinalIgnoreCase) && !PlainTree(folder)) return "path";
                System.IO.File.Delete(full);
                // bir paketten açılmış klasör: içinde başka ekran koruyucu kalmadıysa o da gider
                if (!string.Equals(folder, root, StringComparison.OrdinalIgnoreCase) && Under(root, folder)
                    && System.IO.Directory.GetFiles(folder, "*.scr", System.IO.SearchOption.AllDirectories).Length == 0)
                    System.IO.Directory.Delete(folder, true);
                return null;
            }
        }
        return "kind";
    }
}

static class LiveWallpaper
{
    public const string Name = "lunge-wallpaper";
    const string WINDOW_CLASS = "LogicalLunge.LiveWallpaper";
    const uint WM_CLOSE = 0x0010, WM_APP_RELOAD = 0x8001;
    const string STORE = "https://raw.githubusercontent.com/Taiizor/Store/develop/";
    const long MAX_VIDEO = 500L << 20;
    // bir indirmenin toplam süresi (yavaş damlatan bir sunucu komut satırını saatlerce tutmasın)
    static readonly TimeSpan FETCH_DEADLINE = TimeSpan.FromMinutes(20);
    static readonly string[] VIDEO = { ".mp4", ".m4v", ".mov", ".wmv", ".webm", ".mkv" };
    static readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);

    static string Exe { get { return Paths.In("lunge-wallpaper.exe"); } }
    static string StatePath { get { return Paths.State("live-wallpaper.json"); } }
    static string StoreCache { get { return System.IO.Path.Combine(Paths.DataDir("cache"), "live-store.json"); } }
    static string ProgressFile { get { return System.IO.Path.Combine(Dir, ".progress"); } }

    public static string Dir
    {
        get
        {
            string d = System.IO.Path.Combine(Wallpaper.Dir, "Live");
            System.IO.Directory.CreateDirectory(d);
            return d;
        }
    }

    sealed class Settings
    {
        public List<KeyValuePair<string, string>> Entries = new List<KeyValuePair<string, string>>();
        public bool PauseFullscreen = true, PauseOnBattery = true;
        public bool Active { get { return Entries.Exists(e => e.Value.Length > 0); } }
    }

    // Kayıtlar (monitör -> video) üzerinde saf işlemler; her biri yeni bir liste döndürür
    // Monitöre video ver ("all"/"span": herkese tek video, eski kayıtların yerine)
    static List<KeyValuePair<string, string>> WithVideo(List<KeyValuePair<string, string>> entries, string mode, string video)
    {
        bool all = mode == "all" || mode == "span";
        var list = entries.FindAll(e => !all && !string.Equals(e.Key, mode, StringComparison.OrdinalIgnoreCase));
        list.Add(new KeyValuePair<string, string>(all ? "*" : mode, video));
        return list;
    }

    // Monitörün videosunu kapat; herkese bir video varken yalnız o monitör kapanır. Video kalmazsa liste boşalır.
    static List<KeyValuePair<string, string>> WithoutVideo(List<KeyValuePair<string, string>> entries, string mode)
    {
        if (mode == "all" || mode == "span") return new List<KeyValuePair<string, string>>();
        var list = entries.FindAll(e => !string.Equals(e.Key, mode, StringComparison.OrdinalIgnoreCase));
        if (list.Exists(e => e.Key == "*")) list.Add(new KeyValuePair<string, string>(mode, ""));
        return list.Exists(e => e.Value.Length > 0) ? list : new List<KeyValuePair<string, string>>();
    }

    static bool Same(List<KeyValuePair<string, string>> a, List<KeyValuePair<string, string>> b)
    {
        if (a.Count != b.Count) return false;
        for (int i = 0; i < a.Count; i++) if (a[i].Key != b[i].Key || a[i].Value != b[i].Value) return false;
        return true;
    }

    // Bir monitörün videosu: kendi kaydı (boşsa kapalı), yoksa herkese olan
    static string FileFor(List<KeyValuePair<string, string>> entries, string monitor)
    {
        foreach (var e in entries) if (string.Equals(e.Key, monitor, StringComparison.OrdinalIgnoreCase)) return e.Value;
        foreach (var e in entries) if (e.Key == "*") return e.Value;
        return "";
    }

    // strict: dosya var ama okunamıyor / bozuk ise hata (Set/Clear boş bir ayarı onun yerine yazmasın); okumak için
    // (--wall-info) varsayılanlar yeter
    static Settings Load(bool strict = false)
    {
        var s = new Settings();
        try
        {
            if (!System.IO.File.Exists(StatePath)) return s;
            var d = json.Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(StatePath));
            object v;
            if (d.TryGetValue("wallpapers", out v) && v is System.Collections.ArrayList)
                foreach (var o in (System.Collections.ArrayList)v)
                {
                    var e = o as Dictionary<string, object>;
                    object m, f;
                    if (e != null && e.TryGetValue("monitor", out m) && e.TryGetValue("file", out f) && m is string && f is string && ((string)m).Length > 0)
                        s.Entries.Add(new KeyValuePair<string, string>((string)m, (string)f));
                }
            if (d.TryGetValue("pauseFullscreen", out v) && v is bool) s.PauseFullscreen = (bool)v;
            if (d.TryGetValue("pauseOnBattery", out v) && v is bool) s.PauseOnBattery = (bool)v;
        }
        catch (Exception ex)
        {
            Slider.Log("canlı duvar kağıdı ayarı okunamadı: " + ex.Message);
            if (strict) throw new InvalidOperationException("state");
        }
        return s;
    }

    // Ayarı okuyup değiştiren komutlar (iki monitör, iki anahtar aynı anda) birbirinin yazdığını silmesin
    static T Locked<T>(Func<T> f)
    {
        using (var m = new Mutex(false, @"Local\LogicalLunge.LiveWallpaper.State"))
        {
            bool owned = false;
            try
            {
                try { owned = m.WaitOne(10000); } catch (AbandonedMutexException) { owned = true; }
                if (!owned) throw new TimeoutException("state");
                return f();
            }
            finally { if (owned) m.ReleaseMutex(); }
        }
    }

    static void Save(Settings s)
    {
        var list = new List<Dictionary<string, object>>();
        foreach (var e in s.Entries) list.Add(new Dictionary<string, object> { { "monitor", e.Key }, { "file", e.Value } });
        string text = json.Serialize(new Dictionary<string, object> { { "wallpapers", list }, { "pauseFullscreen", s.PauseFullscreen }, { "pauseOnBattery", s.PauseOnBattery } });
        // oynatıcı yarım yazılmış bir dosya okumasın
        string tmp = StatePath + "." + Process.GetCurrentProcess().Id + ".tmp";
        System.IO.File.WriteAllText(tmp, text, new UTF8Encoding(false));
        if (System.IO.File.Exists(StatePath)) System.IO.File.Replace(tmp, StatePath, null);
        else System.IO.File.Move(tmp, StatePath);
    }

    // --wall-info'nun canlı alanları: monitörün videosu ("" yok) ve duraklatma kuralları
    public static string For(string monitor) { return FileFor(Load().Entries, monitor); }
    public static Dictionary<string, object> Options()
    {
        var s = Load();
        return new Dictionary<string, object> { { "pauseFullscreen", s.PauseFullscreen }, { "pauseOnBattery", s.PauseOnBattery } };
    }

    // Açılışta / masaüstü yeniden kurulurken: ayar varsa ve oynatıcı çalışmıyorsa başlat. Normal kullanıcı haklarıyla
    // (UserLaunch): Explorer'ın masaüstü pencerelerine bağlanır, kabuğun istekleri ona ulaşır.
    public static void EnsureRunning()
    {
        if (FindWindow(WINDOW_CLASS, null) == IntPtr.Zero && Load().Active && System.IO.File.Exists(Exe)) UserLaunch.Start(Exe, "", Paths.Home);
    }

    // Asıl çekirdekte (/cmd?a=live-wallpaper): çalışan oynatıcı ayarı yeniden okur (ayar boşsa kendisi kapanır; kapanırken
    // yeni bir ayar gelirse yerine yenisini bırakır), çalışmıyorsa başlar
    public static void Sync()
    {
        IntPtr w = FindWindow(WINDOW_CLASS, null);
        if (w == IntPtr.Zero) EnsureRunning();
        else PostMessage(w, WM_APP_RELOAD, IntPtr.Zero, IntPtr.Zero);
    }

    // Komut satırından (--live-*): ayar yazıldı, gerisini çekirdek yapar; çekirdek cevap vermezse bu süreç
    static void Notify()
    {
        if (Supervisor.PostToCore("/cmd?a=live-wallpaper", 2000) != 202) Sync();
    }

    // Masaüstü kapanırken; çekirdek açılışta yeniden başlatır
    public static void Stop()
    {
        IntPtr w = FindWindow(WINDOW_CLASS, null);
        if (w != IntPtr.Zero) PostMessage(w, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
    }

    static bool IsVideo(string path) { return Array.IndexOf(VIDEO, System.IO.Path.GetExtension(path).ToLowerInvariant()) >= 0; }

    // --live-set <video> <mod>: videoyu duvar kağıdı yap (mod: all | monitör kimliği; span da "all" sayılır: her monitör
    // videoyu kendi oranında doldurur)
    public static void Set(string video, string mode)
    {
        video = System.IO.Path.GetFullPath(video);
        if (!System.IO.File.Exists(video) || !IsVideo(video)) throw new ArgumentException("video");
        if (!System.IO.File.Exists(Exe)) throw new InvalidOperationException("player");
        bool all = mode == "all" || mode == "span";
        // altta duran resim: videonun bir karesi (bir kez çıkarılır). Kare alınamıyorsa oynatıcı da açamaz (aynı çözücü):
        // "ayarlandı" deyip boş masaüstü bırakmak yerine söylenir
        string frame = System.IO.Path.ChangeExtension(video, ".frame.png");
        if (!System.IO.File.Exists(frame)) SaveFrame(video, frame);
        if (!System.IO.File.Exists(frame)) throw new NotSupportedException("decode");
        Wallpaper.Apply(frame, all ? "all" : mode, true);
        Locked(() =>
        {
            var s = Load(true);
            s.Entries = WithVideo(s.Entries, mode, video);
            Save(s);
            return true;
        });
        Notify();
    }

    static void SaveFrame(string video, string png)
    {
        try
        {
            using (var p = Process.Start(new ProcessStartInfo(Exe, "--frame " + Program.QuoteArg(video) + " " + Program.QuoteArg(png)) { UseShellExecute = false, CreateNoWindow = true }))
                if (!p.WaitForExit(30000)) { try { p.Kill(); } catch { } }
        }
        catch (Exception ex) { Slider.Log("canlı duvar kağıdı karesi alınamadı: " + ex.Message); }
    }

    // --live-clear <mod>: canlı duvar kağıdını kapat; altta kalan resim (videonun karesi) durur. Statik bir duvar kağıdı
    // seçilince de çağrılır.
    public static void Clear(string mode)
    {
        bool changed = Locked(() =>
        {
            var s = Load(true);
            if (!s.Active) return false;
            var next = WithoutVideo(s.Entries, mode);
            if (Same(next, s.Entries)) return false;
            s.Entries = next;
            Save(s);
            return true;
        });
        if (changed) Notify();
    }

    // Remove a default video without discarding other monitors' overrides;
    // removing an override must keep that monitor off instead of revealing '*'.
    static List<KeyValuePair<string, string>> WithoutFile(List<KeyValuePair<string, string>> entries, string video)
    {
        var next = entries.FindAll(e => e.Key != "*" || !string.Equals(e.Value, video, StringComparison.OrdinalIgnoreCase));
        foreach (var e in entries)
            if (e.Key != "*" && string.Equals(e.Value, video, StringComparison.OrdinalIgnoreCase))
                next = WithoutVideo(next, e.Key);
        return next.Exists(e => e.Value.Length > 0) ? next : new List<KeyValuePair<string, string>>();
    }

    public static void Forget(string video)
    {
        bool changed = Locked(() =>
        {
            var s = Load(true);
            var next = WithoutFile(s.Entries, video);
            if (Same(next, s.Entries)) return false;
            s.Entries = next;
            Save(s);
            return true;
        });
        if (changed) Notify();
    }

    // --live-options <tam ekranda duraklat 0|1> <pille çalışırken duraklat 0|1>
    public static void SetOptions(bool fullscreen, bool battery)
    {
        bool notify = Locked(() =>
        {
            var s = Load(true);
            if (s.PauseFullscreen == fullscreen && s.PauseOnBattery == battery) return false;
            s.PauseFullscreen = fullscreen;
            s.PauseOnBattery = battery;
            Save(s);
            return s.Active;
        });
        if (notify) Notify();
    }

    // Klasör adı: harf, rakam, boşluk, tire (mağaza adları ve dosya adları güvenle klasör olur)
    static string Slug(string name)
    {
        string s = System.Text.RegularExpressions.Regex.Replace(name ?? "", @"[^\w\- ]", "_");
        if (s.Length > 60) s = s.Substring(0, 60);
        s = s.Trim();
        if (System.Text.RegularExpressions.Regex.IsMatch(s, @"^(con|prn|aux|nul|com\d|lpt\d)$", System.Text.RegularExpressions.RegexOptions.IgnoreCase)) s = "_" + s;
        return s.Length == 0 ? "video" : s;
    }

    // --live-pick <mod>: video ya da indirilmiş bir Lively / Wallpaper Engine paketi seçtir (.zip, ya da açılmış paketin
    // LivelyInfo.json / project.json dosyası), videoyu kütüphaneye al, uygula -> kopyanın yolu ("" vazgeçildi)
    public static string Pick(string mode)
    {
        bool tr = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr";
        using (var d = new OpenFileDialog
        {
            Title = tr ? "Video ya da paket seç" : "Choose a video or package",
            Filter = (tr ? "Video, Lively, Wallpaper Engine" : "Video, Lively, Wallpaper Engine") + "|*.mp4;*.m4v;*.mov;*.wmv;*.webm;*.mkv;*.zip;LivelyInfo.json;project.json",
            InitialDirectory = Environment.GetFolderPath(Environment.SpecialFolder.MyVideos),
        })
        {
            if (d.ShowDialog() != DialogResult.OK) return "";
            string dst = Import(d.FileName);
            Set(dst, mode);
            return dst;
        }
    }

    // Paketteki dosyalar: göreli yol ("/" ile) -> boyut ve açıcı
    class PackFile { public long Size; public Func<System.IO.Stream> Open; }

    // Videoyu (ya da paketin videosunu) kütüphaneye kopyalar -> kütüphanedeki yol. Lively: LivelyInfo.json {Title, Author,
    // FileName, Thumbnail}; Wallpaper Engine: project.json {title, type: "video", file, preview}. Bildirimsiz paket: en
    // büyük video.
    public static string Import(string picked)
    {
        var files = new Dictionary<string, PackFile>(StringComparer.OrdinalIgnoreCase);
        System.IO.Compression.ZipArchive zip = null;
        try
        {
            string ext = System.IO.Path.GetExtension(picked).ToLowerInvariant();
            if (IsVideo(picked))
            {
                string full = picked;
                files[System.IO.Path.GetFileName(picked)] = new PackFile { Size = new System.IO.FileInfo(picked).Length, Open = () => System.IO.File.OpenRead(full) };
            }
            else if (ext == ".zip")
            {
                zip = System.IO.Compression.ZipFile.OpenRead(picked);
                foreach (var e in zip.Entries)
                {
                    if (e.FullName.EndsWith("/") || e.FullName.EndsWith("\\")) continue;
                    var entry = e;
                    files[e.FullName.Replace('\\', '/')] = new PackFile { Size = e.Length, Open = () => entry.Open() };
                }
            }
            else
            {
                string root = System.IO.Path.GetDirectoryName(System.IO.Path.GetFullPath(picked));
                int n = 0;
                foreach (var f in System.IO.Directory.EnumerateFiles(root, "*", System.IO.SearchOption.AllDirectories))
                {
                    if (++n > 4000) break;
                    string full = f;
                    files[f.Substring(root.Length).TrimStart('\\').Replace('\\', '/')] = new PackFile { Size = new System.IO.FileInfo(f).Length, Open = () => System.IO.File.OpenRead(full) };
                }
            }

            // bildirim dosyası: en kısa yoldaki LivelyInfo.json ya da project.json; diğer dosyalar onun klasörüne göre
            string manifest = null;
            foreach (var k in files.Keys)
            {
                string fn = k.Substring(k.LastIndexOf('/') + 1);
                if ((fn.Equals("LivelyInfo.json", StringComparison.OrdinalIgnoreCase) || fn.Equals("project.json", StringComparison.OrdinalIgnoreCase))
                    && (manifest == null || k.Length < manifest.Length)) manifest = k;
            }
            string prefix = manifest == null ? "" : manifest.Substring(0, manifest.LastIndexOf('/') + 1);
            string title = System.IO.Path.GetFileNameWithoutExtension(picked), author = "", file = "", preview = "";
            if (manifest != null && files[manifest].Size < 512 * 1024)
            {
                Dictionary<string, object> info = null;
                using (var st = files[manifest].Open())
                using (var rd = new System.IO.StreamReader(st, Encoding.UTF8))
                {
                    try { info = json.Deserialize<Dictionary<string, object>>(rd.ReadToEnd()); }
                    catch (Exception ex) { Slider.Log("canlı duvar kağıdı paketi okunamadı: " + ex.Message); }
                }
                if (info != null)
                {
                    bool lively = manifest.EndsWith("LivelyInfo.json", StringComparison.OrdinalIgnoreCase);
                    // Wallpaper Engine'in sahne ve web duvar kağıtları kendi motorunu ister
                    if (!lively && info.ContainsKey("type") && !string.Equals(Str(info, "type", ""), "video", StringComparison.OrdinalIgnoreCase))
                        throw new NotSupportedException("type");
                    title = Str(info, lively ? "Title" : "title", title);
                    author = lively ? Str(info, "Author", "") : "";
                    file = System.IO.Path.GetFileName(Str(info, lively ? "FileName" : "file", "").Replace('\\', '/').Replace('/', System.IO.Path.DirectorySeparatorChar));
                    preview = System.IO.Path.GetFileName(Str(info, lively ? "Thumbnail" : "preview", ""));
                }
            }
            string video = null;
            if (file.Length > 0 && IsVideo(file) && files.ContainsKey(prefix + file)) video = prefix + file;
            if (video == null)
                foreach (var kv in files)
                    if (IsVideo(kv.Key) && (video == null || kv.Value.Size > files[video].Size)) video = kv.Key;
            if (video == null) throw new NotSupportedException("type");
            if (files[video].Size > MAX_VIDEO) throw new NotSupportedException("size");

            string name = Slug(title);
            string dir = System.IO.Path.Combine(Dir, name);
            System.IO.Directory.CreateDirectory(dir);
            // oynatıcı yolu adres gibi okur: # ve % gibi işaretler dosya adına girmesin
            string vname = video.Substring(video.LastIndexOf('/') + 1);
            string dst = System.IO.Path.Combine(dir, Slug(System.IO.Path.GetFileNameWithoutExtension(vname)) + System.IO.Path.GetExtension(vname).ToLowerInvariant());
            bool same = IsVideo(picked) && string.Equals(System.IO.Path.GetFullPath(picked), dst, StringComparison.OrdinalIgnoreCase);
            if (!same)
            {
                using (var src = files[video].Open())
                using (var outf = System.IO.File.Create(dst)) src.CopyTo(outf);
                // aynı adlı başka bir videonun karesi kalmasın
                try { System.IO.File.Delete(System.IO.Path.ChangeExtension(dst, ".frame.png")); } catch { }
            }
            System.IO.File.SetLastWriteTimeUtc(dst, DateTime.UtcNow);
            string pext = System.IO.Path.GetExtension(preview).ToLowerInvariant();
            if ((pext == ".jpg" || pext == ".jpeg" || pext == ".png") && files.ContainsKey(prefix + preview) && files[prefix + preview].Size < 8L << 20)
            {
                try
                {
                    using (var src = files[prefix + preview].Open())
                    using (var outf = System.IO.File.Create(System.IO.Path.Combine(dir, "cover.jpg"))) src.CopyTo(outf);
                }
                catch (Exception ex) { Slider.Log("canlı duvar kağıdı kapağı alınamadı: " + ex.Message); }
            }
            if (manifest != null)
                System.IO.File.WriteAllText(System.IO.Path.Combine(dir, "info.json"),
                    json.Serialize(new Dictionary<string, object> { { "Title", title }, { "Author", author } }), new UTF8Encoding(false));
            return dst;
        }
        finally { if (zip != null) zip.Dispose(); }
    }

    // --live-local -> kütüphanedeki videolar, en yeni önce: [{"path","name","author","thumb"}] (thumb: kapak ya da kare)
    public static string Local()
    {
        var list = new List<Dictionary<string, object>>();
        foreach (var dir in new System.IO.DirectoryInfo(Dir).GetDirectories())
        {
            System.IO.FileInfo video = null;
            foreach (var f in dir.GetFiles()) if (IsVideo(f.Name) && (video == null || f.LastWriteTimeUtc > video.LastWriteTimeUtc)) video = f;
            if (video == null) continue;
            string frame = System.IO.Path.ChangeExtension(video.FullName, ".frame.png"), cover = System.IO.Path.Combine(dir.FullName, "cover.jpg");
            var item = new Dictionary<string, object>
            {
                { "path", video.FullName }, { "name", dir.Name }, { "author", "" },
                { "thumb", System.IO.File.Exists(cover) ? cover : System.IO.File.Exists(frame) ? frame : "" },
                { "time", (long)(video.LastWriteTimeUtc - new DateTime(1970, 1, 1)).TotalMilliseconds },
            };
            // mağazadan gelenler: başlık ve yapan
            string infoPath = System.IO.Path.Combine(dir.FullName, "info.json");
            if (System.IO.File.Exists(infoPath))
            {
                try
                {
                    var info = json.Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(infoPath));
                    item["name"] = Str(info, "Title", dir.Name);
                    item["author"] = Str(info, "Author", "");
                }
                catch (Exception ex) { Slider.Log("canlı duvar kağıdı bilgisi okunamadı: " + infoPath + ": " + ex.Message); }
            }
            list.Add(item);
        }
        list.Sort((a, b) => ((long)b["time"]).CompareTo((long)a["time"]));
        return json.Serialize(list);
    }

    static string Str(Dictionary<string, object> d, string key, string fallback)
    {
        object v;
        return d != null && d.TryGetValue(key, out v) && v is string && ((string)v).Length > 0 ? (string)v : fallback;
    }

    // Mağaza adresi: kategori klasörü ("src/Anime"), duvar kağıdının klasörü, dosya
    static string Url(string source, string id, string file)
    {
        var sb = new StringBuilder(STORE);
        foreach (var part in source.Split('/')) sb.Append(Uri.EscapeDataString(part)).Append('/');
        return sb.Append(Uri.EscapeDataString(id)).Append('/').Append(Uri.EscapeDataString(file)).ToString();
    }

    // Ağdan sınırlı okuma: en çok max bayt, toplamda FETCH_DEADLINE; progress: (alınan, toplam ya da -1) her ~256 KB'de
    static void Download(string url, System.IO.Stream dst, long max, Action<long, long> progress)
    {
        System.Net.ServicePointManager.SecurityProtocol = (System.Net.SecurityProtocolType)3072; // TLS 1.2
        var rq = (System.Net.HttpWebRequest)System.Net.WebRequest.Create(url);
        rq.UserAgent = "LogicalLunge/1.0 (live wallpaper)";
        rq.Proxy = null;
        rq.Timeout = 30000;
        rq.ReadWriteTimeout = 30000;
        var started = Stopwatch.StartNew();
        using (var rs = rq.GetResponse())
        using (var src = rs.GetResponseStream())
        {
            long total = rs.ContentLength, got = 0, shown = 0;
            if (total > max) throw new NotSupportedException("size");
            var buf = new byte[1 << 16];
            int n;
            while ((n = src.Read(buf, 0, buf.Length)) > 0)
            {
                got += n;
                if (got > max) throw new NotSupportedException("size");
                if (started.Elapsed > FETCH_DEADLINE) throw new TimeoutException("slow");
                dst.Write(buf, 0, n);
                if (progress != null && got - shown >= 256 * 1024) { shown = got; progress(got, total); }
            }
            // bağlantı yarıda kesildiyse eksik dosya "indirildi" sayılmasın
            if (total >= 0 && got != total) throw new System.IO.IOException("truncated");
        }
    }

    static string DownloadText(string url, long max)
    {
        using (var ms = new System.IO.MemoryStream())
        {
            Download(url, ms, max, null);
            return new UTF8Encoding(false).GetString(ms.ToArray()).TrimStart('\uFEFF');
        }
    }

    // Dosyaya: önce .part, tamamlanınca yerine (yarım dosya hiç görünmez)
    static void DownloadFile(string url, string file, long max, bool report)
    {
        string tmp = file + "." + Process.GetCurrentProcess().Id + ".part";
        try
        {
            using (var dst = System.IO.File.Create(tmp))
                Download(url, dst, max, report ? (Action<long, long>)((got, total) =>
                {
                    var inv = System.Globalization.CultureInfo.InvariantCulture;
                    try { System.IO.File.WriteAllText(ProgressFile, got.ToString(inv) + " " + total.ToString(inv)); } catch { }
                }) : null);
            if (System.IO.File.Exists(file)) System.IO.File.Delete(file);
            System.IO.File.Move(tmp, file);
        }
        finally
        {
            try { System.IO.File.Delete(tmp); } catch { }
            if (report) try { System.IO.File.Delete(ProgressFile); } catch { }
        }
    }

    // Mağaza dizini (günde bir kez tazelenir; ağ yoksa eldeki kullanılır). Arayüz açılışta iki komut birden çalıştırır:
    // dosya bir kopyada yazılıp yerine konur, okuyan hep tam bir dosya görür
    static Dictionary<string, object> Categories()
    {
        for (int attempt = 0; ; attempt++)
        {
            var f = new System.IO.FileInfo(StoreCache);
            if (!f.Exists || DateTime.UtcNow - f.LastWriteTimeUtc > TimeSpan.FromDays(1))
            {
                try
                {
                    string text = DownloadText(STORE + "src/Store.json", 32L << 20);
                    if (json.Deserialize<Dictionary<string, object>>(text).ContainsKey("Categories"))
                    {
                        string tmp = StoreCache + "." + Process.GetCurrentProcess().Id + ".tmp";
                        System.IO.File.WriteAllText(tmp, text, new UTF8Encoding(false));
                        if (System.IO.File.Exists(StoreCache)) System.IO.File.Replace(tmp, StoreCache, null);
                        else System.IO.File.Move(tmp, StoreCache);
                    }
                }
                catch (Exception ex)
                {
                    // ağ yok: eldeki dizin (yoksa hata); başka bir kopya o an yazdıysa onunki
                    if (!System.IO.File.Exists(StoreCache)) throw new System.Net.WebException("store", ex);
                    Slider.Log("canlı duvar kağıdı mağazası tazelenemedi: " + ex.Message);
                }
            }
            try
            {
                object cats;
                var index = json.Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(StoreCache));
                return index != null && index.TryGetValue("Categories", out cats) && cats is Dictionary<string, object> ? (Dictionary<string, object>)cats : new Dictionary<string, object>();
            }
            catch (Exception ex)
            {
                // bozuk önbellek kendiliğinden düzelsin: silinir, bir kez yeniden indirilir
                Slider.Log("canlı duvar kağıdı mağaza önbelleği bozuk: " + ex.Message);
                try { System.IO.File.Delete(StoreCache); } catch { }
                if (attempt > 0) throw;
            }
        }
    }

    // Bir kategorinin duvar kağıtları; yetişkin içerik (ya da işaretsiz olan) hiç listelenmez
    static List<KeyValuePair<string, Dictionary<string, object>>> Items(object category)
    {
        var list = new List<KeyValuePair<string, Dictionary<string, object>>>();
        var c = category as Dictionary<string, object>;
        object all;
        if (c == null || !c.TryGetValue("Wallpapers", out all) || !(all is Dictionary<string, object>)) return list;
        foreach (var kv in (Dictionary<string, object>)all)
        {
            var it = kv.Value as Dictionary<string, object>;
            object adult;
            if (it == null || !it.TryGetValue("Adult", out adult) || !(adult is bool) || (bool)adult) continue;
            // klasör ve adres parçası olacak: düz bir ad
            if (kv.Key.Trim('.').Length == 0 || kv.Key.IndexOfAny(new[] { '/', '\\' }) >= 0) continue;
            if (!System.Text.RegularExpressions.Regex.IsMatch(Str(it, "Source", ""), @"^src/[^/\\.][^/\\]*$")) continue;
            list.Add(new KeyValuePair<string, Dictionary<string, object>>(kv.Key, it));
        }
        return list;
    }

    // --live-store -> [{"id","count"}] (kalabalık olan önce) | --live-store <kategori> -> [{"id","title","cover","preview"}]
    public static string Store(string category)
    {
        var cats = Categories();
        var list = new List<Dictionary<string, object>>();
        if (string.IsNullOrEmpty(category))
        {
            foreach (var kv in cats)
            {
                int n = Items(kv.Value).Count;
                if (n > 0) list.Add(new Dictionary<string, object> { { "id", kv.Key }, { "count", n } });
            }
            list.Sort((a, b) => ((int)b["count"]).CompareTo((int)a["count"]));
            return json.Serialize(list);
        }
        object c;
        if (cats.TryGetValue(category, out c))
            foreach (var kv in Items(c))
            {
                string source = Str(kv.Value, "Source", "");
                list.Add(new Dictionary<string, object>
                {
                    { "id", kv.Key },
                    { "title", System.Text.RegularExpressions.Regex.Replace(kv.Key, @"-\d+$", "") },
                    { "cover", Url(source, kv.Key, Str(kv.Value, "Cover", "thumbnail.jpg")) },
                    { "preview", Url(source, kv.Key, Str(kv.Value, "Live", "preview.gif")) },
                });
            }
        return json.Serialize(list);
    }

    // --live-get <kategori> <ad> <mod>: mağazadan indir (bir kez), uygula (mod "none": uygulamaz) -> {"ok":true,"path"}
    // Klasör: kategori + ad (aynı ad birden çok kategoride başka videolarla geçiyor)
    public static string Get(string category, string id, string mode)
    {
        object c;
        Dictionary<string, object> item = null;
        if (Categories().TryGetValue(category, out c))
            foreach (var kv in Items(c)) if (kv.Key == id) { item = kv.Value; break; }
        if (item == null) throw new ArgumentException("store item");
        string source = Str(item, "Source", "");
        string dir = System.IO.Path.Combine(Dir, Slug(category) + " - " + Slug(id));
        System.IO.Directory.CreateDirectory(dir);
        var info = json.Deserialize<Dictionary<string, object>>(DownloadText(Url(source, id, "SucroseInfo.json"), 256 * 1024));
        object type;
        string file = Str(info, "Source", "");
        // yalnızca video türü (3); web sayfası, uygulama ve YouTube duvar kağıtları oynatılmaz
        if (info == null || !info.TryGetValue("Type", out type) || !(type is int) || (int)type != 3 || !IsVideo(file) || file.IndexOfAny(new[] { '/', '\\', ':' }) >= 0)
            throw new NotSupportedException("type");
        // yerel ad güvenli: oynatıcı yolu adres gibi okur (# ve % işaret olur)
        string video = System.IO.Path.Combine(dir, Slug(System.IO.Path.GetFileNameWithoutExtension(file)) + System.IO.Path.GetExtension(file).ToLowerInvariant());
        if (!System.IO.File.Exists(video)) DownloadFile(Url(source, id, file), video, MAX_VIDEO, true);
        string cover = System.IO.Path.Combine(dir, "cover.jpg");
        if (!System.IO.File.Exists(cover))
        {
            try { DownloadFile(Url(source, id, Str(item, "Cover", "thumbnail.jpg")), cover, 8L << 20, false); }
            catch (Exception ex) { Slider.Log("canlı duvar kağıdı kapağı indirilemedi: " + ex.Message); }
        }
        // atıf: yapan, lisans, kaynak (kütüphane gösterir)
        info["Store"] = "https://github.com/Taiizor/Store/tree/develop/" + source + "/" + Uri.EscapeDataString(id);
        System.IO.File.WriteAllText(System.IO.Path.Combine(dir, "info.json"), json.Serialize(info), new UTF8Encoding(false));
        // "none": yalnızca indir (ekran koruyucu için)
        if (mode != "none") Set(video, mode);
        return json.Serialize(new Dictionary<string, object> { { "ok", true }, { "path", video } });
    }

    // --live-progress -> {"got","total"} (total -1: bilinmiyor); indirme yoksa {}
    public static string Progress()
    {
        try
        {
            var p = System.IO.File.ReadAllText(ProgressFile).Split(' ');
            long got = long.Parse(p[0], System.Globalization.CultureInfo.InvariantCulture), total = long.Parse(p[1], System.Globalization.CultureInfo.InvariantCulture);
            return "{\"got\":" + got.ToString(System.Globalization.CultureInfo.InvariantCulture) + ",\"total\":" + total.ToString(System.Globalization.CultureInfo.InvariantCulture) + "}";
        }
        catch { return "{}"; }
    }
}

// ---------------- Alt süreçleri helper'a bağla (helper kapanınca onlar da kapansın) ----------------
static class KillJob
{
    [StructLayout(LayoutKind.Sequential)] struct BASIC { public long PerProcessUserTimeLimit, PerJobUserTimeLimit; public uint LimitFlags; public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize; public uint ActiveProcessLimit; public UIntPtr Affinity; public uint PriorityClass, SchedulingClass; }
    [StructLayout(LayoutKind.Sequential)] struct IO { public ulong a, b, c, d, e, f; }
    [StructLayout(LayoutKind.Sequential)] struct EXTENDED { public BASIC Basic; public IO Io; public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr a, string name);
    [DllImport("kernel32.dll")] static extern bool SetInformationJobObject(IntPtr job, int cls, ref EXTENDED info, int len);
    [DllImport("kernel32.dll")] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    static IntPtr job;
    public static void Attach(Process p)
    {
        if (job == IntPtr.Zero)
        {
            job = CreateJobObject(IntPtr.Zero, null);
            var info = new EXTENDED();
            info.Basic.LimitFlags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            SetInformationJobObject(job, 9, ref info, Marshal.SizeOf(typeof(EXTENDED)));
        }
        AssignProcessToJobObject(job, p.Handle);
    }
}
// ---------------- Varsayılan ses cihazı (ii ses menüsü: çıkış / giriş cihazı seçimi) ----------------
// Windows'un belgelenmemiş ama Windows 7'den 11'e kadar aynı kalan IPolicyConfig arayüzü (Ses ayarları da bunu kullanır).
[ComImport, Guid("f8679f50-850a-41cf-9c72-430f290290c8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IPolicyConfig
{
    [PreserveSig] int GetMixFormat([MarshalAs(UnmanagedType.LPWStr)] string id, IntPtr format);
    [PreserveSig] int GetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string id, bool def, IntPtr format);
    [PreserveSig] int ResetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string id);
    [PreserveSig] int SetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string id, IntPtr endpointFormat, IntPtr mixFormat);
    [PreserveSig] int GetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string id, bool def, IntPtr defPeriod, IntPtr minPeriod);
    [PreserveSig] int SetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string id, IntPtr period);
    [PreserveSig] int GetShareMode([MarshalAs(UnmanagedType.LPWStr)] string id, IntPtr mode);
    [PreserveSig] int SetShareMode([MarshalAs(UnmanagedType.LPWStr)] string id, IntPtr mode);
    [PreserveSig] int GetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string id, bool fxStore, IntPtr key, IntPtr value);
    [PreserveSig] int SetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string id, bool fxStore, IntPtr key, IntPtr value);
    [PreserveSig] int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string id, int role);
    [PreserveSig] int SetEndpointVisibility([MarshalAs(UnmanagedType.LPWStr)] string id, bool visible);
}
[ComImport, Guid("870af99c-171d-4f9e-af0d-e63df40c2bc9")] class CPolicyConfigClient { }

static class AudioDefault
{
    // eConsole, eMultimedia, eCommunications: Ses ayarlarındaki "varsayılan" üçünü birden değiştirir
    public static int Set(string id)
    {
        var pc = (IPolicyConfig)new CPolicyConfigClient();
        int hr = 0;
        for (int role = 0; role < 3; role++) { int r = pc.SetDefaultEndpoint(id, role); if (r != 0) hr = r; }
        Marshal.ReleaseComObject(pc);
        return hr;
    }
}
// ---------------- Terminal: arka planda hazır bekleyen WezTerm ----------------
static class WarmTerminal
{
    static string Home { get { return Environment.GetFolderPath(Environment.SpecialFolder.UserProfile); } }
    static string Request { get { return System.IO.Path.Combine(Home, @".config\wezterm\ll-spawn"); } }

    public static bool SplashActive() { Mutex m; if (Mutex.TryOpenExisting("lunge-splash", out m)) { m.Dispose(); return true; } return false; }

    static bool IsWezterm(string path) { return path != null && path.EndsWith("wezterm-gui.exe", StringComparison.OrdinalIgnoreCase); }

    // Kurulumdaki WezTerm'in arka planda bekleyen süreci (0: yok)
    static int ResidentPid(string path)
    {
        int found = 0;
        foreach (var pr in Process.GetProcessesByName("wezterm-gui"))
        {
            try { if (found == 0 && string.Equals(ProcInfo.Path((uint)pr.Id), path, StringComparison.OrdinalIgnoreCase)) found = pr.Id; }
            catch { }
            finally { pr.Dispose(); }
        }
        return found;
    }

    static bool Resident(string path) { return ResidentPid(path) != 0; }

    // İstek o sürece özeldir (ll-spawn.<pid>): başka bir WezTerm (ör. eski kurulumdan kalıp kendi OpenConsole'unu
    // bulamayan) isteği kapıp açamıyordu, terminal hiç gelmiyordu. WezTerm sonucu .ok / .failed ile bildirir. İstek
    // alınmaz, açılamaz ya da sonuç gelmezse false: çağıran normal açılışa düşer; terminal hiçbir durumda kaybolmaz.
    public static bool TrySpawn(string path)
    {
        if (!IsWezterm(path)) return false;
        int pid = ResidentPid(path);
        if (pid == 0) return false;
        string req = Request + "." + pid, ok = req + ".ok", failed = req + ".failed";
        try
        {
            System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(Request));
            foreach (var f in new[] { ok, failed }) { try { System.IO.File.Delete(f); } catch { } }
            System.IO.File.WriteAllText(req, Home);
        }
        catch { return false; }
        var sw = Stopwatch.StartNew();
        // WezTerm dosyaya 40 ms'de bir bakar
        while (System.IO.File.Exists(req) && sw.ElapsedMilliseconds < 500) Thread.Sleep(10);
        if (System.IO.File.Exists(req))
        {
            try { System.IO.File.Delete(req); } catch { }
            Slider.Log("terminal: istek " + pid + " alınmadı, normal açılış");
            return false;
        }
        while (sw.ElapsedMilliseconds < 2500)
        {
            if (System.IO.File.Exists(ok)) { try { System.IO.File.Delete(ok); } catch { } Slider.Log("terminal: sıcak açılış " + pid + ", " + sw.ElapsedMilliseconds + " ms"); return true; }
            if (System.IO.File.Exists(failed)) { try { System.IO.File.Delete(failed); } catch { } Slider.Log("terminal: WezTerm " + pid + " açamadı, normal açılış"); return false; }
            Thread.Sleep(10);
        }
        Slider.Log("terminal: WezTerm " + pid + " sonuç bildirmedi, normal açılış");
        return false;
    }

    // Oturum açılışında (açılış perdesi ekranı örterken) WezTerm'i başlat ve ilk penceresini kapat:
    // quit_when_all_windows_are_closed = false olduğu için süreç arka planda kalır, ilk Super+Enter da anında açılır.
    public static void Prewarm(string path)
    {
        if (!IsWezterm(path) || !System.IO.File.Exists(path) || Resident(path)) return;
        try
        {
            // Terminal kullanıcı olarak açılır (çekirdek yönetici olsa da): yeni süreç, öncekilerde olmayan kimliğinden bulunur
            string name = System.IO.Path.GetFileNameWithoutExtension(path);
            var before = new HashSet<int>();
            foreach (var p in Process.GetProcessesByName(name)) { before.Add(p.Id); p.Dispose(); }
            var sw = Stopwatch.StartNew();
            if (!UserLaunch.Start(path, "", Home)) return;
            while (sw.ElapsedMilliseconds < 8000)
            {
                Thread.Sleep(50);
                foreach (var p in Process.GetProcessesByName(name))
                {
                    using (p)
                    {
                        if (before.Contains(p.Id)) continue;
                        IntPtr h = p.MainWindowHandle;
                        if (h == IntPtr.Zero) continue;
                        Native.PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); // WM_CLOSE
                        Slider.Log("terminal: ön-ısıtıldı " + sw.ElapsedMilliseconds + "ms");
                        return;
                    }
                }
            }
        }
        catch (Exception ex) { Slider.Log("terminal ön-ısıtma: " + ex.Message); }
    }
}
// ---------------- Açılış perdesi ----------------
// Sistem genelindeki olay kancalarının (WinEvent) gecikmesi: olayın üretildiği an (dwmsEventTime) ile bize ulaştığı an
// arası. Bir uygulama olay seli ürettiğinde (ör. Görev Yöneticisi'nin listesi yeniden sıralanırken) kuyruk birikirse
// kaydedilir: bir dahaki kasmanın kaynağı tahminle değil kayıtla bulunsun. Ucuz: çağrı başına bir karşılaştırma.
// Masaüstü hazır mı (açılış örtüsü sorar, POST /desktop-ready): çekirdeğin çalıştırdığı her parça gelmiş olmalı. Pencere
// yöneticisi cevap veriyor; kabuğun açtığı her bar bu açılışta "canlıyım" demiş; canlı duvar kağıdı ayarlıysa oynatıcı
// bütün ekranlarında ilk karesini göstermiş. Örtü ancak hepsi gelince kalkar: bar, pencereler, duvar kağıdı gözün önünde
// tek tek gelmez.
static class DesktopReady
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    const uint WM_APP_WAITING = 0x8000 + 2;   // lunge-wallpaper: ilk karesini bekleyen ekran sayısı

    // null: hazır; değilse beklenenler
    public static string Waiting()
    {
        var missing = new List<string>();
        if (!Supervisor.TilingIpcUp()) missing.Add("pencere yöneticisi");
        string bars = ShellWatchdog.BarsReady();
        if (bars != null) missing.Add(bars);
        string wall = Wallpaper();
        if (wall != null) missing.Add(wall);
        return missing.Count == 0 ? null : string.Join(", ", missing.ToArray());
    }

    public static string Json()
    {
        string w = Waiting();
        return w == null ? "{\"ready\":true}" : "{\"ready\":false,\"waiting\":" + new JavaScriptSerializer().Serialize(w) + "}";
    }

    static string Wallpaper()
    {
        string config;
        try { config = System.IO.File.ReadAllText(Paths.State("live-wallpaper.json")); } catch { return null; }
        // ayarlı bir video yoksa beklenecek bir şey yok
        if (!System.Text.RegularExpressions.Regex.IsMatch(config, @"""file""\s*:\s*""[^""]+""")) return null;
        IntPtr player = FindWindow("LogicalLunge.LiveWallpaper", null);
        if (player == IntPtr.Zero) return "canlı duvar kağıdı";
        IntPtr waiting;
        if (SendMessageTimeout(player, WM_APP_WAITING, IntPtr.Zero, IntPtr.Zero, 0x2 /*SMTO_ABORTIFHUNG*/, 300, out waiting) == IntPtr.Zero) return "canlı duvar kağıdı";
        return waiting == IntPtr.Zero ? null : "canlı duvar kağıdı " + waiting + " ekran";
    }
}

// Açılış örtüsünün kendi oturum açma görevi (\LogicalLunge\Splash): oturum açılınca çekirdeği beklemeden, kullanıcı olarak
// ve yüksek öncelikle gelir; çekirdek yönetici olarak ayağa kalkarken (yük altında saniyeler sürebiliyor) örtü çoktan ekranda.
// Çekirdek de açılışta örtüyü başlatır (yedek; tek kopya kilidi ikisini birleştirir). Görevi yönetici çekirdek her açılışta
// yazar (yol güncel kalsın); kaldırıcı \LogicalLunge\ klasörünün bütün görevlerini siler.
static class SplashTask
{
    public const string Name = "Splash";

    public static string Xml(string sid, string exe, string home)
    {
        Func<string, string> e = System.Security.SecurityElement.Escape;
        return "<?xml version=\"1.0\" encoding=\"UTF-16\"?>" +
            "<Task version=\"1.3\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">" +
            "<RegistrationInfo><Description>Logical Lunge: açılış ekranı</Description></RegistrationInfo>" +
            "<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>" + e(sid) + "</UserId></LogonTrigger></Triggers>" +
            "<Principals><Principal id=\"A\"><UserId>" + e(sid) + "</UserId><LogonType>InteractiveToken</LogonType>" +
            "<RunLevel>LeastPrivilege</RunLevel></Principal></Principals>" +
            "<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>" +
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>" +
            "<ExecutionTimeLimit>PT5M</ExecutionTimeLimit><Priority>1</Priority>" +
            "<UseUnifiedSchedulingEngine>true</UseUnifiedSchedulingEngine></Settings>" +
            "<Actions Context=\"A\"><Exec><Command>" + e(exe) + "</Command><Arguments>--splash</Arguments>" +
            "<WorkingDirectory>" + e(home) + "</WorkingDirectory></Exec></Actions></Task>";
    }

    public static void Ensure()
    {
        if (!UserLaunch.Elevated) return;
        object service = null, folder = null;
        try
        {
            service = Activator.CreateInstance(Type.GetTypeFromProgID("Schedule.Service"));
            DesktopRestart.Com(service, "Connect", false, null, null, null, null);
            folder = DesktopRestart.Com(service, "GetFolder", false, @"\LogicalLunge");
            using (var own = System.Security.Principal.WindowsIdentity.GetCurrent())
                DesktopRestart.ReleaseCom(DesktopRestart.Com(folder, "RegisterTask", false, Name, Xml(own.User.Value, Application.ExecutablePath, Paths.Home), 6 /*CREATE_OR_UPDATE*/, null, null, 3 /*INTERACTIVE_TOKEN*/, null));
        }
        catch (Exception ex) { Slider.Log("açılış örtüsü görevi yazılamadı: " + ex.GetBaseException().Message); }
        finally { DesktopRestart.ReleaseCom(folder); DesktopRestart.ReleaseCom(service); }
    }
}

// Sıcaklık servisinin dosyası (lunge-temps.exe), arayüze süreç başlatmadan. Okuyanın işaretini de koyar (servis sensörleri
// yalnızca son 10 sn'de biri okumak istediyse okur); servis çalışmıyorsa en fazla dakikada bir --read ile uyandırılır.
static class TempsFile
{
    public static string Path = @"C:\Users\Public\lunge-temps.json", Want = @"C:\Users\Public\lunge-temps.want";   // testler değiştirir
    static int lastWake;

    public static string Json()
    {
        try
        {
            if (!System.IO.File.Exists(Want)) System.IO.File.WriteAllText(Want, "");
            System.IO.File.SetLastWriteTimeUtc(Want, DateTime.UtcNow);
        }
        catch { }
        try
        {
            var fi = new System.IO.FileInfo(Path);
            if (fi.Exists && (DateTime.UtcNow - fi.LastWriteTimeUtc).TotalSeconds < 15) return System.IO.File.ReadAllText(Path);
        }
        catch { }
        int now = Environment.TickCount;
        if (lastWake == 0 || now - lastWake > 60000)
        {
            lastWake = now;
            ThreadPool.QueueUserWorkItem(_ =>
            {
                try
                {
                    var psi = new ProcessStartInfo(Paths.Tool(@"temps\lunge-temps.exe"), "--read") { CreateNoWindow = true, UseShellExecute = false, RedirectStandardOutput = true };
                    using (var p = Process.Start(psi)) { p.StandardOutput.ReadToEnd(); p.WaitForExit(5000); }
                }
                catch { }
            });
        }
        return "{\"running\":false}";
    }
}

// core.log'u yazan tek iş parçacığı: satırlar kuyruktan toplu yazılır. Birden çok süreç (çekirdek, yardımcı kipleri) aynı
// dosyaya yazdığı için dosya her toplu yazımda ekleme kipinde açılır. 4 MB'yi geçince eskisi .old olur; taşınamazsa (bir
// okuyucu tutuyorsa) 8 MB'de baştan başlar. Disk takılırsa kuyruk sınırlı kalır, atılanların sayısı yazılır.
static class LogWriter
{
    public static string Path = System.IO.Path.Combine(Paths.LogsDir, "core.log");   // testler değiştirir
    public static long RotateBytes = 4L * 1024 * 1024;
    const int MaxQueued = 20000;
    static readonly Queue<KeyValuePair<DateTime, string>> queue = new Queue<KeyValuePair<DateTime, string>>();
    static readonly object gate = new object();
    static int dropped, writing;
    static DateTime day = DateTime.MinValue;
    static Thread thread;

    public static void Add(DateTime at, string line)
    {
        lock (gate)
        {
            if (queue.Count >= MaxQueued) { queue.Dequeue(); dropped++; }
            queue.Enqueue(new KeyValuePair<DateTime, string>(at, line));
            if (thread == null)
            {
                thread = new Thread(Run) { IsBackground = true, Name = "core-log", Priority = ThreadPriority.BelowNormal };
                thread.Start();
                AppDomain.CurrentDomain.ProcessExit += (s, e) => Flush(2000);
            }
            Monitor.PulseAll(gate);
        }
    }

    // Kuyruk diske inene kadar bekler (en fazla timeoutMs): kapanırken ve çökme kaydından sonra son satırlar kaybolmasın
    public static bool Flush(int timeoutMs)
    {
        var clock = Stopwatch.StartNew();
        lock (gate)
        {
            while (queue.Count > 0 || writing > 0 || unwritten)
            {
                long left = timeoutMs - clock.ElapsedMilliseconds;
                if (left <= 0) return false;
                Monitor.PulseAll(gate);
                Monitor.Wait(gate, (int)Math.Min(left, 50));
            }
        }
        return true;
    }

    static bool unwritten;   // yazılamayan bir toplu yazım elde (dosyayı biri tutuyor): yarım saniyede bir yeniden denenir

    static void Run()
    {
        var pending = new StringBuilder();
        while (true)
        {
            lock (gate)
            {
                while (queue.Count == 0 && pending.Length == 0) Monitor.Wait(gate);
                if (queue.Count == 0) Monitor.Wait(gate, 500);
                if (dropped > 0)
                {
                    pending.Append(DateTime.Now.ToString("HH:mm:ss.fff ")).Append("(disk yetişemedi: " + dropped + " satır atıldı)").Append(Environment.NewLine);
                    dropped = 0;
                }
                while (queue.Count > 0)
                {
                    var e = queue.Dequeue();
                    if (e.Key.Date != day) { pending.Append("---- ").Append(e.Key.ToString("yyyy-MM-dd")).Append(" ----").Append(Environment.NewLine); day = e.Key.Date; }
                    pending.Append(e.Key.ToString("HH:mm:ss.fff ")).Append(e.Value).Append(Environment.NewLine);
                }
                writing++;
            }
            bool ok = false;
            try { ok = Write(Encoding.UTF8.GetBytes(pending.ToString())); } catch { }
            if (ok) pending.Clear();
            else if (pending.Length > 4 * 1024 * 1024) pending.Remove(0, pending.Length - 2 * 1024 * 1024);   // en eskisi gider
            lock (gate) { writing--; unwritten = pending.Length > 0; Monitor.PulseAll(gate); }
        }
    }

    static bool Write(byte[] bytes)
    {
        var fi = new System.IO.FileInfo(Path);
        if (fi.Exists && fi.Length > RotateBytes)
        {
            try { System.IO.File.Delete(Path + ".old"); System.IO.File.Move(Path, Path + ".old"); }
            catch
            {
                if (fi.Length > 2 * RotateBytes)
                    try { using (new System.IO.FileStream(Path, System.IO.FileMode.Truncate, System.IO.FileAccess.Write, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete)) { } } catch { }
            }
        }
        for (int i = 0; i < 5; i++)
        {
            try
            {
                using (var fs = new System.IO.FileStream(Path, System.IO.FileMode.Append, System.IO.FileAccess.Write, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
                    fs.Write(bytes, 0, bytes.Length);
                return true;
            }
            catch (System.IO.IOException) { Thread.Sleep(20); }
        }
        return false;
    }
}

// Windows'un doğrudan çağırdığı fonksiyonlardan (kancalar, olay kancaları, pencere yordamları) istisna sızmaz: sızan
// istisna süreci 0xc000041d ile, log'a hiçbir şey yazmadan öldürüyordu. Her kayıt buradan geçer: hata yığın iziyle (aynı
// yer için dakikada bir) kaydedilir, çağrı varsayılan cevabıyla döner.
static class Callback
{
    // Kancanın iş parçacığında diske dokunmamak için kayıt başka iş parçacığında yazılır (testler yakalayabilsin diye değiştirilebilir)
    public static Action<string> Report = text => ThreadPool.QueueUserWorkItem(_ => Slider.Log(text));
    static readonly Dictionary<string, int> lastReported = new Dictionary<string, int>();

    public static void Failed(string where, Exception ex)
    {
        try
        {
            int now = Environment.TickCount, last;
            lock (lastReported)
            {
                if (lastReported.TryGetValue(where, out last) && now - last < 60000) return;
                lastReported[where] = now;
            }
            Report("geri çağrı hatası (" + where + "): " + ex);
        }
        catch { }
    }

    public static Native.LowLevelKeyboardProc Guard(string where, Native.LowLevelKeyboardProc f)
    {
        return (n, w, l) => { try { return f(n, w, l); } catch (Exception ex) { Failed(where, ex); return Native.CallNextHookEx(IntPtr.Zero, n, w, l); } };
    }

    public static Native.LowLevelMouseProc Guard(string where, Native.LowLevelMouseProc f)
    {
        return (n, w, l) => { try { return f(n, w, l); } catch (Exception ex) { Failed(where, ex); return Native.CallNextHookEx(IntPtr.Zero, n, w, l); } };
    }

    public static Native.WinEventDelegate Guard(string where, Native.WinEventDelegate f)
    {
        return (hook, ev, hwnd, obj, child, thread, time) => { try { f(hook, ev, hwnd, obj, child, thread, time); } catch (Exception ex) { Failed(where, ex); } };
    }
}

// Düşük seviye klavye ve fare kancaları sistemin bütün girdisini bekletir: kancanın iş parçacığı bir an durursa (çöp
// toplama, uzun boşlukta diske atılmış sayfaların geri okunması, işlemci sırası) her tuş ve fare hareketi o kadar gecikir.
// Kancanın içindeyken 2,2 sn'ye kadar duruş ölçülmüştü (çekirdeğin kendi tuşunda, kanca hemen dönerken bile).
static class InputLatency
{
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentThread();
    [DllImport("kernel32.dll")] static extern bool SetThreadPriority(IntPtr thread, int priority);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetProcessWorkingSetSizeEx(IntPtr process, UIntPtr min, UIntPtr max, uint flags);
    [DllImport("psapi.dll")] static extern bool GetProcessMemoryInfo(IntPtr process, out MemoryCounters counters, int size);
    [StructLayout(LayoutKind.Sequential)]
    struct MemoryCounters
    {
        public int cb; public uint PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage,
            QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage;
    }
    const int THREAD_PRIORITY_TIME_CRITICAL = 15;
    const uint QUOTA_LIMITS_HARDWS_MIN_DISABLE = 0x2, QUOTA_LIMITS_HARDWS_MAX_DISABLE = 0x8;
    // Yumuşak taban: Windows çekirdeğin sayfalarını bunun altına ancak bellek sıkışınca indirir
    public const long FloorBytes = 64L * 1024 * 1024;
    public const long SlowMs = 100;

    // Süreç: çöp toplayıcı her şeyi durduran tam toplama yapmasın (arka planda toplasın); uzun boşlukta sayfalar atılmasın
    public static void PrepareProcess()
    {
        try { System.Runtime.GCSettings.LatencyMode = System.Runtime.GCLatencyMode.SustainedLowLatency; } catch { }
        try
        {
            if (!SetProcessWorkingSetSizeEx(GetCurrentProcess(), (UIntPtr)(ulong)FloorBytes, (UIntPtr)(ulong)(1024L * 1024 * 1024),
                    QUOTA_LIMITS_HARDWS_MIN_DISABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE))
                Slider.Log("girdi gecikmesi: çalışma kümesi tabanı konamadı (" + Marshal.GetLastWin32Error() + ")");
        }
        catch { }
    }

    // Kanca iş parçacığı: gerçek zamanlı olmayan en yüksek öncelik (yalnızca girdi geldiğinde kısa süre çalışır)
    public static void PrepareThread()
    {
        try { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL); } catch { }
    }

    public struct Mark { public long Time; public int Gen0, Gen1, Gen2; }

    public static Mark Start()
    {
        return new Mark { Time = Stopwatch.GetTimestamp(), Gen0 = GC.CollectionCount(0), Gen1 = GC.CollectionCount(1), Gen2 = GC.CollectionCount(2) };
    }

    static uint faultsAtSample = Faults();
    // Kanca iş parçacığının saniyelik bekçisinden: yavaş bir kancada aradaki sayfa hataları bununla karşılaştırılır
    public static void Sample() { faultsAtSample = Faults(); }

    static uint Faults()
    {
        try
        {
            MemoryCounters c;
            return GetProcessMemoryInfo(GetCurrentProcess(), out c, Marshal.SizeOf(typeof(MemoryCounters))) ? c.PageFaultCount : 0;
        }
        catch { return 0; }
    }

    // Yavaşsa ("825 ms; çöp toplama 0/1/2 +1/+0/+0, son ölçümden beri sayfa hatası +512"), değilse null: hangisinin durdurduğu okunur
    public static string Slow(Mark m)
    {
        long ms = (Stopwatch.GetTimestamp() - m.Time) * 1000 / Stopwatch.Frequency;
        if (ms <= SlowMs) return null;
        return ms + " ms; çöp toplama 0/1/2 +" + (GC.CollectionCount(0) - m.Gen0) + "/+" + (GC.CollectionCount(1) - m.Gen1) + "/+" +
            (GC.CollectionCount(2) - m.Gen2) + ", son ölçümden beri sayfa hatası +" + unchecked(Faults() - faultsAtSample);
    }
}

static class EventLag
{
    static int lastLog, seen;

    public static void Note(string hook, uint time)
    {
        int lag = unchecked(Environment.TickCount - (int)time);
        Interlocked.Increment(ref seen);
        if (lag < 1000 || lag > 600000) return;
        int now = Environment.TickCount;
        int last = lastLog;
        if (now - last < 5000 || Interlocked.CompareExchange(ref lastLog, now, last) != last) return;
        Slider.Log("olay kancası gecikti: " + hook + " " + lag + " ms (son kayıttan beri " + Interlocked.Exchange(ref seen, 0) + " olay)");
    }
}

// ---------------- Kara kutu (performans bekçisi) ----------------
// "Sistem hiç kasmamalı": masaüstü yavaşladığında (DWM kareleri geç birleştiriyor ya da kaydırma / taşıma animasyonları
// kare kaçırıyor) o anki durumu log'a yazar: en çok işlemci ve GPU kullanan süreçler, bizim parçaların ve DWM'in
// kaynakları, ekran koruyucusu / kilitten ne zaman dönüldüğü. Bir sonraki kasmada nedeni tahminle değil kayıtla bulmak
// için. Beş dakikada en fazla bir kayıt; tam ekran oyun, sunum ve kilit ekranında ölçmez.
static class PerfGuard
{
    [DllImport("shell32.dll")] static extern int SHQueryUserNotificationState(out int state);
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr process, uint flags);

    static readonly object gate = new object();
    static readonly Queue<double> recent = new Queue<double>(); // son animasyonlarda zamanında çizilen karelerin oranı
    static int lastDump = Environment.TickCount - 600000;
    static int lastAway = -1;   // ekran koruyucusu / kilit ekranının en son görüldüğü an
    static int slowProbes;

    public static void Start()
    {
        new Thread(Loop) { IsBackground = true, Name = "perf-guard", Priority = ThreadPriority.BelowNormal }.Start();
    }

    // Her animasyonun sonunda (FrameStats.Report)
    public static void Record(int onTime, int missed)
    {
        int total = onTime + missed;
        if (total < 10) return; // kısa / yarıda kesilen animasyonlar yanıltır
        double avg = 0;
        lock (gate)
        {
            recent.Enqueue(onTime / (double)total);
            while (recent.Count > 5) recent.Dequeue();
            if (recent.Count < 5) return;
            foreach (var r in recent) avg += r;
            avg /= 5;
        }
        if (avg < 0.7)
            ThreadPool.QueueUserWorkItem(_ => Dump("animasyonlar kare kaçırıyor (son 5'te zamanında %" + Math.Round(avg * 100) + ")"));
    }

    // Tam ekran oyun / sunum / kilit ekranı: DWM ölçümü anlamsız
    static bool Quiet()
    {
        if (!FocusGuard.OnDefaultDesktop()) { lastAway = Environment.TickCount; return true; }
        int st;
        return SHQueryUserNotificationState(out st) == 0 && (st == 2 || st == 3 || st == 4); // BUSY, D3D_FULL_SCREEN, PRESENTATION_MODE
    }

    // Saatte bir parçaların belleği (ilki açılıştan bir dakika sonra): masaüstü aylarca açık kalır, bir sızıntı saatler içinde
    // büyüyen bir sayı olarak görünür. Tam ekran oyunda da yazılır.
    const int MemoryEveryMs = 3600000;
    static int lastMemory;

    static void Loop()
    {
        Thread.Sleep(60000);
        bool first = true;
        while (true)
        {
            try
            {
                int now = Environment.TickCount;
                if (first || now - lastMemory >= MemoryEveryMs)
                {
                    first = false;
                    lastMemory = now;
                    Slider.Log("bellek (saatlik): " + Parts());
                }
            }
            catch { }
            Thread.Sleep(20000);
            try
            {
                if (Quiet()) { slowProbes = 0; continue; }
                // DWM'in 8 kareyi ne kadar sürede birleştirdiği (~60 ms'lik iş)
                var sw = Stopwatch.StartNew();
                Native.DwmFlush();
                double start = sw.Elapsed.TotalMilliseconds;
                for (int i = 0; i < 8; i++) Native.DwmFlush();
                double avg = (sw.Elapsed.TotalMilliseconds - start) / 8, period = FrameStats.RefreshPeriodMs();
                if (avg <= period * 1.6) { slowProbes = 0; continue; }
                if (++slowProbes >= 2) Dump(string.Format("DWM yavaş: kare aralığı {0:0.0} ms (olması gereken {1:0.0} ms)", avg, period));
            }
            catch (Exception ex) { Slider.Log("kara kutu: " + ex.GetBaseException().Message); Thread.Sleep(60000); }
        }
    }

    // lunge.exe --black-box: kasma anında elle kayıt (bekleme süresine takılmaz)
    public static string DumpNow(string why) { lock (gate) lastDump = Environment.TickCount - 600000; return Dump(why); }

    static string Dump(string why)
    {
        lock (gate)
        {
            if (Environment.TickCount - lastDump < 300000) return null;
            lastDump = Environment.TickCount;
        }
        try
        {
            var sb = new StringBuilder("KARA KUTU: " + why);
            sb.Append("\n  işlemci (tek çekirdek %, 1,5 sn): ").Append(CpuTop());
            sb.Append("\n  gpu 3D (%): ").Append(GpuTop());
            sb.Append("\n  parçalar: ").Append(Parts());
            sb.Append("\n  ekran koruyucusu / kilitten dönüş: ")
              .Append(lastAway < 0 ? "yok (helper açıkken)" : Math.Round((Environment.TickCount - lastAway) / 60000.0, 1) + " dk önce");
            string record = sb.ToString();
            Slider.Log(record);
            return record;
        }
        catch (Exception ex) { Slider.Log("kara kutu: " + ex.GetBaseException().Message); return null; }
    }

    static string CpuTop()
    {
        var before = new Dictionary<int, double>();
        foreach (var p in Process.GetProcesses()) { try { before[p.Id] = p.TotalProcessorTime.TotalMilliseconds; } catch { } p.Dispose(); }
        var sw = Stopwatch.StartNew();
        Thread.Sleep(1500);
        var list = new List<KeyValuePair<string, double>>();
        foreach (var p in Process.GetProcesses())
        {
            try
            {
                double b;
                if (before.TryGetValue(p.Id, out b))
                    list.Add(new KeyValuePair<string, double>(p.ProcessName + "#" + p.Id, (p.TotalProcessorTime.TotalMilliseconds - b) / sw.Elapsed.TotalMilliseconds * 100));
            }
            catch { }
            p.Dispose();
        }
        list.Sort((a, b) => b.Value.CompareTo(a.Value));
        var sb = new StringBuilder();
        for (int i = 0; i < Math.Min(8, list.Count); i++) sb.AppendFormat("{0} {1:0} · ", list[i].Key, list[i].Value);
        return sb.ToString().TrimEnd(' ', '·');
    }

    // Süreç başına GPU 3D motor kullanımı (Windows'un "GPU Engine" sayaçları, 1 sn'lik örnek)
    static string GpuTop()
    {
        var cat = new PerformanceCounterCategory("GPU Engine");
        var counters = new List<KeyValuePair<int, PerformanceCounter>>();
        foreach (var inst in cat.GetInstanceNames())
        {
            if (!inst.Contains("engtype_3D")) continue;
            var m = System.Text.RegularExpressions.Regex.Match(inst, @"pid_(\d+)_");
            if (!m.Success) continue;
            var c = new PerformanceCounter("GPU Engine", "Utilization Percentage", inst, true);
            try { c.NextValue(); counters.Add(new KeyValuePair<int, PerformanceCounter>(int.Parse(m.Groups[1].Value), c)); } catch { c.Dispose(); }
        }
        Thread.Sleep(1000);
        var byPid = new Dictionary<int, double>();
        foreach (var kv in counters)
        {
            try { double v = kv.Value.NextValue(); double o; byPid.TryGetValue(kv.Key, out o); byPid[kv.Key] = o + v; } catch { }
            kv.Value.Dispose();
        }
        var list = new List<KeyValuePair<int, double>>(byPid);
        list.Sort((a, b) => b.Value.CompareTo(a.Value));
        var sb = new StringBuilder();
        foreach (var kv in list)
        {
            if (kv.Value < 1 || sb.Length > 300) break;
            string name = "?";
            try { using (var p = Process.GetProcessById(kv.Key)) name = p.ProcessName; } catch { }
            sb.AppendFormat("{0}#{1} {2:0} · ", name, kv.Key, kv.Value);
        }
        return sb.Length > 0 ? sb.ToString().TrimEnd(' ', '·') : "hepsi %1'in altında";
    }

    static string Parts()
    {
        var sb = new StringBuilder();
        foreach (var name in new[] { Names.Tiling, Names.Shell, Names.Core, LiveWallpaper.Name, "dwm" })
            foreach (var p in Process.GetProcessesByName(name))
            {
                try
                {
                    sb.AppendFormat("{0}#{1} özel {2} MB, handle {3}", name, p.Id, p.PrivateMemorySize64 / 1048576, p.HandleCount);
                    if (name != "dwm") sb.AppendFormat(", GDI {0}, USER {1}", GetGuiResources(p.Handle, 0), GetGuiResources(p.Handle, 1));
                    sb.Append(" · ");
                }
                catch { }
                p.Dispose();
            }
        return sb.ToString().TrimEnd(' ', '·');
    }
}

// ---------------- Odak penceresi ----------------
// Hyprland'de boş workspace'te klavye hiçbir yere gitmez. Windows'ta ön planda hep bir pencere vardır: tiling boş workspace'te
// masaüstünü (Progman) odaklıyordu, bu da tutmazsa odak başka workspace'teki gizli pencerede kalıyordu; boş workspace'te
// yazılan tuşlar görünmeyen tarayıcıya gidiyordu (YouTube'da "i" mini oynatıcıyı açıyordu). Bu pencere görünmez, ekran
// dışında, tıklanamaz ve tuşları yutar. Tiling boş workspace'te odağı buna verir (sınıf adıyla bulur, yoksa masaüstüne),
// odak bekçisi de odak gizli bir pencereye düşerse ve workspace boşsa buraya alır.
// Pencere normal yetkili bir süreçte yaşar: ön plandaki pencere yönetici yetkili bir sürecinken Windows, normal yetkili
// araçların (ekran paylaşımı, uzaktan erişim, otomasyon) tıklama ve tuşlarını engeller (UIPI); boş workspace'te bu araçlar
// tamamen kilitleniyordu. Çekirdek kullanıcı olarak çalışıyorsa pencere onda, yönetici olarak çalışıyorsa kullanıcı olarak
// başlattığı "lunge.exe --focus-sink <pid>" yardımcısında; yardımcı çekirdekle biter.
static class FocusSink
{
    public const string ClassName = "LogicalLunge.FocusSink";
    public static IntPtr Handle { get { return Current(); } }
    static volatile IntPtr handle;   // bu süreçteki pencere (yardımcı, kullanıcı olarak çalışan çekirdek ya da son çare)
    static IntPtr remote;            // yardımcının penceresi
    static int lastLaunch, launches;
    static volatile bool launching, local;
    static WndProcDelegate proc; // çöpe gitmesin

    delegate IntPtr WndProcDelegate(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WNDCLASSEX
    {
        public int cbSize; public uint style; public WndProcDelegate lpfnWndProc; public int cbClsExtra, cbWndExtra;
        public IntPtr hInstance, hIcon, hCursor, hbrBackground; public string lpszMenuName, lpszClassName; public IntPtr hIconSm;
    }
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam, lParam; public uint time; public int x, y; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern ushort RegisterClassEx(ref WNDCLASSEX c);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateWindowEx(uint ex, string cls, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr DefWindowProc(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern int GetMessage(out MSG m, IntPtr h, uint min, uint max);
    [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
    [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG m);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint attach, uint to, bool on);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern IntPtr SetFocus(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    const uint WM_APP_TAKE_FOCUS = 0x8000 + 7;

    public static void Start()
    {
        if (UserLaunch.Elevated) { Current(); return; }
        StartLocal();
    }

    static void StartLocal()
    {
        local = true;
        var t = new Thread(Run) { IsBackground = true, Name = "focus-sink" };
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
    }

    // lunge.exe --focus-sink <pid>: pencere bu (kullanıcı olarak çalışan) süreçte, çekirdek bitene kadar. Tek kopya: başka biri
    // zaten duruyorsa (çekirdek yeniden başlarken) o kalır; eskisi kendi çekirdeğiyle gidince yenisini çekirdek yeniden açar.
    public static void RunHost(int corePid)
    {
        if (Native.FindWindow(ClassName, null) != IntPtr.Zero) return;
        new Thread(() => { WaitForExit(corePid, 2000); Environment.Exit(0); }) { IsBackground = true, Name = "focus-sink-core" }.Start();
        Run();
    }

    // Süreç bitene kadar bekler: normal yetkili bir süreç yönetici sürecini bekleyemez, yalnızca yoklayabilir
    public static void WaitForExit(int pid, int pollMs)
    {
        while (true)
        {
            IntPtr p = OpenProcess(0x1000 /*PROCESS_QUERY_LIMITED_INFORMATION*/, false, pid);
            if (p == IntPtr.Zero) return;
            uint code;
            bool ok = GetExitCodeProcess(p, out code);
            CloseHandle(p);
            if (!ok || code != 259 /*STILL_ACTIVE*/) return;
            Thread.Sleep(pollMs);
        }
    }

    // Kullanılacak pencere; yönetici çekirdekte yardımcınınki (yoksa yardımcıyı yeniden başlatır)
    static IntPtr Current()
    {
        IntPtr h = handle;
        if (h != IntPtr.Zero || !UserLaunch.Elevated || local) return h;
        h = remote;
        if (h != IntPtr.Zero && Native.IsWindow(h)) return h;
        h = Native.FindWindow(ClassName, null);
        remote = h;
        if (h != IntPtr.Zero) { launches = 0; return h; }
        LaunchHost();
        return IntPtr.Zero;
    }

    static void LaunchHost()
    {
        int now = Environment.TickCount;
        if (launching || (lastLaunch != 0 && now - lastLaunch < 10000)) return;
        lastLaunch = now;
        // Üç denemede pencere gelmediyse (masaüstü kabuğu yok vb.) eskisi gibi çekirdekte: tuşlar gizli pencerelere gitmesin
        if (++launches > 3)
        {
            Slider.Log("odak penceresi: kullanıcı olarak açılamadı, çekirdekte tutuluyor");
            StartLocal();
            return;
        }
        launching = true;
        ThreadPool.QueueUserWorkItem(_ =>
        {
            try { UserLaunch.Start(Application.ExecutablePath, "--focus-sink " + Process.GetCurrentProcess().Id, Paths.Home, true); }
            catch (Exception ex) { Slider.Log("odak penceresi yardımcısı başlatılamadı: " + ex.Message); }
            finally { launching = false; }
        });
    }

    static IntPtr WndProc(IntPtr h, uint msg, IntPtr w, IntPtr l)
    {
        switch (msg)
        {
            case 0x0100: case 0x0101: case 0x0102: case 0x0103: // WM_KEYDOWN / KEYUP / CHAR / DEADCHAR
            case 0x0104: case 0x0105: case 0x0106: case 0x0107: // WM_SYSKEYDOWN / SYSKEYUP / SYSCHAR / SYSDEADCHAR
                return IntPtr.Zero; // yut (Alt+F4 de bu pencereyi kapatmasın)
            case 0x0010: return IntPtr.Zero; // WM_CLOSE: yalnızca süreçle birlikte gider
            case 0x0021: return new IntPtr(3); // WM_MOUSEACTIVATE: MA_NOACTIVATE
            case WM_APP_TAKE_FOCUS: return TakeFocus(w == IntPtr.Zero ? h : w) ? new IntPtr(1) : IntPtr.Zero;
        }
        return DefWindowProc(h, msg, w, l);
    }

    static void Run()
    {
        try
        {
            // Callback kuralı (bu yordamın kendi tipi var): istisna dışarı sızmaz
            proc = (ph, pm, pw, pl) =>
            {
                try { return WndProc(ph, pm, pw, pl); }
                catch (Exception ex) { Callback.Failed("odak penceresi", ex); return DefWindowProc(ph, pm, pw, pl); }
            };
            var wc = new WNDCLASSEX { cbSize = Marshal.SizeOf(typeof(WNDCLASSEX)), lpfnWndProc = proc, hInstance = GetModuleHandle(null), lpszClassName = ClassName };
            if (RegisterClassEx(ref wc) == 0) { Slider.Log("odak penceresi: sınıf kaydedilemedi " + Marshal.GetLastWin32Error()); return; }
            // Ekran dışı 1x1, tamamen saydam, tıklama geçirgen araç penceresi (görev çubuğu / alt-tab / tiling onu görmez)
            var vs = SystemInformation.VirtualScreen;
            IntPtr h = CreateWindowEx(0x80 | 0x80000 | 0x20, ClassName, Names.TitlePrefix + " odak", 0x80000000 | 0x10000000,
                vs.Left - 64, vs.Top - 64, 1, 1, IntPtr.Zero, IntPtr.Zero, wc.hInstance, IntPtr.Zero);
            if (h == IntPtr.Zero) { Slider.Log("odak penceresi: açılamadı " + Marshal.GetLastWin32Error()); return; }
            Native.SetLayeredWindowAttributes(h, 0, 0, 0x2);
            handle = h;
            MSG m;
            while (GetMessage(out m, IntPtr.Zero, 0, 0) > 0) { TranslateMessage(ref m); DispatchMessage(ref m); }
        }
        catch (Exception ex) { Slider.Log("odak penceresi: " + ex.Message); }
        handle = IntPtr.Zero;
    }

    // Klavyeyi bu pencereye ver (boş workspace). Pencerenin kendi thread'inde yapılır: o anki ön plan thread'inin girdisine
    // kısa süre bağlanınca Windows'un ön plan kilidi izin verir. Arka plandaki bir thread'den SetForegroundWindow (sahte
    // tuş hilesiyle bile) ön plan başka süreçteyken reddediliyordu: gizlenen overview önde kalıyordu.
    public static bool Focus() { return Give(IntPtr.Zero); }

    // Başka bir pencereyi öne al (overview kapanınca önceki pencere), aynı yöntemle. Pencere yoksa ya da alamadıysa (ön plan
    // yönetici yetkili bir süreçteyse kullanıcı olarak çalışan yardımcı onu oradan alamaz) çekirdek doğrudan dener.
    public static bool Give(IntPtr target)
    {
        IntPtr h = Current(), r;
        if (h != IntPtr.Zero && SendMessageTimeout(h, WM_APP_TAKE_FOCUS, target, IntPtr.Zero, 0x2 /*SMTO_ABORTIFHUNG*/, 300, out r) != IntPtr.Zero
            && r != IntPtr.Zero)
            return true;
        IntPtr to = target != IntPtr.Zero ? target : h;
        if (to == IntPtr.Zero) return false;
        Native.keybd_event(0xE8, 0, 0, Native.LL_MARK); Native.keybd_event(0xE8, 0, 2, Native.LL_MARK);
        Native.SetForegroundWindow(to);
        return Native.GetForegroundWindow() == to;
    }

    static bool TakeFocus(IntPtr h)
    {
        IntPtr fg = Native.GetForegroundWindow();
        if (fg == h) return true;
        uint fgPid, me = GetCurrentThreadId();
        uint fgTid = fg == IntPtr.Zero ? 0 : Native.GetWindowThreadProcessId(fg, out fgPid);
        bool attached = fgTid != 0 && fgTid != me && AttachThreadInput(me, fgTid, true);
        try
        {
            Native.keybd_event(0xE8, 0, 0, Native.LL_MARK); Native.keybd_event(0xE8, 0, 2, Native.LL_MARK);
            Native.SetForegroundWindow(h);
            SetFocus(h);
        }
        finally { if (attached) AttachThreadInput(me, fgTid, false); }
        return Native.GetForegroundWindow() == h;
    }
}

// ---------------- Odak bekçisi ----------------
// Hyprland'de odak hiç boşta kalmaz. Windows'ta ise odaktaki pencere kapanınca ya da ekran alıntısı, bir iletişim kutusu,
// bildirim kapanınca ön plan masaüstüne, bar'a, gizli (başka workspace'teki) bir pencereye ya da hiçbir şeye düşebiliyordu:
// klavye bir yere gitmiyor, fareyle tıklamak gerekiyordu. Bu durum ~0,75 sn sürerse bekçi odağı görünen workspace'te
// tiling'in odaklı saydığı pencereye (yoksa imlecin altındakine) geri verir. Boş workspace'te, fare tuşu basılıyken, açık
// bir sağ tık menüsünde, kilit ekranında, bakımda ve kabuk yokken (Windows görev çubuğu modu) karışmaz.
static class FocusGuard
{
    [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h, int index, StringBuilder info, int len, out int needed);

    static readonly TilingClient tiling = new TilingClient();

    public static void Start()
    {
        new Thread(Loop) { IsBackground = true, Name = "focus-guard" }.Start();
    }

    static void Loop()
    {
        // "kayıp" ayrı bir işaret: TickCount 24,8 günde eksiye geçer, "-1 = yok" orada bekçiyi susturuyordu
        int lostSince = 0, fails = 0, waitUntil = Environment.TickCount;
        bool lost = false;
        while (true)
        {
            Thread.Sleep(250);
            try
            {
                string why = Lost();
                if (why == null) { lost = false; fails = 0; continue; }
                int now = Environment.TickCount;
                if (!lost) { lost = true; lostSince = now; continue; }
                if (now - lostSince < 750 || now - waitUntil < 0) continue;
                int r = Refocus(why);
                if (r > 0) { lost = false; fails = 0; }
                else if (r == 0) waitUntil = now + (why == SinkReason ? 5000 : 1500); // boş workspace: arada bir bak
                else if (++fails >= 3) { waitUntil = now + 15000; fails = 0; Slider.Log("odak bekçisi: odak verilemedi, 15 sn bekleniyor"); }
                else lostSince = now;
            }
            catch (Exception ex) { Slider.Log("odak bekçisi: " + ex.GetBaseException().Message); Thread.Sleep(2000); }
        }
    }

    // Kilit ekranı / ekran koruyucusu / UAC girdi masaüstündeyken ön plan sorgusu anlamsız
    public static bool OnDefaultDesktop()
    {
        IntPtr d = OpenInputDesktop(0, false, 0x0001); // DESKTOP_READOBJECTS
        if (d == IntPtr.Zero) return false;
        try
        {
            var sb = new StringBuilder(64); int need;
            return GetUserObjectInformation(d, 2, sb, sb.Capacity * 2, out need) && sb.ToString() == "Default"; // UOI_NAME
        }
        finally { CloseDesktop(d); }
    }

    // Odak boştaysa nedeni, değilse null
    static string Lost()
    {
        if (!ShellState.Up || Maint.Quiet() || !OnDefaultDesktop()) return null;
        // Kullanıcı bir şeyle uğraşıyor: sürükleme, tıklama, açık bir menü
        if (Native.GetAsyncKeyState(0x01) < 0 || Native.GetAsyncKeyState(0x02) < 0 || Native.GetAsyncKeyState(0x04) < 0) return null;
        IntPtr menu = Native.FindWindow("#32768", null);
        if (menu != IntPtr.Zero && Native.IsWindowVisible(menu)) return null;

        IntPtr fg = Native.GetForegroundWindow();
        if (fg == IntPtr.Zero) return "ön plan yok";
        IntPtr root = Native.GetAncestor(fg, 2);
        if (root == IntPtr.Zero) root = fg;
        if (root == FocusSink.Handle) return SinkReason; // boş workspace'te olağan; workspace doluysa pencereye verilir
        var cls = new StringBuilder(64); Native.GetClassName(root, cls, 64);
        string c = cls.ToString();
        if (c == "Progman" || c == "WorkerW" || c == "Shell_TrayWnd" || c == "Shell_SecondaryTrayWnd") return "masaüstü";
        if (!Native.IsWindowVisible(root) || Native.IsIconic(root)) return "görünmeyen pencere";
        int cl;
        if (Native.DwmGetWindowAttribute(root, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0) return "gizli pencere";
        // Bar, bildirim ve güncelleme kartı klavye almaz; üzerlerine tıklanınca odak onlarda kalıyordu. Fare üstlerindeyse
        // kullanıcı onlarla uğraşıyordur.
        var t = new StringBuilder(128); Native.GetWindowText(root, t, 128);
        string title = t.ToString();
        if (title == Names.Bar || title == Names.Toast || title == Names.Update)
        {
            Native.RECT r;
            var p = Cursor.Position;
            if (Native.GetWindowRect(root, out r) && p.X >= r.Left && p.X < r.Right && p.Y >= r.Top && p.Y < r.Bottom) return null;
            return "bar";
        }
        return null;
    }

    const string SinkReason = "odak penceresi";

    // Overview kapandı vb.: bekçinin 0,75 sn'lik sabrını beklemeden bak (o arada yazılan tuşlar gizli pencereye gidiyordu)
    public static void Kick()
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            try
            {
                Thread.Sleep(90);
                string why = Lost();
                int r = why != null && why != SinkReason ? Refocus(why) : 1;
                if (why != null && why != SinkReason) Slider.Log("odak bekçisi (hemen): " + why + " -> " + (r > 0 ? "verildi" : r == 0 ? "boş workspace: odak penceresine" : "verilemedi"));
            }
            catch (Exception ex) { Slider.Log("odak bekçisi: " + ex.GetBaseException().Message); }
        });
    }

    // 1: odak verildi, 0: verilecek pencere yok (boş workspace: klavye odak penceresine), -1: verilemedi
    static int Refocus(string why)
    {
        Dictionary<string, object> ws = null;
        foreach (var m in tiling.Monitors())
            foreach (Dictionary<string, object> w in J.Children(m))
                if (J.Bool(w, "hasFocus")) ws = w;
        if (ws == null) return 0;
        var wins = new List<Dictionary<string, object>>();
        J.WindowNodes(ws, wins);
        Dictionary<string, object> focused = null, under = null, first = null;
        var p = Cursor.Position;
        foreach (var w in wins)
        {
            object hv;
            if (!w.TryGetValue("handle", out hv) || hv == null) continue;
            var h = new IntPtr(Convert.ToInt64(hv));
            int cl;
            if (!Native.IsWindowVisible(h) || Native.IsIconic(h) || (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0)) continue;
            if (J.Bool(w, "hasFocus")) focused = w;
            int x = J.Int(w, "x"), y = J.Int(w, "y");
            if (under == null && p.X >= x && p.X < x + J.Int(w, "width") && p.Y >= y && p.Y < y + J.Int(w, "height")) under = w;
            if (first == null) first = w;
        }
        var pick = focused ?? under ?? first;
        if (pick == null)
        {
            // Boş workspace: tuşlar gizli bir pencereye / masaüstüne gitmesin
            if (why != SinkReason && FocusSink.Focus()) Slider.Log("odak boştaydı (" + why + "): boş workspace, odak penceresine verildi");
            return 0;
        }
        var hw = new IntPtr(Convert.ToInt64(pick["handle"]));
        // Önce tiling üzerinden (durumu da güncel kalsın); o pencereyi zaten odaklı sayıyorsa ön plana getirmeyebilir
        tiling.Command("focus --container-id " + J.Str(pick, "id"));
        Thread.Sleep(150);
        if (Lost() != null)
        {
            Native.keybd_event(0xE8, 0, 0, Native.LL_MARK); Native.keybd_event(0xE8, 0, 2, Native.LL_MARK); // odak kilidi
            Native.SetForegroundWindow(hw);
            Thread.Sleep(100);
        }
        bool ok = Lost() == null;
        string name = "?";
        try { uint pid; Native.GetWindowThreadProcessId(hw, out pid); using (var pr = Process.GetProcessById((int)pid)) name = pr.ProcessName; } catch { }
        Slider.Log("odak boştaydı (" + why + "): " + name + (ok ? " odaklandı" : " odaklanamadı"));
        return ok ? 1 : -1;
    }
}

static class Splash
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);

    static bool Ready()
    {
        // tiling IPC portu açık ve shell bar penceresi var mı
        try { using (var c = new System.Net.Sockets.TcpClient()) { if (!c.ConnectAsync("127.0.0.1", 6123).Wait(150)) return false; } }
        catch { return false; }
        return FindWindow(null, Names.Bar) != IntPtr.Zero;
    }

    // Kilitli olabilir (Superpaper gibi araçlar dosyayı yeniden yazarken) -> paylaşımlı aç; olmazsa son iyi kopya.
    static string CachePath { get { return Paths.State(@"splash-wall.jpg"); } }
    internal static bool SpanStyle()
    {
        string st = (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Control Panel\Desktop", "WallpaperStyle", null);
        return st == "22";
    }

    internal static Image Wallpaper()
    {
        foreach (var f in new[] {
            (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Control Panel\Desktop", "WallPaper", null),
            CachePath,
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), @"Microsoft\Windows\Themes\TranscodedWallpaper") })
        {
            try
            {
                if (string.IsNullOrEmpty(f) || !System.IO.File.Exists(f)) continue;
                using (var s = new System.IO.FileStream(f, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
                    return Image.FromStream(new System.IO.MemoryStream(ReadAll(s)));
            }
            catch { }
        }
        return null;
    }

    // Açılışta duvar kağıdı okunamazsa kullanılacak kopya: masaüstü hazırken güncel tutulur.
    public static void SaveCache()
    {
        try
        {
            string f = (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Control Panel\Desktop", "WallPaper", null);
            if (string.IsNullOrEmpty(f) || !System.IO.File.Exists(f)) return;
            System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(CachePath));
            using (var s = new System.IO.FileStream(f, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
            using (var d = System.IO.File.Create(CachePath + ".tmp")) s.CopyTo(d);
            System.IO.File.Copy(CachePath + ".tmp", CachePath, true);
            System.IO.File.Delete(CachePath + ".tmp");
            // Superpaper "span" resmi tüm masaüstüne yayılır: hangi ekranda ne çizileceğini de saklamak için stil bayrağı
        }
        catch { }
    }
    static byte[] ReadAll(System.IO.Stream s) { var m = new System.IO.MemoryStream(); s.CopyTo(m); return m.ToArray(); }

    // Açılış ve yeniden başlatma örtüsü: bütün ekranları duvar kağıdıyla (sakin bir karartmayla) örter; ana ekranda yazı ve
    // altında bir yükleme çemberi. Açılırken arkada bar, pencere yöneticisi ve pencereler gelip yerleşirken görünmez,
    // kullanıcı yanlış bir yere tıklayamaz ya da bir tuşla (Alt+F4) açılmakta olan bir şeyi kapatamaz. Hazır olunca yazı
    // hafifçe kalkıp söner, örtü çekilir. İlk karede (koyu zemin ve çember) hemen gelir: yazının dili, vurgu rengi ve duvar
    // kağıdı (4K bir resmi çözüp ölçeklemek yarım saniye sürebilir) arkada hazırlanıp belirir.
    internal sealed class Cover : Form
    {
        readonly bool primary;
        string text;
        Color accent = Color.FromArgb(0xb6, 0x9d, 0xf8);
        Bitmap background, incoming;  // duvar kağıdı + karartma; gelen, belirene kadar üstüne karışır
        int incomingAt;
        public int TextAt;                          // yazının geldiği an (0: henüz yok)
        public int FirstPaint;                      // ilk karenin çizildiği an
        public double TextAlpha, SpinnerAlpha, TextLift, Clock;   // Run'daki saat sürer (Clock: ms)
        public Rectangle Indicator;                 // her karede yalnızca bu alan yeniden çizilir
        public const int BlendMs = 300;

        public Cover(Rectangle b, bool primary)
        {
            this.primary = primary;
            Text = Names.StartupCover;
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true;
            StartPosition = FormStartPosition.Manual; Bounds = b;
            BackColor = Color.FromArgb(20, 19, 24);
            DoubleBuffered = true;
            Cursor = Cursors.AppStarting;
        }
        protected override CreateParams CreateParams
        {
            get { var p = base.CreateParams; p.ExStyle |= 0x80 | 0x08000000; return p; } // TOOLWINDOW | NOACTIVATE
        }
        protected override bool ShowWithoutActivation { get { return true; } }
        // Kullanıcı kapatamaz (Alt+F4, görev çubuğu): yalnızca örtünün kendisi çekilir
        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            if (e.CloseReason == CloseReason.UserClosing) e.Cancel = true;
            base.OnFormClosing(e);
        }
        protected override void OnPaintBackground(PaintEventArgs e) { }
        protected override void OnPaint(PaintEventArgs e)
        {
            if (FirstPaint == 0) FirstPaint = Environment.TickCount;
            try { PaintBody(e); } catch (Exception ex) { PaintErrors.Report("açılış örtüsü", ex); }
        }

        // Arkadan gelenler (UI iş parçacığında çağrılır)
        public void SetText(string t, Color a) { text = t; accent = a; TextAt = Environment.TickCount; Invalidate(); }
        public void SetBackground(Bitmap bmp) { incoming = bmp; incomingAt = Environment.TickCount; Invalidate(); }

        // Duvar kağıdı belirirken her kare yeniden çizilmeli
        public bool Blending { get { return incoming != null; } }

        void PaintBody(PaintEventArgs e)
        {
            var g = e.Graphics;
            if (background != null) g.DrawImageUnscaled(background, 0, 0); else g.Clear(BackColor);
            if (incoming != null)
            {
                double a = Math.Min(1, (Environment.TickCount - incomingAt) / (double)BlendMs);
                if (a >= 1) { var old = background; background = incoming; incoming = null; g.DrawImageUnscaled(background, 0, 0); if (old != null) old.Dispose(); }
                else
                    using (var attrs = new System.Drawing.Imaging.ImageAttributes())
                    {
                        attrs.SetColorMatrix(new System.Drawing.Imaging.ColorMatrix { Matrix33 = (float)a });
                        g.DrawImage(incoming, new Rectangle(0, 0, incoming.Width, incoming.Height), 0, 0, incoming.Width, incoming.Height, GraphicsUnit.Pixel, attrs);
                    }
            }
            if (!primary) return;
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.AntiAliasGridFit;
            float size = Math.Max(20f, Height * 0.026f);
            float lift = (float)TextLift;
            if (text != null)
            {
                int ta = (int)Math.Round(255 * Math.Max(0, Math.Min(1, TextAlpha)));
                using (var font = new Font(FontFamily(), size, FontStyle.Regular, GraphicsUnit.Pixel))
                using (var brush = new SolidBrush(Color.FromArgb(ta, 230, 224, 233)))
                {
                    var sz = g.MeasureString(text, font);
                    g.DrawString(text, font, brush, (Width - sz.Width) / 2, Height / 2f - sz.Height - lift);
                }
            }
            // Yükleme çemberi: yazının altında, vurgu renginde; ilk karede bile
            int sa = (int)Math.Round(255 * Math.Max(0, Math.Min(1, SpinnerAlpha)));
            int d = (int)Math.Round(Math.Max(26, Height * 0.03));
            float stroke = Math.Max(3f, d * 0.12f);
            var ring = new RectangleF((Width - d) / 2f, Height / 2f + size * 0.8f - lift, d, d);
            Indicator = Rectangle.Inflate(Rectangle.Round(new RectangleF(ring.X, Height / 2f + size * 0.8f - Height * 0.02f, d, d + Height * 0.02f)), (int)stroke + 2, (int)stroke + 2);
            using (var track = new Pen(Color.FromArgb(sa * 36 / 255, 230, 224, 233), stroke))
                g.DrawEllipse(track, ring);
            float start, sweep;
            Spinner(Clock, out start, out sweep);
            using (var pen = new Pen(Color.FromArgb(sa, accent), stroke) { StartCap = System.Drawing.Drawing2D.LineCap.Round, EndCap = System.Drawing.Drawing2D.LineCap.Round })
                g.DrawArc(pen, ring, start, sweep);
        }

        // Material'in belirsiz çemberi: yay dönerken uzar, sonra kuyruğu başına yetişip kısalır; her turda biraz daha ilerler
        public static void Spinner(double ms, out float start, out float sweep)
        {
            const double cycle = 1333, grow = 250, least = 18;
            double n = Math.Floor(ms / cycle), t = (ms - n * cycle) / cycle;
            double head, tail;
            if (t < 0.5) { head = Ease(t * 2) * grow; tail = 0; }
            else { head = grow; tail = Ease((t - 0.5) * 2) * grow; }
            double turn = ms / 1600.0 * 360 + n * grow;
            start = (float)((turn + tail) % 360);
            sweep = (float)(least + head - tail);
        }

        static double Ease(double t) { return t < 0.5 ? 4 * t * t * t : 1 - Math.Pow(-2 * t + 2, 3) / 2; }

        static string FontFamily()
        {
            foreach (var name in new[] { "Segoe UI Variable Display", "Segoe UI" })
                using (var f = new Font(name, 12f))
                    if (f.Name == name) return name;
            return "Segoe UI";
        }

        // Duvar kağıdı bu ekrana ölçeklenmiş ve karartılmış hâlde (arka plan iş parçacığında da çalışır)
        public static Bitmap Prepare(Image img, Rectangle bounds, Rectangle virt, Color back)
        {
            var bmp = new Bitmap(Math.Max(1, bounds.Width), Math.Max(1, bounds.Height), System.Drawing.Imaging.PixelFormat.Format32bppPArgb);
            using (var g = Graphics.FromImage(bmp))
            {
                g.Clear(back);
                if (img != null)
                {
                    g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
                    // "Doldur" yerleşimi: en boy oranını koruyup alanı kapla, taşanı ortadan kırp. Span'da alan tüm
                    // masaüstüdür ve her ekran kendi dilimini gösterir (Windows'un "Yay" yerleşimi).
                    Rectangle area = virt.IsEmpty ? new Rectangle(0, 0, bounds.Width, bounds.Height) : virt;
                    double k = Math.Max((double)area.Width / img.Width, (double)area.Height / img.Height);
                    int w = (int)Math.Ceiling(img.Width * k), h = (int)Math.Ceiling(img.Height * k);
                    int x = area.X + (area.Width - w) / 2 - bounds.Left, y = area.Y + (area.Height - h) / 2 - bounds.Top;
                    if (virt.IsEmpty) { x = (bounds.Width - w) / 2; y = (bounds.Height - h) / 2; }
                    g.DrawImage(img, x, y, w, h);
                }
                // Sakin bir karartma: yazı her duvar kağıdında okunsun, ekranlar birbirine uysun
                using (var scrim = new SolidBrush(Color.FromArgb(120, 12, 11, 15))) g.FillRectangle(scrim, 0, 0, bounds.Width, bounds.Height);
            }
            return bmp;
        }
    }

    // Örtü varken bütün tuşlar yutulur: açılmakta olan bir şey kapatılamasın (Alt+F4), yanlış yere yazılmasın. Örtünün kendi
    // süre sınırı var (30 sn, güncellemede 150 sn); süreç bitince kanca da gider.
    static Native.LowLevelKeyboardProc swallow;
    static IntPtr swallowHook;

    internal static Color Accent()
    {
        try
        {
            object v;
            string hex = Prefs.Read().TryGetValue("focusColor", out v) ? v as string : null;
            if (hex != null && System.Text.RegularExpressions.Regex.IsMatch(hex, "^#[0-9a-fA-F]{6}$")) return ColorTranslator.FromHtml(hex);
        }
        catch { }
        return Color.FromArgb(0xb6, 0x9d, 0xf8);
    }

    // Oturumun açıldığı an (Windows'un tuttuğu), açılıştan bu yana geçen ms için; bilinmiyorsa -1. Unicode sürümü ve
    // yapının kendisi: ANSI sürümü (CharSet belirtilmeyince o çağrılıyordu) 144 baytlık WTSINFOA döner, elle sayılan
    // ofsetler de onun dışından okuyordu (her açılışta "?" ya da 0; yenilemede örtü "yeniden" demiyordu).
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WTSINFOW
    {
        public int State, SessionId, IncomingBytes, OutgoingBytes, IncomingFrames, OutgoingFrames, IncomingCompressedBytes, OutgoingCompressedBytes;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string WinStationName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 17)] public string Domain;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 21)] public string UserName;
        public long ConnectTime, DisconnectTime, LastInputTime, LogonTime, CurrentTime;
    }
    [DllImport("wtsapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool WTSQuerySessionInformation(IntPtr server, int session, int infoClass, out IntPtr buffer, out int bytes);
    [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr memory);
    static long SinceLogonMs()
    {
        IntPtr buf; int bytes;
        if (!WTSQuerySessionInformation(IntPtr.Zero, -1 /*bu oturum*/, 24 /*WTSSessionInfo*/, out buf, out bytes) || buf == IntPtr.Zero) return -1;
        try
        {
            if (bytes < Marshal.SizeOf(typeof(WTSINFOW))) return -1;
            var info = (WTSINFOW)Marshal.PtrToStructure(buf, typeof(WTSINFOW));
            return info.LogonTime > 0 && info.CurrentTime >= info.LogonTime ? (info.CurrentTime - info.LogonTime) / 10000 : -1;
        }
        finally { WTSFreeMemory(buf); }
    }

    // Masaüstü hazır mı: çekirdeğe sorulur (POST /desktop-ready: pencere yöneticisi, bütün barlar, canlı duvar kağıdı)
    static bool CoreSaysReady()
    {
        try
        {
            var rq = (System.Net.HttpWebRequest)System.Net.WebRequest.Create("http://127.0.0.1:6131/desktop-ready");
            rq.Method = "POST"; rq.ContentLength = 0; rq.Timeout = 1000; rq.ReadWriteTimeout = 1000; rq.Proxy = null;
            using (var resp = rq.GetResponse())
            using (var rd = new System.IO.StreamReader(resp.GetResponseStream()))
                return rd.ReadToEnd().Contains("\"ready\":true");
        }
        catch { return false; }
    }

    sealed class ReadyState { public volatile bool Ready, Stop; }

    public static void Run()
    {
        long sinceLogon = SinceLogonMs();
        int start = Environment.TickCount;
        bool created;
        using (var m = new Mutex(true, "lunge-splash", out created))
        {
            if (!created) return;
            // Güncelleme sırasında (LL_SPLASH_WAIT_RESTART=1): örtü yumuşakça belirir, önce mevcut masaüstünün kapanmasını,
            // sonra yenisinin hazır olmasını bekler (en fazla 150 sn).
            bool restartMode = Environment.GetEnvironmentVariable("LL_SPLASH_WAIT_RESTART") == "1";
            // Yeniden başlatma: güncelleme, masaüstü zaten ayakta, ya da oturum açılalı iki dakikadan fazla oldu (masaüstünü
            // yenile: eski parçalar örtüden önce kapanır). "Sistem başlatılıyor" yalnızca oturum açılınca.
            bool restart = restartMode || Ready() || sinceLogon > 120000;
            bool sawDown = !restartMode;
            int maxMs = restartMode ? 150000 : 30000;
            var covers = new List<Cover>();
            Cover main = null;
            // Önce örtü: koyu zemin ve çember, hemen
            foreach (var s in Screen.AllScreens)
            {
                var f = new Cover(s.Bounds, s.Primary);
                if (s.Primary) main = f;
                f.Show();
                covers.Add(f);
            }
            if (main != null) main.Update();   // ilk kare şimdi, mesaj döngüsünü beklemeden
            int shownMs = Environment.TickCount - start;
            ShellTakeover.HideTaskbarForSplash(); // görev çubuğu açılıştan itibaren görünmesin
            swallow = Callback.Guard("açılış örtüsü", (Native.LowLevelKeyboardProc)((n, w, l) => n >= 0 ? (IntPtr)1 : Native.CallNextHookEx(IntPtr.Zero, n, w, l)));
            swallowHook = Native.SetWindowsHookEx(Native.WH_KEYBOARD_LL, swallow, Native.GetModuleHandle(null), 0);
            if (restartMode) foreach (var f in covers) f.Opacity = 0;
            // Açılışta ne kadar sonra geldi: bir sonraki açılışta gecikme olup olmadığı tahminle değil kayıtla bilinsin
            if (!restart) Slider.Log("açılış örtüsü: oturum açılışından " + (sinceLogon < 0 ? "?" : sinceLogon.ToString()) + " ms sonra başladı, ilk kare +" + shownMs + " ms");

            // Arkada: yazının dili, vurgu rengi, duvar kağıdı (dil dosyası, ayarlar ve 4K resim ilk kareyi bekletmesin)
            var targets = new List<KeyValuePair<Cover, Rectangle>>();
            foreach (var f in covers) targets.Add(new KeyValuePair<Cover, Rectangle>(f, f.Bounds));
            ThreadPool.QueueUserWorkItem(_ =>
            {
                try
                {
                    string text = I18n.T(restart ? "Sistem yeniden başlatılıyor" : "Sistem başlatılıyor");
                    var accent = Accent();
                    if (main != null) main.BeginInvoke((Action)(() => main.SetText(text, accent)));
                    var img = Wallpaper();
                    if (img == null) return;
                    var virt = SpanStyle() ? SystemInformation.VirtualScreen : Rectangle.Empty;
                    foreach (var t in targets)
                    {
                        var bmp = Cover.Prepare(img, t.Value, virt, t.Key.BackColor);
                        var f = t.Key;
                        try { f.BeginInvoke((Action)(() => f.SetBackground(bmp))); } catch { bmp.Dispose(); }
                    }
                    img.Dispose();
                }
                catch { }
            });

            // Arkada sorulur: çekirdek meşgulse çember takılmasın
            var state = new ReadyState();
            new Thread(() => { while (!state.Stop) { state.Ready = CoreSaysReady(); Thread.Sleep(150); } }) { IsBackground = true, Name = "splash-ready" }.Start();

            int readyAt = -1, leaving = -1;
            // Tek saat: belirme, yazı ve çember, çekilme. Ana ekranda yalnızca çemberin alanı yeniden çizilir.
            var clock = new System.Windows.Forms.Timer { Interval = 16 };
            clock.Tick += (o, e) =>
            {
                int now = Environment.TickCount;
                if (restartMode && leaving < 0)
                {
                    double t3 = Math.Min(1, (now - start) / 450.0);
                    foreach (var f in covers) f.Opacity = 1 - Math.Pow(1 - t3, 3);
                }
                foreach (var f in covers) if (f != main && f.Blending) f.Invalidate();
                if (main != null)
                {
                    main.Clock = now - start;
                    if (leaving < 0)
                    {
                        main.SpinnerAlpha = Math.Min(1, (now - start) / 200.0);
                        main.TextAlpha = main.TextAt == 0 ? 0 : Math.Min(1, (now - main.TextAt) / 300.0);
                    }
                    else
                    {
                        // Bölüm geçişi gibi: yazı ve çember hafifçe kalkıp söner, sonra örtü çekilir
                        double tt = Math.Min(1, (now - leaving) / 220.0);
                        main.TextAlpha = Math.Min(main.TextAlpha, 1 - tt);
                        main.SpinnerAlpha = Math.Min(main.SpinnerAlpha, 1 - tt);
                        main.TextLift = main.Height * 0.012 * (1 - Math.Pow(1 - tt, 3));
                    }
                    var r = main.Indicator;
                    bool textFading = main.TextAt != 0 && now - main.TextAt < 350;
                    if (r.IsEmpty || leaving >= 0 || textFading || main.Blending) main.Invalidate(); else main.Invalidate(r);
                }
                if (leaving >= 0)
                {
                    double t = Math.Min(1, Math.Max(0, (now - leaving - 160) / 450.0));
                    double eased = 1 - Math.Pow(1 - t, 3);
                    foreach (var f in covers) f.Opacity = 1 - eased;
                    if (t >= 1)
                    {
                        clock.Stop();
                        if (swallowHook != IntPtr.Zero) { Native.UnhookWindowsHookEx(swallowHook); swallowHook = IntPtr.Zero; }
                        Application.ExitThread();
                    }
                }
            };
            clock.Start();

            var timer = new System.Windows.Forms.Timer { Interval = 100 };
            timer.Tick += (o, e) =>
            {
                int now = Environment.TickCount;
                foreach (var f in covers) Native.SetWindowPos(f.Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // en üstte kal
                bool ready = state.Ready;
                if (!sawDown) { if (!ready) sawDown = true; }
                else if (readyAt < 0 && ready) readyAt = now;
                // Hazır olduktan sonra pencerelerin yerleşip bar'ın çizilmesi için kısa bir süre; en fazla 30 sn bekle
                bool done = (readyAt >= 0 && now - readyAt > 1500) || now - start > maxMs;
                if (!done) return;
                timer.Stop();
                state.Stop = true;
                if (!restart) Slider.Log("açılış örtüsü: masaüstü " + (readyAt >= 0 ? (readyAt - start) + " ms'de hazır" : "hazır olmadan süre doldu") + ", örtü kalkıyor");
                ThreadPool.QueueUserWorkItem(_ => SaveCache());
                leaving = Environment.TickCount;
            };
            timer.Start();
            Application.Run();
            if (swallowHook != IntPtr.Zero) Native.UnhookWindowsHookEx(swallowHook);
        }
    }
}

// ---------------- Uygulama açma kuyruğu ----------------// Hızlı art arda Super+Enter'da tüm terminaller, ilki ekrana gelmeden aynı pencereyi bölüyordu
// (yan yana şeritler). Hyprland'deki gibi her pencere, bir öncekinin ekrana geldiği andaki
// yerleşime göre farenin altındaki pencereyi bölsün: açmaları sırayla, birer birer yap.
static class LaunchQueue
{
    public static readonly AutoResetEvent Managed = new AutoResetEvent(false);
    static readonly System.Collections.Concurrent.BlockingCollection<string> queue = new System.Collections.Concurrent.BlockingCollection<string>();
    static Slider slider;

    public static void Start(Slider s)
    {
        slider = s;
        var t = new Thread(() =>
        {
            foreach (var path in queue.GetConsumingEnumerable())
            {
                try { slider.FocusUnderCursor(); } catch { }
                Managed.Reset();
                try
                {
                    // WezTerm arka planda hazırsa yeni süreç başlatma: istek dosyası yaz, pencere o süreçte açılır
                    // (~0.23 s; soğuk açılış ~0.6 s). Bkz. ~/.wezterm.lua "Anında yeni pencere".
                    // Kullanıcı olarak (çekirdek yönetici olsa da)
                    if (!WarmTerminal.TrySpawn(path) && !UserLaunch.Start(path, "", Paths.Home))
                        throw new Exception("başlatılamadı");
                }
                catch (Exception ex)
                {
                    Toasts.Send("error", "Açılamadı", System.IO.Path.GetFileName(path) + ": " + ex.Message, "error");
                    continue;
                }
                // Pencere tiling'e gelene kadar bekle (en fazla 3 sn), sonra biraz yerleşsin
                if (Managed.WaitOne(3000)) Thread.Sleep(120);
            }
        }) { IsBackground = true };
        t.Start();
    }

    public static void Enqueue(string path) { queue.Add(path); }
}

// Native callback sahiplerinin GC'den korunması
static class Keep
{
    public static Rounder Round;
}

static class Program
{
    // Kullanıcı girdisi var ama iki kanca da 1,5 sn'dir çağrılmadı: Windows kancaları sökmüş
    static bool HooksStale()
    {
        var li = new Native.LASTINPUTINFO { cbSize = (uint)Marshal.SizeOf(typeof(Native.LASTINPUTINFO)) };
        if (!Native.GetLastInputInfo(ref li)) return false;
        int last = (int)li.dwTime, now = Environment.TickCount;
        int seen = Math.Max(Keys2.LastHookTick, MouseFocus.LastHookTick);
        return now - last < 3000 && last - seen > 1500;
    }

    // Windows komut satırı kurallarına göre tek argümanı tırnakla
    internal static string QuoteArg(string a)
    {
        if (a.Length > 0 && a.IndexOfAny(new[] { ' ', '\t', '"' }) < 0) return a;
        var sb = new StringBuilder("\"");
        int bs = 0;
        foreach (char ch in a)
        {
            if (ch == '\\') { bs++; continue; }
            if (ch == '"') { sb.Append('\\', bs * 2 + 1); sb.Append('"'); }
            else { sb.Append('\\', bs); sb.Append(ch); }
            bs = 0;
        }
        sb.Append('\\', bs * 2).Append('"');
        return sb.ToString();
    }

    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_BASIC_INFORMATION { public IntPtr r1, peb, r2a, r2b, pid, parentPid; }
    [DllImport("ntdll.dll")] static extern int NtQueryInformationProcess(IntPtr h, int cls, ref PROCESS_BASIC_INFORMATION pbi, int len, out int ret);

    static int ParentPid()
    {
        try
        {
            var pbi = new PROCESS_BASIC_INFORMATION(); int ret;
            if (NtQueryInformationProcess(Process.GetCurrentProcess().Handle, 0, ref pbi, Marshal.SizeOf(pbi), out ret) != 0) return -1;
            return pbi.parentPid.ToInt32();
        }
        catch { return -1; }
    }

    static bool CanWrite(System.IO.StreamWriter w)
    {
        try { w.Write(""); w.Flush(); return true; } catch { return false; }
    }

    // STA şart: pano (OLE), dosya kaydetme penceresi (COM) ana thread'de çalışıyor. Bu satırın altına başka bir metot
    // eklenirse öznitelik ona geçer ve pano / kaydetme bozulur (bir kez oldu).
    [STAThread]
    static void Main(string[] args)
    {
        // Windows'un "sürücü hazır değil / dosya açılamadı" kutuları bu süreçte (ve başlattıklarında, Gezgin'deki gibi)
        // açılmaz: hatayı çağıran görür ve kendi kartımızla söyler
        ErrorUi.Quiet();
        // Notification click helper: resolve the original database record as the desktop user, without starting a core.
        if (args.Length > 0 && args[0] == "--notification-activate")
        {
            if (args.Length != 3) { Environment.Exit(1); return; }
            long id, arrival;
            bool opened = false;
            // A hung third-party COM activator cannot leave an unbounded helper behind.
            using (var limit = new System.Threading.Timer(_ => Environment.Exit(1), null, 20000, Timeout.Infinite))
                if (long.TryParse(args[1], out id) && long.TryParse(args[2], out arrival))
                    try { opened = WinNotifications.ActivateLocal(id, arrival); } catch (Exception ex) { Slider.Log("notification click: " + ex.GetBaseException().Message); }
            if (!opened) Supervisor.PostToCore("/notify?kind=warning&title=" + Uri.EscapeDataString("Bildirim açılamadı") + "&body=" + Uri.EscapeDataString("Bildirim kaldırılmış olabilir veya uygulama hedefi açamadı."), 2000);
            Environment.Exit(opened ? 0 : 1); return;
        }
        // lunge.exe --splash: oturum açılınca (LL\Splash görevi) masaüstünü duvar kağıdıyla örter;
        // tiling ve bar hazır olup pencereler dizilince yumuşakça kaybolur. Windows'un çıplak hali hiç görünmez.
        if (args.Length == 1 && args[0] == "--splash") { Splash.Run(); return; }
        // lunge.exe --shell-menu <ayrıştırma adı> [--dock <exe adı>]
        if (args.Length >= 2 && args[0] == "--shell-menu" && (args.Length == 2 || (args.Length == 4 && args[2] == "--dock")))
        {
            ShellMenu.Run(args[1], args.Length == 4 ? args[3] : null);
            return;
        }
        // lunge-uninstall.exe --uninstall <çalışma klasörü> <kurulum klasörü>: kaldırıcının penceresi (uninstall.ps1 geçici bir
        // kopyayla başlatır: kurulum klasörü silinirken çalışmaya devam eder)
        if (args.Length == 3 && args[0] == "--uninstall") { Uninstaller.Run(args[1], args[2]); return; }
        // lunge.exe --repair-windows: masaüstü kapandıktan sonra başka uygulamaların pencerelerinde bizden kalanı onarır
        if (args.Length == 1 && args[0] == "--repair-windows") { Console.WriteLine(WindowRepair.Run()); return; }
        // lunge.exe --focus-sink <çekirdeğin pid'i>: odak penceresi kullanıcı olarak (yönetici çekirdek başlatır), çekirdekle biter
        if (args.Length == 2 && args[0] == "--focus-sink") { int core; if (int.TryParse(args[1], out core)) FocusSink.RunHost(core); return; }
        // lunge.exe --audio-default <endpoint kimliği>: varsayılan çıkış/giriş cihazını değiştir -> {"ok":true}
        if (args.Length == 2 && args[0] == "--audio-default")
        {
            int hr;
            try { hr = AudioDefault.Set(args[1]); } catch (Exception ex) { hr = Marshal.GetHRForException(ex); }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write("{\"ok\":" + (hr == 0 ? "true" : "false") + ",\"hr\":" + hr + "}"); so.Flush();
            return;
        }        // lunge.exe --songrec [-i 2 -t 30 -s monitor]: müzik tanıma exe'sini konsolsuz çalıştır, sonucu aktar.
        // Overview düğmesi bu süreci durdurursa (kill) Job Object sayesinde tanıma da hemen kapanır.
        if (args.Length >= 1 && args[0] == "--songrec")
        {
            string exe = Paths.Tool(@"songrec\lunge-songrec.exe");
            var sb = new StringBuilder();
            for (int i = 1; i < args.Length; i++) sb.Append(QuoteArg(args[i])).Append(' ');
            string res = "{\"error\":\"audio\"}";
            try
            {
                var psi = new ProcessStartInfo(exe, sb.ToString().TrimEnd()) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, StandardOutputEncoding = new UTF8Encoding(false) };
                var pr = Process.Start(psi);
                KillJob.Attach(pr);
                res = pr.StandardOutput.ReadToEnd();
                pr.WaitForExit();
            }
            catch { }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(res); so.Flush();
            return;
        }        // Duvar kağıdı: --wall-info | --wall-local | --wall-browse <tür> [sayfa] | --wall-get <url> <mod> | --wall-download <url>
        //               --wall-set <dosya> <mod> | --wall-thumb <dosya> | --wall-pick <mod>   (mod: all | span | monitör kimliği)
        if (args.Length >= 1 && args[0].StartsWith("--wall-"))
        {
            string outText;
            try
            {
                switch (args[0])
                {
                    case "--wall-info": outText = Wallpaper.Info(); break;
                    case "--wall-local": outText = Wallpaper.Local(); break;
                    case "--wall-browse": { int pg = 1; if (args.Length > 2) int.TryParse(args[2], out pg); outText = Wallpaper.Browse(args.Length > 1 ? args[1] : "top", pg); break; }
                    case "--wall-get": { string f = Wallpaper.Download(args[1]); Wallpaper.Apply(f, args.Length > 2 ? args[2] : "all"); outText = "{\"ok\":true}"; break; }
                    case "--wall-set": Wallpaper.Apply(args[1], args.Length > 2 ? args[2] : "all"); outText = "{\"ok\":true}"; break;
                    // Gallery download: save in the library and return the path.
                    case "--wall-download": outText = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "ok", true }, { "path", Wallpaper.Download(args[1]) } }); break;
                    case "--wall-thumb": outText = Wallpaper.Thumb(args[1]); break;
                    case "--wall-pick": outText = new JavaScriptSerializer().Serialize(Wallpaper.Pick(args.Length > 1 ? args[1] : "all")); break;
                    default: outText = "{\"error\":\"unknown\"}"; break;
                }
            }
            catch (Exception ex) { outText = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "error", ex.GetBaseException().Message } }); }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(outText); so.Flush();
            return;
        }
        // Canlı duvar kağıdı: --live-store [kategori] | --live-get <kategori> <ad> <mod> | --live-progress | --live-local
        //                     --live-set <video> <mod> | --live-pick <mod> | --live-clear <mod> | --live-options <0|1> <0|1>
        // Ekran koruyucu: --saver-pick | --saver-icons | --saver-videos | --saver-video <set|add|remove> <video> |
        //   --saver-shuffle <1|0> | --saver-store-get <kategori> <ad> | --saver-video-run (aynı çıktı biçimi)
        if (args.Length >= 1 && (args[0].StartsWith("--live-") || args[0].StartsWith("--saver-")))
        {
            string outText, ok = "{\"ok\":true}";
            try
            {
                switch (args[0])
                {
                    case "--live-store": outText = LiveWallpaper.Store(args.Length > 1 ? args[1] : null); break;
                    case "--live-get": outText = LiveWallpaper.Get(args[1], args[2], args.Length > 3 ? args[3] : "all"); break;
                    case "--live-progress": outText = LiveWallpaper.Progress(); break;
                    case "--live-local": outText = LiveWallpaper.Local(); break;
                    case "--live-set": LiveWallpaper.Set(args[1], args.Length > 2 ? args[2] : "all"); outText = ok; break;
                    case "--live-pick": outText = new JavaScriptSerializer().Serialize(LiveWallpaper.Pick(args.Length > 1 ? args[1] : "all")); break;
                    case "--live-clear": LiveWallpaper.Clear(args.Length > 1 ? args[1] : "all"); outText = ok; break;
                    case "--live-options": LiveWallpaper.SetOptions(args[1] == "1", args[2] == "1"); outText = ok; break;
                    case "--saver-pick": outText = ScreenSavers.Pick(); break;
                    case "--saver-icons": outText = ScreenSavers.Icons(); break;
                    case "--saver-videos": outText = SaverVideo.List(); break;
                    case "--saver-video": outText = SaverVideo.Change(args[1], args[2]); break;
                    case "--saver-shuffle": outText = SaverVideo.Shuffle(args[1] == "1"); break;
                    case "--saver-store-get": outText = SaverVideo.StoreGet(args[1], args[2]); break;
                    case "--saver-video-run": SaverVideo.Run(); outText = ok; break;
                    default: outText = "{\"error\":\"unknown\"}"; break;
                }
            }
            catch (Exception ex) { outText = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "error", ex.GetBaseException().Message } }); }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(outText); so.Flush();
            return;
        }
        // lunge.exe --capture: ana helper'dan bir kısayol yakalamasını iste, sonucu yaz ("" = iptal)
        if (args.Length == 1 && args[0] == "--capture")
        {
            string res = System.IO.Path.Combine(Binds.CaptureDir, "capture.res");
            try { System.IO.File.Delete(res); } catch { }
            System.IO.File.WriteAllText(System.IO.Path.Combine(Binds.CaptureDir, "capture.req"), "1");
            string got = null;
            for (int i = 0; i < 150 && got == null; i++) // en fazla 15 sn
            {
                Thread.Sleep(100);
                try { if (System.IO.File.Exists(res)) { got = System.IO.File.ReadAllText(res); System.IO.File.Delete(res); } } catch { }
            }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(got ?? ""); so.Flush();
            return;
        }
        // lunge.exe --bind <id> <combo> | --bind-reset -> {"ok":true|false} (düzenleyici sonucu buna göre gösterir)
        if ((args.Length == 3 && args[0] == "--bind") || (args.Length == 1 && args[0] == "--bind-reset"))
        {
            bool ok = args.Length == 3 ? Binds.Set(args[1], args[2]) : Binds.Set("", null);
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(ok ? "{\"ok\":true}" : "{\"ok\":false}"); so.Flush();
            return;
        }
        // Kısayol düzenleyicisi: model, bekleyen durumun çakışmaları, kayıt, sıfırlama, uygulama seçtirme (çıktı JSON)
        if ((args.Length == 1 && (args[0] == "--keybinds-model" || args[0] == "--keybinds-pick-app" || args[0] == "--keybinds-reset"))
            || (args.Length == 2 && (args[0] == "--keybinds-check" || args[0] == "--keybinds-save" || (args[0] == "--keybinds-reset" && args[1] == "--apps"))))
        {
            string outText;
            switch (args[0])
            {
                case "--keybinds-model": outText = Keymap.Model(); break;
                case "--keybinds-check": outText = Keymap.Check(args[1]); break;
                case "--keybinds-save": outText = Keymap.Save(args[1]); break;
                case "--keybinds-reset": outText = Keymap.Reset(args.Length == 2); break;
                default: outText = Keymap.PickApp(); break;
            }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(outText); so.Flush();
            return;
        }
        if (args.Length == 1 && args[0] == "--keybinds")
        {
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(Binds.ListJson()); so.Flush();
            return;
        }
        // Güncelleme: --update-check | --update-download | --update-status | --update-install
        if (args.Length == 1 && args[0].StartsWith("--update-"))
        {
            string ut;
            try
            {
                switch (args[0])
                {
                    case "--update-check": ut = Updater.Check(); break;
                    case "--update-download": ut = Updater.Download(); break;
                    case "--update-status": ut = Updater.Status(); break;
                    case "--update-install": ut = Updater.Install(); break;
                    default: ut = "{\"error\":\"unknown\"}"; break;
                }
            }
            catch (Exception ex) { ut = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "error", ex.GetBaseException().Message } }); }
            var uo = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            uo.Write(ut); uo.Flush();
            return;
        }
        // lunge.exe --black-box: o anki performans durumunu (işlemci / GPU / parçalar) log'a yaz
        if (args.Length == 1 && args[0] == "--black-box") { PerfGuard.DumpNow("elle istendi"); return; }
        // UTF-8 JSON attachment/device contracts; native spellings remain aliases.
        if ((args.Length == 2 && (args[0] == "--bug-file" || args[0] == "--bug-report-file"))
            || (args.Length == 1 && (args[0] == "--bug-device" || args[0] == "--bug-report-device")))
        {
            string report = args.Length == 1 ? BugReports.Device() : BugReports.File(args[1]);
            var output = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            output.Write(report); output.Flush();
            return;
        }
        // lunge.exe --switcher-demo: Alt+Tab menüsünü 6 sn göster (sınama; kısayolsuz)
        if (args.Length == 1 && args[0] == "--switcher-demo") { Switcher.Demo(); return; }
        // lunge.exe --log <metin>: widget'ların hata ayıklama günlüğü (%LOCALAPPDATA%\LogicalLunge\logs\core.log)
        if (args.Length == 2 && args[0] == "--log") { Slider.Log("widget: " + args[1]); return; }
        // lunge.exe --overview-show clip|plain: overview'u ilgili modda aç (test / betik için; Super / Super+V aynısını yapar)
        if (args.Length == 2 && args[0] == "--overview-show")
        {
            IntPtr ovh = Native.FindWindow(null, "lunge-overview");
            if (ovh != IntPtr.Zero)
            {
                Keys2.ShowOverviewInMode(ovh, args[1] == "clip" ? ";" : "s");
                // Widget'ın beklediği haber bu süreçte değil çalışan helper'da: ona ilet
                try { using (var wc = new System.Net.WebClient()) wc.UploadString("http://127.0.0.1:6131/overview-signal?w=show", ""); } catch { }
                Thread.Sleep(1500);
            }
            return;
        }
        // Pano geçmişi ve overview modu: --clip-list | --clip-set <id> | --clip-del <id> | --clip-clear | --overview-mode
        if (args.Length >= 1 && (args[0].StartsWith("--clip-") || args[0] == "--overview-mode"))
        {
            string ct;
            try
            {
                switch (args[0])
                {
                    case "--clip-list": ct = ClipHistory.List(); break;
                    case "--clip-set": ct = ClipHistory.Set(args[1]); break;
                    case "--clip-del": ct = ClipHistory.Delete(args[1]); break;
                    case "--clip-clear": ct = ClipHistory.Clear(); break;
                    case "--overview-mode":
                    {
                        ct = Keys2.TakeOverviewMode(); // bir kez okunur ve silinir: "", ";" (pano) ya da "#" (dosya araması)
                        break;
                    }
                    default: ct = "{\"error\":\"unknown\"}"; break;
                }
            }
            catch (Exception ex) { ct = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "error", ex.GetBaseException().Message } }); }
            var co = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            co.Write(ct); co.Flush();
            return;
        }
        // lunge.exe --snip-screen: farenin olduğu monitörün tamamı, sormadan panoya + dosyaya
        if (args.Length == 1 && args[0] == "--snip-screen")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            SnipTool.RunScreen();
            return;
        }
        // lunge.exe --snip: bölge ekran alıntısı + düzenleme (Hyprland Print: grim + slurp + swappy)
        if (args.Length >= 1 && args.Length <= 2 && args[0] == "--snip")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            int wait; // --snip 350: paneller kapansın diye önce bekle
            if (args.Length == 2 && int.TryParse(args[1], out wait)) Thread.Sleep(Math.Min(2000, wait));
            bool fresh;
            var sm = new Mutex(true, "lunge-snip", out fresh);
            // Önceki alıntı süreci penceresiz takılı kaldıysa (ör. kaydetme penceresi hiç açılamadı) yenileri sonsuza dek
            // engellenmesin: onu kapatıp kilidi devral
            if (!fresh && SnipTool.KillStale())
            {
                try { fresh = sm.WaitOne(1500); } catch (AbandonedMutexException) { fresh = true; }
            }
            if (fresh)
            {
                try { System.IO.File.WriteAllText(SnipTool.PidFile, Process.GetCurrentProcess().Id.ToString()); } catch { }
                IntPtr prevFg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
                SnipTool.Run();
                SnipTool.GiveFocusBack(prevFg);
            }
            return;
        }
        // lunge.exe --open <https://... | spotify:... | ms-actioncenter:>: bağlantıyı varsayılan uygulamada aç. explorer.exe'ye
        // verilen adres "&" içerince klasör açıyordu; ShellExecute doğrudan protokol işleyicisine gider.
        // "Birlikte aç" (açacak uygulaması olmayan dosyanın kartı): Windows'un uygulama seçicisi, kullanıcı olarak
        if (args.Length == 2 && args[0] == "--open" && args[1].StartsWith("openwith:", StringComparison.OrdinalIgnoreCase))
        {
            string target = args[1].Substring(9);
            if (System.IO.Path.IsPathRooted(target) && System.IO.File.Exists(target))
                UserLaunch.Start("rundll32.exe", "shell32.dll,OpenAs_RunDLL " + target, Paths.Home);
            return;
        }
        if (args.Length == 2 && args[0] == "--open" && System.Text.RegularExpressions.Regex.IsMatch(args[1], "^(https?|spotify|mailto|ms-actioncenter):", System.Text.RegularExpressions.RegexOptions.IgnoreCase))
        {
            UserLaunch.Start(args[1], "", Paths.Home);
            return;
        }
        // lunge.exe --launch <program> [argümanlar]: programı normal kullanıcı olarak başlatır. Yönetici olarak çalışan
        // pencere yöneticisinin shell-exec komutları (config'deki kısayollar) bunu kullanır.
        if (args.Length >= 2 && (args[0] == "--launch" || args[0] == "--launch-hidden"))
        {
            var la = new StringBuilder();
            for (int i = 2; i < args.Length; i++) la.Append(QuoteArg(args[i])).Append(' ');
            Environment.Exit(UserLaunch.Start(args[1], la.ToString().TrimEnd(), Paths.Home, args[0] == "--launch-hidden") ? 0 : 1);
        }        // lunge.exe --lens: ii "region search" — alan seç, Google Lens'te aç
        if (args.Length == 1 && args[0] == "--lens")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            RegionSearch.Lens();
            return;
        }
        // lunge.exe --toast-stream: çalışan helper'ın bildirim kanalına bağlanıp her bildirimi
        // stdout'a tek satır JSON yazar. shell toast widget'ı bunu shellSpawn ile okur (widget'ların
        // yerel adreslere doğrudan bağlanmasına shell izin vermiyor).
        // lunge.exe --ask ...: soruyu kabuğun diyaloğunda sorar (Dialogs.AskCli)
        if (args.Length >= 1 && args[0] == "--ask") Environment.Exit(Dialogs.AskCli(args));
        if (args.Length == 1 && args[0] == "--toast-stream")
        {
            // shell (ebeveyn) kapanınca bu kopya da kapansın: yoksa shell'den miras aldığı sunucu
            // soketini tutarak yeni shell'in açılmasını engelliyor.
            // Kabuğun kapanması beklenir (yoklama değil): kopya hemen çıkar, kurulum klasöründeki lunge.exe'yi de açık tutmaz
            int parent = ParentPid();
            new Thread(() =>
            {
                try { using (var p = Process.GetProcessById(parent)) p.WaitForExit(); }
                catch { }
                Environment.Exit(0);
            }) { IsBackground = true }.Start();
            var stdout = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false)) { AutoFlush = true };
            // Helper'ın koruyucusu: asıl helper (Super, animasyonlar, pano...) tamamen kapanmışsa ve ~10 sn içinde geri
            // gelmediyse onu başlatır. tiling kapalıysa (kasıtlı çıkış) ya da bakım sırasında karışmaz.
            int refused = 0;
            while (true)
            {
                try
                {
                    if (refused >= 5)
                    {
                        refused = 0;
                        System.Threading.Mutex existing;
                        bool alive = System.Threading.Mutex.TryOpenExisting("LogicalLunge.Core", out existing);
                        if (existing != null) existing.Dispose();
                        if (!alive && Maint.Running(Names.Tiling) && !Maint.Quiet() && Maint.Allow("helper-restarts"))
                        {
                            // Oturum görevinden: çekirdek kurulumdaki haklarıyla (yönetici) başlar; görev yoksa doğrudan
                            if (Maint.RunHidden("schtasks.exe", "/run /tn \"\\LogicalLunge\\Start\"", 10000) != 0)
                                Process.Start(new ProcessStartInfo(Maint.CoreExe) { UseShellExecute = true, WorkingDirectory = Paths.Home });
                        }
                    }
                    using (var c = new System.Net.Sockets.TcpClient("127.0.0.1", 6131))
                    using (var s = c.GetStream())
                    {
                        refused = 0;
                        var req = Encoding.ASCII.GetBytes("GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                        s.Write(req, 0, req.Length);
                        var reader = new System.IO.StreamReader(s, Encoding.UTF8);
                        string line;
                        while ((line = reader.ReadLine()) != null)
                            if (line.StartsWith("data: ")) stdout.WriteLine(line.Substring(6));
                    }
                }
                catch (System.IO.IOException) { if (!CanWrite(stdout)) return; }
                catch (System.Net.Sockets.SocketException) { refused++; }
                catch (Exception) { }
                if (!CanWrite(stdout)) return; // widget kapandı
                Thread.Sleep(2000);
            }
        }

        // lunge.exe --ps <script.ps1> [argümanlar]   : PowerShell'i HİÇ pencere açmadan çalıştırır,
        //                                                   çıktısını kendi stdout'una aktarır
        // lunge.exe --ps-bg <script.ps1> [argümanlar]: aynı, ama beklemeden arka planda bırakır
        // (shell'den doğrudan powershell çağırmak bir anlık konsol penceresi gösterebiliyordu.)
        if (args.Length >= 2 && args[0] == "--ps-bg")
        {
            // Uzun ömürlü arka plan betiği (uyanık tut vb.): ShellExecute ile başlat ki shell'den
            // miras kalan soket/tanıtıcıları DEVRALMASIN. Aksi halde shell kapanınca 6124 portu
            // bu süreçte asılı kalıyor ve yeni shell sunucusunu açamıyor (bar boş geliyordu).
            var sbg = new StringBuilder("-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File ");
            for (int i = 1; i < args.Length; i++) sbg.Append(QuoteArg(args[i])).Append(' ');
            Process.Start(new ProcessStartInfo("powershell.exe", sbg.ToString().TrimEnd())
            {
                UseShellExecute = true,
                WindowStyle = ProcessWindowStyle.Hidden,
                WorkingDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
            });
            return;
        }
        if (args.Length >= 2 && args[0] == "--ps")
        {
            var sb = new StringBuilder("-NoProfile -ExecutionPolicy Bypass -File ");
            for (int i = 1; i < args.Length; i++) sb.Append(QuoteArg(args[i])).Append(' ');
            var psi = new ProcessStartInfo("powershell.exe", sb.ToString().TrimEnd())
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WindowStyle = ProcessWindowStyle.Hidden,
                RedirectStandardOutput = args[0] == "--ps",
                StandardOutputEncoding = args[0] == "--ps" ? Encoding.UTF8 : null,
                WorkingDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
            };
            var p = Process.Start(psi);
            if (args[0] == "--ps-bg") return;
            string output = p.StandardOutput.ReadToEnd();
            p.WaitForExit();
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(output); so.Flush();
            Environment.Exit(p.ExitCode);
        }

        // lunge.exe --run <run|term|url> <metin>: Super menüsünün komut / web eylemi (RunCommand); sonucu JSON, açıldıysa boş
        if (args.Length >= 2 && args[0] == "--run")
        {
            string result = RunCommand.Run(args[1], string.Join(" ", args, 2, args.Length - 2));
            if (result != null)
            {
                var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
                so.Write(result); so.Flush();
            }
            return;
        }
        // lunge.exe --eth enable|disable: kurulumun yönetici görevleri (LogicalLunge\Ethernet-On / -Off) çağırır
        if (args.Length == 2 && args[0] == "--eth" && (args[1] == "enable" || args[1] == "disable"))
        {
            Environment.Exit(QuickSettings.SetEth(args[1] == "enable") ? 0 : 1);
            return;
        }
        // lunge.exe --build-apps: Super menüsünün uygulama listesini (state\apps.json) yeniden yazar (AppIndex)
        if (args.Length == 1 && args[0] == "--build-apps")
        {
            try
            {
                int n = AppIndex.Build(Paths.AppsJson);
                var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
                so.Write(n + " uygulama -> " + Paths.AppsJson); so.Flush();
            }
            catch (Exception ex)
            {
                var se = new System.IO.StreamWriter(Console.OpenStandardError(), new UTF8Encoding(false));
                se.Write(ex.GetBaseException().Message); se.Flush();
                Environment.Exit(1);
            }
            return;
        }

        // lunge.exe --mic toggle|on|off|status -> {"muted":true}  (on = mikrofon açık)
        if (args.Length == 2 && args[0] == "--mic")
        {
            if (args[1] == "toggle") Mic.SetAll(!Mic.IsMuted());
            else if (args[1] == "on") Mic.SetAll(false);
            else if (args[1] == "off") Mic.SetAll(true);
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write("{\"muted\":" + (Mic.IsMuted() ? "true" : "false") + "}"); so.Flush();
            return;
        }
        if (args.Length == 1 && args[0] == "--osk") { Osk.Run(); return; }

        // lunge.exe --raise "<pencere başlığı>" : pencereyi her zaman üstte yapıp en öne getir.
        // (shell'in setAlwaysOnTop'u gizle/göster sonrası etkisiz kalıyordu; sağ panel terminalin arkasında açılıyordu.)
        // --top: yalnızca en üste al, odak verme (ekran klavyesi: tuşlar yazılan uygulamaya gitmeli)
        // Pencereyi göstermez: gösterme widget'ın işi. Bu süreç geç başladığında widget bu arada kapanmış olabiliyor;
        // eskiden SWP_SHOWWINDOW onu yeniden açıyordu (içi boş, tıklamaları yutan pencere).
        if (args.Length == 2 && (args[0] == "--raise" || args[0] == "--top"))
        {
            IntPtr rh = Native.FindWindow(null, args[1]);
            if (rh == IntPtr.Zero || !Native.IsWindowVisible(rh)) return;
            Native.SetWindowPos(rh, new IntPtr(-1) /*HWND_TOPMOST*/, 0, 0, 0, 0, 0x0001 | 0x0002 | (args[0] == "--top" ? 0x0010u : 0u) /*NOSIZE|NOMOVE|NOACTIVATE*/);
            if (args[0] == "--top") return;
            Native.keybd_event(0xE8, 0, 0, Native.LL_MARK); Native.keybd_event(0xE8, 0, 2, Native.LL_MARK); // önplan izni
            Native.SetForegroundWindow(rh);
            return;
        }

        // lunge.exe --nightlight on|off|toggle|status  -> {"on":true}
        if (args.Length == 2 && args[0] == "--nightlight")
        {
            if (args[1] == "on") NightLight.Enabled = true;
            else if (args[1] == "off") NightLight.Enabled = false;
            else if (args[1] == "toggle") NightLight.Enabled = !NightLight.Enabled;
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(NightLight.StatusJson()); so.Flush();
            return;
        }
        // lunge.exe --nightlight-set level 60 | mode manual|after|range | from 20:00 | to 07:00
        if (args.Length == 3 && args[0] == "--nightlight-set" && System.Text.RegularExpressions.Regex.IsMatch(args[1], "^(level|mode|from|to)$"))
        {
            NightLight.Set(args[1], args[2]);
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(NightLight.StatusJson()); so.Flush();
            return;
        }

        // Tek seferlik: lunge.exe --focus-under-cursor  (overview uygulama açmadan önce çağırır,
        // yeni pencere Hyprland dwindle'daki gibi farenin altındaki pencereyi bölsün)
        if (args.Length == 1 && args[0] == "--focus-under-cursor")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            new Slider(new TilingClient()).FocusUnderCursor();
            return;
        }

        if (args.Length == 1 && args[0] == "--uncloak-orphans")
        {
            Console.WriteLine(Orphans.Uncloak());
            return;
        }
        if (args.Length == 2 && args[0] == "--uncloak-orphans" && args[1] == "--list")
        {
            Orphans.Uncloak(true);
            return;
        }

        if (args.Length == 1 && args[0] == "--anim-selftest")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            new Slider(new TilingClient()).SelfTest();
            return;
        }

        // Tek seferlik: lunge.exe --slide next|prev|<workspace>  (bar tıklamaları ve test için)
        if (args.Length == 2 && args[0] == "--slide")
        {
            // Çalışan helper varsa işi ona devret (katman ve kenarlıkları hazır, animasyon hemen başlar)
            string act = args[1] == "next" ? "ws-next" : args[1] == "prev" ? "ws-prev" : "ws-" + args[1];
            if (Supervisor.PostToCore("/cmd?a=" + Uri.EscapeDataString(act), 400) == 204) return;
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            var g = new TilingClient();
            var sl = new Slider(g);
            if (args[1] == "next") sl.Run(new[] { "focus --next-workspace" }, 1, null);
            else if (args[1] == "prev") sl.Run(new[] { "focus --prev-workspace" }, -1, null);
            else sl.Run(new[] { "focus --workspace " + args[1] }, 0, args[1]);
            return;
        }

        // lunge.exe --restart-desktop: "Masaüstünü yenile" (oturum menüsü, Başlat kısayolu)
        if (args.Length == 1 && args[0] == "--restart-desktop") { if (!Supervisor.RequestFromCore("restart-desktop")) Supervisor.RestartDesktopDetached(); return; }
        if (args.Length == 1 && args[0] == "--restart-desktop-now") { Supervisor.RestartDesktop(); return; }
        // lunge.exe --stop-desktop: kurulum / güncelleme / kaldırma öncesi masaüstünü kapatır (bakım işareti kalır)
        if (args.Length == 1 && args[0] == "--stop-desktop") { Supervisor.StopDesktopFromAnywhere(); return; }
        // lunge.exe --restore-banners: kaldırırken Windows'un bildirim balonlarını eski haline getirir (ToastBanners)
        if (args.Length == 1 && args[0] == "--restore-banners") { ToastBanners.Restore(); return; }
        // kaldırma / kurulumun geri alması: devredilen Windows parçaları asıl değerlerine (state\shell-takeover.json)
        if (args.Length == 1 && args[0] == "--takeover-restore") { ShellTakeover.ReleaseAll(); return; }
        // lunge.exe --restart-shell: kabuğu yeniden aç (ayarlar penceresi, tercihler değişince)
        if (args.Length == 1 && args[0] == "--restart-shell") { Supervisor.RequestFromCore("restart-shell"); return; }
        // Ayarlar penceresi: --settings-get | --set-focus-color #rrggbb | --set-pref <anahtar> <değer> | --health |
        //                    --edit-config | --wm wm-reload-config|wm-redraw   -> JSON
        if (args.Length >= 1 && (args[0] == "--settings-get" || args[0] == "--set-focus-color" || args[0] == "--set-pref" || args[0] == "--set-workspaces" || args[0] == "--health" || args[0] == "--edit-config" || args[0] == "--wm"))
        {
            string st;
            try { st = Settings.Cli(args); }
            catch (Exception ex) { st = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "ok", false }, { "error", ex.GetBaseException().Message } }); }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(st); so.Flush();
            return;
        }
        // lunge.exe --shutdown: tiling kapanırken (config'deki shutdown_commands) çalışır. Asıl işi çekirdek yapar;
        // bu yedek çekirdek yoksa da shell'in kapanıp Windows görev çubuğunun geri gelmesini sağlar.
        if (args.Length == 1 && args[0] == "--shutdown") { if (!Maint.Quiet()) Supervisor.Shutdown("çıkış komutu", false); return; }

        // Tanınmayan bir komut (ör. eski bir çekirdeğe yeni bir sürümün komutu) asıl çekirdek gibi açılıp masaüstünü
        // başlatmasın: yalnızca argümansız ya da --respawn ile açılan süreç asıl çekirdektir
        if (args.Length > 0 && !(args.Length == 1 && (args[0] == "--respawn" || args[0] == "--restart-desktop-candidate")))
        {
            Slider.Log("bilinmeyen komut: " + string.Join(" ", args));
            Environment.Exit(2);
        }

        System.IO.Pipes.NamedPipeClientStream restart;
        if (!DesktopRestart.Join(args.Length == 1 && args[0] == "--restart-desktop-candidate", out restart)) return;
        var startup = restart == null ? null : new DesktopRestart.Startup(restart);

        // Yakalanmayan her hatayı yığın iziyle log'a yaz (sessiz çökme olmasın)
        AppDomain.CurrentDomain.UnhandledException += (s, e) =>
        {
            Slider.Log("CRASH: " + e.ExceptionObject);
            LogWriter.Flush(2000);
            if (startup != null && !startup.Accepted) { startup.Fail(); startup.Wait(3500); }
            else if (e.IsTerminating) SelfHeal.Respawn("çöktü: " + (e.ExceptionObject is Exception ? e.ExceptionObject.GetType().Name : "?"));
        };
        Application.ThreadException += (s, e) =>
        {
            Slider.Log("UI HATA: " + e.Exception);
            if (startup != null && !startup.Accepted) startup.Fail();
        };
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);

        try
        {
        bool created;
        var mutex = new Mutex(true, "LogicalLunge.Core", out created);
        // Kendini yeniden başlatan kopya: eskisi kapanıp kilidi bırakana kadar bekle
        if (!created && (restart != null || (args.Length == 1 && args[0] == "--respawn")))
        {
            try { created = mutex.WaitOne(15000); }
            catch (AbandonedMutexException) { created = true; }
        }
        if (!created) { if (startup != null) { startup.Fail(); startup.Wait(3500); } return; }
        SelfHeal.IsMain = restart == null; // no autonomous respawn until the coordinator acknowledges full startup
        try { System.IO.File.WriteAllText(Supervisor.PidFile, Process.GetCurrentProcess().Id.ToString()); } catch { }
        // Pencere yöneticisinin config'i kurulum klasörüne %LUNGE_HOME% ile başvurur (shell-exec komutları); başlattığı
        // alt süreçlere bu ortam değişkeni geçer
        Environment.SetEnvironmentVariable("LUNGE_HOME", Paths.Install);
        // Ayarlar penceresinin sağlık sayfası için (yönetici mi, sürüm, ne zamandan beri)
        try
        {
            System.IO.File.WriteAllText(Paths.State("core.json"), new JavaScriptSerializer().Serialize(new Dictionary<string, object> {
                { "pid", Process.GetCurrentProcess().Id }, { "elevated", UserLaunch.Elevated }, { "started", DateTime.Now.ToString("o") }, { "version", Updater.Installed() } }));
        }
        catch { }
        // Kök süreç: eksik parçaları (perde, tiling, shell) kendi alt süreçleri olarak aç. Çekirdeğin kendi kurulumunu
        // beklemez; perde hemen gelsin.
        new Thread(() =>
        {
            try
            {
                Supervisor.BringUp(restart == null);
                if (startup != null) Volatile.Write(ref startup.BringUpDone, 1);
            }
            catch (Exception ex) { Slider.Log("kök: " + ex.Message); if (startup != null) startup.Fail(); }
        }) { IsBackground = true, Name = "bring-up" }.Start();
        Microsoft.Win32.SystemEvents.SessionEnding += (s0, e0) => { Maint.SessionEnding = true; };
        try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { } // PER_MONITOR_AWARE_V2

        Application.EnableVisualStyles();
        var ui = new Form { ShowInTaskbar = false, WindowState = FormWindowState.Minimized, FormBorderStyle = FormBorderStyle.None, Opacity = 0 };
        ui.Load += (s, e) => ui.Hide();
        var h = ui.Handle;

        var tiling = new TilingClient();
        var slider = new Slider(tiling);
        slider.Warm();
        slider.StartWatchdog();
        // Monitör takıldı/çıkarıldı ya da çözünürlük değişti: yeni dikdörtgenlerin katmanı da hazır beklesin
        Microsoft.Win32.SystemEvents.DisplaySettingsChanged += (s0, e0) => { try { ui.BeginInvoke((Action)slider.Warm); } catch { } };
        Slider.Ui = ui;
        ConfigWatch.Start(ui); // prefs.json (animasyonlar) ve config.yaml (odak rengi) değişince
        ui.BeginInvoke((Action)(() => Touchpad.Start(slider))); // dokunmatik yüzey hareketleri (girdi UI thread'ine)
        var dwindle = new Dwindle(new TilingClient(), ui, slider);
        dwindle.Start(); // kendi IPC bağlantısıyla: slide'ı beklemesin
        dwindle.HookNewWindows();
        LaunchQueue.Start(new Slider(new TilingClient())); // kendi bağlantısı: animasyonu beklemesin
        NightLight.StartKeeper();
        Switcher.Init(ui);
        ClipHistory.StartListener();
        Wallpaper.StartKeeper();
        Toasts.Start();
        WinNotifications.Start();
        Urgent.Start();

        // Klavye kancası KENDİ thread'inde ve orada başka hiçbir iş yapılmaz: LL hook ~300ms'de
        // yanıt vermezse Windows kancayı söker ve o sırada klavye donar. (Eskiden köşe yuvarlama
        // aynı thread'deydi; SetWindowRgn askıdaki bir pencerede bekleyince klavye donuyordu.)
        InputLatency.PrepareProcess();
        var hookThread = new Thread(() =>
        {
            InputLatency.PrepareThread();
            var beat = HangWatch.Register("klavye kancası");
            Binds.Watch();
            var keys = new Keys2(ui, slider);
            keys.Start();
            keys.WatchDesktopSwitch();
            keys.StartTestPipe();
            // Windows kancayı bir şekilde sökse bile geri gelsin
            var re = new System.Windows.Forms.Timer { Interval = 15000 };
            re.Tick += (s, e) => keys.Reinstall();
            // Kanca bekçisi: kullanıcı girdisi var ama iki kanca da 1,5 sn'dir çağrılmadı -> Windows sökmüş; 15 sn'lik yenilemeyi
            // beklemeden yeniden kur (o arada Super, Alt+Tab, kısayollar bize gelmiyordu)
            var health = new System.Windows.Forms.Timer { Interval = 1000 };
            health.Tick += (s, e) =>
            {
                beat();
                InputLatency.Sample();
                keys.Unstick();
                if (HooksStale()) { keys.Reinstall(true); Keys2.LastHookTick = Environment.TickCount; Slider.Log("klavye kancası girdi görmüyordu (Windows sökmüş olabilir): yeniden kuruldu"); }
            };
            health.Start();
            re.Start();
            Application.Run();
        });
        hookThread.SetApartmentState(ApartmentState.STA);
        hookThread.IsBackground = true;
        hookThread.Priority = ThreadPriority.Highest;
        hookThread.Start();

        // Fare kancası ayrı thread'de: fare olaylarının işi klavye olaylarını bekletmesin (klavye kancası Windows'un süre
        // sınırını aşınca o tuş kancasız geçiyor, tek başına bir Win basışı Başlat menüsünü açıyordu)
        var mouseThread = new Thread(() =>
        {
            InputLatency.PrepareThread();
            var beat = HangWatch.Register("fare kancası");
            var mouse = new MouseFocus(new TilingClient());
            mouse.InstallHook();
            mouse.StartWorker();
            mouse.StartClickWorker();
            var re = new System.Windows.Forms.Timer { Interval = 15000 };
            re.Tick += (s, e) => mouse.Reinstall();
            var health = new System.Windows.Forms.Timer { Interval = 1000 };
            health.Tick += (s, e) =>
            {
                beat();
                if (HooksStale()) { mouse.Reinstall(); MouseFocus.LastHookTick = Environment.TickCount; Slider.Log("fare kancası girdi görmüyordu (Windows sökmüş olabilir): yeniden kuruldu"); }
            };
            health.Start();
            re.Start();
            Application.Run();
        });
        mouseThread.SetApartmentState(ApartmentState.STA);
        mouseThread.IsBackground = true;
        mouseThread.Priority = ThreadPriority.Highest;
        mouseThread.Start();

        var roundThread = new Thread(() =>
        {
            Keep.Round = new Rounder(); Keep.Round.Start();
            ShellTakeover.Start();
            Application.Run();
        });
        roundThread.SetApartmentState(ApartmentState.STA);
        roundThread.IsBackground = true;
        roundThread.Start();

        // İlk açılış (yeni kurulum): Super menüsünün uygulama listesi ve terminal renkleri yoksa üret
        ThreadPool.QueueUserWorkItem(_ =>
        {
            try
            {
                string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
                string apps = Paths.AppsJson;
                bool refresh = !System.IO.File.Exists(apps);
                if (!refresh)
                {
                    var info = new System.IO.FileInfo(apps);
                    refresh = info.Length <= 2 || info.LastWriteTimeUtc < DateTime.UtcNow.AddDays(-1);
                }
                if (refresh) AppIndex.RebuildInBackground(System.IO.File.Exists(apps) ? "a day old" : "first run");
                AppIndex.WatchStartMenu();
                EverythingIndex.Start();
                string colors = System.IO.Path.Combine(home, @".config\wezterm\ll-colors.lua");
                string tc = Paths.Tool(@"termcolors\lunge-termcolors.exe");
                if (!System.IO.File.Exists(colors) && System.IO.File.Exists(tc))
                    Process.Start(new ProcessStartInfo(tc) { UseShellExecute = false, CreateNoWindow = true });
                // Yalnızca oturum açılışında (perde ekranı örterken); sonradan helper yeniden başlarsa pencere göstermesin
                if (WarmTerminal.SplashActive())
                    WarmTerminal.Prewarm(Keys2.TerminalPath);
            }
            catch (Exception ex) { Slider.Log("first run: " + ex.Message); }
        });
        // Arkada derleme / oyun / güncelleme CPU'yu doldursa da kayma ve odak gecikmesin: helper ve
        // tiling yüksek öncelikte (tiling yeniden başlarsa diye 10 sn'de bir yenilenir; yönetici gerekmez).
        try { Process.GetCurrentProcess().PriorityClass = ProcessPriorityClass.High; } catch { }
        // Yerel HTTP sunucusu uzun yoklamaları havuz thread'lerinde bekletir: havuz dolunca .NET yeni thread'i saniyede
        // ~2 tane ekliyor, kısa işler (tıklama, menü) bekliyordu. Alt sınır yükseltilir (thread'ler ancak gerekince açılır).
        { int w, io; ThreadPool.GetMinThreads(out w, out io); ThreadPool.SetMinThreads(Math.Max(w, 32), io); }
        var prioThread = new Thread(() =>
        {
            // Yakalanmayan bir hata (süreç listesi alınamadı) bu iş parçacığında bütün çekirdeği kapatırdı; süreç tablosunu
            // taramak da ucuz değil: 30 sn'de bir yeter (yeniden başlayan parça önceliğini en geç o zaman alır)
            while (true)
            {
                try
                {
                    foreach (var name in new[] { Names.Tiling, Names.Shell })
                        foreach (var pr in Process.GetProcessesByName(name))
                            try
                            {
                                var want = name == Names.Tiling ? ProcessPriorityClass.High : ProcessPriorityClass.AboveNormal;
                                if (pr.PriorityClass != want) pr.PriorityClass = want;
                            }
                            catch { }
                            finally { pr.Dispose(); }
                }
                catch (Exception ex) { Slider.Log("öncelik: " + ex.GetBaseException().Message); }
                Thread.Sleep(30000);
            }
        }) { IsBackground = true, Priority = ThreadPriority.Lowest };
        prioThread.Start();

        ShellWatchdog.Start();
        TempSweep.Start();
        TilingWatchdog.Start();
        FocusSink.Start();
        ThreadPool.QueueUserWorkItem(_ => SplashTask.Ensure());
        FocusGuard.Start();
        PerfGuard.Start();
        SelfHeal.WatchUi(ui);
        // Arayüzün, klavye ve fare kancalarının nabzı: biri takılınca çekirdek kendini yeniden başlatır (HangWatch)
        var uiBeat = HangWatch.Register("arayüz");
        var uiBeatTimer = new System.Windows.Forms.Timer { Interval = 1000 };
        uiBeatTimer.Tick += (s, e) => uiBeat();
        uiBeatTimer.Start();
        HangWatch.Start();

        if (startup != null)
        {
            Volatile.Write(ref startup.Initialized, 1);
            ui.BeginInvoke((Action)(() => Volatile.Write(ref startup.UiReady, 1)));
        }
        Application.Run(ui);
        GC.KeepAlive(mutex);
        }
        catch (Exception ex)
        {
            if (startup == null || startup.Accepted) throw;
            Slider.Log("restart initialization failed: " + ex.GetBaseException().Message);
            startup.Fail(); startup.Wait(3500);
        }
        finally { if (startup != null) startup.Dispose(); }
    }
}
