// ll-helper — GlazeWM'in yapamadığı, Hyprland/ii'de olan üç şey:
//   1) Workspace geçişinde "slide" animasyonu (Hyprland: animation workspaces, slide, menu_decel)
//      DWM thumbnail'leri ile: eski workspace'in canlı görüntüsü kayarak çıkar, yenisi girer.
//   2) Tüm pencerelerde yuvarlak köşe (Hyprland decoration.rounding) — Win10'da DWM yapmadığı
//      için SetWindowRgn ile.
//   3) Tek başına Super -> ii overview (arama) aç/kapa; Başlat menüsü açılmaz.
//
// Kısayollar (GlazeWM config'den buraya taşındı, animasyonlu olsunlar diye):
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
    [DllImport("user32.dll")] public static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr mod, WinEventDelegate fn, uint pid, uint tid, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", SetLastError = true)] public static extern int SetWindowRgn(IntPtr h, IntPtr rgn, bool redraw);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetLayeredWindowAttributes(IntPtr h, uint key, byte alpha, uint flags);
    [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr h, IntPtr rect, IntPtr rgn, uint flags);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateRoundRectRgn(int l, int t, int r, int b, int w, int h);
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

// ---------------- GlazeWM IPC (ws://localhost:6123) ----------------
class Glaze
{
    ClientWebSocket ws;
    readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };
    readonly object gate = new object();

    Dictionary<string, object> Send(string message)
    {
        lock (gate)
        {
            for (int attempt = 0; attempt < 2; attempt++)
            {
                try
                {
                    if (ws == null || ws.State != WebSocketState.Open)
                    {
                        ws = new ClientWebSocket();
                        ws.Options.Proxy = null; // yoksa WPAD proxy araması bağlantıyı saniyelerce geciktiriyor
                        ws.ConnectAsync(new Uri("ws://127.0.0.1:6123"), CancellationToken.None).Wait(3000);
                    }
                    var bytes = Encoding.UTF8.GetBytes(message);
                    ws.SendAsync(new ArraySegment<byte>(bytes), WebSocketMessageType.Text, true, CancellationToken.None).Wait(1500);
                    while (true)
                    {
                        string text = Receive();
                        if (text == null) break;
                        var obj = json.DeserializeObject(text) as Dictionary<string, object>;
                        if (obj == null) continue;
                        object type, cm;
                        obj.TryGetValue("messageType", out type);
                        obj.TryGetValue("clientMessage", out cm);
                        if ((type as string) == "client_response" && (cm as string) == message) return obj;
                    }
                }
                catch (Exception ex) { Slider.Log("ipc error: " + ex.GetBaseException().Message); ws = null; }
            }
            return null;
        }
    }

    string Receive()
    {
        var buf = new byte[1 << 16];
        var sb = new StringBuilder();
        while (true)
        {
            var t = ws.ReceiveAsync(new ArraySegment<byte>(buf), CancellationToken.None);
            if (!t.Wait(2000)) return null;
            var r = t.Result;
            if (r.MessageType == WebSocketMessageType.Close) { ws = null; return null; }
            sb.Append(Encoding.UTF8.GetString(buf, 0, r.Count));
            if (r.EndOfMessage) return sb.ToString();
        }
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

    public void Command(string cmd) { Send("command " + cmd); }

    // GlazeWM IPC'ye yanıt veriyor mu (pencere yöneticisi nöbetçisi için)
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
// demekti. Pencere kapanırken DWM meşgulken bu kayıt 15-35 ms sürüyor, katman GlazeWM pencereleri kaydırdıktan sonra
// açılıyordu.)
class RingLayer : Form
{
    [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr h, int attr, ref int value, int size);
    public IntPtr Hwnd;
    int ownerThread;
    Rectangle placed;
    bool ready;
    public readonly Stack<IntPtr[]> PoolA = new Stack<IntPtr[]>(), PoolI = new Stack<IntPtr[]>();

    public RingLayer()
    {
        FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true; StartPosition = FormStartPosition.Manual;
        Text = "ll-slide-rings";
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
        Text = "ll-slide";
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
        // Sonradan açılan en üstteki pencerelerin (tacky-borders, Zebar) üstüne çık. Sahibi olmayan thread'den eşzamansız:
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

// Animasyon katmanı gerçek pencereleri örttüğü için tacky-borders'ın kenarlığı animasyon boyunca görünmüyordu; odaklı
// pencerenin kenarlığını katmanın içinde biz çiziyoruz. Önceden bu, her karede yeniden boyutlanan ayrı bir pencereydi
// (SetWindowPos + SetWindowRgn): büyük pencerede kare başına 10-50 ms (15-30 fps) tutuyordu ve animasyon döngüsü boyamaya
// izin vermediği için yeni açılan alan beyaz/siyah yanıp sönüyordu. Şimdi kenarlık, ekran dışında duran küçük bir şablon
// pencereden (kenar yumuşatmalı halka resmi) alınan DWM önizlemeleriyle çizilir: 4 köşe sabit boyutta, 4 kenar tek
// piksellik şeritten gerilir (9 dilim). Kare başına yalnızca önizleme dikdörtgenleri güncellenir (~0,01 ms/çağrı) ve
// kenarlık pencerelerle AYNI DWM karesinde hareket eder.
static class TackyStyle
{
    public static Color Active = Color.FromArgb(0xcc, 0xb6, 0x9d, 0xf8), Inactive = Color.FromArgb(0x99, 0x3a, 0x3a, 0x40);
    public static int Width = 2, Radius = 14;
    static TackyStyle()
    {
        try
        {
            string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            // Kenarlıkları GlazeWM çiziyor: ayarları config.yaml'daki borders: bölümünde (eski kurulumda tacky-borders ayarı)
            string wmCfg = System.IO.Path.Combine(home, @".glzr\glazewm\config.yaml"), tackyCfg = System.IO.Path.Combine(home, @".config\tacky-borders\config.yaml");
            string cfg = System.IO.File.Exists(wmCfg) && System.IO.File.ReadAllText(wmCfg).Contains("borders:") ? System.IO.File.ReadAllText(wmCfg) : System.IO.File.ReadAllText(tackyCfg);
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
        Text = "ll-ring-src";
        Bw = bw; C = radius + bw + 1; S = 2 * C + 9; M = S / 2;
        Bounds = new Rectangle(X, Y, S, S);
        CreateControl(); Hwnd = Handle;
        Show();
        using (var bmp = new Bitmap(S, S, System.Drawing.Imaging.PixelFormat.Format32bppArgb))
        {
            using (var g = Graphics.FromImage(bmp))
            {
                g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
                g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
                g.Clear(Color.Transparent);
                float o = bw / 2f;
                using (var pen = new Pen(color, bw))
                    g.DrawPath(pen, new GraphicsPathHelper(new RectangleF(o, o, S - bw, S - bw), Math.Max(1f, radius - o)).Path);
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
    static double RefreshPeriodMs()
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
        return sb.ToString();
    }
}

class Slider
{
    [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point pt, uint flags);
    [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr hmon, int type, out uint dx, out uint dy);
    // Zebar'ın bar'ı 40 CSS px: DPI ölçeği %125/%150 olan monitörde 50/60 fiziksel piksel. Katmanlar bar'ın altından
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
    const int DURATION_MS = 520;       // Hyprland workspaces speed 7 (~700ms), menu_decel kuyruğu kısaltıldı
    const int GAP = 50;                // Hyprland general.gaps_workspaces = 50
    const int MAX_WS = 30;             // GlazeWM config'deki workspace sayısı (next/prev sarması için)

    readonly Glaze glaze;
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
    // Animasyon sürerken başka işler (dwindle yön komutu) GlazeWM'i meşgul etmesin
    public static volatile bool Animating;
    // Hyprland'de art arda basışta animasyon akmaya devam eder: bir önceki geçişten bu yana geçen süre
    // tam süreden kısaysa yeni animasyonu o kadar kısalt (en az MIN_MS). Tek basış tam uzunlukta kalır.
    static long lastSlideStart, lastMoveStart;
    const int MIN_MS = 180;
    static int Adaptive(ref long last, int full)
    {
        long now = Environment.TickCount;
        long since = now - last;
        last = now;
        return since > 0 && since < full ? (int)Math.Max(MIN_MS, since) : full;
    }

    // Ana helper açılışta: katman hazır ve gizli beklesin, ilk animasyon da bekletmesin
    public void Warm()
    {
        lock (overlays)
            foreach (var sc in Screen.AllScreens)
            {
                var b = sc.Bounds;
                int barH = BarPx(b.X + b.Width / 2, b.Y + b.Height / 2);
                var r = new Rectangle(b.X, b.Y + barH, b.Width, b.Height - barH);
                if (overlays.ContainsKey(r) || overlays.Count >= 8) continue;
                var o = overlays.Count == 0 ? spare : new Overlay();
                o.Prepare(r);
                FillRingPools(o.Rings);
                overlays[r] = o;
            }
    }

    static RingTemplate ringSrc, ringSrcInactive;
    public Slider(Glaze g)
    {
        glaze = g; spare = overlay;
        if (ringSrc == null)
        {
            try
            {
                ringSrc = new RingTemplate(TackyStyle.Active, TackyStyle.Width, TackyStyle.Radius);
                if (TackyStyle.Inactive.A > 0) ringSrcInactive = new RingTemplate(TackyStyle.Inactive, TackyStyle.Width, TackyStyle.Radius);
            }
            catch (Exception ex) { Log("ring: " + ex.Message); }
        }
    }

    // Kenarlıklar (tacky-borders'ınkiler katmanın altında kalır): odaklı pencereye etkin, diğerlerine pasif renkte, her biri
    // şablondan 8 DWM önizlemesi. Kenarlık katmanındaki havuzdan alınır (önceden, katman gizliyken kaydedilmiş); havuz
    // boşsa o an kaydedilir. Animasyon bitince takımlar gizlenip havuza döner.
    readonly List<Thumb> ringed = new List<Thumb>();
    const int POOL_I = 12, POOL_A = 2;
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
    static void FillRingPools(RingLayer layer)
    {
        if (ringSrc == null || layer.Hwnd == IntPtr.Zero) return;
        for (int k = 0; k < 2; k++)
        {
            bool active = k == 0;
            var pool = active ? layer.PoolA : layer.PoolI;
            int want = active ? POOL_A : POOL_I;
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
        foreach (var t in ts) if (t != null && t.IsWin) RingAdd(t, t.Src == focused);
    }
    // Odak değişti: etkin/pasif takımı değiştir (eskisi gizlenir, yenisi bir sonraki RingPlace'te yerleşir)
    void RingsFocus(IntPtr focused)
    {
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
    // winRect: pencere dikdörtgeni (katman koordinatı). Halka görünen çerçevenin kenarına ortalanır (tacky-borders gibi).
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
        FillRingPools(overlay.Rings); // sonraki animasyon için (katman gizliyken)
    }

    static Native.RECT Unshift(Native.RECT r, int ox, int oy)
    {
        return new Native.RECT { Left = r.Left + ox, Top = r.Top + oy, Right = r.Right + ox, Bottom = r.Bottom + oy };
    }

    static IntPtr FocusedTop() { return Native.GetAncestor(Native.GetForegroundWindow(), 2); }

    // Hyprland bezier "menu_decel" = (0.1, 1), (0, 1)
    static double Bezier(double x1, double y1, double x2, double y2, double t)
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
    static readonly string[] Pinned = { "Zebar - logical-lunge / osk", "Zebar - logical-lunge / sidebar-right", "Zebar - logical-lunge / toast" };
    // PiP gibi her workspace'te sabit duran, en üstte tutulan ve GlazeWM'in yönetmediği pencereler animasyon katmanının
    // altında kalıp geçiş boyunca kayboluyor, sonra "yapıştırılmış resim" gibi geri geliyordu. Canlı önizlemeleri kenarlık
    // katmanının en üstüne, kendi yerlerine konur: katman açıldığı karede görünürler, geçiş boyunca sabit kalırlar.
    readonly List<IntPtr> pinIds = new List<IntPtr>();
    static readonly Dictionary<uint, string> pinProcs = new Dictionary<uint, string>();
    static readonly HashSet<string> pinSkipProcs = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { "ll-helper", "tacky-borders", "zebar", "glazewm", "explorer", "ShellExperienceHost", "StartMenuExperienceHost", "SearchApp", "SearchUI", "TextInputHost", "LockApp" };
    void PinsAttach(Rectangle monArea, int ox, int oy, IEnumerable<Thumb> animated)
    {
        PinsClear();
        var skip = new HashSet<IntPtr>();
        if (animated != null) foreach (var t in animated) if (t != null) skip.Add(t.Src);
        var found = new List<KeyValuePair<IntPtr, Native.RECT>>();
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            if (skip.Contains(h) || !Native.IsWindowVisible(h) || Native.IsIconic(h)) return true;
            if ((Native.GetWindowLong(h, Native.GWL_EXSTYLE) & 0x8) == 0) return true; // WS_EX_TOPMOST
            int cl;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0) return true;
            Native.RECT r; Native.GetWindowRect(h, out r);
            if (r.Right - r.Left < 40 || r.Bottom - r.Top < 40) return true;
            if (!monArea.IntersectsWith(Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom))) return true;
            uint pid; Native.GetWindowThreadProcessId(h, out pid);
            string pn;
            lock (pinProcs)
            {
                if (!pinProcs.TryGetValue(pid, out pn))
                {
                    try { pn = Process.GetProcessById((int)pid).ProcessName; } catch { pn = ""; }
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

    // Görünen çerçeve (tacky-borders kenarlığı buna çizilir).
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
    // GlazeWM'in verdiği yerleşim dikdörtgeni (görünen çerçeve) -> pencere dikdörtgeni (gölge payları dahil)
    public static Native.RECT WindowRectForFrame(IntPtr h, Native.RECT frame) { return Inflate(frame, FrameInsets(h)); }
    static bool SameRect(Native.RECT a, Native.RECT b) { return a.Left == b.Left && a.Top == b.Top && a.Right == b.Right && a.Bottom == b.Bottom; }

    // GlazeWM pencereleri SetWindowPos ile (kısmen eşzamansız) taşır: dikdörtgenler iki ölçüm arka arkaya aynı
    // kalana kadar bekle (en fazla ~150 ms), sonra bitiş konumlarını oku.
    // Yeni pencere GlazeWM'in verdiği yere gerçekten oturana kadar bekle (görünen çerçeve, gölge kenarları hariç).
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
    // canlı görüntüleriyle göster. Arkasında GlazeWM ne yaparsa yapsın kullanıcı zıplama görmez. UI thread'inde.
    // Animasyon nesli: her dondurma / kaydırma başlangıcında artar. Arka plandaki önbellek tazelemesi sorgusu sürerken bir
    // animasyon başladıysa sonucu atar (yoksa kapanan pencerenin henüz silinmiş hali önbelleğe yazılıp animasyonu bozuyordu).
    public static int Gen;

    public Frozen Freeze(Rectangle mon, IEnumerable<long> handles, Dictionary<long, Native.RECT> startScreen, long hidden = 0)
    {
        Interrupt = false;
        Interlocked.Increment(ref Gen);
        var fz = Stopwatch.StartNew();
        var fzs = new StringBuilder();
        Action<string> step = n => { fzs.Append(n + " " + fz.Elapsed.TotalMilliseconds.ToString("0.0") + " "); };
        int barH = BarPx(mon.X + mon.Width / 2, mon.Y + mon.Height / 2);
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
        PinsAttach(new Rectangle(mon.X, oy, mon.Width, mon.Height - barH), ox, oy, f.Win.Values);
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
    // targetFrames: GlazeWM'in hesapladığı son yerleşim (görünen çerçeve, ekran koordinatı). Verilince animasyon pencerelerin
    // gerçekten yer değiştirmesini BEKLEMEDEN başlar (önceden 16-150 ms bekleniyordu); pencere yer değiştirdiği an hedef onun
    // gerçek yeridir (en küçük boyutu olan uygulama GlazeWM'in hesabından farklı yere oturabilir).
    class Anim { public Thumb T; public IntPtr H; public Native.RECT Start, End, Before; public bool Moved, Resizes; public int Cx0, Cy0; public long SrcAt = -1; }

    public void Finish(Frozen f, IEnumerable<long> endHandles, long popin, int durationMs, Dictionary<long, Native.RECT> targetFrames = null)
    {
        if (f.Ov != null) overlay = f.Ov;
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
                int hw2 = (int)((end.Right - end.Left) * 0.4), hh = (int)((end.Bottom - end.Top) * 0.4);
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
        var fs = new FrameStats();
        int frames = 0; long lastFrame = 0, maxGap = 0;
        while (!Interrupt)
        {
            fs.Begin();
            long nowMs = sw.ElapsedMilliseconds;
            if (frames > 0 && nowMs - lastFrame > maxGap) maxGap = nowMs - lastFrame;
            lastFrame = nowMs; frames++;
            double p = Math.Min(1.0, nowMs / (double)durationMs);
            double e = Bezier(0.05, 0.7, 0.1, 1, p); // Hyprland emphasizedDecel
            foreach (var a in items)
            {
                if (!a.Moved)
                {
                    var live = VisualDest(a.H, IntPtr.Zero, f.Ox, f.Oy);
                    if (!SameRect(live, a.Before)) { a.Moved = true; a.End = live; }
                }
                else a.End = VisualDest(a.H, IntPtr.Zero, f.Ox, f.Oy);
                var r = Lerp(a.Start, a.End, e);
                byte op = 255;
                if (a.T == pop)
                {
                    // Hyprland windowsIn "popin 80%": ölçekli büyüyerek ve belirerek
                    op = (byte)Math.Min(255, (int)(255 * Math.Min(1.0, p * 2.5)));
                    var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_OPACITY, rcDestination = r, opacity = op };
                    Native.DwmUpdateThumbnailProperties(a.T.Id, ref pr);
                }
                else
                {
                    PlaceVisible(a.T, r, a.Resizes);
                    if (a.Resizes && a.SrcAt < 0 && (a.T.Cx != a.Cx0 || a.T.Cy != a.Cy0)) a.SrcAt = nowMs;
                }
                RingPlace(a.T, r, op);
            }
            fs.Updated();
            Native.DwmFlush();
            fs.Flushed();
            if (p >= 1.0) break;
        }
        var sb = new StringBuilder();
        foreach (var a in items) if (a.Resizes && a.T != pop) sb.Append(" | içerik " + a.Cx0 + "x" + a.Cy0 + "->" + a.T.Cx + "x" + a.T.Cy + (a.SrcAt >= 0 ? " @" + a.SrcAt + "ms" : " (değişmedi)"));
        Log("anim: " + frames + " kare / " + sw.ElapsedMilliseconds + " ms, en uzun kare " + maxGap + " ms, " + items.Count + " pencere" + sb + " " + fs.Report() + " önizleme=" + Native.LiveThumbs);
        overlay.Conceal();
        RingsClear();
        PinsClear();
        foreach (var t in f.All) Native.DwmUnregisterThumbnail(t.Id);
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

    // commands: GlazeWM'e gönderilecekler. dirHint: +1 sağ, -1 sol, 0 = isimden hesapla.
    // ---- Pencere aç/kapa animasyonu (Hyprland windowsMove: speed 3 ≈ 300ms emphasizedDecel,
    // windowsIn: popin 80%). GlazeWM pencereleri anında yerleştirir; biz eski yerleşimden yenisine
    // canlı DWM önizlemelerini kaydırıp ölçekleyerek geçiş yapıyoruz, sonra gerçek pencereler görünür.
    const int MOVE_MS = 300;
    public static int MoveMs { get { return MOVE_MS; } }

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

    // Hyprland dwindle yeni pencereyi odaktakine değil FARENİN ALTINDAKİ pencereye açar. GlazeWM
    // hep odaktakinin yanına koyduğu için: terminal açmadan hemen önce fare altındakini odakla.
    public void FocusUnderCursor()
    {
        var p = Cursor.Position;
        IntPtr under = Native.WindowFromPoint(p);
        long handle = under == IntPtr.Zero ? 0 : Native.GetAncestor(under, 2).ToInt64();
        var mons = glaze.Monitors();
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
                        if (!J.Bool(w, "hasFocus")) glaze.Command("focus --container-id " + J.Str(w, "id"));
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
                    glaze.Command("focus --workspace " + J.Str(ws, "name"));
            return;
        }
    }

    // Super+ok (focus) / Super+Shift+ok (move): yalnızca AYNI workspace içinde. GlazeWM'in
    // "--direction" komutları o yönde pencere yoksa yan monitöre/workspace'e atlıyordu.
    // Sonunda fare hedef pencerenin ortasına taşınır (Hyprland'de odak değişince imleç de gider).
    public void Commands(string[] cmds) { foreach (var c in cmds) glaze.Command(c); }

    public void FocusInWorkspace(string dir) { InWorkspace(dir, false); }
    public void MoveInWorkspace(string dir) { InWorkspace(dir, true); }

    static void WarpTo(Dictionary<string, object> w)
    {
        if (w == null) return;
        Cursor.Position = new Point(J.Int(w, "x") + J.Int(w, "width") / 2, J.Int(w, "y") + J.Int(w, "height") / 2);
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
        mon = FocusedMonitor(glaze.Monitors(), out ws);
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

    // Öz-test (ll-helper.exe --anim-selftest): odaklı monitörü dondurup pencereleri AYNI yerlerine "animasyonla"
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
        Finish(f, hs, 0, MOVE_MS, targets);
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
            glaze.Command("focus --container-id " + J.Str(best, "id"));
            WarpTo(best);
            return;
        }

        string id = J.Str(cur, "id");
        if (best == null)
        {
            // O yönde komşu yok: GlazeWM (fork) pencereyi o kenara çıkarır; pencere ekranın o yarısını alır, geri kalan
            // düzen öbür yarıda şeklini korur (Hyprland dwindle movetoroot). Örn. 2x2'de sağ üstteki sağa -> sağda boydan,
            // solda [sol üst / sol alt] sütunu ile eski sağ alttaki yan yana. Tek durum hariç: pencere doğrudan
            // workspace'in elemanıysa ve workspace zaten o eksendeyse GlazeWM pencereyi diğer monitörün
            // workspace'ine atıyordu; orada hiçbir şey yapma. Tek pencerede de.
            var par0 = ParentOf(ws, J.Str(cur, "id"));
            string axis = dir == "left" || dir == "right" ? "horizontal" : "vertical";
            if (par0 == null || wins.Count < 2) return;
            if (J.Str(par0, "type") == "workspace" && J.Str(par0, "tilingDirection") == axis) return;
        }
        // Önce görüntüyü dondur (pencereler şu an nerede görünüyorsa orada), GlazeWM arkada yerleştirsin
        var monRect = new Rectangle(J.Int(mon, "x"), J.Int(mon, "y"), J.Int(mon, "width"), J.Int(mon, "height"));
        var hs = new List<long>(Rects(wins).Keys);
        Frozen frozen = null;
        if (Ui != null)
        {
            Interrupt = true;
            try { frozen = (Frozen)Ui.Invoke((Func<Frozen>)(() => Freeze(monRect, hs, null))); } catch (Exception ex) { Log("freeze: " + ex.Message); }
        }
        long frozenAt = clk.ElapsedMilliseconds;
        // GlazeWM (fork) Hyprland dwindle movewindow yapar: o yönde pencere varsa onu uzun kenarından böler; yoksa
        // bölme yönü değişir (yan yana iki pencerede Super+Shift+Yukarı -> odaktaki üstte tam genişlik).
        glaze.Command("move --direction " + dir);

        Dictionary<string, object> mA, wsA, curA; List<Dictionary<string, object>> winsA;
        bool ok = Current(out mA, out wsA, out winsA, out curA);
        var targets = ok ? Rects(winsA) : null;
        Log("taşı " + dir + ": donma " + frozenAt + " ms, GlazeWM hazır " + clk.ElapsedMilliseconds + " ms");
        var endHs = ok ? new List<long>(targets.Keys) : hs;
        // Pencerelerin yerleşmesi beklenmez: hedef GlazeWM'in hesabı, yer değişince gerçek yer (Finish)
        if (ok)
        {
            Dictionary<string, object> moved = null;
            foreach (var w in winsA) if (J.Str(w, "id") == id) moved = w;
            WarpTo(moved);
        }
        if (frozen != null)
        {
            int dur = Adaptive(ref lastMoveStart, MOVE_MS);
            Ui.BeginInvoke((Action)(() =>
            {
                try { Finish(frozen, endHs, 0, dur, targets); } catch (Exception ex) { Log("move anim: " + ex.Message); }
            }));
        }
    }
    public static void Log(string s)
    {
        try
        {
            string path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-helper.log");
            // 4 MB'yi geçince eskisi .old olur (animasyon başına satır yazılıyor; sınırsız büyümesin)
            var fi = new System.IO.FileInfo(path);
            if (fi.Exists && fi.Length > 4 * 1024 * 1024)
            {
                try { System.IO.File.Delete(path + ".old"); System.IO.File.Move(path, path + ".old"); } catch { }
            }
            // Aynı anda birden çok thread / süreç yazabiliyor (ör. iki HTTP isteği): paylaşımlı aç, kısa yeniden dene;
            // yoksa satırlar sessizce kayboluyordu
            var bytes = Encoding.UTF8.GetBytes(DateTime.Now.ToString("HH:mm:ss.fff ") + s + Environment.NewLine);
            lock (logLock)
                for (int i = 0; i < 5; i++)
                {
                    try
                    {
                        using (var fs = new System.IO.FileStream(path, System.IO.FileMode.Append, System.IO.FileAccess.Write, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
                            fs.Write(bytes, 0, bytes.Length);
                        break;
                    }
                    catch (System.IO.IOException) { Thread.Sleep(3); }
                }
        }
        catch { }
    }
    static readonly object logLock = new object();

    public void Run(string[] commands, int dirHint, string targetName)
    {
        var clock = Stopwatch.StartNew();
        Interrupt = false;
        var mons = glaze.Monitors();
        Dictionary<string, object> oldWs;
        var mon = FocusedMonitor(mons, out oldWs);
        Log("query " + clock.ElapsedMilliseconds + "ms monitors=" + mons.Count + " focusedMon=" + (mon != null));
        if (mon == null || oldWs == null) { foreach (var c in commands) glaze.Command(c); return; }

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
                if (lastCmd == "focus --next-workspace") otherTarget = (cur0 % MAX_WS + 1).ToString();
                else if (lastCmd == "focus --prev-workspace") otherTarget = ((cur0 + MAX_WS - 2) % MAX_WS + 1).ToString();
            }
        }
        if (otherTarget != null)
            foreach (var m in mons)
                if (J.Str(m, "id") != J.Str(mon, "id"))
                    foreach (Dictionary<string, object> w in J.Children(m))
                        if (J.Str(w, "name") == otherTarget && J.Bool(w, "isDisplayed"))
                        {
                            foreach (var c in commands) glaze.Command(c);
                            // Odaklanan pencerenin (yoksa monitörün) ortasına imleci götür
                            var tw = new List<Dictionary<string, object>>();
                            J.WindowNodes(w, tw);
                            Dictionary<string, object> fw = null;
                            foreach (var x in tw) if (J.Bool(x, "hasFocus")) fw = x;
                            if (fw == null && tw.Count > 0) fw = tw[0];
                            if (fw != null) WarpTo(fw);
                            else Cursor.Position = new Point(J.Int(m, "x") + J.Int(m, "width") / 2, J.Int(m, "y") + J.Int(m, "height") / 2);
                            Log("slide: hedef diğer monitörde, animasyonsuz");
                            return;
                        }

        int mx = J.Int(mon, "x"), my = J.Int(mon, "y"), mw = J.Int(mon, "width"), mh = J.Int(mon, "height");
        int barH = BarPx(mx + mw / 2, my + mh / 2);
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
        // GlazeWM komutunu BEKLEMEDEN başlar. GlazeWM'in geçişi pencere sayısıyla 100-200 ms sürüyor
        // ve animasyon ondan sonra başladığı için her geçişte önce donma hissi oluyordu. Gizli
        // workspace'in pencereleri konumlarını koruduğu için önizlemeleri komuttan önce hazırlanabilir.
        string predicted = targetName;
        string focusCmd = moveFollow ? commands[1] : commands[0];
        if (predicted == null && (commands.Length == 1 || moveFollow))
        {
            int cur;
            if (int.TryParse(oldName, out cur))
            {
                if (focusCmd == "focus --next-workspace") predicted = (cur % MAX_WS + 1).ToString();
                else if (focusCmd == "focus --prev-workspace") predicted = ((cur + MAX_WS - 2) % MAX_WS + 1).ToString();
            }
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
            RingsAttach(thumbs, carried != null ? carried.Src : FocusedTop());
            foreach (var t in oldThumbs) RingPlace(t, t.Dest, 255);
            foreach (var t in newThumbs) { var r0 = t.Dest; r0.Left += fdir * (mw + GAP); r0.Right += fdir * (mw + GAP); RingPlace(t, r0, 255); }
            if (carried != null) RingPlace(carried, carried.Dest, 255);
            PinsAttach(new Rectangle(mx, my + barH, mw, mh - barH), ox, oy, thumbs);
            overlay.Reveal();
            RaisePinned();
            Native.DwmFlush();
            Log("fast shown " + clock.ElapsedMilliseconds + "ms (kayıt " + regMs + "ms) new=" + newThumbs.Count);

            var cmdsAll = (string[])commands.Clone();
            var task = Task.Factory.StartNew(() => { foreach (var cm in cmdsAll) glaze.Command(cm); });
            int dur0 = Adaptive(ref lastSlideStart, moveFollow ? 340 : DURATION_MS); // taşıma daha kısa: pencere beklemeden yerine geçsin
            Animating = true;

            // Taşı+takip et: GlazeWM komutu bittiği an (genelde kaymanın ilk ~50 ms'i) hedef workspace'teki pencereler
            // ve taşınan pencere yeni yerlerine doğru kaymayla AYNI ANDA ve esnemeden ilerler; ayrı bir "yerleşme"
            // adımı yok (Hyprland'de de pencere kayarken boyutlanır). Hedef her karede canlı okunur: GlazeWM pencereyi
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
            var fs = new FrameStats();
            culledCount = 0;
            while (!Interrupt)
            {
                fs.Begin();
                long nowMs0 = sw0.ElapsedMilliseconds;
                if (mfFrames > 0 && nowMs0 - mfLast > mfMax) mfMax = nowMs0 - mfLast;
                mfLast = nowMs0; mfFrames++;
                if (cmdDoneAt < 0 && task.IsCompleted) cmdDoneAt = nowMs0;
                if (movedAt < 0 && moveFollow && WindowsMoved()) movedAt = nowMs0;
                double p = Math.Min(1.0, sw0.ElapsedMilliseconds / (double)dur0);
                double e = Bezier(0.1, 1, 0, 1, p);
                int shift = (int)Math.Round(e * (mw + GAP));
                if (moveFollow && swR == null && (task.IsCompleted || WindowsMoved()))
                {
                    // Pencere yeni boyutuna geçti: yerleşme kayma ile BİRLİKTE, en az 300 ms'lik yumuşak bir geçişle
                    swR = Stopwatch.StartNew();
                    durR = Math.Max(200, (int)(dur0 - sw0.ElapsedMilliseconds));
                }
                double pR = swR == null ? 0 : Math.Min(1.0, swR.ElapsedMilliseconds / (double)durR);
                double eR = Bezier(0.05, 0.7, 0.1, 1, pR);
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
                Native.DwmFlush();
                fs.Flushed();
                if (p >= 1.0 && (!moveFollow || pR >= 1.0)) break;
                if (p >= 1.0 && swR == null && sw0.ElapsedMilliseconds > dur0 + 1500) break; // komut takıldı
            }
            long animEnd = clock.ElapsedMilliseconds;
            Log("slide" + (moveFollow ? "+taşı" : "") + ": " + mfFrames + " kare, en uzun kare " + mfMax + " ms, komut bitti " + cmdDoneAt + " ms, pencere yer değiştirdi " + movedAt + " ms " + fs.Report() + " önizleme=" + Native.LiveThumbs + " gizlenen=" + culledCount);
            // Katmanı GlazeWM'in yanıtını değil GERÇEK durumu bekleyerek kaldır: eski workspace'in pencereleri gizlenip
            // (cloak) yenininkiler göründüğü an. GlazeWM bazen pencereleri gösterdikten ~250 ms sonra yanıt veriyordu
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
            foreach (var t in thumbs) Native.DwmUnregisterThumbnail(t.Id);
            Animating = false;
            Log("fast done " + clock.ElapsedMilliseconds + "ms (animasyon " + dur0 + "ms, bitti " + animEnd + "ms, " + (viaState ? "pencereler hazır" : "komut " + (task.IsCompleted ? "bitti" : "sürüyor")) + ")");
            return;
        }

        RingsAttach(oldThumbs, FocusedTop());
        foreach (var t in oldThumbs) RingPlace(t, t.Dest, 255);
        PinsAttach(new Rectangle(mx, my + barH, mw, mh - barH), ox, oy, thumbs);
        overlay.Reveal();
        RaisePinned();
        Native.DwmFlush();
        Log("shown " + clock.ElapsedMilliseconds + "ms");

        foreach (var c in commands) glaze.Command(c);
        Log("commanded " + clock.ElapsedMilliseconds + "ms");

        // GlazeWM komuta hemen "tamam" der ama workspace'i birkaç ms sonra değiştirir:
        // gösterilen workspace değişene kadar kısa aralıklarla tekrar sor (en fazla ~250ms).
        Dictionary<string, object> newWs = null;
        for (int tries = 0; tries < 25; tries++)
        {
            mons = glaze.Monitors();
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
            Native.DwmFlush();

            var sw = Stopwatch.StartNew();
            while (!Interrupt)
            {
                double p = Math.Min(1.0, sw.ElapsedMilliseconds / (double)DURATION_MS);
                double e = Bezier(0.1, 1, 0, 1, p);
                int shift = (int)Math.Round(e * (mw + GAP));
                foreach (var t in oldThumbs) Move(t, -dir * shift);
                foreach (var t in newThumbs) Move(t, dir * (mw + GAP) - dir * shift);
                Native.DwmFlush();
                if (p >= 1.0) break;
            }
        }

        overlay.Conceal(); RingsClear(); PinsClear();
        foreach (var t in thumbs) Native.DwmUnregisterThumbnail(t.Id);
        Log("done " + clock.ElapsedMilliseconds + "ms new=" + newThumbs.Count);
    }
}

// ---------------- Dwindle (Hyprland varsayılan layout'u) ----------------
// Hyprland dwindle: yeni pencere, odaktaki pencereyi UZUN kenarı boyunca ikiye böler
// (geniş -> yan yana, uzun -> alt alta) ve içe dönen bir spiral oluşur. GlazeWM'de bu layout
// yok; odak her değiştiğinde odaktaki pencerenin en/boy oranına göre tiling yönünü ayarlıyoruz,
// böylece bir sonraki pencere dwindle'daki gibi yerleşiyor.
class Dwindle
{
    readonly Glaze glaze;
    readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    public Dwindle(Glaze g) { glaze = g; }

    // ---- Aç/kapa animasyonu için yerleşim hafızası ----
    Control ui; Slider slider;
    int animSeq;
    readonly Glaze cacheGlaze = new Glaze();
    readonly object cacheLock = new object();
    Dictionary<long, Native.RECT> rects = new Dictionary<long, Native.RECT>();   // görünen pencereler
    Dictionary<long, string> monOf = new Dictionary<long, string>();              // pencere -> monitör id
    Dictionary<string, Rectangle> monRects = new Dictionary<string, Rectangle>();

    public Dwindle(Glaze g, Control ui, Slider slider) : this(g)
    {
        this.ui = ui; this.slider = slider;
        // Klavyeyle yeniden boyutlandırma vb. için hafızayı düzenli tazele
        var t = new Thread(() => { while (true) { Thread.Sleep(300); if (Slider.Animating) continue; try { RefreshCache(); } catch { } } }) { IsBackground = true };
        t.Start();
    }

    HashSet<long> tiledSet = new HashSet<long>(); // görünen workspace'lerdeki döşeli pencereler (kapanma ön-dondurması için)

    void Snapshot(out Dictionary<long, Native.RECT> r, out Dictionary<long, string> m, out Dictionary<string, Rectangle> mr, int gen = -1)
    {
        r = new Dictionary<long, Native.RECT>(); m = new Dictionary<long, string>(); mr = new Dictionary<string, Rectangle>();
        var tiled = new HashSet<long>();
        foreach (var mon in cacheGlaze.Monitors())
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
            if (gen != Slider.Gen || Slider.Animating) return; // sorgu sürerken animasyon başladı: bu sonuç eski
            rects = r; monOf = m; monRects = mr; visual = v;
        }
    }

    // ---- Yeni pencere: Windows onu önce kendi varsayılan yerinde (ortada) gösterir, GlazeWM birkaç on ms sonra
    // yerleştirir. Hyprland pencereyi son yerini alana kadar hiç göstermez: burada da pencere görünür olduğu an
    // (EVENT_OBJECT_SHOW) ekranı mevcut pencerelerle donduruyoruz; ortadaki pencere katmanın altında kalır,
    // GlazeWM yer açınca son yerinde %80'den büyüyüp belirir. Yönetilmezse (açılış ekranı vb.) 0.9 sn'de kalkar.
    Native.WinEventDelegate showCb;
    readonly object pendLock = new object();
    Slider.Frozen pendFrozen;
    long pendHandle;
    int pendAt;
    static readonly HashSet<string> noFreezeProcs = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { "zebar", "ll-helper", "tacky-borders", "glazewm", "ShellExperienceHost", "SearchUI", "SearchApp", "StartMenuExperienceHost",
          "LockApp", "TextInputHost", "ApplicationFrameHost", "msedgewebview2", "ll-songrec", "ll-termcolors" };

    public void HookNewWindows()
    {
        if (ui == null) return;
        ui.BeginInvoke((Action)(() =>
        {
            showCb = OnWinEvent;
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
            if (Slider.Animating) return;
            lock (cacheLock) if (rects.ContainsKey(h)) return;               // zaten yönetilen pencere (workspace dönüşü vb.)
            lock (pendLock) if (pendFrozen != null) return;
            int style = Native.GetWindowLong(hwnd, Native.GWL_STYLE), ex = Native.GetWindowLong(hwnd, Native.GWL_EXSTYLE);
            // Yalnızca döşenecek türden uygulama pencereleri (AutoFloat'ın yüzdürmeyeceği)
            if ((style & Native.WS_CAPTION) != Native.WS_CAPTION || (style & 0x00040000) == 0) return;
            if ((ex & Native.WS_EX_TOOLWINDOW) != 0 || (ex & Native.WS_EX_NOACTIVATE) != 0) return;
            if (Native.GetWindow(hwnd, 4) != IntPtr.Zero) return; // sahibi olan (diyalog)
            uint pid; Native.GetWindowThreadProcessId(hwnd, out pid);
            string proc;
            try { proc = Process.GetProcessById((int)pid).ProcessName; } catch { return; }
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
        if (ev == Native.EVENT_OBJECT_SHOW) { OnWindowShown(hook, ev, hwnd, idObject, idChild, thread, time); return; }
        if (idObject != 0 || idChild != 0 || hwnd == IntPtr.Zero) return;
        try { OnWindowGone(hwnd); } catch (Exception ex) { Slider.Log("gone hook: " + ex.Message); }
    }

    // ---- Pencere kapanıyor / gizleniyor: Windows'un olayı GlazeWM'in bildiriminden ~30-40 ms önce gelir; o arada GlazeWM
    // kalan pencereleri yeniden yerleştirdiği için pencereler animasyon başlamadan zıplıyordu. Ekranı hemen, pencerelerin
    // görüldükleri yerlerde donduruyoruz; bildirim gelince AnimateChange bu katmanı alıp kaydırır. Gelmezse 0,4 sn'de kalkar.
    void OnWindowGone(IntPtr hwnd)
    {
        long h = hwnd.ToInt64();
        if (Slider.Animating) return;
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
                slider.Finish(left, now, 0, 120); // GlazeWM bildirmedi: katmanı yumuşakça kaldır
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

    // Pencere açıldı/kapandı. GlazeWM yerleşimi zaten değiştirdi; katmanı hemen, pencerelerin GÖRÜLDÜKLERİ eski
    // yerlerinden (önbellek) açıp arkada fareye göre yerleştirmeyi de yapıyoruz, sonra hepsi gerçek yerine kayar.
    // (Eskiden: önce zıplama, 60 ms sonra katman, sonra fareye göre ikinci zıplama ve 400 ms sonra üçüncüsü.)
    void AnimateChange(long anchorHandle, bool opened, Dictionary<string, object> win)
    {
        var clk = Stopwatch.StartNew();
        if (ui == null) return;
        Dictionary<long, Native.RECT> beforeVis; Dictionary<long, string> beforeMon;
        lock (cacheLock) { beforeVis = visual; beforeMon = monOf; }

        Dictionary<long, Native.RECT> after = null; Dictionary<long, string> afterMon = null; Dictionary<string, Rectangle> mr = null;
        string mid = null;
        Rectangle mon = Rectangle.Empty;
        var hs = new List<long>();
        var start = new Dictionary<long, Native.RECT>();
        long pop = opened ? anchorHandle : 0;

        // Açılışta SHOW, kapanışta HIDE/DESTROY anında dondurulmuş olabilir. Kapanış dondurulduysa ilk GlazeWM sorgusu
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
            // Kapanma kancası bu arada dondurmuş olabilir (ikisi de UI thread'inde sırayla çalışır): iki katman olmasın
            try { f = (Slider.Frozen)ui.Invoke((Func<Slider.Frozen>)(() => TakePending(anchorHandle) ?? slider.Freeze(mon, hs, start, pop))); }
            catch (Exception ex) { Slider.Log("freeze: " + ex.Message); }
        }


        Snapshot(out after, out afterMon, out mr);
        var end = new List<long>();
        foreach (var kv in afterMon) if (kv.Value == mid) end.Add(kv.Key);
        // Pencerelerin yerleşmesi beklenmez (önceden 16-500 ms): hedef GlazeWM'in yerleşimi, pencere yer değiştirdiği
        // an gerçek yeri (Finish). Önbellekteki görünür dikdörtgenler de hedef yerleşimden hesaplanır.
        var v = new Dictionary<long, Native.RECT>();
        foreach (var kv in after) v[kv.Key] = Slider.WindowRectForFrame(new IntPtr(kv.Key), kv.Value);
        lock (cacheLock) { rects = after; monOf = afterMon; monRects = mr; visual = v; }
        Slider.Log((opened ? "açıldı" : "kapandı") + ": " + start.Count + "->" + end.Count + " pencere, donma+hedef " + clk.ElapsedMilliseconds + " ms" + (f == null ? " (katman yok)" : ""));
        if (opened && end.Contains(anchorHandle))
        {
            Native.RECT nr;
            IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
            if (fg.ToInt64() == anchorHandle && after.TryGetValue(anchorHandle, out nr))
                Cursor.Position = new Point((nr.Left + nr.Right) / 2, (nr.Top + nr.Bottom) / 2);
        }
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
    // Yüzen/tam ekran pencereler GlazeWM ayarıyla her zaman üstte.
    bool AutoFloat(Dictionary<string, object> win)
    {
        object hv;
        if (!win.TryGetValue("handle", out hv) || hv == null) return false;
        IntPtr h = new IntPtr(Convert.ToInt64(hv));
        object st;
        var state = win.TryGetValue("state", out st) ? st as Dictionary<string, object> : null;
        if (state != null && J.Str(state, "type") != "tiling") return false; // GlazeWM zaten yüzdürmüş

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
        if (!caption && !thick && coversMonitor) cmd = "set-fullscreen";
        else if (owned || !thick || (popup && !caption)) cmd = "set-floating --centered";
        if (cmd == null) return false;
        glaze.Command("--id " + id + " " + cmd);
        Slider.Log("auto " + cmd + ": " + J.Str(win, "processName") + " | " + J.Str(win, "title"));
        return true;
    }

    // ---- Odak geçmişi (Hyprland gibi): odaklı pencere kapanınca aynı workspace'te en son
    // odaklanan pencereye dön. GlazeWM ağaçtaki komşuyu odaklıyordu; art arda Alt+F4'te
    // sıra karışıyor, ilk açılan pencere en sona kalmıyordu.
    const int AUTO_MS = 250; // kapanıştan hemen önce/sonra gelen odak, GlazeWM'in otomatik seçimidir
    readonly List<string> mru = new List<string>();
    readonly List<KeyValuePair<string, long>> pending = new List<KeyValuePair<string, long>>();

    void CommitOld(long now)
    {
        var keep = new List<KeyValuePair<string, long>>();
        foreach (var p in pending)
        {
            if (now - p.Value > AUTO_MS) { mru.Remove(p.Key); mru.Insert(0, p.Key); }
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
        // Kapanışın hemen öncesi/sonrasındaki odaklar GlazeWM'in otomatik seçimi: geçmişe yazma
        pending.Clear();
        mru.Remove(id);
        if (!wasFocused) return;

        // Hedef: odaktaki workspace'te en son odaklanmış, hâlâ açık pencere
        var ids = new HashSet<string>();
        string focusedNow = null;
        foreach (var m in glaze.Monitors())
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
            if (candidate != focusedNow) glaze.Command("focus --container-id " + candidate);
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
            try
            {
                var ws = new ClientWebSocket();
                ws.Options.Proxy = null;
                ws.ConnectAsync(new Uri("ws://127.0.0.1:6123"), CancellationToken.None).Wait(3000);
                var sub = Encoding.UTF8.GetBytes("sub --events focus_changed window_managed window_unmanaged");
                ws.SendAsync(new ArraySegment<byte>(sub), WebSocketMessageType.Text, true, CancellationToken.None).Wait(1500);
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
            Thread.Sleep(2000); // GlazeWM yeniden başlarsa tekrar bağlan
        }
    }

    void Handle(string text)
    {
        var msg = json.DeserializeObject(text) as Dictionary<string, object>;
        if (msg == null || J.Str(msg, "messageType") != "event_subscription") return;
        var data = msg["data"] as Dictionary<string, object>;
        if (data == null) return;
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
            if (AutoFloat(win)) { LaunchQueue.Managed.Set(); return; }
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
class MouseFocus
{
    readonly Glaze glaze;
    readonly AutoResetEvent moved = new AutoResetEvent(false);
    Native.LowLevelMouseProc proc;
    IntPtr hookHandle;
    volatile int lastX = int.MinValue, lastY = int.MinValue;

    public MouseFocus(Glaze g) { glaze = g; }

    // Kanca, klavye kancasıyla aynı (başka iş yapmayan) thread'de kurulur; burada sadece sinyal verilir.
    public void InstallHook()
    {
        proc = Hook;
        hookHandle = Native.SetWindowsHookEx(Native.WH_MOUSE_LL, proc, Native.GetModuleHandle(null), 0);
    }

    public void Reinstall()
    {
        IntPtr fresh = Native.SetWindowsHookEx(Native.WH_MOUSE_LL, proc, Native.GetModuleHandle(null), 0);
        if (fresh == IntPtr.Zero) return;
        IntPtr old = hookHandle; hookHandle = fresh;
        if (old != IntPtr.Zero) Native.UnhookWindowsHookEx(old);
    }

    IntPtr Hook(int nCode, IntPtr wParam, IntPtr lParam)
    {
        if (nCode >= 0 && wParam.ToInt32() == 0x200) // WM_MOUSEMOVE
        {
            var m = (Native.MSLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(Native.MSLLHOOKSTRUCT));
            if ((m.flags & 1) == 0 && (m.pt.X != lastX || m.pt.Y != lastY)) // LLMHF_INJECTED değil
            {
                lastX = m.pt.X; lastY = m.pt.Y;
                moved.Set();
            }
        }
        return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
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
                // Öndeki pencere yüzen/diyalog (sahibi olan ya da her zaman üstte) ise fare hareketi
                // odağı ondan çalmasın: dosya seçme penceresi vb. fare gezdikçe arkaya düşüp gelmiyordu.
                if (fg != IntPtr.Zero && (Native.GetWindow(fg, 4) != IntPtr.Zero ||
                    (Native.GetWindowLong(fg, Native.GWL_EXSTYLE) & Native.WS_EX_TOPMOST) != 0)) continue;
                if (root == lastRoot) continue; // son bakılan yönetilmeyen pencere (bar, masaüstü...)

                // Yalnızca GlazeWM'in yönettiği ve odaktaki workspace'te görünen pencereler
                long handle = root.ToInt64();
                foreach (var m in glaze.Monitors())
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
                                if (!J.Bool(w, "hasFocus")) glaze.Command("focus --container-id " + J.Str(w, "id"));
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
// GlazeWM gizli workspace'lerin pencerelerini kabuğun "cloak" özelliğiyle gizler. GlazeWM çökerse ya da zorla kapatılırsa
// (güncelleme, kaldırma) bu pencereler görünmez kalıyordu. ll-helper.exe --uncloak-orphans: kabuğun gizlediği uygulama
// pencerelerini geri getirir (GlazeWM'in kendi kullandığı arayüzle). Askıya alınmış UWP pencerelerine dokunmaz.
static class Orphans
{
    [ComImport, Guid("6D5140C1-7436-11CE-8034-00AA006009FA"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IServiceProviderLL { [return: MarshalAs(UnmanagedType.IUnknown)] object QueryService(ref Guid service, ref Guid riid); }
    [ComImport, Guid("372E1D3B-38D3-42E4-A15B-8AB2B178F513"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IApplicationViewLL { void m1(); void m2(); void m3(); void m4(); void m5(); void m6(); void m7(); void m8(); void m9(); [PreserveSig] int SetCloak(uint type, int flag); }
    [ComImport, Guid("1841C6D7-4F9D-42C0-AF41-8747538F10E5"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IApplicationViewCollectionLL { void m1(); void m2(); void m3(); [PreserveSig] int GetViewForHwnd(IntPtr hwnd, out IApplicationViewLL view); }

    // listOnly: yalnızca adayları yazdır. GlazeWM çalışırken hiçbir şey yapılmaz: gizli workspace'lerin pencereleri
    // bilerek gizlidir, açılırlarsa ekrana dökülürlerdi.
    public static int Uncloak(bool listOnly = false)
    {
        if (!listOnly && Process.GetProcessesByName("glazewm").Length > 0) { Slider.Log("uncloak: GlazeWM çalışıyor, atlandı"); return -1; }
        var targets = new List<IntPtr>();
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            if (!Native.IsWindowVisible(h)) return true;
            int cl;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cl, 4) != 0 || cl != 2) return true; // 2 = kabuk gizlemiş
            var c = new StringBuilder(128); Native.GetClassName(h, c, 128);
            string cs = c.ToString();
            if (cs == "Windows.UI.Core.CoreWindow" || cs == "ApplicationFrameWindow") return true; // askıdaki UWP
            int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
            if ((ex & Native.WS_EX_TOOLWINDOW) != 0) return true;
            // Tıklamayı geçiren şeffaf katmanlar (ör. görev çubuğu oyunu TaskBarHero): GlazeWM bunları yönetmez; gizliyse öyle
            // kalsın (geri getirilince görev çubuğu gizliyken efektleri ekranın üstünde yüzüyordu)
            if ((ex & Native.WS_EX_TRANSPARENT) != 0 && (ex & 0x00080000) != 0) return true; // TRANSPARENT + LAYERED
            targets.Add(h);
            return true;
        }, IntPtr.Zero);
        if (listOnly)
        {
            foreach (var h in targets) { var t = new StringBuilder(120); Native.GetWindowText(h, t, 120); Console.WriteLine(h.ToInt64() + " " + t); }
            return targets.Count;
        }
        if (targets.Count == 0) return 0;
        int n = 0;
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

// ---------------- Zebar nöbetçisi ----------------
// Bar ve tüm paneller Zebar'da. Zebar hiç açılmazsa (ör. yeni kurulumda PATH) ya da açık olduğu halde widget sunucusu
// (127.0.0.1:6124) çalışmıyorsa (port o an önceki Zebar'da kaldıysa sunucusuz açılıyor, bar "bağlantı reddedildi"
// gösteriyordu) masaüstü yarım kalmasın: GlazeWM çalışıyorken iki ardışık kontrolde (~10 sn) sorun sürerse Zebar'ı temiz
// biçimde (port boşalana kadar bekleyip) yeniden başlat. Art arda başarısızlıkta beklemeyi uzatır.
static class ZebarWatchdog
{
    const int PORT = 6124;
    static string lastPath;

    static bool PortOpen()
    {
        try
        {
            using (var c = new System.Net.Sockets.TcpClient())
            {
                var ar = c.BeginConnect("127.0.0.1", PORT, null, null);
                bool ok = ar.AsyncWaitHandle.WaitOne(700) && c.Connected;
                try { c.EndConnect(ar); } catch { ok = false; }
                return ok;
            }
        }
        catch { return false; }
    }

    static List<Process> Zebars()
    {
        var l = new List<Process>(Process.GetProcessesByName("zebar"));
        foreach (var p in l) { try { lastPath = p.MainModule.FileName; } catch { } }
        return l;
    }

    static string ExePath()
    {
        if (lastPath != null && System.IO.File.Exists(lastPath)) return lastPath;
        string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        foreach (var exe in new[] {
            System.IO.Path.Combine(home, @".glzr\logical-lunge\bin\zebar.exe"),
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), @"glzr.io\Zebar\zebar.exe") })
            if (System.IO.File.Exists(exe)) return exe;
        return null;
    }

    static bool GlazeRunning()
    {
        var ps = Process.GetProcessesByName("glazewm");
        foreach (var p in ps) p.Dispose();
        return ps.Length > 0;
    }

    static void Restart(string why)
    {
        string exe = ExePath();
        if (exe == null) { Slider.Log("zebar nöbetçisi: " + why + ", zebar.exe bulunamadı"); return; }
        foreach (var p in Zebars()) { try { p.Kill(); p.WaitForExit(3000); } catch { } finally { p.Dispose(); } }
        // Önceki süreç portu bırakana kadar bekle (yoksa yeni Zebar da sunucusuz açılabiliyor)
        var sw = Stopwatch.StartNew();
        while (PortOpen() && sw.ElapsedMilliseconds < 5000) Thread.Sleep(200);
        string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        // ShellExecute: helper'ın tutamaçları (GlazeWM IPC bağlantısı vb.) Zebar'a miras kalmasın
        try { Process.Start(new ProcessStartInfo(exe) { UseShellExecute = true, WorkingDirectory = home }); Slider.Log("zebar nöbetçisi: " + why + ", yeniden başlatıldı: " + exe); }
        catch (Exception ex) { Slider.Log("zebar nöbetçisi: başlatılamadı: " + ex.Message); }
    }

    public static void Start()
    {
        new Thread(() =>
        {
            Thread.Sleep(15000); // oturum açılışında GlazeWM Zebar'ı kendisi başlatır
            int bad = 0, failures = 0;
            while (true)
            {
                Thread.Sleep(5000);
                try
                {
                    if (!GlazeRunning()) { bad = 0; continue; } // GlazeWM kapalıyken (çıkış / yeniden başlatma) karışma
                    if (Maint.Quiet() || WmWatchdog.Recovering) { bad = 0; continue; }
                    var zs = Zebars();
                    bool running = zs.Count > 0;
                    foreach (var p in zs) p.Dispose();
                    string problem = !running ? "zebar çalışmıyordu" : !PortOpen() ? "widget sunucusu (6124) yanıt vermiyordu" : null;
                    if (problem == null) { bad = 0; failures = 0; continue; }
                    if (++bad < 2) continue;
                    bad = 0;
                    Restart(problem);
                    failures++;
                    if (failures >= 3) Thread.Sleep(Math.Min(300000, 30000 * failures)); // sürekli başarısızsa sık sık deneme
                }
                catch (Exception ex) { Slider.Log("zebar nöbetçisi: " + ex.Message); }
            }
        }) { IsBackground = true, Priority = ThreadPriority.BelowNormal }.Start();
    }
}

// ---------------- Kendini toparlama ----------------
// Bir OS gibi: bir parça çökerse ya da donarsa kullanıcı komut satırı, taskkill bilmeden masaüstü kendiliğinden
// toparlanır. Nöbetçiler birbirini korur (yeni süreç yok):
//   GlazeWM çöker / donar      -> helper (WmWatchdog) masaüstünü yeniden başlatır
//   Zebar çöker                -> helper (ZebarWatchdog)
//   helper çöker (yönetilen hata) ya da arayüzü donar -> helper kendini yeniden başlatır (SelfHeal)
//   helper tamamen ölür        -> Zebar'ın bildirim kopyası (ll-helper --toast-stream) onu başlatır
// Hepsi kasıtlı çıkışta (GlazeWM 0 koduyla kapanır, kapanırken helper'ı ve Zebar'ı da kapatır), oturum kapanırken
// ve bakım sırasında (kurulum, güncelleme, kaldırma, "masaüstünü yenile") hiçbir şey yapmaz. Çöküş döngüsüne
// girmesinler diye her biri 5 dakikada en fazla 3 kez dener.
static class Maint
{
    public static volatile bool SessionEnding;
    static string Dir { get { return System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "logical-lunge"); } }

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

    public static string HelperExe { get { return System.IO.Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "ll-helper.exe"); } }

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

// GlazeWM'i izler. Çıkış kodu 0 değilse (çökme, zorla kapatılma, başlatma hatası) ya da 15 sn'den uzun IPC'ye yanıt
// vermezse masaüstünü yeniden başlatır. Kasıtlı çıkış 0 koduyla olur (ve helper'ı da kapatır). Yeniden başlatılan GlazeWM
// açılamazsa (ör. çöken sürecin IPC portu, süreci tamamen kapanana kadar dolu kalabiliyor) port boşalınca yeniden denenir.
static class WmWatchdog
{
    const int IPC_PORT = 6123;
    static int recoveringUntil;
    public static bool Recovering { get { return Environment.TickCount - Volatile.Read(ref recoveringUntil) < 0; } }

    public static void Start()
    {
        new Thread(Loop) { IsBackground = true, Priority = ThreadPriority.BelowNormal, Name = "wm-watchdog" }.Start();
    }

    // Pencere yöneticisi: en eski glazewm süreci (glazewm.exe "command ..." gibi kısa ömürlü CLI çağrıları da aynı adla
    // görünür; 5 sn'den genç olan sayılmaz)
    static Process FindWm()
    {
        Process best = null;
        foreach (var p in Process.GetProcessesByName("glazewm"))
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

    static string wmPath;

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
        var ipc = new Glaze();
        Process wm = null;
        bool wanted = false;  // GlazeWM çalışmalı mı: çalışırken görüldü ve kasıtlı kapanmadı
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
                        try { var handle = wm.Handle; wmPath = wm.MainModule.FileName; } catch { }
                        if (starts > 0)
                        {
                            Slider.Log("wm nöbetçisi: GlazeWM yeniden çalışıyor");
                            // Bildirim, Zebar geri gelip bildirim kanalına bağlanınca
                            new Thread(() =>
                            {
                                Thread.Sleep(9000);
                                Toasts.Send("info", "Masaüstü toparlandı", "Pencere yöneticisi beklenmedik biçimde kapanmıştı; yeniden başlatıldı.", "restart_alt");
                            }) { IsBackground = true }.Start();
                        }
                        wanted = true; hung = 0; missing = 0; starts = 0; portNoted = false;
                        continue;
                    }
                    if (Maint.Quiet() || Maint.Running("glazewm")) { missing = 0; continue; } // bakım / açılıyor
                    if (!wanted)
                    {
                        // Helper GlazeWM'siz başladı ya da GlazeWM'i hiç görmedi. Kasıtlı çıkış helper'ı da kapattığı için
                        // helper yaşarken GlazeWM ~20 sn yoksa masaüstü bozuktur: GlazeWM çalışmalı.
                        if (++missing < 10) continue;
                        missing = 0; wanted = true; nextStartAt = Environment.TickCount;
                    }
                    if (Environment.TickCount - nextStartAt < 0) continue;
                    if (!PortFree())
                    {
                        if (!portNoted) { portNoted = true; Slider.Log("wm nöbetçisi: IPC portu hâlâ eski süreçte; boşalınca başlatılacak"); }
                        continue;
                    }
                    starts++;
                    Slider.Log("wm nöbetçisi: GlazeWM çalışmıyor; başlatılıyor (deneme " + starts + ")");
                    Maint.RunHidden(Maint.HelperExe, "--uncloak-orphans", 15000);
                    StartWm();
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
                    if (code == 0) { wanted = false; Slider.Log("wm nöbetçisi: GlazeWM kapandı (kasıtlı, kod 0)"); continue; }
                    Thread.Sleep(1500); // kasıtlı kapanışta helper bu arada kapatılır; oturum kapanıyorsa bayrak kalkar
                    if (Maint.Quiet()) { wanted = false; Slider.Log("wm nöbetçisi: GlazeWM kapandı (kod " + code + "), bakım / oturum kapanışı: dokunulmadı"); continue; }
                    if (!Recover("GlazeWM beklenmedik biçimde kapandı (kod " + code + ")")) { wanted = false; continue; }
                    wanted = true; starts = 1; portNoted = false; nextStartAt = Environment.TickCount + 15000;
                    continue;
                }

                // Donma: 5 sn'de bir yokla; üst üste 3 başarısız yoklama (en az 15 sn) donmuş demektir. Ardışık sayım
                // uykudan dönüşte yanlış alarm vermez.
                if (++tick % 3 != 0) continue;
                if ((DateTime.Now - wm.StartTime).TotalSeconds < 30) { hung = 0; continue; }
                if (PingWithTimeout(ipc, 8000)) { hung = 0; continue; }
                if (++hung < 3) { Slider.Log("wm nöbetçisi: GlazeWM yanıt vermedi (" + hung + "/3)"); continue; }
                hung = 0;
                if (Maint.Quiet()) continue;
                Slider.Log("wm nöbetçisi: GlazeWM 15 sn'den uzun yanıt vermedi; kapatılıyor");
                try { wm.Kill(); wm.WaitForExit(5000); } catch { }
                // Bir sonraki turda çıkış kodu 0 olmadığı için masaüstü yeniden başlatılır
            }
            catch (Exception ex) { Slider.Log("wm nöbetçisi: " + ex.Message); if (wm != null) { try { wm.Dispose(); } catch { } wm = null; } }
        }
    }

    static bool PingWithTimeout(Glaze g, int ms)
    {
        var done = new ManualResetEvent(false);
        bool ok = false;
        ThreadPool.QueueUserWorkItem(_ => { try { ok = g.Ping(); } catch { } done.Set(); });
        return done.WaitOne(ms) && ok;
    }

    // GlazeWM'i kurulumdaki görevinden (ayarlarıyla) başlatır; görev yoksa exe'den
    static void StartWm()
    {
        if (Maint.RunHidden("schtasks.exe", "/run /tn \"\\LL\\GlazeWM\"", 10000) == 0) return;
        string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        string exe = wmPath != null && System.IO.File.Exists(wmPath) ? wmPath : System.IO.Path.Combine(home, @".glzr\logical-lunge\bin\glazewm.exe");
        if (!System.IO.File.Exists(exe)) exe = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), @"glzr.io\glazewm.exe");
        // ShellExecute: helper'ın tutamaçları GlazeWM'e miras kalmasın
        try { Process.Start(new ProcessStartInfo(exe) { UseShellExecute = true, WorkingDirectory = home }); }
        catch (Exception ex) { Slider.Log("wm nöbetçisi: GlazeWM başlatılamadı: " + ex.Message); }
    }

    // Masaüstünü temiz baştan başlatır: kalanları kapat, gizli kalmış pencereleri geri getir, GlazeWM'i başlat (o da
    // Zebar'ı açar; helper zaten çalıştığı için yeni kopyası kendiliğinden kapanır). Çöküş döngüsünde (5 dakikada 3)
    // vazgeçer ve false döner.
    static bool Recover(string why)
    {
        if (!Maint.Allow("wm-restarts"))
        {
            Slider.Log("wm nöbetçisi: " + why + "; son 5 dakikada 3 kez yeniden başlatıldı, bırakıldı");
            Toasts.Send("error", "Pencere yöneticisi tekrar tekrar kapanıyor",
                "Otomatik olarak yeniden başlatılmadı. Oturum menüsünden \"Masaüstünü yenile\"yi seçin ya da oturumu kapatıp açın.", "error");
            return false;
        }
        Volatile.Write(ref recoveringUntil, Environment.TickCount + 30000);
        Slider.Log("wm nöbetçisi: " + why + "; masaüstü yeniden başlatılıyor");
        foreach (var name in new[] { "zebar", "glazewm" })
            foreach (var p in Process.GetProcessesByName(name))
            {
                try { p.Kill(); p.WaitForExit(3000); } catch { }
                finally { p.Dispose(); }
            }
        Maint.RunHidden(Maint.HelperExe, "--uncloak-orphans", 15000);
        if (PortFree()) StartWm();
        else Slider.Log("wm nöbetçisi: IPC portu hâlâ eski süreçte; boşalınca başlatılacak");
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
            if (!Maint.Allow("helper-restarts")) { Slider.Log("kendini toparlama: son 5 dakikada 3 kez denendi, bırakıldı (" + why + ")"); return; }
            Slider.Log("helper yeniden başlıyor: " + why);
            Process.Start(new ProcessStartInfo(Maint.HelperExe, "--respawn") { UseShellExecute = true, WorkingDirectory = AppDomain.CurrentDomain.BaseDirectory });
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
// Zebar toast widget'ı http://127.0.0.1:6131/events adresine EventSource ile bağlanır; helper
// buradan bildirim gönderir (Windows hata pencereleri yerine). Yalnızca loopback dinlenir.
static class Toasts
{
    const int PORT = 6131;
    static readonly List<System.Net.Sockets.NetworkStream> clients = new List<System.Net.Sockets.NetworkStream>();
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    public static void Start()
    {
        var t = new Thread(() =>
        {
            try
            {
                var l = new System.Net.Sockets.TcpListener(System.Net.IPAddress.Loopback, PORT);
                l.Start();
                while (true)
                {
                    var c = l.AcceptTcpClient();
                    ThreadPool.QueueUserWorkItem(_ => Accept(c));
                }
            }
            catch (Exception ex) { Slider.Log("toast server: " + ex.GetBaseException().Message); }
        }) { IsBackground = true };
        t.Start();
        var ping = new Thread(() => { while (true) { Thread.Sleep(20000); Write(": ping\n\n"); } }) { IsBackground = true };
        ping.Start();
    }

    static void Accept(System.Net.Sockets.TcpClient c)
    {
        try
        {
            var s = c.GetStream();
            var buf = new byte[4096]; var req = new StringBuilder();
            while (!req.ToString().Contains("\r\n\r\n"))
            {
                int n = s.Read(buf, 0, buf.Length);
                if (n <= 0) { c.Close(); return; }
                req.Append(Encoding.ASCII.GetString(buf, 0, n));
            }
            string cors = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Allow-Headers: *\r\n";
            string reqs = req.ToString();
            // Widget'lar POST kullanır: Zebar'ın service worker'ı başka adreslere giden GET'leri önbelleğe alıyordu (ilk
            // cevap hep tekrar geliyordu: Super hep pano modunu açıyor, bar tıklamaları helper'a ulaşmıyordu)
            string verbless = reqs.StartsWith("POST ") ? reqs.Substring(5) : reqs.StartsWith("GET ") ? reqs.Substring(4) : "";
            if (verbless.StartsWith("/cmd?") || verbless.StartsWith("/overview-mode") || verbless.StartsWith("/overview-wait") || verbless.StartsWith("/overview-signal") || verbless.StartsWith("/log?")) { Command(s, reqs); c.Close(); return; }
            if (reqs.StartsWith("OPTIONS"))
            {
                var ok = Encoding.ASCII.GetBytes("HTTP/1.1 204 No Content\r\n" + cors + "Content-Length: 0\r\n\r\n");
                s.Write(ok, 0, ok.Length); c.Close(); return;
            }
            var head = Encoding.ASCII.GetBytes("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n" + cors + "\r\n: hazır\n\n");
            s.Write(head, 0, head.Length);
            lock (clients) clients.Add(s);
        }
        catch { try { c.Close(); } catch { } }
    }

    // Bar ve overview'dan anında komut (her tıklamada yeni helper süreci başlatmak ~100-300 ms sürüyordu).
    // Yalnızca Zebar widget'larının kökeninden (yerel varlık sunucusu) ve yalnızca zararsız komutlar: başka bir sitenin
    // tarayıcıdan bu kanalı kullanması mümkün değil (tarayıcı Origin'i gönderir).
    const string ZEBAR_ORIGIN = "http://127.0.0.1:6124";
    static void Command(System.Net.Sockets.NetworkStream s, string req)
    {
        string origin = null;
        foreach (var line in req.Split(new[] { "\r\n" }, StringSplitOptions.None))
            if (line.StartsWith("Origin:", StringComparison.OrdinalIgnoreCase)) origin = line.Substring(7).Trim();
        string status = "403 Forbidden", body = "";
        if (origin == null || origin == ZEBAR_ORIGIN)
        {
            int sp1 = req.IndexOf(' '), sp2 = sp1 < 0 ? -1 : req.IndexOf(' ', sp1 + 1);
            string target = sp2 > sp1 ? req.Substring(sp1 + 1, sp2 - sp1 - 1) : ""; // "/cmd?a=ws-3"
            if (target.StartsWith("/overview-mode")) { body = Keys2.TakeOverviewMode(); status = "200 OK"; Slider.Log("overview modu okundu: '" + body + "'"); }
            else if (target.StartsWith("/overview-wait"))
            {
                // Uzun yoklama: istek overview gösterilene / gizlenene ya da 25 sn dolana kadar bekletilir
                int q = target.IndexOf("since="), since;
                if (q < 0 || !int.TryParse(target.Substring(q + 6).Split('&')[0], out since)) since = -1;
                body = Keys2.WaitOverviewSignal(since, 25000); status = "200 OK";
            }
            else if (target.StartsWith("/overview-signal?w=show") || target.StartsWith("/overview-signal?w=hide"))
            {
                Keys2.OverviewSignal(target.EndsWith("hide") ? "hide" : "show"); status = "204 No Content";
            }
            else if (target.StartsWith("/log?m=")) { Slider.Log("widget: " + Uri.UnescapeDataString(target.Substring(7))); status = "204 No Content"; }
            else
            {
                int q = target.IndexOf("a=");
                string act = q < 0 ? "" : Uri.UnescapeDataString(target.Substring(q + 2).Split('&')[0]);
                if (System.Text.RegularExpressions.Regex.IsMatch(act, @"^ws-(\d{1,2}|next|prev)$") && Keys2.Instance != null)
                {
                    Keys2.Instance.Dispatch(act);
                    status = "204 No Content";
                }
                else status = "400 Bad Request";
            }
        }
        var bytes = Encoding.UTF8.GetBytes(body);
        var head = Encoding.ASCII.GetBytes("HTTP/1.1 " + status + "\r\nAccess-Control-Allow-Origin: " + ZEBAR_ORIGIN + "\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: " + bytes.Length + "\r\nConnection: close\r\n\r\n");
        s.Write(head, 0, head.Length);
        if (bytes.Length > 0) s.Write(bytes, 0, bytes.Length);
        s.Flush();
    }

    static void Write(string text)
    {
        var bytes = Encoding.UTF8.GetBytes(text);
        lock (clients)
        {
            clients.RemoveAll(s =>
            {
                try { s.Write(bytes, 0, bytes.Length); s.Flush(); return false; }
                catch { return true; }
            });
        }
    }

    public static void Send(string kind, string title, string body, string icon)
    {
        var d = new Dictionary<string, object> { { "kind", kind }, { "title", title }, { "body", body }, { "icon", icon } };
        Write("data: " + json.Serialize(d) + "\n\n");
    }
}

// ---------------- Windows hata pencerelerini yakala ----------------
// Tek "Tamam" butonlu bilgi/hata kutularını (GlazeWM "Non-fatal error", Explorer "bulunamıyor",
// Zebar/tacky hataları...) kapatıp metnini toast olarak gösterir. Cevap bekleyen (Evet/Hayır)
// kutulara dokunmaz.
class DialogCatcher
{
    Native.WinEventDelegate cb;
    static readonly HashSet<string> owners = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { "glazewm", "zebar", "ll-helper", "tacky-borders", "explorer", "powershell", "rundll32", "cmd" };
    // Oluşturulurken görünmez yapılan, henüz karar verilmemiş kutular -> özgün genişletilmiş stil
    readonly Dictionary<IntPtr, int> pending = new Dictionary<IntPtr, int>();
    System.Windows.Forms.Timer safety;

    [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr hwnd, uint id, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out object obj);
    [DllImport("oleacc.dll")] static extern int AccessibleChildren(Accessibility.IAccessible container, int start, int count, [Out] object[] children, out int obtained);

    // Mesaj döngüsü olan kendi thread'inde çağrılır
    public void Start()
    {
        cb = OnEvent;
        // CREATE..SHOW: kutu oluşturulduğu anda (gösterilmeden) görünmez yapılır, gösterilince karar verilir
        Native.SetWinEventHook(0x8000, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, cb, 0, 0, 0x0002);
        // Güvenlik ağı: 2 sn'de karar verilemeyen kutu (olay kaçtıysa) geri görünür olur; hiçbir pencere görünmez kalmaz
        safety = new System.Windows.Forms.Timer { Interval = 500 };
        safety.Tick += (s, e) =>
        {
            foreach (var kv in new List<KeyValuePair<IntPtr, int>>(pending))
                if (!Native.IsWindow(kv.Key)) pending.Remove(kv.Key);
                else if (Environment.TickCount - Created(kv.Key) > 2000) { Restore(kv.Key, kv.Value); Slider.Log("dialog: karar verilemedi, geri gösterildi"); }
        };
        safety.Start();
    }

    readonly Dictionary<IntPtr, int> createdAt = new Dictionary<IntPtr, int>();
    int Created(IntPtr h) { int t; return createdAt.TryGetValue(h, out t) ? t : 0; }

    static string ClassOf(IntPtr h) { var c = new StringBuilder(64); Native.GetClassName(h, c, 64); return c.ToString(); }

    void Hide(IntPtr h)
    {
        int ex0 = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        if ((ex0 & 0x00080000) == 0) Native.SetWindowLong(h, Native.GWL_EXSTYLE, ex0 | 0x00080000); // WS_EX_LAYERED
        Native.SetLayeredWindowAttributes(h, 0, 0, 0x2); // LWA_ALPHA, tamamen saydam
        pending[h] = ex0; createdAt[h] = Environment.TickCount;
    }

    void Restore(IntPtr h, int ex0)
    {
        pending.Remove(h); createdAt.Remove(h);
        Native.SetLayeredWindowAttributes(h, 0, 255, 0x2);
        Native.SetWindowLong(h, Native.GWL_EXSTYLE, ex0);
        Native.RedrawWindow(h, IntPtr.Zero, IntPtr.Zero, 0x0001 | 0x0004 | 0x0080 | 0x0400); // INVALIDATE|UPDATENOW|ALLCHILDREN|FRAME
    }

    void OnEvent(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        if (idObject != 0 || hwnd == IntPtr.Zero) return;
        try
        {
            if (ev != 0x8000 && ev != Native.EVENT_OBJECT_SHOW) return;
            if (ClassOf(hwnd) != "#32770") return;
            uint pid; Native.GetWindowThreadProcessId(hwnd, out pid);
            string proc;
            try { proc = Process.GetProcessById((int)pid).ProcessName; } catch { return; }
            if (!owners.Contains(proc)) return;

            if (ev == 0x8000) { Hide(hwnd); return; } // EVENT_OBJECT_CREATE: henüz ekranda değil
            if (!pending.ContainsKey(hwnd)) Hide(hwnd); // oluşturma olayı kaçtıysa şimdi
            int ex0 = pending[hwnd];

            var texts = new List<string>(); var buttons = new List<IntPtr>(); IntPtr dui = IntPtr.Zero;
            Native.EnumChildWindows(hwnd, delegate (IntPtr ch, IntPtr l)
            {
                string cn = ClassOf(ch);
                var t = new StringBuilder(2048); Native.GetWindowText(ch, t, 2048);
                string tx = t.ToString().Trim();
                if (cn == "Button") { if (Native.IsWindowVisible(ch)) buttons.Add(ch); }
                else if (cn == "Static" && tx.Length > 0) texts.Add(tx);
                else if (cn == "DirectUIHWND" && dui == IntPtr.Zero) dui = ch;
                return true;
            }, IntPtr.Zero);
            // Yeni tür kutular (TaskDialog: Çalıştır, explorer, kısayol hataları): metin DirectUIHWND'in içinde çiziliyor,
            // pencere metni olarak okunmuyor; erişilebilirlik arabiriminden (ekran okuyucuların yolu) okunur.
            if (texts.Count == 0 && dui != IntPtr.Zero) ReadAccessibleTexts(dui, texts);

            // Yalnızca tek düğmeli (Tamam) bilgi / hata kutusu bildirime döner; soru soranlar (Evet/Hayır, özellikler,
            // dosya işlemleri) olduğu gibi görünür
            if (buttons.Count != 1 || texts.Count == 0) { Restore(hwnd, ex0); return; }

            pending.Remove(hwnd); createdAt.Remove(hwnd);
            var title = new StringBuilder(256); Native.GetWindowText(hwnd, title, 256);
            Native.PostMessage(buttons[0], 0x00F5, IntPtr.Zero, IntPtr.Zero); // BM_CLICK
            string head = title.ToString();
            if (proc.Equals("glazewm", StringComparison.OrdinalIgnoreCase)) head = "GlazeWM: " + head;
            Toasts.Send("error", head.Length > 0 ? head : "Hata", string.Join("\n", texts), "error");
            Slider.Log("dialog -> toast: " + proc + " | " + head);
        }
        catch (Exception ex)
        {
            Slider.Log("dialog: " + ex.GetBaseException().Message);
            int ex0; if (pending.TryGetValue(hwnd, out ex0)) Restore(hwnd, ex0);
        }
    }

    // MSAA: kutunun içindeki metin öğeleri (ROLE_SYSTEM_STATICTEXT / TEXT), sırayla ve tekrarsız
    static void ReadAccessibleTexts(IntPtr h, List<string> into)
    {
        try
        {
            var iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71"); // IID_IAccessible
            object o;
            if (AccessibleObjectFromWindow(h, 0xFFFFFFFC, ref iid, out o) != 0) return; // OBJID_CLIENT
            var acc = o as Accessibility.IAccessible;
            if (acc != null) Walk(acc, into, 0);
        }
        catch { }
    }

    static void Walk(Accessibility.IAccessible acc, List<string> into, int depth)
    {
        if (depth > 8) return;
        int n;
        try { n = acc.accChildCount; } catch { return; }
        if (n <= 0 || n > 200) return;
        var kids = new object[n]; int got;
        if (AccessibleChildren(acc, 0, n, kids, out got) != 0) return;
        for (int i = 0; i < got; i++)
        {
            try
            {
                var child = kids[i] as Accessibility.IAccessible;
                object role; string name;
                if (child != null) { role = child.get_accRole(0); name = child.get_accName(0); }
                else { role = acc.get_accRole(kids[i]); name = acc.get_accName(kids[i]); }
                int r = role is int ? (int)role : 0;
                if ((r == 41 || r == 42) && !string.IsNullOrWhiteSpace(name) && !into.Contains(name.Trim())) into.Add(name.Trim());
                if (child != null) Walk(child, into, depth + 1);
            }
            catch { }
        }
    }
}

// ---------------- Yuvarlak köşeler ----------------
class Rounder
{
    const int RADIUS = 14; // tacky-borders border_radius ile aynı
    readonly Dictionary<IntPtr, long> applied = new Dictionary<IntPtr, long>();
    readonly Dictionary<IntPtr, List<long>> resets = new Dictionary<IntPtr, List<long>>();
    readonly HashSet<IntPtr> giveUp = new HashSet<IntPtr>();
    Native.WinEventDelegate cb;
    static readonly HashSet<string> skipProcs = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        { "zebar", "tacky-borders", "glazewm", "ll-helper", "explorer", "ShellExperienceHost", "SearchUI", "SearchApp", "StartMenuExperienceHost", "LockApp" };
    static readonly Dictionary<uint, string> procCache = new Dictionary<uint, string>();

    public void Start()
    {
        cb = OnEvent;
        Native.SetWinEventHook(Native.EVENT_OBJECT_SHOW, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, cb, 0, 0, 0x0002);
        Native.SetWinEventHook(Native.EVENT_OBJECT_LOCATIONCHANGE, Native.EVENT_OBJECT_LOCATIONCHANGE, IntPtr.Zero, cb, 0, 0, 0x0002);
        Native.SetWinEventHook(Native.EVENT_SYSTEM_FOREGROUND, Native.EVENT_SYSTEM_FOREGROUND, IntPtr.Zero, cb, 0, 0, 0x0002);
        Native.EnumWindows(delegate (IntPtr h, IntPtr l) { Apply(h); return true; }, IntPtr.Zero);

        // Aynı thread'de (mesaj döngüsü var) periyodik kontrol: sıfırlanan bölgeleri geri koy
        var timer = new System.Windows.Forms.Timer { Interval = 700 };
        timer.Tick += (s, e) => Native.EnumWindows(delegate (IntPtr h, IntPtr l) { Apply(h); MarkNoActivate(h); return true; }, IntPtr.Zero);
        timer.Start();
        Native.EnumWindows(delegate (IntPtr h, IntPtr l) { MarkNoActivate(h); return true; }, IntPtr.Zero);
    }

    // Bar ve bildirim penceresi odak almasın: "fareyle üzerine gelince etkinleştir" açıkken
    // bar'ın üstüne gelmek klavyeyi uygulamadan çalmasın. Tık ve tekerlek yine çalışır.
    static void MarkNoActivate(IntPtr h)
    {
        var sb = new StringBuilder(64);
        if (Native.GetWindowText(h, sb, 64) == 0) return;
        string t = sb.ToString();
        if (t != "Zebar - logical-lunge / bar" && t != "Zebar - logical-lunge / toast" && t != "Zebar - logical-lunge / osk") return;
        int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        if ((ex & Native.WS_EX_NOACTIVATE) == 0) Native.SetWindowLong(h, Native.GWL_EXSTYLE, ex | Native.WS_EX_NOACTIVATE);
    }

    void OnEvent(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        if (idObject != 0 || hwnd == IntPtr.Zero) return; // OBJID_WINDOW
        Apply(hwnd);
    }

    static string ProcName(IntPtr h)
    {
        uint pid; Native.GetWindowThreadProcessId(h, out pid);
        string name;
        if (procCache.TryGetValue(pid, out name)) return name;
        try { name = Process.GetProcessById((int)pid).ProcessName; } catch { name = ""; }
        procCache[pid] = name;
        return name;
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
        if ((style & Native.WS_CAPTION) != Native.WS_CAPTION && (style & 0x00040000) == 0) return; // başlık ya da kalın çerçeve
        if (skipProcs.Contains(ProcName(h))) return;

        Native.RECT wr, fr;
        if (!Native.GetWindowRect(h, out wr)) return;
        if (Native.DwmGetWindowAttribute(h, Native.DWMWA_EXTENDED_FRAME_BOUNDS, out fr, Marshal.SizeOf(typeof(Native.RECT))) != 0) fr = wr;

        // Tam ekran / maximize: köşe yok (Hyprland'de de fullscreen'de rounding kalkar)
        var wp = new Native.WINDOWPLACEMENT { length = Marshal.SizeOf(typeof(Native.WINDOWPLACEMENT)) };
        Native.GetWindowPlacement(h, ref wp);
        var screen = Screen.FromHandle(h).Bounds;
        bool full = wp.showCmd == 3 || (fr.Left <= screen.Left && fr.Top <= screen.Top && fr.Right >= screen.Right && fr.Bottom >= screen.Bottom);

        long key = ((long)(fr.Right - fr.Left) << 32) | (uint)(fr.Bottom - fr.Top);
        if (full) key = -1;
        long prev;
        Native.RECT box;
        bool hasRgn = Native.GetWindowRgnBox(h, out box) != 0;
        // Bazı uygulamalar (Terminal, Firefox/Zen) bölgeyi kendileri sıfırlıyor: yoksa yeniden uygula
        if (applied.TryGetValue(h, out prev) && prev == key && (hasRgn || full)) return;
        if (giveUp.Contains(h)) return;
        if (prev == key && !hasRgn)
        {
            // Aynı boyutta bölge silinmiş -> uygulama kendisi sıfırlıyor. Kısa sürede çok tekrarlarsa
            // kavga etme (titreme + CPU): o pencereyi köşesiz bırak.
            long now = Environment.TickCount;
            List<long> hits;
            if (!resets.TryGetValue(h, out hits)) resets[h] = hits = new List<long>();
            hits.Add(now);
            hits.RemoveAll(x => now - x > 3000);
            if (hits.Count > 4) { giveUp.Add(h); Slider.Log("gave up rounding " + ProcName(h)); return; }
        }
        applied[h] = key;

        if (full) { Native.SetWindowRgn(h, IntPtr.Zero, true); return; }

        int l = fr.Left - wr.Left, t = fr.Top - wr.Top;
        int r = l + (fr.Right - fr.Left), b = t + (fr.Bottom - fr.Top);
        IntPtr rgn = Native.CreateRoundRectRgn(l, t, r + 1, b + 1, RADIUS * 2, RADIUS * 2);
        if (Native.SetWindowRgn(h, rgn, true) == 0)
        {
            Slider.Log("SetWindowRgn failed " + ProcName(h) + " err=" + Marshal.GetLastWin32Error());
            Native.DeleteObject(rgn); // başarılıysa sistem sahiplenir
        }
    }
}

// ---------------- Kısayollar (%LOCALAPPDATA%\logical-lunge\keybinds.json) ----------------
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
    };

    static readonly object gate = new object();
    static Dictionary<long, string> table = new Dictionary<long, string>();
    static System.IO.FileSystemWatcher watcher, captureWatcher;

    public static string FilePath
    {
        get
        {
            string dir = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "logical-lunge");
            System.IO.Directory.CreateDirectory(dir);
            return System.IO.Path.Combine(dir, "keybinds.json");
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
                if (user != null) foreach (var kv in user) if (d.ContainsKey(kv.Key)) d[kv.Key] = kv.Value == null ? "" : kv.Value.ToString();
            }
        }
        catch (Exception ex) { Slider.Log("keybinds: " + ex.Message); }
        return d;
    }

    public static void Load()
    {
        var t = new Dictionary<long, string>();
        foreach (var kv in Effective())
        {
            int m, vk;
            if (!Parse(kv.Value, out m, out vk)) continue;
            long key = ((long)m << 16) | (uint)vk;
            if (!t.ContainsKey(key)) t[key] = kv.Key; // çakışmada listedeki ilk eylem kazanır
        }
        lock (gate) table = t;
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
        try
        {
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

    // Kullanıcı değerini yaz (varsayılana eşitse dosyadan çıkar); id "" ise hepsini sıfırla
    public static void Set(string id, string combo)
    {
        var d = new Dictionary<string, object>();
        try
        {
            if (id != "" && System.IO.File.Exists(FilePath))
            {
                var old = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(FilePath));
                if (old != null) foreach (var kv in old) d[kv.Key] = kv.Value;
            }
        }
        catch { }
        if (id != "")
        {
            string def = null;
            for (int i = 0; i < Defaults.GetLength(0); i++) if (Defaults[i, 0] == id) def = Defaults[i, 1];
            if (def == null) return;
            if (combo == def) d.Remove(id); else d[id] = combo;
        }
        System.IO.File.WriteAllText(FilePath, new JavaScriptSerializer().Serialize(d));
        Load();
    }

    // ll-helper.exe --keybinds -> [{"id","combo","default"}] (düzenleyici için)
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
    const int VK_LWIN = 0x5B, VK_RWIN = 0x5C, VK_CONTROL = 0x11, VK_SHIFT = 0x10, VK_MENU = 0x12;
    const int VK_LEFT = 0x25, VK_UP = 0x26, VK_RIGHT = 0x27, VK_DOWN = 0x28;
    const byte VK_DUMMY = 0xE8; // atanmamış tuş: Başlat menüsünü bastırmak için

    readonly Control ui;
    readonly Slider slider;
    Native.LowLevelKeyboardProc proc;
    bool winDown, otherKeyWhileWin, swallowedWithWin, modifierWhileWin, winInjected;
    int winVk = VK_LWIN, lastWinEvent;

    public static Keys2 Instance;
    public Keys2(Control ui, Slider slider) { this.ui = ui; this.slider = slider; Instance = this; }
    // Bar'dan (yerel HTTP) gelen workspace komutu: klavyedeki kısayolla aynı yol
    public bool Dispatch(string act) { return RunAction(act); }
    static int lastMoveAction = Environment.TickCount - 100000;

    // Test kanalı (yalnızca LL_TEST=1 ortam değişkeniyle başlatılınca açılır): \\.\pipe\ll-helper-test'e yazılan her
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
                    using (var pipe = new System.IO.Pipes.NamedPipeServerStream("ll-helper-test", System.IO.Pipes.PipeDirection.In))
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
        proc = Hook;
        hookHandle = Native.SetWindowsHookEx(Native.WH_KEYBOARD_LL, proc, Native.GetModuleHandle(null), 0);
    }

    public void Reinstall()
    {
        // Tuş basılıyken değiştirme (durum karışmasın)
        if (winDown) return;
        IntPtr fresh = Native.SetWindowsHookEx(Native.WH_KEYBOARD_LL, proc, Native.GetModuleHandle(null), 0);
        if (fresh == IntPtr.Zero) return;
        IntPtr old = hookHandle;
        hookHandle = fresh;
        if (old != IntPtr.Zero) Native.UnhookWindowsHookEx(old);
    }

    static bool Down(int vk) { return (Native.GetAsyncKeyState(vk) & 0x8000) != 0; }

    // Hızlı art arda workspace geçişleri sıraya girip her biri animasyonunu beklemesin:
    // süren animasyon hemen biter, biriken istekler komut olarak uygulanır, yalnızca SONUNCUSU kayar.
    readonly object pendLock = new object();
    readonly object inWsLock = new object(); // odak/taşıma istekleri sırayla, ama UI thread'ini (slide) beklemeden
    readonly List<object[]> pend = new List<object[]>();

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

    static void SuppressStart() { Native.keybd_event(VK_DUMMY, 0, 0, UIntPtr.Zero); Native.keybd_event(VK_DUMMY, 0, 2, UIntPtr.Zero); }

    IntPtr Hook(int nCode, IntPtr wParam, IntPtr lParam)
    {
        if (nCode < 0) return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
        var k = (Native.KBDLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(Native.KBDLLHOOKSTRUCT));
        if ((k.flags & Native.LLKHF_INJECTED) != 0) return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
        // Kabuk (bar) çökmüş / açılamamışsa tuşlar olduğu gibi Windows'a: Win tuşu Başlat menüsünü açar (ShellState).
        // Basılı bir Win ya da açık değiştirici varsa önce o biter.
        if (!ShellState.Up && !winDown && !Switcher.Active) return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);

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

        // Gerçek Win tuşu Windows'a HİÇ iletilmez: Windows tek başına bir Win basışı görmediği için Başlat
        // menüsü (ve görev çubuğundaki logo) hiçbir tuş sırasıyla açılamaz. Bizim işlemediğimiz bir kombinasyon
        // (Win+L, Win+V, GlazeWM'in lwin+f'i...) gelince Win'i o anda enjekte edip tuşu arkasından yeniden
        // göndeririz; bırakmada önce sahte tuş, sonra Win bırakma gider.
        if (vk == VK_LWIN || vk == VK_RWIN)
        {
            // Win+L gibi durumlarda bırakma olayı hiç gelmeyebilir: otomatik tekrar ~30 ms'de bir gelir,
            // uzun bir aradan sonraki basış yeni basıştır.
            int now = Environment.TickCount;
            bool fresh = !winDown || now - lastWinEvent > 700;
            lastWinEvent = now;
            if (isDown && fresh)
            {
                if (winInjected) Native.keybd_event((byte)winVk, 0, 0x2 | 0x1, UIntPtr.Zero);
                winDown = true; winInjected = false; winVk = vk;
                otherKeyWhileWin = false; swallowedWithWin = false;
                // Win'den önce basılı tutulan Ctrl/Shift/Alt da "kombinasyon" sayılır
                modifierWhileWin = Down(VK_CONTROL) || Down(VK_SHIFT) || Down(VK_MENU);
            }
            if (isUp)
            {
                winDown = false;
                if (winInjected)
                {
                    winInjected = false;
                    SuppressStart();
                    Native.keybd_event((byte)winVk, 0, 0x2 | 0x1, UIntPtr.Zero); // KEYUP | EXTENDEDKEY
                }
                // Yalnızca tek başına Super: ll overview
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
            if (isModifier) modifierWhileWin = true;
            else otherKeyWhileWin = true;
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
                    if (winDown) swallowedWithWin = true;
                    return (IntPtr)1;
                }
            }
        }
        if (isUp && held.Remove(vk)) return (IntPtr)1;
        // Masaüstünü göster / tüm pencereleri küçült kısayolları (Win+D, Win+M, Win+Home, Win+,) pencere yöneticisinin
        // düzenini bozar (küçültülen pencereler yerleşimden düşer): hiç iletilmez.
        if (winDown && isDown && (vk == 0x44 || vk == 0x4D || vk == 0x24 || vk == 0xBC))
        {
            held.Add(vk);
            swallowedWithWin = true;
            return (IntPtr)1;
        }
        // Bizim işlemediğimiz Win+tuş: Win'i şimdi enjekte et, tuşu da arkasından yeniden gönder
        // (kancadan enjekte edilen olay mevcut olaydan SONRA işlenir; sıra bozulmasın diye bunu yutuyoruz).
        if (winDown && isDown && !(vk == VK_CONTROL || vk == VK_SHIFT || vk == VK_MENU || (vk >= 0xA0 && vk <= 0xA5)))
        {
            if (!winInjected)
            {
                winInjected = true;
                Native.keybd_event((byte)winVk, 0, 0x1, UIntPtr.Zero); // EXTENDEDKEY
            }
            Native.keybd_event((byte)vk, (byte)k.scanCode, (k.flags & 0x1) != 0 ? 0x1u : 0u, UIntPtr.Zero);
            return (IntPtr)1;
        }

        return Native.CallNextHookEx(IntPtr.Zero, nCode, wParam, lParam);
    }

    readonly HashSet<int> held = new HashSet<int>();

    // Kısayol eylemleri. false: işlenmedi (tuş normal yoluna devam eder)
    bool RunAction(string act)
    {
        // Pencere hareketi / odak / workspace kısayolları overview'u (arama, pano) kapatır
        if (act.StartsWith("move-") || act.StartsWith("focus-") || act.StartsWith("ws-"))
        {
            lastMoveAction = Environment.TickCount;
            IntPtr ov = Native.FindWindow(null, "ll-overview");
            if (ov != IntPtr.Zero && Native.IsWindowVisible(ov)) HideOverview(ov);
        }
        if (act == "clipboard") { ui.BeginInvoke((Action)ToggleClipboard); return true; }
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
            bool shell = fg == IntPtr.Zero || c == "Progman" || c == "WorkerW" || c == "Shell_TrayWnd" || t.StartsWith("Zebar") || t.StartsWith("ll-");
            if (shell) return false;
            Native.PostMessage(fg, 0x0112, (IntPtr)0xF060, IntPtr.Zero); // WM_SYSCOMMAND SC_CLOSE
            return true;
        }
        return false;
    }
    public static string TerminalPath { get { return Terminal; } }
    // Hyprland $terminal (kitty) karşılığı: logical-lunge içindeki WezTerm; yoksa Windows Terminal
    static readonly string Terminal = System.IO.File.Exists(Environment.ExpandEnvironmentVariables(@"%USERPROFILE%\.glzr\logical-lunge\tools\wezterm\wezterm-gui.exe"))
        ? Environment.ExpandEnvironmentVariables(@"%USERPROFILE%\.glzr\logical-lunge\tools\wezterm\wezterm-gui.exe") : "wt.exe";

    // Hyprland keybinds.lua: Super+W tarayıcı, E dosya yöneticisi, C kod editörü, X metin editörü.
    // GlazeWM'in shell-exec'i boşluklu tırnaklı yolları ayrıştıramıyordu ("doesn't have an ending").
    // Her bilgisayarda çalışsın: tarayıcı = sistemin varsayılanı, kod editörü = bulunan ilk editör
    static readonly Dictionary<string, string> Apps = new Dictionary<string, string>
    {
        { "browser", DefaultBrowser() },
        { "files", "explorer.exe" },
        { "code", FirstExisting(@"%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe", @"%ProgramFiles%\Microsoft VS Code\Code.exe",
                                @"%LOCALAPPDATA%\Programs\cursor\Cursor.exe", @"%LOCALAPPDATA%\Programs\Windsurf\Windsurf.exe", @"%ProgramFiles%\Notepad++\notepad++.exe") ?? "notepad.exe" },
        { "editor", "notepad.exe" },
    };
    static string FirstExisting(params string[] paths)
    {
        foreach (var p in paths) { var e = Environment.ExpandEnvironmentVariables(p); if (System.IO.File.Exists(e)) return e; }
        return null;
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
        return "msedge.exe";
    }

    void Launch(string path) { LaunchQueue.Enqueue(path); }

    // Overview'u önce saydam göster; widget helper'ın bıraktığı mod bayrağını okuyup arayüzü kurunca (bayrak silinir)
    // görünür yap. Aksi halde önce düz arama, sonra ";" pano modu görünüyordu.
    // Overview modu bayrağı: bir kez okunur ve silinir ("" ya da ";" = pano)
    public static string TakeOverviewMode()
    {
        string f = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"logical-lunge\overview-mode.txt");
        string m = "";
        if (System.IO.File.Exists(f)) { try { m = System.IO.File.ReadAllText(f).Trim(); System.IO.File.Delete(f); } catch { } }
        return m;
    }

    // Overview (Super / Super+V) kapanınca odak boşta kalıyordu (elle tıklamak gerekiyordu): açılmadan önceki pencereye
    // geri ver. Kapanırken başka bir pencere odak aldıysa (overview'dan uygulama açıldı, başka yere tıklandı) ya da bir
    // workspace / taşıma kısayoluyla kapandıysa (odağı GlazeWM yönetir) dokunma.
    static int overviewGen;
    static bool ShellLike(IntPtr w)
    {
        if (w == IntPtr.Zero) return true;
        var c = new StringBuilder(64); Native.GetClassName(w, c, 64);
        var t = new StringBuilder(128); Native.GetWindowText(w, t, 128);
        string cs = c.ToString(), ts = t.ToString();
        return cs == "Progman" || cs == "WorkerW" || cs == "Shell_TrayWnd" || ts.StartsWith("Zebar") || ts.StartsWith("ll-");
    }
    static void RestoreFocusAfterOverview(IntPtr ov, IntPtr prev, int gen)
    {
        var sw = Stopwatch.StartNew();
        while (!Native.IsWindowVisible(ov) && sw.ElapsedMilliseconds < 1500) Thread.Sleep(15);
        while (Native.IsWindowVisible(ov)) { if (gen != overviewGen || sw.Elapsed.TotalMinutes > 30) return; Thread.Sleep(15); }
        if (gen != overviewGen) return;
        Thread.Sleep(60); // yeni açılan pencere / GlazeWM odağı alsın
        if (gen != overviewGen || Environment.TickCount - lastMoveAction < 700) return;
        IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
        if (fg != ov && !ShellLike(fg)) return;
        int cl;
        if (!Native.IsWindow(prev) || !Native.IsWindowVisible(prev) || Native.IsIconic(prev)) return;
        if (Native.DwmGetWindowAttribute(prev, Native.DWMWA_CLOAKED, out cl, 4) == 0 && cl != 0) return; // başka workspace'te
        Native.keybd_event(VK_DUMMY, 0, 0, UIntPtr.Zero); Native.keybd_event(VK_DUMMY, 0, 2, UIntPtr.Zero);
        Native.SetForegroundWindow(prev);
    }

    // Overview widget'ı helper'ın Win32 ile gösterip gizlediğini görünürlüğü 40 ms'de bir sorarak anlıyordu (gün boyu
    // saniyede 25 IPC: boştaki Zebar'ın başlıca işi). Artık helper haber verir; widget /overview-wait uzun yoklamasıyla
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
    public static void HideOverview(IntPtr h)
    {
        Native.ShowWindow(h, 0);
        OverviewSignal("hide");
    }

    public static void ShowOverviewInMode(IntPtr h, string mode)
    {
        IntPtr prevFg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
        int gen = Interlocked.Increment(ref overviewGen);
        if (prevFg != h && !ShellLike(prevFg)) ThreadPool.QueueUserWorkItem(_ => RestoreFocusAfterOverview(h, prevFg, gen));
        string d = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "logical-lunge");
        string flag = System.IO.Path.Combine(d, "overview-mode.txt");
        try { System.IO.Directory.CreateDirectory(d); System.IO.File.WriteAllText(flag, mode); } catch { }
        int ex = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
        Native.SetWindowLong(h, Native.GWL_EXSTYLE, ex | 0x00080000); // WS_EX_LAYERED
        Native.SetLayeredWindowAttributes(h, 0, 0, 0x2);              // tamamen saydam
        Native.ShowWindow(h, 5);
        OverviewSignal("show");
        Native.keybd_event(VK_DUMMY, 0, 0, UIntPtr.Zero); Native.keybd_event(VK_DUMMY, 0, 2, UIntPtr.Zero);
        Native.SetForegroundWindow(h);
        ThreadPool.QueueUserWorkItem(_ =>
        {
            var sw = Stopwatch.StartNew();
            while (sw.ElapsedMilliseconds < 800 && System.IO.File.Exists(flag)) Thread.Sleep(6);
            Thread.Sleep(70); // widget'ın çizimi tamamlaması
            Native.SetLayeredWindowAttributes(h, 0, 255, 0x2);
            int e2 = Native.GetWindowLong(h, Native.GWL_EXSTYLE);
            Native.SetWindowLong(h, Native.GWL_EXSTYLE, e2 & ~0x00080000);
        });
    }

    // Super+V: overview'u pano modunda (";" öneki) aç; açıkken tekrar basınca kapat
    static void ToggleClipboard()
    {
        IntPtr h = Native.FindWindow(null, "ll-overview");
        if (h == IntPtr.Zero) return;
        if (Native.IsWindowVisible(h) && Native.GetForegroundWindow() == h) { HideOverview(h); return; }
        ShowOverviewInMode(h, ";");
    }

    static void ToggleOverview()
    {
        IntPtr h = Native.FindWindow(null, "ll-overview");
        if (h == IntPtr.Zero) return;
        if (Native.IsWindowVisible(h) && Native.GetForegroundWindow() == h) { HideOverview(h); return; }
        // Mod bayrağı: "s" = düz arama (pano modunun bayrağı ";"); widget taze açılmış gibi davransın
        ShowOverviewInMode(h, "s");
    }
}

// ---------------- Mikrofon (Windows Core Audio) ----------------
// Zebar'ın setMute'u yalnızca varsayılan kayıt cihazını susturuyordu; Discord gibi uygulamalar
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
// ll-helper.exe --osk : stdin'den satır okur ("tap <vk>", "down <vk>", "up <vk>", "text <karakterler>")
// ve SendInput ile odaktaki pencereye yollar. Zebar'daki ii tarzı ekran klavyesi bunu kullanır.
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
// Durum %LOCALAPPDATA%\logical-lunge\nightlight dosyasında; açıkken ana helper birkaç sn'de bir
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
        var d = new Dictionary<string, string> { { "on", "0" }, { "level", "50" }, { "mode", "manual" }, { "from", "20:00" }, { "to", "07:00" } };
        try
        {
            foreach (var line in System.IO.File.ReadAllLines(StateFile))
            {
                var t = line.Trim();
                if (t == "1" || t == "0") { d["on"] = t; continue; } // eski biçim
                int eq = t.IndexOf('=');
                if (eq > 0) d[t.Substring(0, eq)] = t.Substring(eq + 1);
            }
        }
        catch { }
        return d;
    }
    public static void Set(string key, string value)
    {
        var d = Settings(); d[key] = value;
        var lines = new List<string>(); foreach (var kv in d) lines.Add(kv.Key + "=" + kv.Value);
        System.IO.File.WriteAllLines(StateFile, lines.ToArray());
        Apply(Active);
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
            string dir = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "logical-lunge");
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
    // %LOCALAPPDATA%\logical-lunge\gamma: "\\.\DISPLAY1=60" satırları; yeniden başlatınca da korunur.
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
    public static bool SetGamma(string dev, int v)
    {
        var d = Gammas(); d[dev] = Math.Max(0, Math.Min(100, v));
        var lines = new List<string>(); foreach (var kv in d) if (kv.Value < 100) lines.Add(kv.Key + "=" + kv.Value);
        System.IO.File.WriteAllLines(GammaFile, lines.ToArray());
        return Apply(Active);
    }
    public static bool AnyActive() { if (Active) return true; foreach (var v in Gammas().Values) if (v < 100) return true; return false; }

    public static bool Apply(bool on)
    {
        var gammas = Gammas();
        bool ok = true;
        foreach (var s in Screen.AllScreens)
        {
            int g; if (!gammas.TryGetValue(s.DeviceName, out g)) g = 100;
            double k = 0.2 + 0.8 * g / 100.0;
            int lv; if (!int.TryParse(Settings()["level"], out lv)) lv = 50;
            double R, G, B; Rgb(lv, out R, out G, out B);
            var ramp = new ushort[256 * 3];
            for (int i = 0; i < 256; i++)
            {
                ramp[i] = (ushort)(i * 257 * k * (on ? R : 1));
                ramp[256 + i] = (ushort)(i * 257 * k * (on ? G : 1));
                ramp[512 + i] = (ushort)(i * 257 * k * (on ? B : 1));
            }
            IntPtr dc = CreateDC(null, s.DeviceName, null, IntPtr.Zero);
            if (dc == IntPtr.Zero) continue;
            if (!SetDeviceGammaRamp(dc, ramp)) ok = false;
            DeleteDC(dc);
        }
        return ok;
    }

    public static void StartKeeper()
    {
        bool last = AnyActive();
        Apply(Active);
        var t = new Thread(() =>
        {
            while (true)
            {
                Thread.Sleep(5000);
                bool now = AnyActive();
                if (now || now != last) Apply(Active);
                last = now;
            }
        }) { IsBackground = true };
        t.Start();
    }
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
            // Ekran alıntısı HER ŞEYİN üstünde kalmalı: odak değişince pencereler (ve Zebar penceresi) kendini
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
        string path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-lens-" + ts + ".html");
        System.IO.File.WriteAllText(path, html);
        string url = new Uri(path).AbsoluteUri;

        string cmd = BrowserCommand();
        try
        {
            if (!string.IsNullOrEmpty(cmd))
            {
                // "C:\...\zen.exe" -osint -url "%1"  ->  exe + argümanlar
                string exe, rest;
                if (cmd.StartsWith("\"")) { int q = cmd.IndexOf('"', 1); exe = cmd.Substring(1, q - 1); rest = cmd.Substring(q + 1); }
                else { int sp = cmd.IndexOf(' '); exe = sp < 0 ? cmd : cmd.Substring(0, sp); rest = sp < 0 ? "" : cmd.Substring(sp); }
                rest = rest.Contains("%1") ? rest.Replace("%1", url) : rest + " \"" + url + "\"";
                Process.Start(new ProcessStartInfo(exe, rest.Trim()) { UseShellExecute = true });
            }
            else Process.Start(new ProcessStartInfo("msedge.exe", "\"" + url + "\"") { UseShellExecute = true });
        }
        catch { Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }); }
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
                || t.ToString().StartsWith("Zebar - logical-lunge / bar") || !Native.IsWindowVisible(fg);
            if (!emptyFocus) return;
        }
        Native.keybd_event(0xE8, 0, 0, UIntPtr.Zero); Native.keybd_event(0xE8, 0, 2, UIntPtr.Zero); // odak kilidi
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
            Native.keybd_event(0xE8, 0, 0, UIntPtr.Zero); Native.keybd_event(0xE8, 0, 2, UIntPtr.Zero);
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

    public static string PidFile { get { return System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-snip.pid"); } }

    // Önceki alıntı süreci hâlâ çalışıyor ama hiç görünür penceresi yoksa takılıdır: kapat
    public static bool KillStale()
    {
        try
        {
            int pid = int.Parse(System.IO.File.ReadAllText(PidFile).Trim());
            var pr = Process.GetProcessById(pid);
            if (!pr.ProcessName.Equals("ll-helper", StringComparison.OrdinalIgnoreCase)) return false;
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
            pr.Kill(); pr.WaitForExit(1500);
            return true;
        }
        catch { return false; }
    }
}

// ---------------- Pano geçmişi (Super+V: ii "overviewClipboardToggle" / cliphist) ----------------
// Çalışan helper panoyu dinler (WM_CLIPBOARDUPDATE); metinler ve görüntüler %LOCALAPPDATA%\logical-lunge\clipboard'a
// yazılır (en çok 100 kayıt). Overview'da ";" öneki bu listeyi gösterir. Parola yöneticileri gibi geçmişe eklenmesini
// istemeyen uygulamalar (ExcludeClipboardContentFromMonitorProcessing / CanIncludeInClipboardHistory) atlanır.
//   --clip-list        -> [{"id","kind":"text|image","text","lines","thumb","time"}]  (en yeni önce)
//   --clip-set <id>    -> kaydı panoya koyar
//   --clip-del <id> | --clip-clear
static class ClipHistory
{
    const int MAX = 100, MAX_TEXT = 200000;
    static readonly object gate = new object();
    static readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };

    static string Dir
    {
        get
        {
            string d = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"logical-lunge\clipboard");
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
        string hash;
        using (var sha = System.Security.Cryptography.SHA1.Create()) hash = BitConverter.ToString(sha.ComputeHash(png)).Replace("-", "").Substring(0, 16);
        string file = "img-" + hash + ".png";
        lock (gate)
        {
            var l = Load();
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
            string d = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"logical-lunge\update");
            System.IO.Directory.CreateDirectory(d);
            return d;
        }
    }
    static string StatusPath { get { return System.IO.Path.Combine(Dir, "status.json"); } }

    static string Installed()
    {
        try { string f = System.IO.Path.Combine(Home, @".glzr\logical-lunge\VERSION"); if (System.IO.File.Exists(f)) return System.IO.File.ReadAllText(f).Trim(); } catch { }
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
            string body = Client().DownloadString(string.IsNullOrEmpty(api) ? "https://api.github.com/repos/" + Repo + "/releases/latest" : api);
            var r = json.DeserializeObject(body) as Dictionary<string, object>;
            var rel = new Rel { Tag = Convert.ToString(r["tag_name"]), Notes = r.ContainsKey("body") ? Convert.ToString(r["body"]) : "" };
            var assets = r["assets"] as System.Collections.IEnumerable;
            foreach (Dictionary<string, object> a in assets)
            {
                string n = Convert.ToString(a["name"]), u = Convert.ToString(a["browser_download_url"]);
                if (n.StartsWith("LogicalLunge-") && n.EndsWith(".zip")) { rel.ZipName = n; rel.ZipUrl = u; rel.Size = Convert.ToInt64(a["size"]); }
                else if (n.EndsWith(".zip.sha256")) rel.ShaUrl = u;
            }
            if (rel.ZipUrl == null) { err = "none"; return null; }
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
        d["latest"] = r.Tag.TrimStart('v', 'V'); d["tag"] = r.Tag; d["available"] = avail;
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
        string ver = r.Tag.TrimStart('v', 'V'), zip = ZipPath(r), part = zip + ".part";
        if (IsReady(r)) { SetStatus("ready", ver, r.Size, r.Size, ""); return Status(); }
        try
        {
            foreach (var f in System.IO.Directory.GetFiles(Dir, "LogicalLunge-*")) { try { System.IO.File.Delete(f); } catch { } }
            System.Net.ServicePointManager.SecurityProtocol = (System.Net.SecurityProtocolType)3072;
            var req = (System.Net.HttpWebRequest)System.Net.WebRequest.Create(r.ZipUrl);
            req.UserAgent = "LogicalLunge-Updater/1.0";
            req.AllowAutoRedirect = true;
            long got = 0, total = r.Size;
            SetStatus("downloading", ver, 0, total, "");
            using (var resp = req.GetResponse())
            {
                if (resp.ContentLength > 0) total = resp.ContentLength;
                using (var s = resp.GetResponseStream())
                using (var f = new System.IO.FileStream(part, System.IO.FileMode.Create))
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
            if (r.ShaUrl != null)
            {
                string raw = Client().DownloadString(r.ShaUrl);
                var parts = raw.Split(new[] { ' ', '\t', '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries);
                string expected = parts.Length > 0 ? parts[0].ToUpperInvariant() : "";
                string actual;
                using (var sha = System.Security.Cryptography.SHA256.Create())
                using (var fs = System.IO.File.OpenRead(part)) actual = BitConverter.ToString(sha.ComputeHash(fs)).Replace("-", "");
                if (expected.Length > 0 && expected != actual) { try { System.IO.File.Delete(part); } catch { } throw new Exception("Sağlama toplamı uyuşmuyor, indirme geçersiz."); }
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
            string script = System.IO.Path.Combine(Home, @".glzr\logical-lunge\scripts\update-install.ps1");
            if (!System.IO.File.Exists(script)) throw new Exception("update-install.ps1 bulunamadı.");
            // Kurulum betik klasörünü değiştirir: kendi kopyasından çalıştır
            string copy = System.IO.Path.Combine(Dir, "update-install.ps1");
            System.IO.File.Copy(script, copy, true);
            SetStatus("installing", System.IO.Path.GetFileNameWithoutExtension(zip).Replace("LogicalLunge-", ""), 0, 0, "");
            Process.Start(new ProcessStartInfo("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"" + copy + "\" -Zip \"" + zip + "\"")
                { UseShellExecute = false, CreateNoWindow = true, WindowStyle = ProcessWindowStyle.Hidden });
        }
        catch (Exception ex) { SetStatus("error", "", 0, 0, ex.GetBaseException().Message); }
        return Status();
    }
}

// ---------------- Alt+Tab pencere değiştirici ----------------
// Super arama menüsü ve workspace önizlemesi gibi ii görünümünde: koyu yuvarlak panel, canlı DWM önizlemeli kartlar,
// seçili kart mor vurgulu. Tüm workspace'lerdeki pencereler (GlazeWM) son kullanıma göre sıralı; Alt basılı tutulup
// Tab ile ilerlenir (Shift+Tab geri, ok tuşları, Enter, Esc iptal, fare ile tık), Alt bırakılınca seçilen pencere açılır.
// Arayüz ll-helper içinde çizilir (WebView yok): yük altında bile anında açılır.
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
    readonly Glaze glaze = new Glaze();
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
        BackColor = Surface; DoubleBuffered = true; Opacity = 0; Text = "ll-switcher";
        anim.Tick += (o, e) => Tick();
        altWatch.Tick += (o, e) =>
        {
            // Alt bırakıldı ama kanca olayını kaçırdıysa yine de seçimi uygula
            if (!demo && Active && (Native.GetAsyncKeyState(VK_MENU) & 0x8000) == 0) CommitCurrent();
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
            fgCb = OnForeground;
            Native.SetWinEventHook(Native.EVENT_SYSTEM_FOREGROUND, Native.EVENT_SYSTEM_FOREGROUND, IntPtr.Zero, fgCb, 0, 0, 0x0002);
            IntPtr cur = Native.GetAncestor(Native.GetForegroundWindow(), 2);
            if (cur != IntPtr.Zero) fgOrder.Add(cur.ToInt64());
        }));
    }

    static void OnForeground(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
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
            Native.keybd_event(VK_DUMMY, 0, 0, UIntPtr.Zero); Native.keybd_event(VK_DUMMY, 0, 2, UIntPtr.Zero);
            ui.BeginInvoke((Action)(() => inst.CommitCurrent()));
            return false;
        }
        // Alt basılı değilken menü açık kalmış olamaz (kaçırılan bırakma olayı): kilitlenme olmasın, tuşu geçir
        if (!altDown && !demo) { Active = false; ui.BeginInvoke((Action)(() => inst.CloseOnly())); return false; }
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
            foreach (var m in glaze.Monitors())
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

    static Image IconFor(IntPtr h)
    {
        try
        {
            uint pid; Native.GetWindowThreadProcessId(h, out pid);
            string path = Process.GetProcessById((int)pid).MainModule.FileName;
            Image img;
            if (iconCache.TryGetValue(path, out img)) return img;
            using (var ic = Icon.ExtractAssociatedIcon(path)) img = new Bitmap(ic.ToBitmap(), new Size(22, 22));
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
            cards.AddRange(Collect());
            Slider.Log("switcher: " + cards.Count + " pencere");
            if (cards.Count == 0) { Active = false; return; }
            foreach (var c in cards) c.Icon = IconFor(c.H);
            sel = cards.Count > 1 ? (reverse ? cards.Count - 1 : 1) : 0;

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
        if (dy != 0) { int t = sel + dy * perRow; if (t >= 0 && t < n) sel = t; }
        else sel = ((sel + dx) % n + n) % n;
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
                glaze.Command("focus --workspace " + c.Ws);
                ThreadPool.QueueUserWorkItem(_ => { Thread.Sleep(120); try { glaze.Command("focus --container-id " + c.Id); } catch { } });
            }
            else glaze.Command("focus --container-id " + c.Id);
        }
        catch (Exception ex) { Slider.Log("switcher activate: " + ex.Message); }
    }

    // ---- çizim ----
    static GraphicsPathHelper RoundPath(RectangleF r, float rad) { return new GraphicsPathHelper(r, rad); }

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.ClearTypeGridFit;
        g.Clear(Surface);
        using (var bp = new Pen(Border, 1.5f)) g.DrawPath(bp, RoundPath(new RectangleF(0.75f, 0.75f, Width - 2f, Height - 2f), RADIUS).Path);
        var hl = (Environment.TickCount - animStart < 170) ? drawHi : hiTarget;
        if (hl.Width > 0 && cards.Count > 0)
        {
            using (var b = new SolidBrush(SelFill)) g.FillPath(b, RoundPath(hl, 18).Path);
            using (var p = new Pen(SelBorder, 2f)) g.DrawPath(p, RoundPath(hl, 18).Path);
        }
        for (int i = 0; i < cards.Count; i++)
        {
            var c = cards[i];
            var inner = new RectangleF(c.R.X, c.R.Y, c.R.Width, c.R.Height);
            if (i != sel) using (var b = new SolidBrush(CardFill)) g.FillPath(b, RoundPath(inner, 16).Path);
            // önizleme yuvası
            var slot = new RectangleF(c.ThumbR.X, c.ThumbR.Y, c.ThumbR.Width, c.ThumbR.Height);
            using (var b = new SolidBrush(Color.FromArgb(24, 22, 28))) g.FillPath(b, RoundPath(slot, 10).Path);
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

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
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
class GraphicsPathHelper
{
    public System.Drawing.Drawing2D.GraphicsPath Path = new System.Drawing.Drawing2D.GraphicsPath();
    public GraphicsPathHelper(RectangleF r, float rad)
    {
        float d = Math.Min(rad * 2, Math.Min(r.Width, r.Height));
        Path.AddArc(r.X, r.Y, d, d, 180, 90);
        Path.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        Path.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        Path.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
        Path.CloseFigure();
    }
}

// ---------------- Duvar kağıdı (sağ panel > Duvar kağıtları) ----------------
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
            mons.Add(new Dictionary<string, object> { { "id", id }, { "x", r.Left }, { "y", r.Top }, { "w", r.Right - r.Left }, { "h", r.Bottom - r.Top }, { "path", path } });
        }
        return json.Serialize(new Dictionary<string, object> { { "span", w.GetPosition() == SPAN }, { "monitors", mons }, { "dir", Dir } });
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
    // içinde bizim seçimimiz geri yüklenir. Kayıt: satır başına "mod<TAB>yol".
    static string StatePath { get { return System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"logical-lunge\wallpaper.txt"); } }

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

    public static void StartKeeper()
    {
        var t = new Thread(() =>
        {
            int fixes = 0;
            for (int i = 0; i < 40 && fixes < 3; i++) // ~3 dk
            {
                Thread.Sleep(i == 0 ? 15000 : 5000);
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
                    if (fixedOne) fixes++;
                }
                catch { }
            }
        }) { IsBackground = true };
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
    }

    // mode: "all" | "span" | monitör kimliği
    public static void Apply(string path, string mode)
    {
        path = System.IO.Path.GetFullPath(path);
        SetRaw(path, mode);
        SaveState(path, mode);
        // ii switchwall.sh gibi terminal renklerini yeni duvar kağıdından üret (varsa)
        try
        {
            string exe = Environment.ExpandEnvironmentVariables(@"%USERPROFILE%\.glzr\logical-lunge\tools\termcolors\ll-termcolors.exe");
            string tc = Environment.ExpandEnvironmentVariables(@"%USERPROFILE%\.glzr\logical-lunge\tools\termcolors\wezterm-colors.py");
            string py = Environment.ExpandEnvironmentVariables(@"%USERPROFILE%\.glzr\logical-lunge\tools\songrec\venv\Scripts\pythonw.exe");
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

    public static bool SplashActive() { Mutex m; if (Mutex.TryOpenExisting("ll-splash", out m)) { m.Dispose(); return true; } return false; }

    static bool IsWezterm(string path) { return path != null && path.EndsWith("wezterm-gui.exe", StringComparison.OrdinalIgnoreCase); }

    static bool Resident(string path)
    {
        foreach (var pr in Process.GetProcessesByName("wezterm-gui"))
        {
            try { if (string.Equals(pr.MainModule.FileName, path, StringComparison.OrdinalIgnoreCase)) return true; }
            catch { }
            finally { pr.Dispose(); }
        }
        return false;
    }

    public static bool TrySpawn(string path)
    {
        if (!IsWezterm(path) || !Resident(path)) return false;
        try
        {
            System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(Request));
            System.IO.File.WriteAllText(Request, Home);
            Slider.Log("terminal: sıcak açılış (istek dosyası)");
            return true;
        }
        catch { return false; }
    }

    // Oturum açılışında (açılış perdesi ekranı örterken) WezTerm'i başlat ve ilk penceresini kapat:
    // quit_when_all_windows_are_closed = false olduğu için süreç arka planda kalır, ilk Super+Enter da anında açılır.
    public static void Prewarm(string path)
    {
        if (!IsWezterm(path) || !System.IO.File.Exists(path) || Resident(path)) return;
        try
        {
            var pr = Process.Start(new ProcessStartInfo(path) { UseShellExecute = true, WorkingDirectory = Home });
            var sw = Stopwatch.StartNew();
            while (sw.ElapsedMilliseconds < 8000)
            {
                Thread.Sleep(50);
                pr.Refresh();
                IntPtr h = pr.MainWindowHandle;
                if (h != IntPtr.Zero)
                {
                    Native.PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero); // WM_CLOSE
                    Slider.Log("terminal: ön-ısıtıldı " + sw.ElapsedMilliseconds + "ms");
                    return;
                }
            }
        }
        catch (Exception ex) { Slider.Log("terminal ön-ısıtma: " + ex.Message); }
    }
}
// ---------------- Açılış perdesi ----------------
// Windows görev çubuğu, Başlat düğmesi ve ses/parlaklık OSD'si hiç görünmez: işlerini Zebar'daki bar ve OSD görüyor.
// Windows 10'da ana görev çubuğunu kaldıran bir registry ayarı yok; yalnızca otomatik gizleme ve diğer monitörlerde
// kapatma var (kurulum ikisini de yapıyor). Explorer onu kendisi yeniden gösterebiliyor (bir uygulama düğmesini yanıp
// söndürünce, Explorer yeniden başlayınca...): göründüğü anda (EVENT_OBJECT_SHOW) gizlenir. Eskiden bunu hide-taskbar.ps1
// 700 ms'lik yoklamayla yapıyordu ve görev çubuğu o arada "yanıp gidiyordu". LL kapanınca show-taskbar.ps1 geri getirir.
// Bizim kabuk (Zebar'daki bar) ayakta mı. Bar 20 sn'den uzun yoksa (Zebar ya da GlazeWM çöktü / açılamadı) helper
// güvenli tarafa açılır: Windows görev çubuğu ve Win tuşu (Başlat menüsü) geri gelir, kullanıcı hiçbir zaman barsız,
// görev çubuğusuz ve Başlat'sız kalmaz. Bar dönünce ikisi yine bizim. (Kısa Zebar yeniden başlatmaları sayılmaz.)
static class ShellState
{
    static volatile bool up = true;
    static int missingSince = -1;
    public static bool Up { get { return up; } }

    // Durum değiştiyse true (TaskbarGuard'ın 2 sn'lik zamanlayıcısından)
    public static bool Update()
    {
        bool bar = Native.FindWindowEx(IntPtr.Zero, IntPtr.Zero, null, "Zebar - logical-lunge / bar") != IntPtr.Zero;
        if (bar)
        {
            missingSince = -1;
            if (up) return false;
            up = true;
            Slider.Log("bar geri geldi: görev çubuğu ve Win tuşu yine kabuğun");
            return true;
        }
        if (missingSince < 0) { missingSince = Environment.TickCount; return false; }
        if (!up || Environment.TickCount - missingSince < 20000) return false;
        up = false;
        Slider.Log("bar 20 sn'dir yok: Windows görev çubuğu ve Başlat menüsü geri açıldı");
        return true;
    }
}

// ---------------- Odak bekçisi ----------------
// Hyprland'de odak hiç boşta kalmaz. Windows'ta ise odaktaki pencere kapanınca ya da ekran alıntısı, bir iletişim kutusu,
// bildirim kapanınca ön plan masaüstüne, bar'a, gizli (başka workspace'teki) bir pencereye ya da hiçbir şeye düşebiliyordu:
// klavye bir yere gitmiyor, fareyle tıklamak gerekiyordu. Bu durum ~0,75 sn sürerse bekçi odağı görünen workspace'te
// GlazeWM'in odaklı saydığı pencereye (yoksa imlecin altındakine) geri verir. Boş workspace'te, fare tuşu basılıyken, açık
// bir sağ tık menüsünde, kilit ekranında, bakımda ve kabuk yokken (Windows görev çubuğu modu) karışmaz.
static class FocusGuard
{
    [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h, int index, StringBuilder info, int len, out int needed);

    static readonly Glaze glaze = new Glaze();

    public static void Start()
    {
        new Thread(Loop) { IsBackground = true, Name = "focus-guard" }.Start();
    }

    static void Loop()
    {
        int lostSince = -1, fails = 0, waitUntil = Environment.TickCount;
        while (true)
        {
            Thread.Sleep(250);
            try
            {
                string why = Lost();
                if (why == null) { lostSince = -1; fails = 0; continue; }
                int now = Environment.TickCount;
                if (lostSince < 0) { lostSince = now; continue; }
                if (now - lostSince < 750 || now - waitUntil < 0) continue;
                int r = Refocus(why);
                if (r > 0) { lostSince = -1; fails = 0; }
                else if (r == 0) waitUntil = now + 1500;          // boş workspace: masaüstü odağı olağan, arada bir bak
                else if (++fails >= 3) { waitUntil = now + 15000; fails = 0; Slider.Log("odak bekçisi: odak verilemedi, 15 sn bekleniyor"); }
                else lostSince = now;
            }
            catch (Exception ex) { Slider.Log("odak bekçisi: " + ex.GetBaseException().Message); Thread.Sleep(2000); }
        }
    }

    // Kilit ekranı / ekran koruyucusu / UAC girdi masaüstündeyken ön plan sorgusu anlamsız
    static bool OnDefaultDesktop()
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
        if (title == "Zebar - logical-lunge / bar" || title == "Zebar - logical-lunge / toast" || title == "Zebar - logical-lunge / update")
        {
            Native.RECT r;
            var p = Cursor.Position;
            if (Native.GetWindowRect(root, out r) && p.X >= r.Left && p.X < r.Right && p.Y >= r.Top && p.Y < r.Bottom) return null;
            return "bar";
        }
        return null;
    }

    // 1: odak verildi, 0: verilecek pencere yok (boş workspace), -1: verilemedi
    static int Refocus(string why)
    {
        Dictionary<string, object> ws = null;
        foreach (var m in glaze.Monitors())
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
        if (pick == null) return 0;
        var hw = new IntPtr(Convert.ToInt64(pick["handle"]));
        // Önce GlazeWM üzerinden (durumu da güncel kalsın); o pencereyi zaten odaklı sayıyorsa ön plana getirmeyebilir
        glaze.Command("focus --container-id " + J.Str(pick, "id"));
        Thread.Sleep(150);
        if (Lost() != null)
        {
            Native.keybd_event(0xE8, 0, 0, UIntPtr.Zero); Native.keybd_event(0xE8, 0, 2, UIntPtr.Zero); // odak kilidi
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

static class TaskbarGuard
{
    static Native.WinEventDelegate cb;
    static System.Windows.Forms.Timer timer;

    // Mesaj döngüsü olan bir thread'den çağrılır: hook ve zamanlayıcı o thread'de çalışır.
    // failOpen (asıl helper): bizim bar'ımız yoksa görev çubuğu geri açılır (ShellState). Açılış perdesi her zaman gizler.
    public static void Install(bool failOpen = false)
    {
        if (cb != null) return;
        FailOpen = failOpen;
        cb = (hook, ev, h, idObject, idChild, thread, time) => { if (idObject == 0 && h != IntPtr.Zero) Hide(h); };
        Native.SetWinEventHook(Native.EVENT_OBJECT_SHOW, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, cb, 0, 0, 0x0002);
        Sweep();
        // Yoğunlukta kaçan olay olursa diye seyrek yedek tarama; bar'ın durumu da burada izlenir
        timer = new System.Windows.Forms.Timer { Interval = 2000 };
        timer.Tick += (s, e) =>
        {
            if (FailOpen && ShellState.Update() && !ShellState.Up) ShowAll();
            else Sweep();
        };
        timer.Start();
    }

    static bool FailOpen;

    // Güvenli tarafa açılma: görev çubukları ve Başlat düğmesi yeniden görünür (otomatik gizlemede kenara gelince açılır)
    static void ShowAll()
    {
        Native.EnumWindows(delegate (IntPtr h, IntPtr l)
        {
            string cs = Cls(h);
            if (cs == "Shell_TrayWnd" || cs == "Shell_SecondaryTrayWnd" || (cs == "Button" && Cls(Native.GetWindow(h, 4)).StartsWith("Shell_")))
                Native.ShowWindowAsync(h, 8); // SW_SHOWNA
            return true;
        }, IntPtr.Zero);
    }

    static void Sweep()
    {
        Native.EnumWindows(delegate (IntPtr h, IntPtr l) { if (Native.IsWindowVisible(h)) Hide(h); return true; }, IntPtr.Zero);
    }

    static string Cls(IntPtr h)
    {
        var c = new StringBuilder(64);
        Native.GetClassName(h, c, 64);
        return c.ToString();
    }

    static void Hide(IntPtr h)
    {
        if (FailOpen && !ShellState.Up) return;
        string cs = Cls(h);
        // Başlat düğmesi: görev çubuğunun sahip olduğu ayrı bir üst pencere (Button)
        if (cs == "Shell_TrayWnd" || cs == "Shell_SecondaryTrayWnd" || (cs == "Button" && Cls(Native.GetWindow(h, 4)).StartsWith("Shell_")))
            Native.ShowWindowAsync(h, 0); // SW_HIDE; Explorer askıdaysa beklemez
        // Ses/parlaklık/medya OSD'si (NativeHWNDHost > DirectUIHWND): küçültülmüş host bir daha görünmez (HideVolumeOSD'nin yöntemi)
        else if (cs == "NativeHWNDHost" && !Native.IsIconic(h) && Native.FindWindowEx(h, IntPtr.Zero, "DirectUIHWND", null) != IntPtr.Zero)
            Native.ShowWindowAsync(h, 6); // SW_MINIMIZE
    }
}

static class Splash
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);

    static bool Ready()
    {
        // GlazeWM IPC portu açık ve Zebar bar penceresi var mı
        try { using (var c = new System.Net.Sockets.TcpClient()) { if (!c.ConnectAsync("127.0.0.1", 6123).Wait(150)) return false; } }
        catch { return false; }
        return FindWindow(null, "Zebar - logical-lunge / bar") != IntPtr.Zero;
    }

    // Kilitli olabilir (Superpaper gibi araçlar dosyayı yeniden yazarken) -> paylaşımlı aç; olmazsa son iyi kopya.
    static string CachePath { get { return System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"logical-lunge\splash-wall.jpg"); } }
    static bool SpanStyle()
    {
        string st = (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Control Panel\Desktop", "WallpaperStyle", null);
        return st == "22";
    }

    static Image Wallpaper()
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

    class Cover : Form
    {
        readonly Image img;
        readonly Rectangle virt; // span resmi bu dikdörtgene yayılır (boşsa her ekran kendi resmini doldurur)
        public Cover(Rectangle b, Image img, Rectangle virt)
        {
            this.img = img; this.virt = virt;
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true;
            StartPosition = FormStartPosition.Manual; Bounds = b;
            BackColor = Color.FromArgb(20, 19, 24);
            DoubleBuffered = true;
        }
        protected override CreateParams CreateParams
        {
            get { var p = base.CreateParams; p.ExStyle |= 0x80 | 0x08000000; return p; } // TOOLWINDOW | NOACTIVATE
        }
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override void OnPaint(PaintEventArgs e)
        {
            if (img == null) return;
            e.Graphics.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
            // "Doldur" yerleşimi: en boy oranını koruyup alanı kapla, taşanı ortadan kırp. Span'da alan tüm
            // masaüstüdür ve her ekran kendi dilimini gösterir (Windows'un "Yay" yerleşimi).
            Rectangle area = virt.IsEmpty ? new Rectangle(0, 0, Width, Height) : virt;
            double k = Math.Max((double)area.Width / img.Width, (double)area.Height / img.Height);
            int w = (int)Math.Ceiling(img.Width * k), h = (int)Math.Ceiling(img.Height * k);
            int x = area.X + (area.Width - w) / 2 - Left, y = area.Y + (area.Height - h) / 2 - Top;
            if (virt.IsEmpty) { x = (Width - w) / 2; y = (Height - h) / 2; }
            e.Graphics.DrawImage(img, x, y, w, h);
        }
    }

    public static void Run()
    {
        bool created;
        using (var m = new Mutex(true, "ll-splash", out created))
        {
            if (!created) return;
            // Güncelleme sırasında (LL_SPLASH_WAIT_RESTART=1): örtü yumuşakça belirir, önce mevcut masaüstünün kapanmasını,
            // sonra yenisinin hazır olmasını bekler (en fazla 150 sn).
            bool restartMode = Environment.GetEnvironmentVariable("LL_SPLASH_WAIT_RESTART") == "1";
            bool sawDown = !restartMode;
            int maxMs = restartMode ? 150000 : 30000;
            var img = Wallpaper();
            var covers = new List<Cover>();
            var virt = SpanStyle() ? SystemInformation.VirtualScreen : Rectangle.Empty;
            foreach (var s in Screen.AllScreens) { var f = new Cover(s.Bounds, img, virt); f.Show(); covers.Add(f); }
            TaskbarGuard.Install(); // görev çubuğu açılıştan itibaren görünmesin
            if (restartMode)
            {
                foreach (var f in covers) f.Opacity = 0;
                var fin = new System.Windows.Forms.Timer { Interval = 15 };
                int fis = Environment.TickCount;
                fin.Tick += (o3, e3) =>
                {
                    double t3 = Math.Min(1, (Environment.TickCount - fis) / 450.0);
                    foreach (var f in covers) f.Opacity = 1 - Math.Pow(1 - t3, 3);
                    if (t3 >= 1) fin.Stop();
                };
                fin.Start();
            }

            var start = Environment.TickCount;
            int readyAt = -1;
            var timer = new System.Windows.Forms.Timer { Interval = 100 };
            timer.Tick += (o, e) =>
            {
                int now = Environment.TickCount;
                foreach (var f in covers) Native.SetWindowPos(f.Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // en üstte kal
                if (!sawDown) { if (!Ready()) sawDown = true; }
                else if (readyAt < 0 && Ready()) readyAt = now;
                // Hazır olduktan sonra pencerelerin yerleşip bar'ın çizilmesi için kısa bir süre; en fazla 30 sn bekle
                bool done = (readyAt >= 0 && now - readyAt > 1500) || now - start > maxMs;
                if (!done) return;
                timer.Stop();
                ThreadPool.QueueUserWorkItem(_ => SaveCache());
                var fade = new System.Windows.Forms.Timer { Interval = 15 };
                int fs = Environment.TickCount;
                fade.Tick += (o2, e2) =>
                {
                    double t = Math.Min(1, (Environment.TickCount - fs) / 350.0);
                    double eased = 1 - Math.Pow(1 - t, 3);
                    foreach (var f in covers) f.Opacity = 1 - eased;
                    if (t >= 1) { fade.Stop(); Application.ExitThread(); }
                };
                fade.Start();
            };
            timer.Start();
            Application.Run();
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
                    if (!WarmTerminal.TrySpawn(path))
                        Process.Start(new ProcessStartInfo(path)
                        {
                            UseShellExecute = true,
                            WorkingDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
                        });
                }
                catch (Exception ex)
                {
                    Toasts.Send("error", "Açılamadı", System.IO.Path.GetFileName(path) + ": " + ex.Message, "error");
                    continue;
                }
                // Pencere GlazeWM'e gelene kadar bekle (en fazla 3 sn), sonra biraz yerleşsin
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
    public static DialogCatcher Dialogs;
    public static Rounder Round;
}

static class Program
{
    // Windows komut satırı kurallarına göre tek argümanı tırnakla
    static string QuoteArg(string a)
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
        // ll-helper.exe --splash: oturum açılınca (LL\Splash görevi) masaüstünü duvar kağıdıyla örter;
        // GlazeWM ve bar hazır olup pencereler dizilince yumuşakça kaybolur. Windows'un çıplak hali hiç görünmez.
        if (args.Length == 1 && args[0] == "--splash") { Splash.Run(); return; }
        // ll-helper.exe --audio-default <endpoint kimliği>: varsayılan çıkış/giriş cihazını değiştir -> {"ok":true}
        if (args.Length == 2 && args[0] == "--audio-default")
        {
            int hr;
            try { hr = AudioDefault.Set(args[1]); } catch (Exception ex) { hr = Marshal.GetHRForException(ex); }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write("{\"ok\":" + (hr == 0 ? "true" : "false") + ",\"hr\":" + hr + "}"); so.Flush();
            return;
        }        // ll-helper.exe --songrec [-i 2 -t 30 -s monitor]: müzik tanıma exe'sini konsolsuz çalıştır, sonucu aktar.
        // Overview düğmesi bu süreci durdurursa (kill) Job Object sayesinde tanıma da hemen kapanır.
        if (args.Length >= 1 && args[0] == "--songrec")
        {
            string exe = Environment.ExpandEnvironmentVariables(@"%USERPROFILE%\.glzr\logical-lunge\tools\songrec\ll-songrec.exe");
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
        }        // Duvar kağıdı: --wall-info | --wall-local | --wall-browse <tür> [sayfa] | --wall-get <url> <mod>
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
                    case "--wall-thumb": outText = Wallpaper.Thumb(args[1]); break;
                    case "--wall-pick": outText = new JavaScriptSerializer().Serialize(Wallpaper.Pick(args.Length > 1 ? args[1] : "all")); break;
                    default: outText = "{\"error\":\"unknown\"}"; break;
                }
            }
            catch (Exception ex) { outText = new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "error", ex.GetBaseException().Message } }); }
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(outText); so.Flush();
            return;
        }        // ll-helper.exe --capture: ana helper'dan bir kısayol yakalamasını iste, sonucu yaz ("" = iptal)
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
        // ll-helper.exe --bind <id> <combo> | --bind-reset
        if (args.Length == 3 && args[0] == "--bind") { Binds.Set(args[1], args[2]); return; }
        if (args.Length == 1 && args[0] == "--bind-reset") { Binds.Set("", null); return; }
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
        // ll-helper.exe --switcher-demo: Alt+Tab menüsünü 6 sn göster (sınama; kısayolsuz)
        if (args.Length == 1 && args[0] == "--switcher-demo") { Switcher.Demo(); return; }
        // ll-helper.exe --log <metin>: widget'ların hata ayıklama günlüğü (%TEMP%\ll-helper.log)
        if (args.Length == 2 && args[0] == "--log") { Slider.Log("widget: " + args[1]); return; }
        // ll-helper.exe --overview-show clip|plain: overview'u ilgili modda aç (test / betik için; Super / Super+V aynısını yapar)
        if (args.Length == 2 && args[0] == "--overview-show")
        {
            IntPtr ovh = Native.FindWindow(null, "ll-overview");
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
                        ct = Keys2.TakeOverviewMode(); // bir kez okunur ve silinir: "" ya da ";" (pano)
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
        // ll-helper.exe --snip-screen: farenin olduğu monitörün tamamı, sormadan panoya + dosyaya
        if (args.Length == 1 && args[0] == "--snip-screen")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            SnipTool.RunScreen();
            return;
        }
        // ll-helper.exe --snip: bölge ekran alıntısı + düzenleme (Hyprland Print: grim + slurp + swappy)
        if (args.Length >= 1 && args.Length <= 2 && args[0] == "--snip")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            int wait; // --snip 350: paneller kapansın diye önce bekle
            if (args.Length == 2 && int.TryParse(args[1], out wait)) Thread.Sleep(Math.Min(2000, wait));
            bool fresh;
            var sm = new Mutex(true, "ll-snip", out fresh);
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
        // ll-helper.exe --open <https://... | spotify:...>: bağlantıyı varsayılan uygulamada aç. explorer.exe'ye
        // verilen adres "&" içerince klasör açıyordu; ShellExecute doğrudan protokol işleyicisine gider.
        if (args.Length == 2 && args[0] == "--open" && System.Text.RegularExpressions.Regex.IsMatch(args[1], "^(https?|spotify|mailto):", System.Text.RegularExpressions.RegexOptions.IgnoreCase))
        {
            try { Process.Start(new ProcessStartInfo(args[1]) { UseShellExecute = true }); } catch { }
            return;
        }        // ll-helper.exe --lens: ii "region search" — alan seç, Google Lens'te aç
        if (args.Length == 1 && args[0] == "--lens")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            RegionSearch.Lens();
            return;
        }
        // ll-helper.exe --toast-stream: çalışan helper'ın bildirim kanalına bağlanıp her bildirimi
        // stdout'a tek satır JSON yazar. Zebar toast widget'ı bunu shellSpawn ile okur (widget'ların
        // yerel adreslere doğrudan bağlanmasına Zebar izin vermiyor).
        if (args.Length == 1 && args[0] == "--toast-stream")
        {
            // Zebar (ebeveyn) kapanınca bu kopya da kapansın: yoksa Zebar'dan miras aldığı sunucu
            // soketini tutarak yeni Zebar'ın açılmasını engelliyor.
            int parent = ParentPid();
            new Thread(() =>
            {
                while (true)
                {
                    Thread.Sleep(1500);
                    try { if (parent <= 0 || Process.GetProcessById(parent).HasExited) Environment.Exit(0); }
                    catch { Environment.Exit(0); }
                }
            }) { IsBackground = true }.Start();
            var stdout = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false)) { AutoFlush = true };
            // Helper'ın koruyucusu: asıl helper (Super, animasyonlar, pano...) tamamen kapanmışsa ve ~10 sn içinde geri
            // gelmediyse onu başlatır. GlazeWM kapalıysa (kasıtlı çıkış) ya da bakım sırasında karışmaz.
            int refused = 0;
            while (true)
            {
                try
                {
                    if (refused >= 5)
                    {
                        refused = 0;
                        System.Threading.Mutex existing;
                        bool alive = System.Threading.Mutex.TryOpenExisting("ll-helper-single", out existing);
                        if (existing != null) existing.Dispose();
                        if (!alive && Maint.Running("glazewm") && !Maint.Quiet() && Maint.Allow("helper-restarts"))
                            Process.Start(new ProcessStartInfo(Maint.HelperExe) { UseShellExecute = true, WorkingDirectory = AppDomain.CurrentDomain.BaseDirectory });
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

        // ll-helper.exe --ps <script.ps1> [argümanlar]   : PowerShell'i HİÇ pencere açmadan çalıştırır,
        //                                                   çıktısını kendi stdout'una aktarır
        // ll-helper.exe --ps-bg <script.ps1> [argümanlar]: aynı, ama beklemeden arka planda bırakır
        // (Zebar'dan doğrudan powershell çağırmak bir anlık konsol penceresi gösterebiliyordu.)
        if (args.Length >= 2 && args[0] == "--ps-bg")
        {
            // Uzun ömürlü arka plan betiği (uyanık tut vb.): ShellExecute ile başlat ki Zebar'dan
            // miras kalan soket/tanıtıcıları DEVRALMASIN. Aksi halde Zebar kapanınca 6124 portu
            // bu süreçte asılı kalıyor ve yeni Zebar sunucusunu açamıyor (bar boş geliyordu).
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

        // ll-helper.exe --mic toggle|on|off|status -> {"muted":true}  (on = mikrofon açık)
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

        // ll-helper.exe --raise "<pencere başlığı>" : pencereyi her zaman üstte yapıp en öne getir.
        // (Zebar'ın setAlwaysOnTop'u gizle/göster sonrası etkisiz kalıyordu; sağ panel terminalin arkasında açılıyordu.)
        // --top: yalnızca en üste al, odak verme (ekran klavyesi: tuşlar yazılan uygulamaya gitmeli)
        if (args.Length == 2 && (args[0] == "--raise" || args[0] == "--top"))
        {
            IntPtr rh = Native.FindWindow(null, args[1]);
            if (rh == IntPtr.Zero) return;
            Native.SetWindowPos(rh, new IntPtr(-1) /*HWND_TOPMOST*/, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0040 | (args[0] == "--top" ? 0x0010u : 0u) /*NOSIZE|NOMOVE|SHOWWINDOW|NOACTIVATE*/);
            if (args[0] == "--top") return;
            Native.keybd_event(0xE8, 0, 0, UIntPtr.Zero); Native.keybd_event(0xE8, 0, 2, UIntPtr.Zero); // önplan izni
            Native.SetForegroundWindow(rh);
            return;
        }

        // ll-helper.exe --gamma <\\.\DISPLAY1> [0-100]  -> {"gamma":60,"ok":true}  (değer yoksa yalnızca okur)
        if ((args.Length == 2 || args.Length == 3) && args[0] == "--gamma")
        {
            bool ok = true; int gv;
            if (args.Length == 3 && int.TryParse(args[2], out gv)) ok = NightLight.SetGamma(args[1], gv);
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write("{\"gamma\":" + NightLight.Gamma(args[1]) + ",\"ok\":" + (ok ? "true" : "false") + "}"); so.Flush();
            return;
        }

        // ll-helper.exe --nightlight on|off|toggle|status  -> {"on":true}
        if (args.Length == 2 && args[0] == "--nightlight")
        {
            if (args[1] == "on") NightLight.Enabled = true;
            else if (args[1] == "off") NightLight.Enabled = false;
            else if (args[1] == "toggle") NightLight.Enabled = !NightLight.Enabled;
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(NightLight.StatusJson()); so.Flush();
            return;
        }
        // ll-helper.exe --nightlight-set level 60 | mode manual|after|range | from 20:00 | to 07:00
        if (args.Length == 3 && args[0] == "--nightlight-set" && System.Text.RegularExpressions.Regex.IsMatch(args[1], "^(level|mode|from|to)$"))
        {
            NightLight.Set(args[1], args[2]);
            var so = new System.IO.StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
            so.Write(NightLight.StatusJson()); so.Flush();
            return;
        }

        // Tek seferlik: ll-helper.exe --focus-under-cursor  (overview uygulama açmadan önce çağırır,
        // yeni pencere Hyprland dwindle'daki gibi farenin altındaki pencereyi bölsün)
        if (args.Length == 1 && args[0] == "--focus-under-cursor")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            new Slider(new Glaze()).FocusUnderCursor();
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
            new Slider(new Glaze()).SelfTest();
            return;
        }

        // Tek seferlik: ll-helper.exe --slide next|prev|<workspace>  (bar tıklamaları ve test için)
        if (args.Length == 2 && args[0] == "--slide")
        {
            // Çalışan helper varsa işi ona devret (katman ve kenarlıkları hazır, animasyon hemen başlar)
            try
            {
                string act = args[1] == "next" ? "ws-next" : args[1] == "prev" ? "ws-prev" : "ws-" + args[1];
                var rq = (System.Net.HttpWebRequest)System.Net.WebRequest.Create("http://127.0.0.1:6131/cmd?a=" + Uri.EscapeDataString(act));
                rq.Timeout = 400; rq.Proxy = null;
                using (var rs = (System.Net.HttpWebResponse)rq.GetResponse()) if ((int)rs.StatusCode == 204) return;
            }
            catch { }
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            var g = new Glaze();
            var sl = new Slider(g);
            if (args[1] == "next") sl.Run(new[] { "focus --next-workspace" }, 1, null);
            else if (args[1] == "prev") sl.Run(new[] { "focus --prev-workspace" }, -1, null);
            else sl.Run(new[] { "focus --workspace " + args[1] }, 0, args[1]);
            return;
        }

        // Yakalanmayan her hatayı yığın iziyle log'a yaz (sessiz çökme olmasın)
        AppDomain.CurrentDomain.UnhandledException += (s, e) =>
        {
            Slider.Log("CRASH: " + e.ExceptionObject);
            if (e.IsTerminating) SelfHeal.Respawn("çöktü: " + (e.ExceptionObject is Exception ? e.ExceptionObject.GetType().Name : "?"));
        };
        Application.ThreadException += (s, e) => Slider.Log("UI HATA: " + e.Exception);
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);

        bool created;
        var mutex = new Mutex(true, "ll-helper-single", out created);
        // Kendini yeniden başlatan kopya: eskisi kapanıp kilidi bırakana kadar bekle
        if (!created && args.Length == 1 && args[0] == "--respawn")
        {
            try { created = mutex.WaitOne(15000); }
            catch (AbandonedMutexException) { created = true; }
        }
        if (!created) return;
        SelfHeal.IsMain = true;
        Microsoft.Win32.SystemEvents.SessionEnding += (s0, e0) => { Maint.SessionEnding = true; };
        try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { } // PER_MONITOR_AWARE_V2

        Application.EnableVisualStyles();
        var ui = new Form { ShowInTaskbar = false, WindowState = FormWindowState.Minimized, FormBorderStyle = FormBorderStyle.None, Opacity = 0 };
        ui.Load += (s, e) => ui.Hide();
        var h = ui.Handle;

        var glaze = new Glaze();
        var slider = new Slider(glaze);
        slider.Warm();
        // Monitör takıldı/çıkarıldı ya da çözünürlük değişti: yeni dikdörtgenlerin katmanı da hazır beklesin
        Microsoft.Win32.SystemEvents.DisplaySettingsChanged += (s0, e0) => { try { ui.BeginInvoke((Action)slider.Warm); } catch { } };
        Slider.Ui = ui;
        var dwindle = new Dwindle(new Glaze(), ui, slider);
        dwindle.Start(); // kendi IPC bağlantısıyla: slide'ı beklemesin
        dwindle.HookNewWindows();
        LaunchQueue.Start(new Slider(new Glaze())); // kendi bağlantısı: animasyonu beklemesin
        NightLight.StartKeeper();
        Switcher.Init(ui);
        ClipHistory.StartListener();
        Wallpaper.StartKeeper();
        Toasts.Start();
        // Windows'a verilen callback'lerin sahibi nesneler canlı kalmalı: aksi halde çöp toplayıcı
        // onları siler ve Windows silinmiş fonksiyonu çağırınca helper sessizce çöker.
        var dialogThread = new Thread(() => { Keep.Dialogs = new DialogCatcher(); Keep.Dialogs.Start(); Application.Run(); });
        dialogThread.SetApartmentState(ApartmentState.STA);
        dialogThread.IsBackground = true;
        dialogThread.Start();

        // Klavye kancası KENDİ thread'inde ve orada başka hiçbir iş yapılmaz: LL hook ~300ms'de
        // yanıt vermezse Windows kancayı söker ve o sırada klavye donar. (Eskiden köşe yuvarlama
        // aynı thread'deydi; SetWindowRgn askıdaki bir pencerede bekleyince klavye donuyordu.)
        var hookThread = new Thread(() =>
        {
            Binds.Watch();
            var keys = new Keys2(ui, slider);
            keys.Start();
            keys.StartTestPipe();
            var mouse = new MouseFocus(new Glaze());
            mouse.InstallHook();
            mouse.StartWorker();
            // Windows kancayı bir şekilde sökse bile geri gelsin
            var re = new System.Windows.Forms.Timer { Interval = 15000 };
            re.Tick += (s, e) => { keys.Reinstall(); mouse.Reinstall(); };
            re.Start();
            Application.Run();
        });
        hookThread.SetApartmentState(ApartmentState.STA);
        hookThread.IsBackground = true;
        hookThread.Priority = ThreadPriority.Highest;
        hookThread.Start();

        var roundThread = new Thread(() =>
        {
            Keep.Round = new Rounder(); Keep.Round.Start();
            TaskbarGuard.Install(true);
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
                string apps = System.IO.Path.Combine(home, @".glzr\zebar\logical-lunge\apps.json");
                string build = System.IO.Path.Combine(home, @".glzr\logical-lunge\scripts\build-apps.ps1");
                if (!System.IO.File.Exists(apps) && System.IO.File.Exists(build))
                    Process.Start(new ProcessStartInfo("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"" + build + "\"") { UseShellExecute = false, CreateNoWindow = true });
                string colors = System.IO.Path.Combine(home, @".config\wezterm\ll-colors.lua");
                string tc = System.IO.Path.Combine(home, @".glzr\logical-lunge\tools\termcolors\ll-termcolors.exe");
                if (!System.IO.File.Exists(colors) && System.IO.File.Exists(tc))
                    Process.Start(new ProcessStartInfo(tc) { UseShellExecute = false, CreateNoWindow = true });
                // Yalnızca oturum açılışında (perde ekranı örterken); sonradan helper yeniden başlarsa pencere göstermesin
                if (WarmTerminal.SplashActive())
                    WarmTerminal.Prewarm(Keys2.TerminalPath);
            }
            catch (Exception ex) { Slider.Log("first run: " + ex.Message); }
        });
        // Arkada derleme / oyun / güncelleme CPU'yu doldursa da kayma ve odak gecikmesin: helper ve
        // GlazeWM yüksek öncelikte (GlazeWM yeniden başlarsa diye 10 sn'de bir yenilenir; yönetici gerekmez).
        try { Process.GetCurrentProcess().PriorityClass = ProcessPriorityClass.High; } catch { }
        var prioThread = new Thread(() =>
        {
            while (true)
            {
                foreach (var name in new[] { "glazewm", "zebar" })
                    foreach (var pr in Process.GetProcessesByName(name))
                        try
                        {
                            var want = name == "glazewm" ? ProcessPriorityClass.High : ProcessPriorityClass.AboveNormal;
                            if (pr.PriorityClass != want) pr.PriorityClass = want;
                        }
                        catch { }
                        finally { pr.Dispose(); }
                Thread.Sleep(10000);
            }
        }) { IsBackground = true, Priority = ThreadPriority.Lowest };
        prioThread.Start();

        ZebarWatchdog.Start();
        WmWatchdog.Start();
        FocusGuard.Start();
        SelfHeal.WatchUi(ui);

        Application.Run(ui);
        GC.KeepAlive(mutex);
    }
}
