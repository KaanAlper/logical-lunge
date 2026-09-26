// ll-bar — ii bar'ı, WebView2 olmadan: doğrudan Win32 + GDI+ ile çizilir (kabuk sürecinde, ll-helper --shell).
// Zebar'daki bar.html'in native karşılığı; ölçüler ve renkler aynı (ii Appearance.qml, styles.css):
//   sol: arama (overview), odaktaki pencere | orta: kaynaklar + medya, workspace'ler, saat + araçlar + pil
//   sağ: göstergeler (sağ panel), tray | kenarlarda tekerlek: solda parlaklık (0'ın altı gama), sağda ses
// Veriler süreç başlatmadan okunur: GlazeWM olayları (IPC), işlemci / bellek / pil / ağ Win32 API'leriyle, ses Core Audio
// ile, parlaklık DDC/CI ya da WMI ile, medya media-art.exe --watch'tan (tek, olay güdümlü küçük süreç).
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Net.WebSockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// ---------------- Görünüm: renkler, fontlar, ikonlar ----------------
static class Look
{
    // Material You koyu, mor tohum (styles.css :root)
    public static readonly Color Primary = Hex("d0bcff"), OnPrimary = Hex("381e72");
    public static readonly Color SecCont = Hex("4a4458"), OnSecCont = Hex("e8def8");
    public static readonly Color Outline = Hex("938f99"), Error = Hex("ffb4ab");
    public static readonly Color Layer0 = Hex("141218"), Layer1 = Hex("1d1b20"), Layer1Hover = Hex("36323b");
    public static readonly Color OnLayer = Hex("e6e0e9"), Subtext = Hex("938f99"), Inactive = Hex("8a8591");
    public static readonly Color Occupied = Color.FromArgb(153, 74, 68, 88);
    public static readonly Color PopBorder = Color.FromArgb(153, 73, 69, 79);

    static Color Hex(string h) { return Color.FromArgb(Convert.ToInt32(h.Substring(0, 2), 16), Convert.ToInt32(h.Substring(2, 2), 16), Convert.ToInt32(h.Substring(4, 2), 16)); }
    public static Color Alpha(Color c, double a) { return Color.FromArgb((int)Math.Round(255 * Math.Max(0, Math.Min(1, a))), c); }

    // ---- Yazı: ii'deki Google Sans Flex / Rubik yüklü değilse Windows'un kendi fontu ----
    static string textFamily;
    public static string TextFamily
    {
        get
        {
            if (textFamily != null) return textFamily;
            textFamily = "Segoe UI";
            using (var fc = new InstalledFontCollection())
                foreach (var want in new[] { "Google Sans Flex", "Google Sans", "Rubik", "Segoe UI Variable Text", "Segoe UI" })
                    foreach (var f in fc.Families) if (f.Name == want) { textFamily = want; return textFamily; }
            return textFamily;
        }
    }
    static readonly Dictionary<string, Font> fonts = new Dictionary<string, Font>();
    // px: CSS pikseli (çizim, ölçeklenmiş Graphics'e yapılır)
    public static Font Text(float px) { return Get("t" + px, () => new Font(TextFamily, px, FontStyle.Regular, GraphicsUnit.Pixel)); }
    public static Font TextBold(float px) { return Get("b" + px, () => new Font(TextFamily, px, FontStyle.Bold, GraphicsUnit.Pixel)); }
    static Font Get(string k, Func<Font> make) { Font f; lock (fonts) { if (!fonts.TryGetValue(k, out f)) fonts[k] = f = make(); } return f; }

    // ---- İkonlar: Material Symbols Rounded (helper klasöründe; build.ps1 indirir) ----
    // GDI+ ligatür yapmaz: ikonlar kod noktasıyla çizilir. Font yoksa Windows'un Segoe Fluent Icons / MDL2 Assets'i.
    static readonly Dictionary<string, int> Material = new Dictionary<string, int>
    {
        { "search", 0xef7a }, { "volume_up", 0xe050 }, { "volume_off", 0xe04f }, { "mic", 0xe31d }, { "mic_off", 0xe02b },
        { "wifi4", 0xf065 }, { "wifi3", 0xebe1 }, { "wifi2", 0xebd6 }, { "wifi1", 0xebe4 }, { "wifi_off", 0xe1da }, { "lan", 0xeb2f },
        { "expand_more", 0xe5cf }, { "memory", 0xe322 }, { "swap_horiz", 0xe8d4 }, { "planner_review", 0xe694 },
        { "light_mode", 0xe518 }, { "dark_mode", 0xe51c }, { "keyboard", 0xe312 }, { "screenshot_region", 0xf7d2 }, { "bolt", 0xea0b },
        { "music_note", 0xe405 }, { "pause", 0xe034 }, { "play_arrow", 0xe037 }, { "skip_next", 0xe044 }, { "skip_previous", 0xe045 },
        { "keyboard_arrow_up", 0xe316 }, { "keyboard_arrow_down", 0xe313 }, { "brightness_4", 0xe3a9 },
        { "device_thermostat", 0xe1ff }, { "developer_board", 0xe30d }, { "speed", 0xe9e4 }, { "local_fire_department", 0xef55 },
        { "clock_loader_60", 0xf37c }, { "check_circle", 0xe86c }, { "empty_dashboard", 0xf844 },
    };
    static readonly Dictionary<string, int> Mdl2 = new Dictionary<string, int>
    {
        { "search", 0xE721 }, { "volume_up", 0xE767 }, { "volume_off", 0xE74F }, { "mic", 0xE720 }, { "wifi4", 0xE701 },
        { "wifi3", 0xE701 }, { "wifi2", 0xE701 }, { "wifi1", 0xE701 }, { "lan", 0xE839 }, { "expand_more", 0xE70D },
        { "light_mode", 0xE706 }, { "keyboard", 0xE765 }, { "bolt", 0xE945 }, { "music_note", 0xE8D6 }, { "pause", 0xE769 },
        { "play_arrow", 0xE768 }, { "skip_next", 0xE893 }, { "skip_previous", 0xE892 }, { "keyboard_arrow_up", 0xE70E },
        { "keyboard_arrow_down", 0xE70D }, { "brightness_4", 0xE706 },
    };
    static PrivateFontCollection pfc;
    static FontFamily iconFamily;
    static bool iconIsMaterial, iconTried;
    static void LoadIcons()
    {
        if (iconTried) return;
        iconTried = true;
        try
        {
            string f = System.IO.Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "MaterialSymbolsRounded.ttf");
            if (System.IO.File.Exists(f))
            {
                pfc = new PrivateFontCollection();
                pfc.AddFontFile(f);
                if (pfc.Families.Length > 0) { iconFamily = pfc.Families[0]; iconIsMaterial = true; return; }
            }
        }
        catch (Exception ex) { Slider.Log("bar: ikon fontu yüklenemedi: " + ex.Message); }
        using (var fc = new InstalledFontCollection())
            foreach (var want in new[] { "Segoe Fluent Icons", "Segoe MDL2 Assets" })
                foreach (var f in fc.Families) if (f.Name == want) { iconFamily = new FontFamily(want); return; }
    }
    static readonly Dictionary<float, Font> iconFonts = new Dictionary<float, Font>();
    static readonly StringFormat center = new StringFormat(StringFormat.GenericTypographic) { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center, FormatFlags = StringFormatFlags.NoWrap | StringFormatFlags.NoClip };

    public static void Icon(Graphics g, string name, float cx, float cy, float px, Color color)
    {
        LoadIcons();
        if (iconFamily == null) return;
        int cp;
        if (!(iconIsMaterial ? Material : Mdl2).TryGetValue(name, out cp)) return;
        Font f;
        // MDL2 ikonları Material'a göre iri görünür
        float size = iconIsMaterial ? px : px * 0.8f;
        lock (iconFonts) if (!iconFonts.TryGetValue(size, out f)) iconFonts[size] = f = new Font(iconFamily, size, FontStyle.Regular, GraphicsUnit.Pixel);
        using (var b = new SolidBrush(color))
            g.DrawString(((char)cp).ToString(), f, b, new RectangleF(cx - px, cy - px, px * 2, px * 2), center);
    }

    // ---- Metin ----
    public static readonly StringFormat Left = new StringFormat(StringFormat.GenericTypographic) { LineAlignment = StringAlignment.Center, Trimming = StringTrimming.EllipsisCharacter, FormatFlags = StringFormatFlags.NoWrap | StringFormatFlags.MeasureTrailingSpaces };
    public static readonly StringFormat Center = new StringFormat(StringFormat.GenericTypographic) { Alignment = StringAlignment.Center, LineAlignment = StringAlignment.Center, Trimming = StringTrimming.EllipsisCharacter, FormatFlags = StringFormatFlags.NoWrap };
    public static void Str(Graphics g, string s, Font f, Color c, RectangleF r, StringFormat fmt)
    {
        if (string.IsNullOrEmpty(s) || r.Width <= 0) return;
        using (var b = new SolidBrush(c)) g.DrawString(s, f, b, r, fmt);
    }
    public static float Width(Graphics g, string s, Font f) { return string.IsNullOrEmpty(s) ? 0 : g.MeasureString(s, f, 10000, Left).Width; }

    public static GraphicsPath Round(RectangleF r, float rad)
    {
        var p = new GraphicsPath();
        rad = Math.Min(rad, Math.Min(r.Width, r.Height) / 2);
        if (rad <= 0.1f) { p.AddRectangle(r); return p; }
        float d = rad * 2;
        p.AddArc(r.X, r.Y, d, d, 180, 90);
        p.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        p.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        p.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
        p.CloseFigure();
        return p;
    }
    public static void Fill(Graphics g, RectangleF r, float rad, Color c)
    {
        using (var p = Round(r, rad)) using (var b = new SolidBrush(c)) g.FillPath(b, p);
    }
    // ii CircularProgress: iz + değer yayı, tepeden saat yönünde
    public static void Ring(Graphics g, float cx, float cy, float r, float stroke, double value, Color track, Color fill)
    {
        var rect = new RectangleF(cx - r, cy - r, r * 2, r * 2);
        using (var pt = new Pen(track, stroke)) g.DrawEllipse(pt, rect);
        double v = Math.Max(0, Math.Min(1, value));
        if (v <= 0) return;
        using (var pv = new Pen(fill, stroke) { StartCap = LineCap.Round, EndCap = LineCap.Round })
            g.DrawArc(pv, rect, -90, (float)(360 * v));
    }
}

// ---------------- Çeviri (Zebar paketinin i18n.json'u) ----------------
static class Tr
{
    static Dictionary<string, string> dict;
    public static string T(string s)
    {
        if (dict == null)
        {
            dict = new Dictionary<string, string>();
            try
            {
                string code = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
                string f = System.IO.Path.Combine(LLShell.Home, @".glzr\zebar\logical-lunge\i18n.json");
                if (code != "tr" && System.IO.File.Exists(f))
                {
                    var d = new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.DeserializeObject(System.IO.File.ReadAllText(f)) as Dictionary<string, object>;
                    var langs = new List<object>((object[])d["langs"]);
                    if (!langs.Contains(code)) code = "en";
                    var keys = (object[])d["keys"]; var vals = (object[])d[code];
                    for (int i = 0; i < keys.Length && i < vals.Length; i++) dict[(string)keys[i]] = (string)vals[i];
                }
            }
            catch (Exception ex) { Slider.Log("bar: i18n: " + ex.Message); }
        }
        string v;
        return dict.TryGetValue(s, out v) ? v : s;
    }
}

// ---------------- Ses (Core Audio, varsayılan çıkış) ----------------
static class Volume
{
    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumerator { }
    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceEnumerator
    {
        int EnumAudioEndpoints(int dataFlow, int stateMask, out IntPtr devices);
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
    }
    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDevice { int Activate(ref Guid iid, int clsCtx, IntPtr p, [MarshalAs(UnmanagedType.IUnknown)] out object iface); }
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

    static IAudioEndpointVolume Endpoint()
    {
        try
        {
            var en = (IMMDeviceEnumerator)new MMDeviceEnumerator();
            IMMDevice d;
            if (en.GetDefaultAudioEndpoint(0 /*eRender*/, 1 /*eMultimedia*/, out d) != 0 || d == null) return null;
            var iid = typeof(IAudioEndpointVolume).GUID; object o;
            d.Activate(ref iid, 23, IntPtr.Zero, out o);
            return (IAudioEndpointVolume)o;
        }
        catch { return null; }
    }

    public static int Level = -1;   // -1: çıkış cihazı yok
    public static bool Muted;

    // Değiştiyse true
    public static bool Read()
    {
        var v = Endpoint();
        int lv = -1; bool m = false;
        if (v != null) { float f; v.GetMasterVolumeLevelScalar(out f); v.GetMute(out m); lv = (int)Math.Round(f * 100); }
        bool changed = lv != Level || m != Muted;
        Level = lv; Muted = m;
        return changed;
    }
    public static void Set(int level)
    {
        var v = Endpoint(); if (v == null) return;
        var ctx = Guid.Empty;
        level = Math.Max(0, Math.Min(100, level));
        v.SetMasterVolumeLevelScalar(level / 100f, ref ctx);
        if (Muted && level > 0) v.SetMute(false, ref ctx);
        Read();
    }
    public static void Step(int d) { Read(); if (Level >= 0) Set(Level + d); BarHost.Refresh(); }
    public static void ToggleMute()
    {
        var v = Endpoint(); if (v == null) return;
        var ctx = Guid.Empty; bool m; v.GetMute(out m); v.SetMute(!m, ref ctx);
        Read(); BarHost.Refresh();
    }
}

// ---------------- Parlaklık: harici ekran DDC/CI, dizüstü ekranı WMI (PowerShell'siz) ----------------
static class Brightness
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct PHYSICAL_MONITOR { public IntPtr h; [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string desc; }
    [DllImport("dxva2.dll")] static extern bool GetNumberOfPhysicalMonitorsFromHMONITOR(IntPtr mon, out uint n);
    [DllImport("dxva2.dll")] static extern bool GetPhysicalMonitorsFromHMONITOR(IntPtr mon, uint n, [Out] PHYSICAL_MONITOR[] arr);
    [DllImport("dxva2.dll")] static extern bool DestroyPhysicalMonitors(uint n, PHYSICAL_MONITOR[] arr);
    [DllImport("dxva2.dll")] static extern bool GetMonitorBrightness(IntPtr h, out uint min, out uint cur, out uint max);
    [DllImport("dxva2.dll")] static extern bool SetMonitorBrightness(IntPtr h, uint v);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point p, uint flags);

    static readonly Dictionary<string, int> cache = new Dictionary<string, int>(); // ekran -> 0..100 (-1: desteklemiyor)
    static readonly object gate = new object();

    // DDC/CI okuma ~50-500 ms: bar'ın thread'inde değil, önbellekten. Bilinmiyorsa null.
    public static int? Cached(string dev) { int v; lock (cache) return cache.TryGetValue(dev, out v) ? (v < 0 ? (int?)null : v) : null; }
    public static bool Known(string dev) { lock (cache) return cache.ContainsKey(dev); }

    public static void ReadAsync(Screen s, Action done)
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            int v = -1;
            lock (gate)
            {
                try { v = Ddc(s, -1); } catch { }
                if (v < 0 && s.Primary) try { v = Wmi(-1); } catch { }
            }
            lock (cache) cache[s.DeviceName] = v;
            if (done != null) done();
        });
    }

    static int pending = -1; static string pendingDev; static Screen pendingScreen; static bool writer;
    // Tekerlek hızlı döner: son değer yazılır, aradakiler atlanır
    public static void Set(Screen s, int v)
    {
        lock (cache) cache[s.DeviceName] = v;
        lock (gate)
        {
            pending = v; pendingDev = s.DeviceName; pendingScreen = s;
            if (writer) return;
            writer = true;
        }
        ThreadPool.QueueUserWorkItem(_ =>
        {
            while (true)
            {
                int val; Screen sc;
                lock (gate) { if (pending < 0) { writer = false; return; } val = pending; sc = pendingScreen; pending = -1; }
                try { if (Ddc(sc, val) < 0 && sc.Primary) Wmi(val); } catch { }
            }
        });
    }

    // set < 0: oku. Dönen: 0..100, desteklenmiyorsa -1
    static int Ddc(Screen s, int set)
    {
        var mon = MonitorFromPoint(new Point(s.Bounds.X + s.Bounds.Width / 2, s.Bounds.Y + s.Bounds.Height / 2), 2);
        uint n;
        if (!GetNumberOfPhysicalMonitorsFromHMONITOR(mon, out n) || n == 0) return -1;
        var arr = new PHYSICAL_MONITOR[n];
        if (!GetPhysicalMonitorsFromHMONITOR(mon, n, arr)) return -1;
        try
        {
            uint min, cur, max;
            if (!GetMonitorBrightness(arr[0].h, out min, out cur, out max) || max <= min) return -1;
            if (set < 0) return (int)Math.Round(100.0 * (cur - min) / (max - min));
            SetMonitorBrightness(arr[0].h, (uint)(min + Math.Round((max - min) * set / 100.0)));
            return set;
        }
        finally { DestroyPhysicalMonitors(n, arr); }
    }

    static int Wmi(int set)
    {
        using (var s = new System.Management.ManagementObjectSearcher("root\\WMI", set < 0 ? "SELECT CurrentBrightness FROM WmiMonitorBrightness" : "SELECT * FROM WmiMonitorBrightnessMethods"))
            foreach (System.Management.ManagementObject o in s.Get())
            {
                if (set < 0) return Convert.ToInt32(o["CurrentBrightness"]);
                o.InvokeMethod("WmiSetBrightness", new object[] { (uint)1, (byte)set });
                return set;
            }
        return -1;
    }
}

// ---------------- Sistem: işlemci, bellek, pil, ağ ----------------
static class Stats
{
    [StructLayout(LayoutKind.Sequential)] struct FILETIME { public uint lo, hi; public ulong V { get { return ((ulong)hi << 32) | lo; } } }
    [DllImport("kernel32.dll")] static extern bool GetSystemTimes(out FILETIME idle, out FILETIME kernel, out FILETIME user);
    [StructLayout(LayoutKind.Sequential)]
    class MEMORYSTATUSEX { public uint dwLength = 64, dwMemoryLoad; public ulong ullTotalPhys, ullAvailPhys, ullTotalPageFile, ullAvailPageFile, ullTotalVirtual, ullAvailVirtual, ullAvailExtendedVirtual; }
    [DllImport("kernel32.dll")] static extern bool GlobalMemoryStatusEx([In, Out] MEMORYSTATUSEX m);
    [StructLayout(LayoutKind.Sequential)] struct SYSTEM_POWER_STATUS { public byte ACLineStatus, BatteryFlag, BatteryLifePercent, SystemStatusFlag; public int BatteryLifeTime, BatteryFullLifeTime; }
    [DllImport("kernel32.dll")] static extern bool GetSystemPowerStatus(out SYSTEM_POWER_STATUS s);

    public static double Cpu, Mem, Swap = -1;
    public static ulong MemTotal, MemUsed, SwapTotal, SwapUsed;
    public static int Battery = -1; public static bool Charging;
    public static string Net = "wifi_off";
    public static bool MicMuted;
    static ulong lastIdle, lastTotal;

    public static void Start()
    {
        new Thread(() =>
        {
            int n = 0;
            while (true)
            {
                try { if (Sample(n++ % 2 == 0)) BarHost.Refresh(); } catch (Exception ex) { Slider.Log("bar: ölçüm: " + ex.Message); }
                Thread.Sleep(1500);
            }
        }) { IsBackground = true, Priority = ThreadPriority.BelowNormal, Name = "bar-stats" }.Start();
    }

    // Değişen bir şey varsa true. İşlemci / bellek 3 sn'de bir (Zebar'daki gibi), ses / mikrofon / pil / ağ 1,5 sn'de bir.
    static bool Sample(bool slow)
    {
        string before = Key();
        if (slow)
        {
            FILETIME i, k, u;
            if (GetSystemTimes(out i, out k, out u))
            {
                ulong idle = i.V, total = k.V + u.V;
                if (lastTotal > 0 && total > lastTotal) Cpu = 100.0 * (1 - (double)(idle - lastIdle) / (total - lastTotal));
                lastIdle = idle; lastTotal = total;
            }
            var m = new MEMORYSTATUSEX();
            if (GlobalMemoryStatusEx(m))
            {
                MemTotal = m.ullTotalPhys; MemUsed = m.ullTotalPhys - m.ullAvailPhys;
                Mem = 100.0 * MemUsed / Math.Max(1, MemTotal);
                // Takas dosyası: sayfa dosyası sınırı fiziksel belleği aşan kısım
                SwapTotal = m.ullTotalPageFile > m.ullTotalPhys ? m.ullTotalPageFile - m.ullTotalPhys : 0;
                ulong commit = m.ullTotalPageFile - m.ullAvailPageFile;
                SwapUsed = commit > MemUsed ? Math.Min(SwapTotal, commit - MemUsed) : 0;
                Swap = SwapTotal > 0 ? 100.0 * SwapUsed / SwapTotal : -1;
            }
            Net = Network();
        }
        SYSTEM_POWER_STATUS ps;
        if (GetSystemPowerStatus(out ps) && (ps.BatteryFlag & 128) == 0 && ps.BatteryLifePercent <= 100) { Battery = ps.BatteryLifePercent; Charging = ps.ACLineStatus == 1; }
        else Battery = -1;
        Volume.Read();
        try { MicMuted = Mic.IsMuted(); } catch { }
        return Key() != before;
    }
    static string Key() { return Math.Round(Cpu) + "|" + Math.Round(Mem) + "|" + Math.Round(Swap) + "|" + Battery + Charging + "|" + Net + "|" + Volume.Level + Volume.Muted + MicMuted; }

    // ---- Ağ: varsayılan ağ geçidi olan bağlantı; Wi-Fi ise sinyal gücü (wlanapi) ----
    static string Network()
    {
        string best = "wifi_off";
        try
        {
            foreach (var ni in System.Net.NetworkInformation.NetworkInterface.GetAllNetworkInterfaces())
            {
                if (ni.OperationalStatus != System.Net.NetworkInformation.OperationalStatus.Up) continue;
                var t = ni.NetworkInterfaceType;
                if (t == System.Net.NetworkInformation.NetworkInterfaceType.Loopback || t == System.Net.NetworkInformation.NetworkInterfaceType.Tunnel) continue;
                if (ni.GetIPProperties().GatewayAddresses.Count == 0) continue;
                if (t == System.Net.NetworkInformation.NetworkInterfaceType.Wireless80211)
                {
                    int q = WifiQuality();
                    best = q >= 75 || q < 0 ? "wifi4" : q >= 50 ? "wifi3" : q >= 25 ? "wifi2" : "wifi1";
                }
                else return "lan"; // kablo varsa o
            }
        }
        catch { }
        return best;
    }

    [DllImport("wlanapi.dll")] static extern int WlanOpenHandle(uint ver, IntPtr r, out uint neg, out IntPtr h);
    [DllImport("wlanapi.dll")] static extern int WlanCloseHandle(IntPtr h, IntPtr r);
    [DllImport("wlanapi.dll")] static extern int WlanEnumInterfaces(IntPtr h, IntPtr r, out IntPtr list);
    [DllImport("wlanapi.dll")] static extern int WlanQueryInterface(IntPtr h, ref Guid iface, int opcode, IntPtr r, out int size, out IntPtr data, IntPtr type);
    [DllImport("wlanapi.dll")] static extern void WlanFreeMemory(IntPtr p);
    static IntPtr wlan;
    // WLAN_CONNECTION_ATTRIBUTES: durum(4) + mod(4) + profil adı(512) + DOT11_SSID(36) + BSS türü(4) + MAC(6, +2 hizalama)
    // + PHY türü(4) + PHY sırası(4) = 576: wlanSignalQuality (0..100)
    static int WifiQuality()
    {
        try
        {
            uint neg;
            if (wlan == IntPtr.Zero && WlanOpenHandle(2, IntPtr.Zero, out neg, out wlan) != 0) { wlan = IntPtr.Zero; return -1; }
            IntPtr list;
            if (WlanEnumInterfaces(wlan, IntPtr.Zero, out list) != 0) return -1;
            try
            {
                int count = Marshal.ReadInt32(list, 0);
                for (int i = 0; i < count; i++)
                {
                    IntPtr info = list + 8 + i * 532; // WLAN_INTERFACE_INFO: GUID(16) + açıklama(512) + durum(4)
                    var gb = new byte[16]; Marshal.Copy(info, gb, 0, 16);
                    var g = new Guid(gb);
                    if (Marshal.ReadInt32(info, 528) != 1) continue; // wlan_interface_state_connected
                    int size; IntPtr data;
                    if (WlanQueryInterface(wlan, ref g, 7 /*current_connection*/, IntPtr.Zero, out size, out data, IntPtr.Zero) != 0) continue;
                    try { if (size >= 580) return Marshal.ReadInt32(data, 576); }
                    finally { WlanFreeMemory(data); }
                }
            }
            finally { WlanFreeMemory(list); }
        }
        catch { }
        return -1;
    }

    // ---- Sıcaklıklar (yalnızca kaynak kutusu açıkken; ll-temps.exe, LibreHardwareMonitor) ----
    public static Dictionary<string, object> Temps;
    static int tempsReading;
    public static void ReadTemps()
    {
        if (Interlocked.Exchange(ref tempsReading, 1) == 1) return;
        ThreadPool.QueueUserWorkItem(_ =>
        {
            try
            {
                string exe = System.IO.Path.Combine(LLShell.Home, @".glzr\logical-lunge\tools\lhm\ll-temps.exe");
                if (!System.IO.File.Exists(exe)) return;
                using (var p = Process.Start(new ProcessStartInfo(exe, "--read") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true }))
                {
                    string o = p.StandardOutput.ReadToEnd(); p.WaitForExit(3000);
                    Temps = new JavaScriptSerializer().DeserializeObject(o) as Dictionary<string, object>;
                }
                BarHost.Refresh();
            }
            catch { }
            finally { tempsReading = 0; }
        });
    }
}

// ---------------- Medya (media-art.exe --watch: Windows'un medya oturumu, WinRT) ----------------
static class MediaWatch
{
    public static string Title = "", Artist = "";
    public static bool Playing;
    public static double Pos, End;
    public static DateTime At = DateTime.UtcNow;
    public static Bitmap Art;
    static Process proc;
    static readonly object gate = new object();

    public static double Position { get { return Math.Min(End > 0 ? End : double.MaxValue, Pos + (Playing ? (DateTime.UtcNow - At).TotalSeconds : 0)); } }

    public static void Start()
    {
        new Thread(() =>
        {
            string exe = System.IO.Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "media-art.exe");
            if (!System.IO.File.Exists(exe)) { Slider.Log("bar: media-art.exe yok, medya gösterilmeyecek"); return; }
            int fails = 0;
            while (true)
            {
                try
                {
                    var p = Process.Start(new ProcessStartInfo(exe, "--watch") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardInput = true, StandardOutputEncoding = new UTF8Encoding(false) });
                    KillJob.Attach(p); // kabuk kapanınca o da kapansın
                    lock (gate) proc = p;
                    var json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };
                    string line;
                    var started = DateTime.UtcNow;
                    while ((line = p.StandardOutput.ReadLine()) != null)
                    {
                        try { Apply(json.DeserializeObject(line) as Dictionary<string, object>); } catch { }
                    }
                    fails = (DateTime.UtcNow - started).TotalSeconds < 10 ? fails + 1 : 0;
                }
                catch (Exception ex) { Slider.Log("bar: media-art: " + ex.Message); fails++; }
                Title = ""; BarHost.Refresh();
                Thread.Sleep(Math.Min(60000, 2000 * (1 << Math.Min(fails, 5))));
            }
        }) { IsBackground = true, Name = "bar-media" }.Start();
    }

    static void Apply(Dictionary<string, object> d)
    {
        if (d == null) return;
        object v;
        if (d.TryGetValue("art", out v))
        {
            Bitmap b = null;
            string s = v as string;
            int comma = s == null ? -1 : s.IndexOf(',');
            if (comma > 0) try { b = new Bitmap(new System.IO.MemoryStream(Convert.FromBase64String(s.Substring(comma + 1)))); } catch { }
            var old = Art; Art = b; if (old != null) old.Dispose();
        }
        else
        {
            Title = J.Str(d, "title"); Artist = J.Str(d, "artist"); Playing = J.Bool(d, "playing");
            Pos = d.TryGetValue("pos", out v) && v != null ? Convert.ToDouble(v) : 0;
            End = d.TryGetValue("end", out v) && v != null ? Convert.ToDouble(v) : 0;
            At = DateTime.UtcNow;
        }
        BarHost.Refresh();
    }

    // toggle | next | prev | stop | seek <sn>
    public static void Send(string cmd)
    {
        if (cmd.StartsWith("seek ")) { double s; if (double.TryParse(cmd.Substring(5), System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out s)) { Pos = s; At = DateTime.UtcNow; } }
        ThreadPool.QueueUserWorkItem(_ =>
        {
            lock (gate)
            {
                try { if (proc != null && !proc.HasExited) { proc.StandardInput.WriteLine(cmd); proc.StandardInput.Flush(); } }
                catch { }
            }
        });
    }
}

// ---------------- GlazeWM: workspace'ler (olaylarla, IPC) ----------------
static class Gw
{
    public class Ws { public int Num; public string Name; public bool Displayed, Focused, Occupied; public string Title; public Bitmap Icon; }
    public class Mon { public Rectangle Rect; public bool Focused; public List<Ws> Workspaces = new List<Ws>(); }
    public static volatile List<Mon> Monitors = new List<Mon>();
    public static volatile Dictionary<int, Ws> ByNum = new Dictionary<int, Ws>(); // dolu workspace'ler (tüm monitörler)
    public static volatile bool Paused;
    public static volatile string[] Modes = new string[0];
    public static int FocusedNum = 1;

    static readonly AutoResetEvent dirty = new AutoResetEvent(true);
    static readonly Glaze glaze = new Glaze();
    public static void Command(string cmd) { ThreadPool.QueueUserWorkItem(_ => glaze.Command(cmd)); }

    public static void Start()
    {
        new Thread(Listen) { IsBackground = true, Name = "bar-gw-events" }.Start();
        new Thread(() =>
        {
            while (true)
            {
                dirty.WaitOne();
                Thread.Sleep(25); // olay yağmurunu tek sorguda topla
                try { Query(); } catch (Exception ex) { Slider.Log("bar: glazewm: " + ex.Message); }
                BarHost.Refresh();
            }
        }) { IsBackground = true, Name = "bar-gw-query" }.Start();
        // Başlık değişiklikleri için olay yok: seyrek tazele (workspace ipuçları)
        new Thread(() => { while (true) { Thread.Sleep(5000); dirty.Set(); } }) { IsBackground = true, Priority = ThreadPriority.Lowest }.Start();
    }

    static void Listen()
    {
        while (true)
        {
            try
            {
                var ws = new ClientWebSocket();
                ws.Options.Proxy = null;
                if (!ws.ConnectAsync(new Uri("ws://127.0.0.1:6123"), CancellationToken.None).Wait(3000)) throw new Exception("bağlanamadı");
                var sub = Encoding.UTF8.GetBytes("sub --events all");
                ws.SendAsync(new ArraySegment<byte>(sub), WebSocketMessageType.Text, true, CancellationToken.None).Wait(1500);
                dirty.Set();
                var buf = new byte[1 << 16];
                while (ws.State == WebSocketState.Open)
                {
                    WebSocketReceiveResult r;
                    do { r = ws.ReceiveAsync(new ArraySegment<byte>(buf), CancellationToken.None).Result; } while (!r.EndOfMessage && r.MessageType != WebSocketMessageType.Close);
                    if (r.MessageType == WebSocketMessageType.Close) break;
                    dirty.Set();
                }
            }
            catch { }
            Monitors = new List<Mon>(); ByNum = new Dictionary<int, Ws>(); BarHost.Refresh();
            Thread.Sleep(2000); // GlazeWM yeniden başlarsa tekrar bağlan
        }
    }

    static void Query()
    {
        var mons = new List<Mon>();
        var occ = new Dictionary<int, Ws>();
        foreach (var m in glaze.Monitors())
        {
            var mon = new Mon { Rect = new Rectangle(J.Int(m, "x"), J.Int(m, "y"), J.Int(m, "width"), J.Int(m, "height")), Focused = J.Bool(m, "hasFocus") };
            foreach (Dictionary<string, object> w in J.Children(m))
            {
                if (J.Str(w, "type") != "workspace") continue;
                var ws = new Ws { Name = J.Str(w, "name"), Displayed = J.Bool(w, "isDisplayed"), Focused = J.Bool(w, "hasFocus") };
                int.TryParse(ws.Name, out ws.Num);
                // En büyük (simge durumunda olmayan) pencerenin ikonu: ii bar.workspaces.showAppIcons
                var wins = new List<Dictionary<string, object>>();
                J.WindowNodes(w, wins);
                Dictionary<string, object> big = null; long bigArea = -1;
                foreach (var win in wins)
                {
                    var st = win.ContainsKey("state") ? win["state"] as Dictionary<string, object> : null;
                    if (st != null && J.Str(st, "type") == "minimized") continue;
                    long area = (long)J.Int(win, "width") * J.Int(win, "height");
                    if (area > bigArea) { bigArea = area; big = win; }
                }
                if (big != null)
                {
                    ws.Occupied = true;
                    ws.Title = J.Str(big, "title");
                    object hv; long hl = 0;
                    if (big.TryGetValue("handle", out hv) && hv != null) hl = Convert.ToInt64(hv);
                    ws.Icon = AppIcons.For(new IntPtr(hl));
                    if (ws.Num > 0) occ[ws.Num] = ws;
                }
                if (ws.Focused && ws.Num > 0) FocusedNum = ws.Num;
                mon.Workspaces.Add(ws);
            }
            mons.Add(mon);
        }
        Monitors = mons; ByNum = occ;
        var pr = QueryData("query paused");
        if (pr != null) Paused = J.Bool(pr, "paused");
        var br = QueryData("query binding-modes");
        var modes = new List<string>();
        object bm;
        if (br != null && br.TryGetValue("bindingModes", out bm) && bm is object[])
            foreach (Dictionary<string, object> x in (object[])bm) { string dn = J.Str(x, "displayName"); modes.Add(dn.Length > 0 ? dn : J.Str(x, "name")); }
        Modes = modes.ToArray();
    }

    static Dictionary<string, object> QueryData(string q)
    {
        try
        {
            var mi = typeof(Glaze).GetMethod("Send", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var res = mi.Invoke(glaze, new object[] { q }) as Dictionary<string, object>;
            return res == null ? null : res["data"] as Dictionary<string, object>;
        }
        catch { return null; }
    }
}

// ---------------- Uygulama ikonları (exe'nin kendi ikonu; önbellekli) ----------------
static class AppIcons
{
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool QueryFullProcessImageName(IntPtr h, int flags, StringBuilder sb, ref int size);
    [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "GetClassLongPtr")] static extern IntPtr GetClassLongPtr(IntPtr h, int idx);

    static readonly Dictionary<string, Bitmap> cache = new Dictionary<string, Bitmap>(StringComparer.OrdinalIgnoreCase);

    public static string ExePath(IntPtr hwnd)
    {
        uint pid; Native.GetWindowThreadProcessId(hwnd, out pid);
        return ExePath(pid);
    }
    public static string ExePath(uint pid)
    {
        IntPtr h = OpenProcess(0x1000 /*QUERY_LIMITED_INFORMATION*/, false, pid);
        if (h == IntPtr.Zero) return null;
        try { var sb = new StringBuilder(1024); int n = sb.Capacity; return QueryFullProcessImageName(h, 0, sb, ref n) ? sb.ToString() : null; }
        finally { CloseHandle(h); }
    }

    public static Bitmap For(IntPtr hwnd)
    {
        if (hwnd == IntPtr.Zero) return null;
        string path = ExePath(hwnd);
        if (path == null) return null;
        // UWP uygulamaları ApplicationFrameHost içinde: exe'nin ikonu değil, pencerenin ikonu
        bool frame = path.EndsWith("ApplicationFrameHost.exe", StringComparison.OrdinalIgnoreCase);
        string key = frame ? "hwnd:" + hwnd.ToInt64() : path;
        Bitmap b;
        lock (cache) if (cache.TryGetValue(key, out b)) return b;
        try
        {
            if (frame)
            {
                IntPtr hi;
                SendMessageTimeout(hwnd, 0x7F /*WM_GETICON*/, (IntPtr)1 /*ICON_BIG*/, IntPtr.Zero, 0x2 /*ABORTIFHUNG*/, 100, out hi);
                if (hi == IntPtr.Zero) hi = GetClassLongPtr(hwnd, -14 /*GCLP_HICON*/);
                if (hi != IntPtr.Zero) using (var ic = Icon.FromHandle(hi)) b = ic.ToBitmap();
            }
            else using (var ic = Icon.ExtractAssociatedIcon(path)) if (ic != null) b = ic.ToBitmap();
        }
        catch { }
        lock (cache) cache[key] = b;
        return b;
    }
}

// ---------------- Bar'lar (her monitöre bir tane) ----------------
static class BarHost
{
    static readonly List<BarForm> bars = new List<BarForm>();
    static Control ui;

    public static void Start(Control owner)
    {
        ui = owner;
        Build();
        Microsoft.Win32.SystemEvents.DisplaySettingsChanged += (s, e) => { try { ui.BeginInvoke((Action)Build); } catch { } };
        TrayHost.Changed += Refresh;
        Gw.Start();
        Stats.Start();
        Volume.Read();
        // Saat (dakika başında) ve çalan medyanın ilerleme halkası
        var t = new System.Windows.Forms.Timer { Interval = 1000 };
        string minute = "";
        t.Tick += (s, e) =>
        {
            string m = DateTime.Now.ToString("yyyyMMddHHmm");
            if (m != minute || MediaWatch.Playing) { minute = m; Refresh(); }
        };
        t.Start();
        ActiveWindow.Start(ui);
    }

    static void Build()
    {
        foreach (var b in bars) { try { b.Close(); b.Dispose(); } catch { } }
        bars.Clear();
        foreach (var s in Screen.AllScreens)
        {
            var b = new BarForm(s);
            b.Show();
            bars.Add(b);
        }
        Slider.Log("bar: " + bars.Count + " monitörde native bar");
    }

    // Her thread'den çağrılabilir
    public static void Refresh()
    {
        var c = ui;
        if (c == null || !c.IsHandleCreated) return;
        try { c.BeginInvoke((Action)(() => { foreach (var b in bars) b.Invalidate(); })); } catch { }
    }

    public static void ShowOsd(string kind, int value)
    {
        var c = ui;
        if (c == null) return;
        c.BeginInvoke((Action)(() =>
        {
            // Farenin olduğu monitörün bar'ında (ii: odaktaki ekran)
            BarForm target = null;
            foreach (var b in bars) if (b.Screen.Bounds.Contains(Cursor.Position)) target = b;
            if (target == null && bars.Count > 0) target = bars[0];
            if (target != null) target.ShowOsd(kind, value);
        }));
    }
}

// Odaktaki pencere (sol bölüm): Win32 olaylarıyla, GlazeWM'e sormadan
static class ActiveWindow
{
    public static string Cls = "", Title = "";
    public static bool Desktop = true;
    static Native.WinEventDelegate cb;
    static IntPtr fg;

    public static void Start(Control ui)
    {
        cb = (hook, ev, h, obj, child, thread, time) =>
        {
            if (ev == 3) { fg = h; Update(); }
            else if (obj == 0 && h == fg) Update(); // EVENT_OBJECT_NAMECHANGE (başlık)
        };
        Native.SetWinEventHook(3, 3, IntPtr.Zero, cb, 0, 0, 0);                 // EVENT_SYSTEM_FOREGROUND
        Native.SetWinEventHook(0x800C, 0x800C, IntPtr.Zero, cb, 0, 0, 0);       // EVENT_OBJECT_NAMECHANGE
        fg = Native.GetForegroundWindow();
        Update();
    }

    static void Update()
    {
        string cls = "", title = "";
        bool desk = true;
        if (fg != IntPtr.Zero)
        {
            var t = new StringBuilder(256); Native.GetWindowText(fg, t, 256); title = t.ToString();
            var c = new StringBuilder(64); Native.GetClassName(fg, c, 64);
            string cs = c.ToString();
            uint pid; Native.GetWindowThreadProcessId(fg, out pid);
            bool mine = pid == (uint)Process.GetCurrentProcess().Id;
            if (!mine && cs != "Progman" && cs != "WorkerW" && cs != "Shell_TrayWnd" && !title.StartsWith("Zebar") && !title.StartsWith("ll-"))
            {
                string path = AppIcons.ExePath(pid);
                cls = path == null ? "" : System.IO.Path.GetFileNameWithoutExtension(path).ToLowerInvariant();
                desk = false;
            }
        }
        if (cls == Cls && title == Title && desk == Desktop) return;
        Cls = cls; Title = title; Desktop = desk;
        BarHost.Refresh();
    }
}

// ---------------- Bar penceresi ----------------
class BarForm : Form
{
    const int SHOWN = 10, WS = 26, MARGIN = 2, BAR_H = 40, EDGE = 23;
    static readonly int[] SIDE_W = { 360, 280, 190 };

    public readonly Screen Screen;
    readonly float S; // DPI ölçeği
    readonly ToolTip tip = new ToolTip { InitialDelay = 500, ReshowDelay = 100, ShowAlways = true };

    // Tıklanabilir alanlar (CSS pikseli; her çizimde yeniden kurulur)
    class Hit
    {
        public RectangleF R; public string Id, Tip; public bool Round = true;
        public Action Left, Right, Middle, Double, Hover;
    }
    readonly List<Hit> hits = new List<Hit>();
    Hit hover;
    RectangleF wsTrack, leftZone, rightZone;

    // Aktif workspace göstergesi: ön kenar 100 ms, arka kenar 300 ms (ii AnimatedTabIndexPair)
    float pillL = -1, pillR = -1, fromL, fromR, toL, toR;
    DateTime animAt;
    readonly System.Windows.Forms.Timer anim = new System.Windows.Forms.Timer { Interval = 15 };

    Popup pop;          // üzerine gelince açılan kutu (kaynaklar / medya) ya da tray taşması
    OsdPopup osd;

    [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr mon, int type, out uint x, out uint y);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point p, uint flags);

    public BarForm(Screen s)
    {
        Screen = s;
        uint dx = 96, dy;
        try { GetDpiForMonitor(MonitorFromPoint(new Point(s.Bounds.X + 1, s.Bounds.Y + 1), 2), 0, out dx, out dy); } catch { }
        S = dx / 96f;
        AutoScaleMode = AutoScaleMode.None;
        FormBorderStyle = FormBorderStyle.None;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.Manual;
        Text = LLShell.BarTitle;
        BackColor = Look.Layer0;
        SetStyle(ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.UserPaint | ControlStyles.ResizeRedraw, true);
        Bounds = new Rectangle(s.Bounds.X, s.Bounds.Y, s.Bounds.Width, (int)Math.Round(BAR_H * S));
        anim.Tick += (o, e) => Invalidate();
        Brightness.ReadAsync(s, BarHost.Refresh);
    }

    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= 0x80 /*TOOLWINDOW*/ | 0x08000000 /*NOACTIVATE*/;
            return cp;
        }
    }
    protected override bool ShowWithoutActivation { get { return true; } }

    protected override void WndProc(ref Message m)
    {
        switch (m.Msg)
        {
            case 0x0021: m.Result = (IntPtr)3; return;  // WM_MOUSEACTIVATE -> MA_NOACTIVATE: tıklayınca odak uygulamada kalsın
            case 0x02E0: return;                        // WM_DPICHANGED: yerimizi biz belirliyoruz
            case 0x020A:                                // WM_MOUSEWHEEL
                {
                    int delta = (short)((m.WParam.ToInt64() >> 16) & 0xFFFF);
                    var p = PointToClient(new Point((short)(m.LParam.ToInt64() & 0xFFFF), (short)((m.LParam.ToInt64() >> 16) & 0xFFFF)));
                    Wheel(new PointF(p.X / S, p.Y / S), delta);
                    m.Result = IntPtr.Zero;
                    return;
                }
        }
        base.WndProc(ref m);
    }

    // ---- Çizim ----
    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.Clear(Look.Layer0);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
        g.PixelOffsetMode = PixelOffsetMode.HighQuality;
        g.InterpolationMode = InterpolationMode.HighQualityBicubic;
        g.ScaleTransform(S, S);
        hits.Clear();

        float W = ClientSize.Width / S;
        int level = W <= 1100 ? 2 : W <= 1440 ? 1 : 0;
        float sideW = SIDE_W[level];
        float middleW = sideW * 2 + (SHOWN * WS + 6) + 8;
        float sideSpace = (W - middleW) / 2;
        leftZone = new RectangleF(0, 0, sideSpace, BAR_H);
        rightZone = new RectangleF(W - sideSpace, 0, sideSpace, BAR_H);

        DrawHints(g, W);
        DrawLeft(g, sideSpace, level);
        float x = sideSpace;
        DrawLeftGroup(g, new RectangleF(x, 4, sideW, 32), level);
        x += sideW + 4;
        DrawWorkspaces(g, new RectangleF(x, 4, SHOWN * WS + 6, 32));
        x += SHOWN * WS + 6 + 4;
        DrawRightGroup(g, new RectangleF(x, 4, sideW, 32), level);
        DrawRight(g, W, sideSpace);
    }

    bool Hovered(string id) { return hover != null && hover.Id == id; }

    void HoverBg(Graphics g, string id, RectangleF r, float rad)
    {
        if (Hovered(id)) Look.Fill(g, r, rad, Look.Layer1Hover);
    }

    // ii ScrollHint: kenarlarda tekerleğin ne yaptığını gösteren soluk oklar
    void DrawHints(Graphics g, float W)
    {
        var mouse = PointToClient(Cursor.Position);
        var mp = new PointF(mouse.X / S, mouse.Y / S);
        foreach (var side in new[] { 0, 1 })
        {
            var zone = side == 0 ? leftZone : rightZone;
            float a = zone.Contains(mp) ? 1f : 0.45f;
            float cx = side == 0 ? 4 + 7 : W - 4 - 7;
            var c = Look.Alpha(Look.Subtext, a);
            Look.Icon(g, "keyboard_arrow_up", cx, 20 - 9, 14, c);
            Look.Icon(g, side == 0 ? "light_mode" : "volume_up", cx, 20, 14, c);
            Look.Icon(g, "keyboard_arrow_down", cx, 20 + 9, 14, c);
        }
    }

    void DrawLeft(Graphics g, float sideSpace, int level)
    {
        float x = EDGE;
        var btn = new RectangleF(x, 4, 32, 32);
        HoverBg(g, "search", btn, 16);
        Look.Icon(g, "search", x + 16, 20, 21, Look.OnLayer);
        hits.Add(new Hit { R = btn, Id = "search", Tip = "Arama / Overview (Super)", Left = () => LLShell.HelperAsync("/shell?a=overview") });
        x += 32;
        // GlazeWM duraklatıldı / bağlama modları (ii'deki gibi, tıklayınca kapanır)
        if (Gw.Paused) x = Pill(g, x + 8, Tr.T("Duraklatıldı"), Color.FromArgb(0x93, 0, 0x0a), Color.FromArgb(0xff, 0xda, 0xd6), "paused", () => Gw.Command("wm-toggle-pause"));
        foreach (var mode in Gw.Modes) { string md = mode; x = Pill(g, x + 8, md, Look.Primary, Look.OnPrimary, "mode:" + md, () => Gw.Command("wm-disable-binding-mode --name " + md)); }
        if (level != 0) return;
        // Odaktaki pencere: süreç adı (küçük, soluk) + başlık
        float w = sideSpace - x - 10 - EDGE;
        if (w < 40) return;
        string cls = ActiveWindow.Desktop ? "Masaüstü" : ActiveWindow.Cls;
        string title = ActiveWindow.Desktop ? "Workspace " + Gw.FocusedNum : ActiveWindow.Title;
        Look.Str(g, Tr.T(cls), Look.Text(12), Look.Subtext, new RectangleF(x + 10, 5, w, 16), Look.Left);
        Look.Str(g, title, Look.Text(15), Look.OnLayer, new RectangleF(x + 10, 17, w, 20), Look.Left);
    }

    float Pill(Graphics g, float x, string text, Color bg, Color fg, string id, Action click)
    {
        var f = Look.Text(13);
        float w = Look.Width(g, text, f) + 20;
        var r = new RectangleF(x, 7, w, 26);
        Look.Fill(g, r, 13, bg);
        Look.Str(g, text, f, fg, r, Look.Center);
        hits.Add(new Hit { R = r, Id = id, Left = click });
        return x + w;
    }

    // Kaynaklar (RAM, takas, CPU halkaları) + medya
    void DrawLeftGroup(Graphics g, RectangleF r, int level)
    {
        Look.Fill(g, r, 12, Look.Layer1);
        float x = r.X + 5 + 4;
        float start = x;
        x = Resource(g, x, "memory", Stats.Mem, 95);
        if (Stats.Swap >= 0) x = Resource(g, x + 6, "swap_horiz", Stats.Swap, 85);
        x = Resource(g, x + 6, "planner_review", Stats.Cpu, 90);
        hits.Add(new Hit { R = new RectangleF(start - 4, r.Y, x - start + 8, r.Height), Id = "res", Round = false, Hover = () => OpenPop("res") });
        if (level >= 2) return;
        // Medya: ilerleme halkalı düğme + başlık • sanatçı
        x += 4 + 6;
        var mr = new RectangleF(x, r.Y, r.Right - 5 - 4 - x, r.Height);
        if (mr.Width < 30) return;
        float cx = x + 11, cy = r.Y + 16;
        if (MediaWatch.Title.Length == 0)
        {
            Look.Fill(g, new RectangleF(x, cy - 11, 22, 22), 11, Look.SecCont);
            Look.Icon(g, "music_note", cx, cy, 16, Look.OnSecCont);
            Look.Str(g, Tr.T("Medya yok"), Look.Text(15), Look.Subtext, new RectangleF(x + 28, r.Y, mr.Width - 28, r.Height), Look.Left);
            return;
        }
        double prog = MediaWatch.End > 0 ? MediaWatch.Position / MediaWatch.End : 0;
        Look.Ring(g, cx, cy, 9.5f, 2, prog, Look.SecCont, Look.OnSecCont);
        Look.Icon(g, MediaWatch.Playing ? "pause" : "music_note", cx, cy, 14, Look.OnSecCont);
        string text = MediaWatch.Artist.Length > 0 ? MediaWatch.Title + " • " + MediaWatch.Artist : MediaWatch.Title;
        Look.Str(g, text, Look.Text(15), Look.OnLayer, new RectangleF(x + 28, r.Y, mr.Width - 28, r.Height), Look.Left);
        hits.Add(new Hit { R = mr, Id = "media", Round = false, Left = () => MediaWatch.Send("toggle"), Right = () => MediaWatch.Send("next"), Middle = () => MediaWatch.Send("prev"), Hover = () => OpenPop("media") });
    }

    float Resource(Graphics g, float x, string icon, double pct, int warn)
    {
        int p = (int)Math.Round(Math.Max(0, Math.Min(100, pct)));
        Look.Ring(g, x + 10, 20, 8.5f, 2, p / 100.0, Look.SecCont, p >= warn ? Look.Error : Look.OnSecCont);
        Look.Icon(g, icon, x + 10, 20, 13, Look.OnSecCont);
        Look.Str(g, p.ToString(), Look.Text(15), Look.OnLayer, new RectangleF(x + 22, 4, 26, 32), Look.Center);
        return x + 22 + 26;
    }

    // ii Workspaces.qml: dolu olanların birleşik arka planı, kayan aktif gösterge, en büyük pencerenin ikonu
    void DrawWorkspaces(Graphics g, RectangleF r)
    {
        Look.Fill(g, r, 12, Look.Layer1);
        float tx = r.X + 3, ty = r.Y + 3;
        wsTrack = new RectangleF(tx, ty, SHOWN * WS, WS);

        Gw.Mon mon = MyMonitor();
        int current = Gw.FocusedNum;
        if (mon != null) foreach (var w in mon.Workspaces) if (w.Displayed && w.Num > 0) current = w.Num;
        int bse = (current - 1) / SHOWN * SHOWN;
        int idx = current - bse - 1;
        var occ = Gw.ByNum;

        // Dolu workspace'lerin arka planı: yan yana olanlar tek hap
        int i = 0;
        while (i < SHOWN)
        {
            if (!occ.ContainsKey(bse + i + 1)) { i++; continue; }
            int j = i;
            while (j + 1 < SHOWN && occ.ContainsKey(bse + j + 2)) j++;
            Look.Fill(g, new RectangleF(tx + i * WS, ty, (j - i + 1) * WS, WS), WS / 2f, Look.Occupied);
            i = j + 1;
        }

        // Aktif gösterge
        float tl = idx * WS + MARGIN, tr = (idx + 1) * WS - MARGIN;
        if (pillL < 0) { pillL = fromL = toL = tl; pillR = fromR = toR = tr; }
        if (tl != toL || tr != toR) { fromL = pillL; fromR = pillR; toL = tl; toR = tr; animAt = DateTime.UtcNow; anim.Start(); }
        double ms = (DateTime.UtcNow - animAt).TotalMilliseconds;
        bool right = toL >= fromL;
        // Gidilen yöndeki kenar hızlı (100 ms), arkadaki yavaş (300 ms) — OutSine
        double fastT = Math.Min(1, ms / 100), slowT = Math.Min(1, ms / 300);
        Func<double, double> ease = t => Math.Sin(t * Math.PI / 2);
        pillL = (float)(fromL + (toL - fromL) * ease(right ? slowT : fastT));
        pillR = (float)(fromR + (toR - fromR) * ease(right ? fastT : slowT));
        if (slowT >= 1) anim.Stop();
        Look.Fill(g, new RectangleF(tx + pillL, ty + 2, pillR - pillL, WS - 4), (WS - 4) / 2f, Look.Primary);

        for (int k = 0; k < SHOWN; k++)
        {
            int n = bse + k + 1;
            Gw.Ws w; occ.TryGetValue(n, out w);
            var cell = new RectangleF(tx + k * WS, ty, WS, WS);
            string id = "ws" + n;
            if (Hovered(id)) Look.Fill(g, RectangleF.Inflate(cell, -2, -2), WS / 2f, Color.FromArgb(26, 208, 188, 255));
            if (w != null && w.Icon != null)
            {
                var ir = new RectangleF(cell.X + 4, cell.Y + 4, 18, 18);
                var state = g.Save();
                using (var clip = new GraphicsPath()) { clip.AddEllipse(ir); g.SetClip(clip); }
                g.DrawImage(w.Icon, ir);
                g.Restore(state);
            }
            else
            {
                var c = k == idx ? Look.OnPrimary : w != null ? Look.OnSecCont : Look.Inactive;
                using (var b = new SolidBrush(c)) g.FillEllipse(b, cell.X + WS / 2f - 2.35f, cell.Y + WS / 2f - 2.35f, 4.7f, 4.7f);
            }
            int num = n;
            hits.Add(new Hit { R = cell, Id = id, Tip = w != null ? n + ": " + w.Title : n.ToString(), Left = () => Slide(num.ToString()), Right = () => LLShell.HelperAsync("/shell?a=overview") });
        }
    }

    Gw.Mon MyMonitor()
    {
        foreach (var m in Gw.Monitors) if (m.Rect.X == Screen.Bounds.X && m.Rect.Y == Screen.Bounds.Y) return m;
        return null;
    }

    // Workspace geçişi asıl helper üzerinden (Hyprland slide animasyonu); helper yoksa doğrudan GlazeWM
    static void Slide(string target)
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            if (LLShell.Helper("/cmd?a=ws-" + target)) return;
            Gw.Command(target == "next" ? "focus --next-workspace" : target == "prev" ? "focus --prev-workspace" : "focus --workspace " + target);
        });
    }

    // Saat • tarih, araç düğmeleri, pil
    void DrawRightGroup(Graphics g, RectangleF r, int level)
    {
        Look.Fill(g, r, 12, Look.Layer1);
        float right = r.Right - 5;
        if (level < 2 && Stats.Battery >= 0)
        {
            var br = new RectangleF(right - 4 - 38, r.Y + 7, 38, 18);
            right = br.X - 4;
            bool low = Stats.Battery <= 20 && !Stats.Charging;
            var st = g.Save();
            using (var clip = Look.Round(br, 9)) { g.SetClip(clip); }
            using (var b = new SolidBrush(Look.SecCont)) g.FillRectangle(b, br);
            using (var b = new SolidBrush(low ? Look.Error : Look.OnSecCont)) g.FillRectangle(b, br.X, br.Y, br.Width * Stats.Battery / 100f, br.Height);
            g.Restore(st);
            var fc = Stats.Battery > 55 ? Look.Layer1 : Look.OnSecCont;
            string lbl = Stats.Battery.ToString();
            if (Stats.Charging && Stats.Battery < 100)
            {
                Look.Icon(g, "bolt", br.X + 11, br.Y + 9, 12, fc);
                Look.Str(g, lbl, Look.TextBold(11), fc, new RectangleF(br.X + 14, br.Y, br.Width - 16, br.Height), Look.Center);
            }
            else Look.Str(g, lbl, Look.TextBold(11), fc, br, Look.Center);
            hits.Add(new Hit { R = br, Id = "battery", Tip = Stats.Battery + "%" + (Stats.Charging ? " ⚡" : "") });
        }
        if (level == 0)
        {
            // ii UtilButtons: bölge ekran görüntüsü, ekran klavyesi
            foreach (var u in new[] { new[] { "osk", "keyboard", "Ekran klavyesi" }, new[] { "snip", "screenshot_region", "Bölge ekran görüntüsü" } })
            {
                var br = new RectangleF(right - 26, r.Y + 3, 26, 26);
                right = br.X - 4;
                Look.Fill(g, br, 13, Hovered(u[0]) ? Color.FromArgb(0x5a, 0x53, 0x6a) : Look.SecCont);
                Look.Icon(g, u[1], br.X + 13, br.Y + 13, 16, Look.OnSecCont);
                string what = u[0];
                hits.Add(new Hit
                {
                    R = br, Id = what, Tip = Tr.T(u[2]),
                    Left = () =>
                    {
                        if (what == "snip") { try { Process.Start(new ProcessStartInfo(Maint.HelperExe, "--snip") { UseShellExecute = true }); } catch { } }
                        else LLShell.HelperAsync("/shell?a=osk-toggle");
                    }
                });
            }
        }
        // Saat ortada
        var now = DateTime.Now;
        string time = now.ToString("HH:mm");
        string date = level < 2 ? now.ToString("dddd, dd/MM", System.Globalization.CultureInfo.CurrentCulture) : "";
        var tf = Look.Text(17); var df = Look.Text(15);
        float tw = Look.Width(g, time, tf), sw = date.Length > 0 ? Look.Width(g, " • ", df) : 0, dw = Look.Width(g, date, df);
        float avail = right - (r.X + 5);
        float total = tw + sw + dw;
        float cx = r.X + 5 + Math.Max(0, (avail - total) / 2);
        Look.Str(g, time, tf, Look.OnLayer, new RectangleF(cx, r.Y, tw + 2, r.Height), Look.Left);
        if (date.Length > 0)
        {
            Look.Str(g, " • ", df, Look.OnLayer, new RectangleF(cx + tw, r.Y, sw + 2, r.Height), Look.Left);
            Look.Str(g, date, df, Look.OnLayer, new RectangleF(cx + tw + sw, r.Y, Math.Min(dw + 2, right - cx - tw - sw), r.Height), Look.Left);
        }
    }

    // Sağ: göstergeler (sağ panel) + tray
    void DrawRight(Graphics g, float W, float sideSpace)
    {
        var icons = new List<string>();
        if (Volume.Level >= 0 && (Volume.Muted || Volume.Level == 0)) icons.Add("volume_off");
        if (Stats.MicMuted) icons.Add("mic_off");
        icons.Add(Stats.Net);
        float iw = 20 + icons.Count * 19 + (icons.Count - 1) * 15;
        var ir = new RectangleF(W - EDGE - iw, 5, iw, 30);
        HoverBg(g, "ind", ir, 15);
        float x = ir.X + 10;
        foreach (var ic in icons) { Look.Icon(g, ic, x + 9.5f, 20, 19, Look.OnLayer); x += 19 + 15; }
        hits.Add(new Hit { R = ir, Id = "ind", Left = () => LLShell.HelperAsync("/shell?a=sidebar-right-toggle") });

        // Tray: sığdığı kadar bar'da, kalanı ok işaretinin altındaki kutuda (ii SysTray)
        var tray = TrayHost.Visible();
        if (tray.Count == 0) return;
        float room = ir.X - 5 - (W - sideSpace) - 8;
        int fit = (int)Math.Floor((room + 2) / 28);
        bool overflow = tray.Count > fit;
        if (overflow) fit = Math.Max(0, fit - 1);
        float tx = ir.X - 5;
        for (int i = 0; i < Math.Min(fit, tray.Count); i++)
        {
            var ic = tray[i];
            tx -= 26;
            TrayCell(g, ic, new RectangleF(tx, 7, 26, 26));
            tx -= 2;
        }
        if (overflow)
        {
            tx -= 26;
            var mr = new RectangleF(tx, 7, 26, 26);
            HoverBg(g, "traymore", mr, 13);
            Look.Icon(g, "expand_more", mr.X + 13, mr.Y + 13, 20, Look.OnLayer);
            var rest = tray.GetRange(Math.Min(fit, tray.Count), tray.Count - Math.Min(fit, tray.Count));
            hits.Add(new Hit { R = mr, Id = "traymore", Left = () => OpenTray(rest) });
        }
    }

    void TrayCell(Graphics g, TrayHost.Icon ic, RectangleF r)
    {
        string id = "tray:" + ic.Key;
        HoverBg(g, id, r, 13);
        g.DrawImage(ic.Image, new RectangleF(r.X + 5, r.Y + 5, 16, 16));
        ic.Screen = ToScreen(r);
        var icon = ic;
        hits.Add(new Hit
        {
            R = r, Id = id, Tip = ic.Tip,
            Left = () => TrayHost.Click(icon, 0, false, Cursor.Position),
            Right = () => TrayHost.Click(icon, 1, false, Cursor.Position),
            Middle = () => TrayHost.Click(icon, 2, false, Cursor.Position),
            Double = () => TrayHost.Click(icon, 0, true, Cursor.Position),
            Hover = () => TrayHost.Hover(icon, Cursor.Position),
        });
    }

    public Rectangle ToScreen(RectangleF r)
    {
        return new Rectangle(Left + (int)(r.X * S), Top + (int)(r.Y * S), (int)(r.Width * S), (int)(r.Height * S));
    }

    // ---- Fare ----
    Hit At(Point client)
    {
        var p = new PointF(client.X / S, client.Y / S);
        for (int i = hits.Count - 1; i >= 0; i--) if (hits[i].R.Contains(p)) return hits[i];
        return null;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        var h = At(e.Location);
        string before = hover == null ? null : hover.Id;
        hover = h;
        if ((h == null ? null : h.Id) != before)
        {
            tip.SetToolTip(this, h != null && !string.IsNullOrEmpty(h.Tip) ? h.Tip : "");
            if (h != null && h.Hover != null) h.Hover();
            Invalidate();
        }
        else Invalidate(); // kenar ipuçları imleci izler
    }
    protected override void OnMouseLeave(EventArgs e) { base.OnMouseLeave(e); hover = null; tip.SetToolTip(this, ""); Invalidate(); }

    protected override void OnMouseUp(MouseEventArgs e)
    {
        base.OnMouseUp(e);
        var h = At(e.Location);
        if (h == null) return;
        Action a = e.Button == MouseButtons.Left ? h.Left : e.Button == MouseButtons.Right ? h.Right : e.Button == MouseButtons.Middle ? h.Middle : null;
        if (a != null) a();
    }
    protected override void OnMouseDoubleClick(MouseEventArgs e)
    {
        base.OnMouseDoubleClick(e);
        var h = At(e.Location);
        if (h != null && h.Double != null && e.Button == MouseButtons.Left) h.Double();
    }

    DateTime lastWheel;
    void Wheel(PointF p, int delta)
    {
        if (delta == 0) return;
        bool up = delta > 0;
        var now = DateTime.UtcNow;
        if (wsTrack.Contains(p))
        {
            if ((now - lastWheel).TotalMilliseconds < 100) return;
            lastWheel = now;
            Slide(up ? "prev" : "next");
            return;
        }
        if ((now - lastWheel).TotalMilliseconds < 40) return;
        lastWheel = now;
        if (rightZone.Contains(p))
        {
            if (Volume.Level < 0) return;
            Volume.Set(Volume.Level + (up ? 5 : -5));
            ShowOsd("volume", Volume.Muted ? 0 : Volume.Level);
            Invalidate();
        }
        else if (leftZone.Contains(p)) BrightnessWheel(up);
    }

    // Hyprland'deki gibi tek eksen: gama 0..100, sonra parlaklık 0..100 (0'ın altına inince yazılımsal karartma)
    void BrightnessWheel(bool up)
    {
        string dev = Screen.DeviceName;
        int gamma = NightLight.Gamma(dev);
        int? cur = Brightness.Cached(dev);
        if ((gamma < 100 && up) || !cur.HasValue || (!up && cur.Value == 0))
        {
            if (!cur.HasValue && !Brightness.Known(dev)) return; // henüz okunuyor
            gamma = Math.Max(0, Math.Min(100, gamma + (up ? 5 : -5)));
            int gv = gamma;
            ThreadPool.QueueUserWorkItem(_ => NightLight.SetGamma(dev, gv));
            ShowOsd("gamma", gamma);
            return;
        }
        int next = Math.Max(0, Math.Min(100, cur.Value + (up ? 5 : -5)));
        Brightness.Set(Screen, next);
        ShowOsd("brightness", next);
    }

    // ---- Açılır kutular ----
    void OpenPop(string kind)
    {
        if (pop != null && !pop.IsDisposed && pop.Kind == kind) return;
        ClosePop();
        var h = hover;
        if (h == null) return;
        pop = kind == "res" ? (Popup)new ResourcesPopup(this, S) : new MediaPopup(this, S);
        pop.OpenBelow(ToScreen(h.R), ToScreen(h.R));
    }
    void OpenTray(List<TrayHost.Icon> rest)
    {
        if (pop != null && !pop.IsDisposed && pop.Kind == "tray") { ClosePop(); return; }
        ClosePop();
        var h = hover;
        pop = new TrayPopup(this, S, rest);
        pop.OpenBelow(ToScreen(h.R), Rectangle.Empty);
    }
    void ClosePop() { if (pop != null && !pop.IsDisposed) pop.Close(); pop = null; }

    public void ShowOsd(string kind, int value)
    {
        if (osd == null || osd.IsDisposed) osd = new OsdPopup(this, S);
        osd.Set(kind, value);
    }

    protected override void OnFormClosed(FormClosedEventArgs e)
    {
        ClosePop();
        if (osd != null && !osd.IsDisposed) osd.Close();
        base.OnFormClosed(e);
    }
}

// ---------------- Açılır kutu tabanı: yarı saydam köşeli pencere (UpdateLayeredWindow, piksel başına alfa) ----------------
abstract class Popup : Form
{
    public abstract string Kind { get; }
    protected readonly BarForm Bar;
    protected readonly float S;
    protected SizeF Size0;                 // CSS pikseli
    Rectangle anchor;                     // üzerinde durulduğu sürece açık kalan alan (bar'daki öğe); boşsa tıklayınca kapanır
    readonly System.Windows.Forms.Timer watch = new System.Windows.Forms.Timer { Interval = 60 };
    int outside;
    DateTime opened;

    protected class Btn { public RectangleF R; public Action Click; public string Id; }
    protected readonly List<Btn> Buttons = new List<Btn>();
    protected string HoverId;

    [StructLayout(LayoutKind.Sequential)] struct BLEND { public byte op, flags, alpha, format; }
    [StructLayout(LayoutKind.Sequential)] struct PT { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct SZ { public int cx, cy; }
    [DllImport("user32.dll")] static extern bool UpdateLayeredWindow(IntPtr h, IntPtr dst, ref PT pos, ref SZ size, IntPtr src, ref PT srcPos, uint key, ref BLEND b, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr o);

    protected Popup(BarForm bar, float s)
    {
        Bar = bar; S = s;
        FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; StartPosition = FormStartPosition.Manual;
        AutoScaleMode = AutoScaleMode.None; Text = "ll-bar-popup";
        watch.Tick += (o, e) => Watch();
    }
    protected override CreateParams CreateParams
    {
        get { var cp = base.CreateParams; cp.ExStyle |= 0x80000 /*LAYERED*/ | 0x80 | 0x08000000 | 0x8 /*TOPMOST*/; return cp; }
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override void WndProc(ref Message m)
    {
        if (m.Msg == 0x0021) { m.Result = (IntPtr)3; return; } // MA_NOACTIVATE
        base.WndProc(ref m);
    }

    // Bar'daki öğenin altında, ekranın içinde kalacak şekilde
    public void OpenBelow(Rectangle item, Rectangle keepOpenOver)
    {
        anchor = keepOpenOver;
        var mon = Bar.Screen.Bounds;
        int w = (int)Math.Ceiling(Size0.Width * S), h = (int)Math.Ceiling(Size0.Height * S);
        int x = item.X + item.Width / 2 - w / 2;
        x = Math.Max(mon.X + (int)(8 * S), Math.Min(mon.Right - w - (int)(8 * S), x));
        Bounds = new Rectangle(x, Bar.Bottom + (int)(4 * S), w, h);
        opened = DateTime.UtcNow;
        Show();
        Render();
        watch.Start();
    }

    void Watch()
    {
        var p = Cursor.Position;
        bool inside = Bounds.Contains(p) || (!anchor.IsEmpty && anchor.Contains(p));
        if (anchor.IsEmpty)
        {
            // Tıklamayla açılan kutu: dışarıda bir yere tıklanınca kapanır
            if (!inside && (Control.MouseButtons != MouseButtons.None) && (DateTime.UtcNow - opened).TotalMilliseconds > 200) Close();
            return;
        }
        outside = inside ? 0 : outside + 1;
        if (outside * watch.Interval >= 200) Close(); // ii: 200 ms sonra kapanır
        else if (inside) Tick();
    }
    protected virtual void Tick() { }

    public void Render()
    {
        if (IsDisposed || Width <= 0 || Height <= 0) return;
        using (var bmp = new Bitmap(Width, Height, System.Drawing.Imaging.PixelFormat.Format32bppPArgb))
        {
            using (var g = Graphics.FromImage(bmp))
            {
                g.Clear(Color.Transparent);
                g.SmoothingMode = SmoothingMode.AntiAlias;
                g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
                g.PixelOffsetMode = PixelOffsetMode.HighQuality;
                g.InterpolationMode = InterpolationMode.HighQualityBicubic;
                g.ScaleTransform(S, S);
                Buttons.Clear();
                Draw(g);
            }
            IntPtr screen = GetDC(IntPtr.Zero), mem = CreateCompatibleDC(screen), hb = bmp.GetHbitmap(Color.FromArgb(0)), old = SelectObject(mem, hb);
            try
            {
                var pos = new PT { x = Left, y = Top }; var size = new SZ { cx = Width, cy = Height }; var src = new PT();
                var b = new BLEND { op = 0, flags = 0, alpha = 255, format = 1 /*AC_SRC_ALPHA*/ };
                UpdateLayeredWindow(Handle, screen, ref pos, ref size, mem, ref src, 0, ref b, 2 /*ULW_ALPHA*/);
            }
            finally { SelectObject(mem, old); Native.DeleteObject(hb); DeleteDC(mem); ReleaseDC(IntPtr.Zero, screen); }
        }
    }
    protected abstract void Draw(Graphics g);

    // Kutu çerçevesi (ii StyledPopup)
    protected void Frame(Graphics g, float rad)
    {
        var r = new RectangleF(0.5f, 0.5f, Size0.Width - 1, Size0.Height - 1);
        Look.Fill(g, r, rad, Look.Layer0);
        using (var p = Look.Round(r, rad)) using (var pen = new Pen(Look.PopBorder, 1)) g.DrawPath(pen, p);
    }

    Btn At(Point client)
    {
        var p = new PointF(client.X / S, client.Y / S);
        for (int i = Buttons.Count - 1; i >= 0; i--) if (Buttons[i].R.Contains(p)) return Buttons[i];
        return null;
    }
    protected override void OnMouseMove(MouseEventArgs e)
    {
        var b = At(e.Location);
        string id = b == null ? null : b.Id;
        if (id != HoverId) { HoverId = id; Render(); }
    }
    protected override void OnMouseUp(MouseEventArgs e)
    {
        var b = At(e.Location);
        if (b != null && b.Click != null) { b.Click(); Render(); }
        OnClick(e);
    }
    protected virtual void OnClick(MouseEventArgs e) { }
    protected override void OnFormClosed(FormClosedEventArgs e) { watch.Stop(); watch.Dispose(); base.OnFormClosed(e); }
}

// ii ResourcesPopup: RAM | Takas | CPU | Sıcaklık sütunları
class ResourcesPopup : Popup
{
    public override string Kind { get { return "res"; } }
    int ticks;
    public ResourcesPopup(BarForm bar, float s) : base(bar, s) { Size0 = new SizeF(Stats.Swap >= 0 ? 560 : 430, 94); Stats.ReadTemps(); }
    protected override void Tick() { if (++ticks % 33 == 0) Stats.ReadTemps(); if (ticks % 8 == 0) Render(); } // sıcaklık ~2 sn'de bir

    static string GB(ulong b) { return (b / 1073741824.0).ToString("0.0") + " GB"; }
    static string Temp(string k)
    {
        var t = Stats.Temps; object v;
        return t != null && t.TryGetValue(k, out v) && v != null ? Convert.ToInt32(v) + "°C" : "—";
    }

    protected override void Draw(Graphics g)
    {
        Frame(g, 12);
        float x = 14;
        x = Column(g, x, "memory", "RAM", new[] { new[] { "clock_loader_60", "Kullanılan", GB(Stats.MemUsed) }, new[] { "check_circle", "Boş", GB(Stats.MemTotal - Stats.MemUsed) }, new[] { "empty_dashboard", "Toplam", GB(Stats.MemTotal) } });
        if (Stats.Swap >= 0)
            x = Column(g, x, "swap_horiz", "Swap", new[] { new[] { "clock_loader_60", "Kullanılan", GB(Stats.SwapUsed) }, new[] { "check_circle", "Boş", GB(Stats.SwapTotal - Stats.SwapUsed) }, new[] { "empty_dashboard", "Toplam", GB(Stats.SwapTotal) } });
        x = Column(g, x, "planner_review", "CPU", new[] { new[] { "bolt", "Yük", Math.Round(Stats.Cpu) + "%" } });
        Column(g, x, "device_thermostat", "Sıcaklık", new[] { new[] { "planner_review", "CPU", Temp("cpu") }, new[] { "developer_board", "GPU", Temp("gpu") } });
    }

    float Column(Graphics g, float x, string icon, string head, string[][] rows)
    {
        Look.Icon(g, icon, x + 7, 18, 15, Look.Subtext);
        Look.Str(g, Tr.T(head), Look.Text(13), Look.OnLayer, new RectangleF(x + 18, 8, 120, 20), Look.Left);
        float w = 0;
        for (int i = 0; i < rows.Length; i++)
        {
            float y = 32 + i * 19;
            Look.Icon(g, rows[i][0], x + 6, y + 8, 13, Look.Subtext);
            string lbl = Tr.T(rows[i][1]) + ":";
            float lw = Look.Width(g, lbl, Look.Text(12));
            Look.Str(g, lbl, Look.Text(12), Look.OnLayer, new RectangleF(x + 16, y, lw + 4, 17), Look.Left);
            float vw = Look.Width(g, rows[i][2], Look.Text(12));
            Look.Str(g, rows[i][2], Look.Text(12), Look.OnLayer, new RectangleF(x + 20 + lw, y, vw + 4, 17), Look.Left);
            w = Math.Max(w, 20 + lw + vw);
        }
        return x + Math.Max(w, 70) + 18;
    }
}

// ii medya kutusu: kapak, başlık, sanatçı, süre, önceki / ilerleme (tıklayınca sar) / sonraki, oynat-duraklat
class MediaPopup : Popup
{
    public override string Kind { get { return "media"; } }
    int ticks;
    public MediaPopup(BarForm bar, float s) : base(bar, s) { Size0 = new SizeF(360, 110); }
    protected override void Tick() { if (++ticks % 4 == 0) Render(); }

    static string Fmt(double s) { if (double.IsNaN(s) || s < 0 || s > 1e7) s = 0; return (int)(s / 60) + ":" + ((int)(s % 60)).ToString("00"); }

    protected override void Draw(Graphics g)
    {
        Frame(g, 12);
        if (MediaWatch.Title.Length == 0) { Look.Str(g, Tr.T("Medya yok"), Look.Text(15), Look.OnLayer, new RectangleF(14, 0, 300, 110), Look.Left); return; }
        var art = new RectangleF(10, 10, 90, 90);
        if (MediaWatch.Art != null)
        {
            var st = g.Save();
            using (var clip = Look.Round(art, 12)) g.SetClip(clip);
            g.DrawImage(MediaWatch.Art, art);
            g.Restore(st);
        }
        else { Look.Fill(g, art, 12, Look.SecCont); Look.Icon(g, "music_note", art.X + 45, art.Y + 45, 36, Look.OnSecCont); }
        float x = 112, w = 360 - x - 10 - 36 - 12;
        Look.Str(g, MediaWatch.Title, Look.Text(15), Look.OnLayer, new RectangleF(x, 12, w, 20), Look.Left);
        Look.Str(g, MediaWatch.Artist, Look.Text(11), Look.Subtext, new RectangleF(x, 32, w, 16), Look.Left);
        double pos = MediaWatch.Position, end = MediaWatch.End;
        Look.Str(g, Fmt(pos) + " / " + Fmt(end), Look.Text(12), Look.Subtext, new RectangleF(x, 56, w, 16), Look.Left);
        // Kontroller
        float cy = 89;
        Ctl(g, "prev", "skip_previous", x + 11, cy, () => MediaWatch.Send("prev"));
        var bar = new RectangleF(x + 28, cy - 1.5f, w - 56, 3);
        Look.Fill(g, bar, 1.5f, Color.FromArgb(51, 255, 255, 255));
        float pct = end > 0 ? (float)Math.Min(1, pos / end) : 0;
        Look.Fill(g, new RectangleF(bar.X, bar.Y, bar.Width * pct, 3), 1.5f, Look.Primary);
        Look.Fill(g, new RectangleF(bar.X + bar.Width * pct - 1.5f, cy - 7, 3, 14), 1.5f, Look.Primary);
        Buttons.Add(new Btn { Id = "seek", R = RectangleF.Inflate(bar, 0, 7), Click = () => Seek(bar, end) });
        Ctl(g, "next", "skip_next", x + w - 11, cy, () => MediaWatch.Send("next"));
        // Büyük oynat / duraklat
        var pr = new RectangleF(360 - 10 - 36, 55 - 18, 36, 36);
        Look.Fill(g, pr, 18, Color.FromArgb(HoverId == "play" ? 51 : 31, 255, 255, 255));
        Look.Icon(g, MediaWatch.Playing ? "pause" : "play_arrow", pr.X + 18, pr.Y + 18, 22, Look.OnLayer);
        Buttons.Add(new Btn { Id = "play", R = pr, Click = () => { MediaWatch.Send("toggle"); } });
    }

    void Ctl(Graphics g, string id, string icon, float cx, float cy, Action a)
    {
        var r = new RectangleF(cx - 11, cy - 11, 22, 22);
        if (HoverId == id) Look.Fill(g, r, 11, Color.FromArgb(26, 255, 255, 255));
        Look.Icon(g, icon, cx, cy, 18, Look.OnLayer);
        Buttons.Add(new Btn { Id = id, R = r, Click = a });
    }

    void Seek(RectangleF bar, double end)
    {
        if (end <= 0) return;
        var p = PointToClient(Cursor.Position);
        double f = Math.Max(0, Math.Min(1, (p.X / S - bar.X) / bar.Width));
        MediaWatch.Send("seek " + (f * end).ToString("0.0", System.Globalization.CultureInfo.InvariantCulture));
    }
}

// Tray taşması: bar'a sığmayan ikonlar (ii SysTray, 6 sütun)
class TrayPopup : Popup
{
    public override string Kind { get { return "tray"; } }
    const int COLS = 6;
    readonly List<TrayHost.Icon> icons;
    public TrayPopup(BarForm bar, float s, List<TrayHost.Icon> icons) : base(bar, s)
    {
        this.icons = icons;
        int rows = Math.Max(1, (icons.Count + COLS - 1) / COLS);
        Size0 = new SizeF(COLS * 30 + (COLS - 1) * 2 + 18, rows * 32 + 16);
    }
    protected override void Draw(Graphics g)
    {
        Frame(g, 17);
        for (int i = 0; i < icons.Count; i++)
        {
            var ic = icons[i];
            var r = new RectangleF(8 + (i % COLS) * 32, 8 + (i / COLS) * 32, 30, 30);
            string id = "t" + i;
            if (HoverId == id) Look.Fill(g, r, 15, Look.Layer1Hover);
            if (ic.Image != null) g.DrawImage(ic.Image, new RectangleF(r.X + 6, r.Y + 6, 18, 18));
            ic.Screen = new Rectangle(Left + (int)(r.X * S), Top + (int)(r.Y * S), (int)(r.Width * S), (int)(r.Height * S));
            Buttons.Add(new Btn { Id = id, R = r });
        }
    }
    protected override void OnClick(MouseEventArgs e)
    {
        var p = new PointF(e.X / S, e.Y / S);
        for (int i = 0; i < Buttons.Count && i < icons.Count; i++)
            if (Buttons[i].R.Contains(p))
            {
                TrayHost.Click(icons[i], e.Button == MouseButtons.Right ? 1 : e.Button == MouseButtons.Middle ? 2 : 0, false, Cursor.Position);
                Close();
                return;
            }
    }
}

// ii OsdValueIndicator: bar'ın altında ortada, 1 sn sonra kaybolan ses / parlaklık göstergesi
class OsdPopup : Popup
{
    public override string Kind { get { return "osd"; } }
    string kind = "volume"; int value;
    readonly System.Windows.Forms.Timer hide = new System.Windows.Forms.Timer { Interval = 1000 };
    public OsdPopup(BarForm bar, float s) : base(bar, s)
    {
        Size0 = new SizeF(200, 48);
        hide.Tick += (o, e) => { hide.Stop(); Hide(); };
    }
    public void Set(string k, int v)
    {
        kind = k; value = v;
        if (!Visible)
        {
            var mon = Bar.Screen.Bounds;
            int w = (int)(Size0.Width * S), h = (int)(Size0.Height * S);
            Bounds = new Rectangle(mon.X + mon.Width / 2 - w / 2, Bar.Bottom + (int)(10 * S), w, h);
            Show();
        }
        Render();
        hide.Stop(); hide.Start();
    }
    protected override void Draw(Graphics g)
    {
        Look.Fill(g, new RectangleF(0, 0, Size0.Width, Size0.Height), 24, Look.Layer0);
        bool vol = kind == "volume", mic = kind == "mic";
        string icon = vol ? (value == 0 ? "volume_off" : "volume_up") : mic ? (value == 0 ? "mic_off" : "mic") : kind == "gamma" ? "brightness_4" : "light_mode";
        float isz = kind == "brightness" ? 20 + 10 * (value / 100f) : 30;
        Look.Icon(g, icon, 10 + 15, 24, isz, Look.OnLayer);
        string lbl = Tr.T(vol ? "Ses" : mic ? "Mikrofon" : kind == "gamma" ? "Gama" : "Parlaklık");
        Look.Str(g, lbl, Look.Text(15), Look.OnLayer, new RectangleF(52, 8, 100, 18), Look.Left);
        Look.Str(g, value.ToString(), Look.Text(15), Look.OnLayer, new RectangleF(140, 8, 38, 18), new StringFormat(Look.Left) { Alignment = StringAlignment.Far });
        var bar = new RectangleF(52, 31, 128, 4);
        Look.Fill(g, bar, 2, Look.SecCont);
        Look.Fill(g, new RectangleF(bar.X, bar.Y, bar.Width * value / 100f, 4), 2, Look.Primary);
        using (var b = new SolidBrush(Look.Primary)) g.FillEllipse(b, bar.Right - 4, bar.Y, 4, 4);
    }
    protected override void OnFormClosed(FormClosedEventArgs e) { hide.Dispose(); base.OnFormClosed(e); }
}
