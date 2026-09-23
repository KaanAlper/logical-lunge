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
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int v, int size);
    [DllImport("dwmapi.dll")] public static extern int DwmRegisterThumbnail(IntPtr dest, IntPtr src, out IntPtr thumb);
    [DllImport("dwmapi.dll")] public static extern int DwmUnregisterThumbnail(IntPtr thumb);
    [DllImport("dwmapi.dll")] public static extern int DwmUpdateThumbnailProperties(IntPtr thumb, ref DWM_THUMBNAIL_PROPERTIES p);
    [DllImport("dwmapi.dll")] public static extern int DwmFlush();
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
class Overlay : Form
{
    public Overlay()
    {
        FormBorderStyle = FormBorderStyle.None;
        ShowInTaskbar = false;
        TopMost = true;
        BackColor = Color.Black;
        StartPosition = FormStartPosition.Manual;
        Text = "ll-slide";
    }
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

// Animasyon katmanı gerçek pencereleri örttüğü için tacky-borders'ın mor kenarlığı animasyon boyunca görünmüyordu.
// Odaklı pencerenin kenarlığını katmanın üstünde, pencereyle birlikte biz çiziyoruz (tacky-borders ayarından:
// active_color, border_width, border_radius).
class Ring : Form
{
    [DllImport("gdi32.dll")] static extern int CombineRgn(IntPtr dest, IntPtr a, IntPtr b, int mode);
    readonly int bw = 2, radius = 14;
    int lastW = -1, lastH = -1;
    bool shown;

    public Ring()
    {
        FormBorderStyle = FormBorderStyle.None;
        ShowInTaskbar = false;
        TopMost = true;
        StartPosition = FormStartPosition.Manual;
        Text = "ll-ring";
        var color = Color.FromArgb(0xb6, 0x9d, 0xf8);
        double alpha = 0.8;
        try
        {
            string cfg = System.IO.File.ReadAllText(System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), @".config\tacky-borders\config.yaml"));
            var m = System.Text.RegularExpressions.Regex.Match(cfg, @"active_color:\s*""?#([0-9a-fA-F]{6})([0-9a-fA-F]{2})?");
            if (m.Success)
            {
                int rgb = Convert.ToInt32(m.Groups[1].Value, 16);
                color = Color.FromArgb((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
                if (m.Groups[2].Success) alpha = Convert.ToInt32(m.Groups[2].Value, 16) / 255.0;
            }
            m = System.Text.RegularExpressions.Regex.Match(cfg, @"border_width:\s*(\d+)");
            if (m.Success) bw = Math.Max(1, int.Parse(m.Groups[1].Value));
            m = System.Text.RegularExpressions.Regex.Match(cfg, @"border_radius:\s*(\d+)");
            if (m.Success) radius = int.Parse(m.Groups[1].Value);
        }
        catch { }
        BackColor = color;
        Opacity = alpha;
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= Native.WS_EX_TOOLWINDOW | Native.WS_EX_NOACTIVATE | Native.WS_EX_TOPMOST | 0x20; // WS_EX_TRANSPARENT
            return cp;
        }
    }

    // r: pencerenin görünen çerçevesi, ekran koordinatlarında. Halka çerçeve kenarının üstüne ortalanır.
    public void Place(Native.RECT r)
    {
        int x = r.Left - bw / 2, y = r.Top - bw / 2;
        int w = r.Right - r.Left + bw, h = r.Bottom - r.Top + bw;
        if (w <= 2 * bw || h <= 2 * bw) return;
        if (w != lastW || h != lastH)
        {
            IntPtr outer = Native.CreateRoundRectRgn(0, 0, w + 1, h + 1, 2 * radius, 2 * radius);
            IntPtr inner = Native.CreateRoundRectRgn(bw, bw, w - bw + 1, h - bw + 1, 2 * Math.Max(0, radius - bw), 2 * Math.Max(0, radius - bw));
            CombineRgn(outer, outer, inner, 4); // RGN_DIFF
            Native.DeleteObject(inner);
            Native.SetWindowRgn(Handle, outer, false); // bölgenin sahibi artık pencere
            lastW = w; lastH = h;
        }
        Native.SetWindowPos(Handle, new IntPtr(-1), x, y, w, h, 0x0010 | 0x0040); // TOPMOST, NOACTIVATE | SHOWWINDOW
        shown = true;
    }

    public void HideRing()
    {
        if (!shown) return;
        Native.ShowWindow(Handle, 0);
        shown = false;
    }
}

class Slider
{
    const int BAR_H = 40;             // ii baseBarHeight — bar sabit kalır, altı kayar
    const int DURATION_MS = 520;       // Hyprland workspaces speed 7 (~700ms), menu_decel kuyruğu kısaltıldı
    const int GAP = 50;                // Hyprland general.gaps_workspaces = 50
    const int MAX_WS = 30;             // GlazeWM config'deki workspace sayısı (next/prev sarması için)

    readonly Glaze glaze;
    readonly Overlay overlay = new Overlay();
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

    readonly Ring ring = new Ring();
    public Slider(Glaze g) { glaze = g; overlay.CreateControl(); var h = overlay.Handle; ring.CreateControl(); var rh = ring.Handle; }

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

    public class Thumb { public IntPtr Id; public Native.RECT Dest; public IntPtr Src; }

    // Ekran klavyesi, sağ panel, bildirimler monitöre "yapışık": workspace kayarken animasyon
    // katmanının altında kalmasınlar, en üstte sabit dursunlar.
    static readonly string[] Pinned = { "Zebar - logical-lunge / osk", "Zebar - logical-lunge / sidebar-right", "Zebar - logical-lunge / toast" };
    static void RaisePinned()
    {
        foreach (var title in Pinned)
        {
            IntPtr h = Native.FindWindow(null, title);
            if (h != IntPtr.Zero && Native.IsWindowVisible(h))
                Native.SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // TOPMOST, NOMOVE|NOSIZE|NOACTIVATE
        }
    }

    static IntPtr WallpaperSource(out Native.RECT srcRect)
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
        return wall;
    }

    Thumb Register(IntPtr src, Native.RECT dest, Native.RECT? source)
    {
        IntPtr id;
        if (Native.DwmRegisterThumbnail(overlay.Handle, src, out id) != 0) return null;
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
    static Native.RECT FrameInsets(IntPtr h)
    {
        var wr = WinRect(h); var fr = FrameRect(h);
        return new Native.RECT { Left = Math.Max(0, fr.Left - wr.Left), Top = Math.Max(0, fr.Top - wr.Top), Right = Math.Max(0, wr.Right - fr.Right), Bottom = Math.Max(0, wr.Bottom - fr.Bottom) };
    }

    static Native.RECT Deflate(Native.RECT r, Native.RECT d)
    {
        return new Native.RECT { Left = r.Left + d.Left, Top = r.Top + d.Top, Right = r.Right - d.Right, Bottom = r.Bottom - d.Bottom };
    }

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
    static void PlaceVisible(Thumb t, Native.RECT dest)
    {
        Native.SIZE src;
        var r = dest;
        if (Native.DwmQueryThumbnailSourceSize(t.Id, out src) == 0 && src.cx > 0 && src.cy > 0) r = Deflate(dest, SourceInsets(t.Src, src));
        var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION, rcDestination = r };
        Native.DwmUpdateThumbnailProperties(t.Id, ref pr);
    }

    public class Frozen { public int Ox, Oy; public Rectangle Mon; public readonly List<Thumb> All = new List<Thumb>(); public readonly Dictionary<long, Thumb> Win = new Dictionary<long, Thumb>(); }

    // Dondur: katmanı aç, pencereleri şu anki görünür yerlerinde (ya da verilen eski ekran dikdörtgenlerinde)
    // canlı görüntüleriyle göster. Arkasında GlazeWM ne yaparsa yapsın kullanıcı zıplama görmez. UI thread'inde.
    public Frozen Freeze(Rectangle mon, IEnumerable<long> handles, Dictionary<long, Native.RECT> startScreen, long hidden = 0)
    {
        Interrupt = false;
        int ox = mon.X, oy = mon.Y + BAR_H;
        var f = new Frozen { Ox = ox, Oy = oy, Mon = mon };
        overlay.Bounds = new Rectangle(mon.X, oy, mon.Width, mon.Height - BAR_H);
        Native.RECT wsrc;
        IntPtr wall = WallpaperSource(out wsrc);
        if (wall != IntPtr.Zero)
        {
            var src = new Native.RECT { Left = mon.X - wsrc.Left, Top = oy - wsrc.Top, Right = mon.X - wsrc.Left + mon.Width, Bottom = oy - wsrc.Top + mon.Height - BAR_H };
            var wt = Register(wall, new Native.RECT { Left = 0, Top = 0, Right = mon.Width, Bottom = mon.Height - BAR_H }, src);
            if (wt != null) f.All.Add(wt);
        }
        foreach (var h in handles)
        {
            var hw = new IntPtr(h);
            if (!Native.IsWindowVisible(hw)) continue;
            var t = Register(hw, new Native.RECT(), null);
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
        Animating = true;
        overlay.Show();
        RaisePinned();
        overlay.Refresh();
        Thumb focusedT; // kenarlık katmanın üstünde
        if (f.Win.TryGetValue(FocusedTop().ToInt64(), out focusedT)) ring.Place(Deflate(Unshift(focusedT.Dest, ox, oy), FrameInsets(focusedT.Src)));
        Native.DwmFlush();
        return f;
    }

    // Bitir: her pencereyi gerçek son yerine kaydır/ölçekle (emphasizedDecel), yeni açılan pencere %80'den büyüyüp
    // belirir (Hyprland windowsIn: popin 80%), sonra katmanı kaldır. UI thread'inde.
    public void Finish(Frozen f, IEnumerable<long> endHandles, long popin, int durationMs)
    {
        var anims = new List<KeyValuePair<Thumb, KeyValuePair<Native.RECT, Native.RECT>>>();
        Thumb pop = null;
        var keep = new HashSet<long>();
        foreach (var h in endHandles)
        {
            var hw = new IntPtr(h);
            if (!Native.IsWindowVisible(hw)) continue;
            keep.Add(h);
            Thumb t;
            bool isNew = !f.Win.TryGetValue(h, out t);
            if (isNew) { t = Register(hw, new Native.RECT(), null); if (t == null) continue; f.All.Add(t); }
            var end = VisualDest(hw, t.Id, f.Ox, f.Oy);
            Native.RECT start = t.Dest;
            if (isNew || h == popin)
            {
                int cx = (end.Left + end.Right) / 2, cy = (end.Top + end.Bottom) / 2;
                int hw2 = (int)((end.Right - end.Left) * 0.4), hh = (int)((end.Bottom - end.Top) * 0.4);
                start = new Native.RECT { Left = cx - hw2, Top = cy - hh, Right = cx + hw2, Bottom = cy + hh };
                pop = t;
            }
            anims.Add(new KeyValuePair<Thumb, KeyValuePair<Native.RECT, Native.RECT>>(t, new KeyValuePair<Native.RECT, Native.RECT>(start, end)));
        }
        // Artık bu workspace'te olmayan (kapanan / taşınan) pencereler katmanda kalmasın
        foreach (var kv in f.Win)
            if (!keep.Contains(kv.Key))
            {
                var hide = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_VISIBLE, fVisible = false };
                Native.DwmUpdateThumbnailProperties(kv.Value.Id, ref hide);
            }

        IntPtr focusedH = FocusedTop();
        var sw = Stopwatch.StartNew();
        while (!Interrupt)
        {
            double p = Math.Min(1.0, sw.ElapsedMilliseconds / (double)durationMs);
            double e = Bezier(0.05, 0.7, 0.1, 1, p); // Hyprland emphasizedDecel
            foreach (var a in anims)
            {
                var r = Lerp(a.Value.Key, a.Value.Value, e);
                if (a.Key.Src == focusedH) ring.Place(Deflate(Unshift(r, f.Ox, f.Oy), FrameInsets(focusedH)));
                if (a.Key == pop)
                {
                    // Hyprland windowsIn "popin 80%": ölçekli büyüyerek ve belirerek
                    var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_OPACITY, rcDestination = r };
                    pr.opacity = (byte)Math.Min(255, (int)(255 * Math.Min(1.0, p * 2.5)));
                    Native.DwmUpdateThumbnailProperties(a.Key.Id, ref pr);
                }
                else PlaceVisible(a.Key, r);
            }
            Native.DwmFlush();
            if (p >= 1.0) break;
        }
        overlay.Hide(); ring.HideRing();
        foreach (var t in f.All) Native.DwmUnregisterThumbnail(t.Id);
        Animating = false;
    }

    Thumb RegisterWindow(IntPtr h, int ox, int oy)
    {
        if (!Native.IsWindow(h)) return null;
        var t = Register(h, Shift(WinRect(h), ox, oy), null);
        if (t == null) return null;
        PlaceVisible(t, t.Dest);
        return t;
    }

    static void Move(Thumb t, int dx)
    {
        var r = t.Dest; r.Left += dx; r.Right += dx;
        PlaceVisible(t, r);
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

    public void AnimateLayout(Dictionary<long, Native.RECT> from, Dictionary<long, Native.RECT> to, Rectangle mon, long popin)
    {
        // Değişen bir şey yoksa hiç overlay açma
        bool changed = popin != 0;
        foreach (var kv in to)
        {
            Native.RECT o;
            if (!from.TryGetValue(kv.Key, out o) || o.Left != kv.Value.Left || o.Top != kv.Value.Top || o.Right != kv.Value.Right || o.Bottom != kv.Value.Bottom) changed = true;
        }
        if (!changed || to.Count == 0) { Log("anim: değişiklik yok (" + from.Count + "->" + to.Count + ")"); return; }
        Log("anim: " + from.Count + "->" + to.Count + " pencere" + (popin != 0 ? " +popin" : ""));

        Interrupt = false;
        int ox = mon.X, oy = mon.Y + BAR_H;
        overlay.Bounds = new Rectangle(mon.X, oy, mon.Width, mon.Height - BAR_H);
        var all = new List<Thumb>();
        var anims = new List<KeyValuePair<Thumb, KeyValuePair<Native.RECT, Native.RECT>>>();

        Native.RECT wsrc;
        IntPtr wall = WallpaperSource(out wsrc);
        if (wall != IntPtr.Zero)
        {
            var src = new Native.RECT { Left = mon.X - wsrc.Left, Top = oy - wsrc.Top, Right = mon.X - wsrc.Left + mon.Width, Bottom = oy - wsrc.Top + mon.Height - BAR_H };
            var t = Register(wall, new Native.RECT { Left = 0, Top = 0, Right = mon.Width, Bottom = mon.Height - BAR_H }, src);
            if (t != null) all.Add(t);
        }

        foreach (var kv in to)
        {
            var end = kv.Value;
            Native.RECT start;
            if (kv.Key == popin)
            {
                // popin 80%: merkezden %80 boyuttan başla
                int cx = (end.Left + end.Right) / 2, cy = (end.Top + end.Bottom) / 2;
                int hw = (int)((end.Right - end.Left) * 0.4), hh = (int)((end.Bottom - end.Top) * 0.4);
                start = new Native.RECT { Left = cx - hw, Top = cy - hh, Right = cx + hw, Bottom = cy + hh };
            }
            else if (!from.TryGetValue(kv.Key, out start)) start = end;

            var t = Register(new IntPtr(kv.Key), Shift(start, ox, oy), null);
            if (t == null) continue;
            all.Add(t);
            anims.Add(new KeyValuePair<Thumb, KeyValuePair<Native.RECT, Native.RECT>>(t, new KeyValuePair<Native.RECT, Native.RECT>(Shift(start, ox, oy), Shift(end, ox, oy))));
        }

        int moveMs = popin != 0 ? MOVE_MS : Adaptive(ref lastMoveStart, MOVE_MS);
        Animating = true;
        overlay.Show();
        RaisePinned();
        overlay.Refresh();
        Native.DwmFlush();

        var sw = Stopwatch.StartNew();
        while (!Interrupt)
        {
            double p = Math.Min(1.0, sw.ElapsedMilliseconds / (double)moveMs);
            double e = Bezier(0.05, 0.7, 0.1, 1, p); // Hyprland emphasizedDecel
            foreach (var a in anims)
            {
                var pr = new Native.DWM_THUMBNAIL_PROPERTIES { dwFlags = Native.DWM_TNP_RECTDESTINATION, rcDestination = Lerp(a.Value.Key, a.Value.Value, e) };
                Native.DwmUpdateThumbnailProperties(a.Key.Id, ref pr);
            }
            Native.DwmFlush();
            if (p >= 1.0) break;
        }

        overlay.Hide(); ring.HideRing();
        foreach (var t in all) Native.DwmUnregisterThumbnail(t.Id);
        Animating = false;
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

    void InWorkspace(string dir, bool move)
    {
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
            // O yönde komşu yok: GlazeWM (fork) düzeni çevirir. Örn. üstte tam genişlik + altta iki yarı, sağ alttaki
            // sağa -> solda üst üste iki parça, bu pencere sağda boydan. Tek durum hariç: pencere doğrudan
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
        // GlazeWM (fork) Hyprland dwindle movewindow yapar: o yönde pencere varsa onu uzun kenarından böler; yoksa
        // bölme yönü değişir (yan yana iki pencerede Super+Shift+Yukarı -> odaktaki üstte tam genişlik).
        glaze.Command("move --direction " + dir);

        Dictionary<string, object> mA, wsA, curA; List<Dictionary<string, object>> winsA;
        bool ok = Current(out mA, out wsA, out winsA, out curA);
        var endHs = ok ? new List<long>(Rects(winsA).Keys) : hs;
        WaitSettled(endHs);
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
                try { Finish(frozen, endHs, 0, dur); } catch (Exception ex) { Log("move anim: " + ex.Message); }
            }));
        }
    }
    public static void Log(string s)
    {
        try { System.IO.File.AppendAllText(System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-helper.log"), DateTime.Now.ToString("HH:mm:ss.fff ") + s + Environment.NewLine); }
        catch { }
    }

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
        overlay.Bounds = new Rectangle(mx, my + BAR_H, mw, mh - BAR_H);
        int ox = mx, oy = my + BAR_H;

        var thumbs = new List<Thumb>();
        var oldThumbs = new List<Thumb>();
        var newThumbs = new List<Thumb>();

        // Duvar kağıdı (sabit, Hyprland'de de kaymaz)
        Native.RECT wsrc;
        IntPtr wall = WallpaperSource(out wsrc);
        if (wall != IntPtr.Zero)
        {
            var src = new Native.RECT { Left = mx - wsrc.Left, Top = oy - wsrc.Top, Right = mx - wsrc.Left + mw, Bottom = oy - wsrc.Top + mh - BAR_H };
            var t = Register(wall, new Native.RECT { Left = 0, Top = 0, Right = mw, Bottom = mh - BAR_H }, src);
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

            overlay.Show();
            RaisePinned();
            overlay.Refresh();
            Native.DwmFlush();
            Log("fast shown " + clock.ElapsedMilliseconds + "ms new=" + newThumbs.Count);

            var cmdsAll = (string[])commands.Clone();
            var task = Task.Factory.StartNew(() => { foreach (var cm in cmdsAll) glaze.Command(cm); });
            int dur0 = Adaptive(ref lastSlideStart, DURATION_MS);
            Animating = true;

            // Taşı+takip et: GlazeWM komutu bittiği an (genelde kaymanın ilk ~50 ms'i) hedef workspace'teki pencereler
            // ve taşınan pencere yeni yerlerine doğru kaymayla AYNI ANDA ve esnemeden ilerler; ayrı bir "yerleşme"
            // adımı yok (Hyprland'de de pencere kayarken boyutlanır). Hedef her karede canlı okunur: GlazeWM pencereyi
            // eşzamansız taşıdığı için ilk okuma eski yer olabilir.
            var from = new Dictionary<Thumb, Native.RECT>();
            if (moveFollow) { foreach (var t in newThumbs) from[t] = t.Dest; if (carried != null) from[carried] = carried.Dest; }
            Stopwatch swR = null;
            int durR = 0;
            var sw0 = Stopwatch.StartNew();
            while (!Interrupt)
            {
                double p = Math.Min(1.0, sw0.ElapsedMilliseconds / (double)dur0);
                double e = Bezier(0.1, 1, 0, 1, p);
                int shift = (int)Math.Round(e * (mw + GAP));
                if (moveFollow && swR == null && task.IsCompleted)
                {
                    swR = Stopwatch.StartNew();
                    durR = Math.Max(MIN_MS, (int)(dur0 - sw0.ElapsedMilliseconds));
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
                    PlaceVisible(t, r);
                }
                if (moveFollow && carried != null)
                {
                    var rc = swR == null ? from[carried] : Lerp(from[carried], VisualDest(carried.Src, carried.Id, ox, oy), eR);
                    PlaceVisible(carried, rc);
                    ring.Place(Deflate(Unshift(rc, ox, oy), FrameInsets(carried.Src)));
                }
                Native.DwmFlush();
                if (p >= 1.0 && (!moveFollow || pR >= 1.0)) break;
                if (p >= 1.0 && swR == null && sw0.ElapsedMilliseconds > dur0 + 1500) break; // komut takıldı
            }
            long animEnd = clock.ElapsedMilliseconds;
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
            overlay.Hide(); ring.HideRing();
            foreach (var t in thumbs) Native.DwmUnregisterThumbnail(t.Id);
            Animating = false;
            Log("fast done " + clock.ElapsedMilliseconds + "ms (animasyon " + dur0 + "ms, bitti " + animEnd + "ms, " + (viaState ? "pencereler hazır" : "komut " + (task.IsCompleted ? "bitti" : "sürüyor")) + ")");
            return;
        }

        overlay.Show();
        RaisePinned();
        overlay.Refresh();
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
                if (t != null) { newThumbs.Add(t); thumbs.Add(t); Move(t, dir * (mw + GAP)); }
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

        overlay.Hide(); ring.HideRing();
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
        var t = new Thread(() => { while (true) { Thread.Sleep(300); try { RefreshCache(); } catch { } } }) { IsBackground = true };
        t.Start();
    }

    void Snapshot(out Dictionary<long, Native.RECT> r, out Dictionary<long, string> m, out Dictionary<string, Rectangle> mr)
    {
        r = new Dictionary<long, Native.RECT>(); m = new Dictionary<long, string>(); mr = new Dictionary<string, Rectangle>();
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
                }
            }
        }
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
        Dictionary<long, Native.RECT> r; Dictionary<long, string> m; Dictionary<string, Rectangle> mr;
        Snapshot(out r, out m, out mr);
        var v = Visual(r.Keys);
        lock (cacheLock) { rects = r; monOf = m; monRects = mr; visual = v; }
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
        { "zebar", "ll-helper", "tacky-borders", "ShellExperienceHost", "SearchUI", "SearchApp", "StartMenuExperienceHost",
          "LockApp", "TextInputHost", "ApplicationFrameHost", "msedgewebview2", "ll-songrec", "ll-termcolors" };

    public void HookNewWindows()
    {
        if (ui == null) return;
        ui.BeginInvoke((Action)(() =>
        {
            showCb = OnWindowShown;
            Native.SetWinEventHook(Native.EVENT_OBJECT_SHOW, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, showCb, 0, 0, 0x0002 | 0x0000); // OUTOFCONTEXT
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
        if (ui == null) return;
        Dictionary<long, Native.RECT> beforeVis; Dictionary<long, string> beforeMon;
        lock (cacheLock) { beforeVis = visual; beforeMon = monOf; }

        Dictionary<long, Native.RECT> after; Dictionary<long, string> afterMon; Dictionary<string, Rectangle> mr;
        Snapshot(out after, out afterMon, out mr);
        string mid;
        Rectangle mon;
        if (!(opened ? afterMon : beforeMon).TryGetValue(anchorHandle, out mid) || !mr.TryGetValue(mid, out mon))
        {
            RefreshCache();
            return;
        }
        var hs = new List<long>();
        foreach (var kv in afterMon) if (kv.Value == mid) hs.Add(kv.Key);
        var start = new Dictionary<long, Native.RECT>();
        foreach (var h in hs) { Native.RECT r; if (beforeVis.TryGetValue(h, out r)) start[h] = r; }
        long pop = opened ? anchorHandle : 0;

        Slider.Frozen f = opened ? TakePending(anchorHandle) : null;
        if (f != null && f.Mon != mon)
        {
            // Pencere başka monitöre geldi: o katmanı hemen kaldır, bu monitörde yeniden dondur
            var wrong = f; f = null;
            try { ui.Invoke((Action)(() => slider.Finish(wrong, new List<long>(), 0, 1))); } catch { }
        }
        if (f == null)
        {
            slider.Interrupt = true;
            try { f = (Slider.Frozen)ui.Invoke((Func<Slider.Frozen>)(() => slider.Freeze(mon, hs, start, pop))); }
            catch (Exception ex) { Slider.Log("freeze: " + ex.Message); }
        }


        Snapshot(out after, out afterMon, out mr);
        var end = new List<long>();
        foreach (var kv in afterMon) if (kv.Value == mid) end.Add(kv.Key);
        Native.RECT target;
        if (opened && after.TryGetValue(anchorHandle, out target)) Slider.WaitPlaced(anchorHandle, target, 500);
        Slider.WaitSettled(end);
        var v = Visual(after.Keys);
        lock (cacheLock) { rects = after; monOf = afterMon; monRects = mr; visual = v; }
        Slider.Log("anim: " + start.Count + "->" + end.Count + " pencere" + (opened ? " +popin" : ""));
        if (opened && end.Contains(anchorHandle))
        {
            Native.RECT nr;
            IntPtr fg = Native.GetAncestor(Native.GetForegroundWindow(), 2);
            if (fg.ToInt64() == anchorHandle && Native.GetWindowRect(new IntPtr(anchorHandle), out nr))
                Cursor.Position = new Point((nr.Left + nr.Right) / 2, (nr.Top + nr.Bottom) / 2);
        }
        if (f != null)
            ui.BeginInvoke((Action)(() =>
            {
                try { slider.Finish(f, end, pop, Slider.MoveMs); } catch (Exception ex) { Slider.Log("anim: " + ex.Message); }
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
            if (req.ToString().StartsWith("OPTIONS"))
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

    public void Start()
    {
        cb = OnShow;
        Native.SetWinEventHook(Native.EVENT_OBJECT_SHOW, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, cb, 0, 0, 0x0002);
    }

    void OnShow(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        if (idObject != 0 || hwnd == IntPtr.Zero) return;
        try
        {
            var cls = new StringBuilder(64);
            Native.GetClassName(hwnd, cls, 64);
            if (cls.ToString() != "#32770") return;
            uint pid; Native.GetWindowThreadProcessId(hwnd, out pid);
            string proc;
            try { proc = Process.GetProcessById((int)pid).ProcessName; } catch { return; }
            if (!owners.Contains(proc)) return;

            var texts = new List<string>(); var buttons = new List<IntPtr>();
            Native.EnumChildWindows(hwnd, delegate (IntPtr ch, IntPtr l)
            {
                var c = new StringBuilder(64); Native.GetClassName(ch, c, 64);
                var t = new StringBuilder(2048); Native.GetWindowText(ch, t, 2048);
                string cn = c.ToString(), tx = t.ToString().Trim();
                if (cn == "Button") buttons.Add(ch);
                else if ((cn == "Static" || cn == "DirectUIHWND") && tx.Length > 0) texts.Add(tx);
                return true;
            }, IntPtr.Zero);
            if (buttons.Count != 1 || texts.Count == 0) return; // soru soran kutuya dokunma

            var title = new StringBuilder(256); Native.GetWindowText(hwnd, title, 256);
            Native.PostMessage(buttons[0], 0x00F5, IntPtr.Zero, IntPtr.Zero); // BM_CLICK
            string head = title.ToString();
            if (proc.Equals("glazewm", StringComparison.OrdinalIgnoreCase)) head = "GlazeWM: " + head;
            Toasts.Send("error", head.Length > 0 ? head : "Hata", string.Join("\n", texts), "error");
            Slider.Log("dialog -> toast: " + proc + " | " + head);
        }
        catch (Exception ex) { Slider.Log("dialog: " + ex.GetBaseException().Message); }
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
        { "zebar", "tacky-borders", "ll-helper", "explorer", "ShellExperienceHost", "SearchUI", "SearchApp", "StartMenuExperienceHost", "LockApp" };
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
        { "close", "Alt+F4" }, { "screenshot", "Print" },
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

    public Keys2(Control ui, Slider slider) { this.ui = ui; this.slider = slider; }

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

        int msg = wParam.ToInt32();
        bool isDown = msg == Native.WM_KEYDOWN || msg == Native.WM_SYSKEYDOWN;
        bool isUp = msg == Native.WM_KEYUP || msg == Native.WM_SYSKEYUP;
        int vk = (int)k.vkCode;

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

    static void ToggleOverview()
    {
        IntPtr h = Native.FindWindow(null, "ll-overview");
        if (h == IntPtr.Zero) return;
        if (Native.IsWindowVisible(h) && Native.GetForegroundWindow() == h) { Native.ShowWindow(h, 0); return; }
        Native.ShowWindow(h, 5);
        // Önplana almak için "son giriş bizden" olsun (SetForegroundWindow kısıtı)
        Native.keybd_event(VK_DUMMY, 0, 0, UIntPtr.Zero); Native.keybd_event(VK_DUMMY, 0, 2, UIntPtr.Zero);
        Native.SetForegroundWindow(h);
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
        protected override void OnShown(EventArgs e) { base.OnShown(e); Activate(); }
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
        protected override void OnShown(EventArgs e) { base.OnShown(e); Activate(); }

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
            try { Clipboard.SetDataObject(outBmp, true, 5, 100); } catch { }
            return;
        }
        string dir = System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyPictures), "Screenshots");
        System.IO.Directory.CreateDirectory(dir);
        using (var dlg = new SaveFileDialog
        {
            Title = Tr ? "Ekran alıntısını kaydet" : "Save screenshot",
            InitialDirectory = dir,
            FileName = "Screenshot_" + DateTime.Now.ToString("yyyy-MM-dd_HH-mm-ss") + ".png",
            Filter = "PNG (*.png)|*.png|JPEG (*.jpg)|*.jpg",
            AddExtension = true,
        })
        {
            if (dlg.ShowDialog() != DialogResult.OK) return;
            var fmt = dlg.FileName.EndsWith(".jpg", StringComparison.OrdinalIgnoreCase) ? System.Drawing.Imaging.ImageFormat.Jpeg : System.Drawing.Imaging.ImageFormat.Png;
            outBmp.Save(dlg.FileName, fmt);
        }
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

    // mode: "all" | "span" | monitör kimliği
    public static void Apply(string path, string mode)
    {
        path = System.IO.Path.GetFullPath(path);
        var w = Api();
        if (mode == "span") { w.SetPosition(SPAN); w.SetWallpaper(null, path); }
        else
        {
            if (w.GetPosition() == SPAN) w.SetPosition(FILL);
            else w.SetPosition(FILL);
            w.SetWallpaper(mode == "all" ? null : mode, path);
        }
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

    static Image Wallpaper()
    {
        foreach (var f in new[] {
            (string)Microsoft.Win32.Registry.GetValue(@"HKEY_CURRENT_USER\Control Panel\Desktop", "WallPaper", null),
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), @"Microsoft\Windows\Themes\TranscodedWallpaper") })
        {
            try { if (!string.IsNullOrEmpty(f) && System.IO.File.Exists(f)) using (var s = System.IO.File.OpenRead(f)) return Image.FromStream(new System.IO.MemoryStream(ReadAll(s))); }
            catch { }
        }
        return null;
    }
    static byte[] ReadAll(System.IO.Stream s) { var m = new System.IO.MemoryStream(); s.CopyTo(m); return m.ToArray(); }

    class Cover : Form
    {
        readonly Image img;
        public Cover(Rectangle b, Image img)
        {
            this.img = img;
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
            // "Doldur" yerleşimi: en boy oranını koruyup ekranı kapla, taşanı ortadan kırp
            double k = Math.Max((double)Width / img.Width, (double)Height / img.Height);
            int w = (int)Math.Ceiling(img.Width * k), h = (int)Math.Ceiling(img.Height * k);
            e.Graphics.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
            e.Graphics.DrawImage(img, (Width - w) / 2, (Height - h) / 2, w, h);
        }
    }

    public static void Run()
    {
        bool created;
        using (var m = new Mutex(true, "ll-splash", out created))
        {
            if (!created) return;
            var img = Wallpaper();
            var covers = new List<Cover>();
            foreach (var s in Screen.AllScreens) { var f = new Cover(s.Bounds, img); f.Show(); covers.Add(f); }

            var start = Environment.TickCount;
            int readyAt = -1;
            var timer = new System.Windows.Forms.Timer { Interval = 100 };
            timer.Tick += (o, e) =>
            {
                int now = Environment.TickCount;
                foreach (var f in covers) Native.SetWindowPos(f.Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); // en üstte kal
                if (readyAt < 0 && Ready()) readyAt = now;
                // Hazır olduktan sonra pencerelerin yerleşip bar'ın çizilmesi için kısa bir süre; en fazla 30 sn bekle
                bool done = (readyAt >= 0 && now - readyAt > 1500) || now - start > 30000;
                if (!done) return;
                timer.Stop();
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
        // ll-helper.exe --snip: bölge ekran alıntısı + düzenleme (Hyprland Print: grim + slurp + swappy)
        if (args.Length >= 1 && args.Length <= 2 && args[0] == "--snip")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            int wait; // --snip 350: paneller kapansın diye önce bekle
            if (args.Length == 2 && int.TryParse(args[1], out wait)) Thread.Sleep(Math.Min(2000, wait));
            bool fresh; using (var sm = new Mutex(true, "ll-snip", out fresh)) { if (fresh) SnipTool.Run(); }
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
            while (true)
            {
                try
                {
                    using (var c = new System.Net.Sockets.TcpClient("127.0.0.1", 6131))
                    using (var s = c.GetStream())
                    {
                        var req = Encoding.ASCII.GetBytes("GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                        s.Write(req, 0, req.Length);
                        var reader = new System.IO.StreamReader(s, Encoding.UTF8);
                        string line;
                        while ((line = reader.ReadLine()) != null)
                            if (line.StartsWith("data: ")) stdout.WriteLine(line.Substring(6));
                    }
                }
                catch (System.IO.IOException) { if (!CanWrite(stdout)) return; }
                catch (System.Net.Sockets.SocketException) { }
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

        // Tek seferlik: ll-helper.exe --slide next|prev|<workspace>  (bar tıklamaları ve test için)
        if (args.Length == 2 && args[0] == "--slide")
        {
            try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            var g = new Glaze();
            var sl = new Slider(g);
            if (args[1] == "next") sl.Run(new[] { "focus --next-workspace" }, 1, null);
            else if (args[1] == "prev") sl.Run(new[] { "focus --prev-workspace" }, -1, null);
            else sl.Run(new[] { "focus --workspace " + args[1] }, 0, args[1]);
            return;
        }

        // Yakalanmayan her hatayı yığın iziyle log'a yaz (sessiz çökme olmasın)
        AppDomain.CurrentDomain.UnhandledException += (s, e) => Slider.Log("CRASH: " + e.ExceptionObject);
        Application.ThreadException += (s, e) => Slider.Log("UI HATA: " + e.Exception);
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);

        bool created;
        var mutex = new Mutex(true, "ll-helper-single", out created);
        if (!created) return;
        try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { } // PER_MONITOR_AWARE_V2

        Application.EnableVisualStyles();
        var ui = new Form { ShowInTaskbar = false, WindowState = FormWindowState.Minimized, FormBorderStyle = FormBorderStyle.None, Opacity = 0 };
        ui.Load += (s, e) => ui.Hide();
        var h = ui.Handle;

        var glaze = new Glaze();
        var slider = new Slider(glaze);
        Slider.Ui = ui;
        var dwindle = new Dwindle(new Glaze(), ui, slider);
        dwindle.Start(); // kendi IPC bağlantısıyla: slide'ı beklemesin
        dwindle.HookNewWindows();
        LaunchQueue.Start(new Slider(new Glaze())); // kendi bağlantısı: animasyonu beklemesin
        NightLight.StartKeeper();
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

        Application.Run(ui);
        GC.KeepAlive(mutex);
    }
}
