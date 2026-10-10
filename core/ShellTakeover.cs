// Logical Lunge çekirdeği: Windows kabuğunun devri (tek yer).
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using Microsoft.Win32;

// ---------------- Windows kabuğunun devri ----------------
// Logical Lunge çalışırken onun yerini aldığı Windows parçaları kaynağında kapatılır; masaüstü durunca, bar çöküp
// geri gelmeyince (ShellState), çekirdek vazgeçince ya da kaldırılınca aynen geri gelir:
//  - görev çubukları ve Başlat düğmesi: Windows'ta onları kaldıran bir ayar yok. Otomatik gizlemeye alınır (yer
//    ayırmaz; ABM_SETSTATE, belgelenmiş) ve görünür olduğu anda gizlenir (EVENT_OBJECT_SHOW). Explorer yeniden
//    başlayınca (TaskbarCreated), ekranlar değişince ve uykudan dönünce yeniden taranır; yoklama yok.
//  - ses / parlaklık / medya göstergesi (OSD): kapatan bir ayar yok; host'u küçültülür (HideVolumeOSD'nin yöntemi).
//  - kullanıcı ayarları (HKCU; canlı uygulanır, Explorer yeniden başlatılmaz): Aero Snap ve yerleşim önerileri,
//    pencere sallama, diğer ekranlardaki görev çubuğu, Pencere öğeleri / Görev Görünümü / Copilot düğmeleri,
//    dokunmatik klavyenin kendiliğinden açılması.
//  - Windows bildirim balonları: ToastBanners (WinNotifications); bar yokken ve masaüstü dururken geri açılır.
// Win tuşu kısayolları çekirdeğin klavye kancasında (Keys2); Explorer'ın DisabledHotkeys / NoWinKeys ayarları ancak
// Explorer yeniden başlayınca okunur, bar çöktüğünde geri açılamazdı: kullanılmaz.
// Eski değerler değiştirilmeden ÖNCE state\shell-takeover.json'a yazılır. Dosya duruyorsa (önceki çalışma çöktü) asıl
// değerler korunur; geri yükleme dosyadan yapılır ve dosyayı siler (lunge.exe --takeover-restore: kaldırma, kurulum).
static class ShellTakeover
{
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr w, string l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", SetLastError = true)] static extern bool SystemParametersInfo(uint action, uint uiParam, IntPtr pvParam, uint winIni);
    [DllImport("shell32.dll")] static extern IntPtr SHAppBarMessage(uint msg, ref APPBARDATA data);

    [StructLayout(LayoutKind.Sequential)]
    struct APPBARDATA
    {
        public int cbSize;
        public IntPtr hWnd;
        public uint uCallbackMessage, uEdge;
        public int left, top, right, bottom;
        public IntPtr lParam;
    }

    const uint ABM_GETSTATE = 4, ABM_SETSTATE = 10, ABS_AUTOHIDE = 1;
    const uint SPI_GETWINARRANGING = 0x0082, SPI_SETWINARRANGING = 0x0083, SPIF_UPDATEINIFILE = 1, SPIF_SENDCHANGE = 2;

    const string Adv = @"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
    internal const string ArrangeKey = @"Control Panel\Desktop", ArrangeName = "WindowArrangementActive";

    // Değiştirilen kullanıcı ayarları: anahtar, değer, Logical Lunge çalışırken ne olduğu (DWORD)
    internal static readonly object[][] Owned =
    {
        new object[] { Adv, "SnapAssist", 0 },              // yerleştirince yanına ne konacağını önermek
        new object[] { Adv, "EnableSnapAssistFlyout", 0 },  // büyütme düğmesinin yerleşim menüsü (Windows 11)
        new object[] { Adv, "EnableSnapBar", 0 },           // üste sürüklerken yerleşim çubuğu (Windows 11)
        new object[] { Adv, "JointResize", 0 },             // yan yana pencereleri birlikte boyutlandırma
        new object[] { Adv, "SnapFill", 0 },                // kalan boşluğu doldurma
        new object[] { Adv, "DisallowShaking", 1 },         // başlık çubuğunu sallayınca diğerlerini küçültme
        new object[] { Adv, "MMTaskbarEnabled", 0 },        // diğer ekranlarda görev çubuğu
        new object[] { Adv, "TaskbarDa", 0 },               // Pencere öğeleri (Widgets) düğmesi
        new object[] { Adv, "ShowTaskViewButton", 0 },      // Görev Görünümü düğmesi
        new object[] { Adv, "ShowCopilotButton", 0 },       // Copilot düğmesi
        new object[] { @"Software\Microsoft\TabletTip\1.7", "EnableDesktopModeAutoInvoke", 0 }, // dokunmatik klavye kendiliğinden
    };

    static readonly object gate = new object();
    static string StatePath { get { return Paths.State("shell-takeover.json"); } }

    // Devir şu an geçerli mi: tercih açık, bar ayakta, masaüstü kapanmıyor
    static volatile bool active;
    public static bool Active { get { return active; } }

    // ---- saf kısım (testler bunu kullanır) ----

    // Şimdiki değerler: her kayıt {k, n, had, old}. read: (anahtar, ad) -> değer ya da null (yok)
    internal static List<Dictionary<string, object>> Capture(Func<string, string, object> read)
    {
        var list = new List<Dictionary<string, object>>();
        foreach (var o in Owned) list.Add(Entry((string)o[0], (string)o[1], read((string)o[0], (string)o[1])));
        list.Add(Entry(ArrangeKey, ArrangeName, read(ArrangeKey, ArrangeName)));
        return list;
    }

    static Dictionary<string, object> Entry(string k, string n, object v)
    {
        return new Dictionary<string, object> { { "k", k }, { "n", n }, { "had", v != null }, { "old", v is int || v is string ? v : null } };
    }

    internal static string Serialize(List<Dictionary<string, object>> reg, int autoHide)
    {
        return new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "v", 1 }, { "reg", reg }, { "autoHide", autoHide } });
    }

    // Kayıt dosyasını okur; bozuksa false (asıl değerler bilinmiyor: üstüne yazılmaz)
    internal static bool TryParse(string json, out List<Dictionary<string, object>> reg, out int autoHide)
    {
        reg = null; autoHide = -1;
        try
        {
            var d = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(json);
            object v;
            if (d == null || !d.TryGetValue("reg", out v) || !(v is System.Collections.ArrayList)) return false;
            reg = new List<Dictionary<string, object>>();
            foreach (var o in (System.Collections.ArrayList)v)
            {
                var e = o as Dictionary<string, object>;
                if (e == null || !(e.ContainsKey("k") && e["k"] is string) || !(e.ContainsKey("n") && e["n"] is string) || !(e.ContainsKey("had") && e["had"] is bool)) return false;
                reg.Add(e);
            }
            if (d.TryGetValue("autoHide", out v) && v is int) autoHide = (int)v;
            return true;
        }
        catch { reg = null; return false; }
    }

    // Asıl değerler: önceki bir çalışmanın kaydı okunabiliyorsa o (çöktüyse onun değiştirdikleri asıl değer değil),
    // yoksa şimdiki değerler
    internal static string Originals(string existing, Func<string, string, object> read, out List<Dictionary<string, object>> reg, out int autoHide)
    {
        if (existing != null && TryParse(existing, out reg, out autoHide))
        {
            // sonradan eklenen bir ayar da asıl değeriyle kayda girer
            var have = new HashSet<string>();
            foreach (var e in reg) have.Add((string)e["k"] + "\\" + (string)e["n"]);
            bool added = false;
            foreach (var e in Capture(read))
                if (have.Add((string)e["k"] + "\\" + (string)e["n"])) { reg.Add(e); added = true; }
            return added ? Serialize(reg, autoHide) : existing;
        }
        reg = Capture(read);
        autoHide = -1;
        return Serialize(reg, autoHide);
    }

    // Geri yüklemede her kayda ne yazılır: değer ya da null (yoktu: silinir)
    internal static object RestoreValue(Dictionary<string, object> e)
    {
        object old;
        return (bool)e["had"] && e.TryGetValue("old", out old) ? old : null;
    }

    // ---- uygulama ----

    static object ReadValue(string key, string name)
    {
        try { using (var k = Registry.CurrentUser.OpenSubKey(key)) return k == null ? null : k.GetValue(name, null, RegistryValueOptions.DoNotExpandEnvironmentNames); }
        catch { return null; }
    }

    static void WriteState(string json)
    {
        string tmp = StatePath + ".tmp";
        System.IO.File.WriteAllText(tmp, json, new UTF8Encoding(false));
        if (System.IO.File.Exists(StatePath)) System.IO.File.Replace(tmp, StatePath, null);
        else System.IO.File.Move(tmp, StatePath);
    }

    static string ReadState()
    {
        try { return System.IO.File.Exists(StatePath) ? System.IO.File.ReadAllText(StatePath) : null; }
        catch { return null; }
    }

    static uint AutoHideState(IntPtr tray)
    {
        var d = new APPBARDATA { cbSize = Marshal.SizeOf(typeof(APPBARDATA)), hWnd = tray };
        return (uint)SHAppBarMessage(ABM_GETSTATE, ref d).ToInt64();
    }

    static void SetAutoHideState(IntPtr tray, uint state)
    {
        var d = new APPBARDATA { cbSize = Marshal.SizeOf(typeof(APPBARDATA)), hWnd = tray, lParam = (IntPtr)state };
        SHAppBarMessage(ABM_SETSTATE, ref d);
    }

    static void SetArranging(bool on)
    {
        if (!SystemParametersInfo(SPI_SETWINARRANGING, on ? 1u : 0u, on ? (IntPtr)1 : IntPtr.Zero, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE))
            Slider.Log("devir: Aero Snap ayarı yazılamadı (" + Marshal.GetLastWin32Error() + ")");
    }

    // Logical Lunge'ın değerlerini uygular; asıl değerler önce kaydedilir. Tercih kapalıysa geri yükler.
    public static void Apply()
    {
        if (!Prefs.Takeover) { Restore(); return; }
        lock (gate)
        {
            string existing = ReadState();
            List<Dictionary<string, object>> reg;
            int autoHide;
            string json = Originals(existing, ReadValue, out reg, out autoHide);
            IntPtr tray = FindWindow("Shell_TrayWnd", null);
            if (autoHide < 0 && tray != IntPtr.Zero) { autoHide = (int)AutoHideState(tray); json = Serialize(reg, autoHide); }
            if (json != existing)
            {
                try { WriteState(json); }
                catch (Exception ex) { Slider.Log("devir: asıl değerler kaydedilemedi, hiçbir şey değiştirilmedi: " + ex.Message); return; }
            }
            active = true;
            foreach (var o in Owned)
            {
                try { using (var k = Registry.CurrentUser.CreateSubKey((string)o[0])) k.SetValue((string)o[1], (int)o[2], RegistryValueKind.DWord); }
                catch (Exception ex) { Slider.Log("devir: " + o[1] + " yazılamadı: " + ex.Message); } // ör. Windows 11'in korumalı TaskbarDa'sı
            }
            SetArranging(false);
            if (tray != IntPtr.Zero) SetAutoHideState(tray, AutoHideState(tray) | ABS_AUTOHIDE);
        }
        Broadcast();
        SyncTheme();
    }

    // Kaydedilmiş asıl değerleri geri yazar ve kaydı siler (kayıt yoksa bir şey yapmaz)
    public static void Restore()
    {
        bool themeBack = false;
        lock (gate)
        {
            active = false;
            string existing = ReadState();
            if (existing == null) return;
            List<Dictionary<string, object>> reg;
            int autoHide;
            if (!TryParse(existing, out reg, out autoHide)) { Slider.Log("devir: kayıt okunamadı, geri yüklenmedi: " + StatePath); return; }
            foreach (var e in reg)
            {
                string key = (string)e["k"], name = (string)e["n"];
                object v = RestoreValue(e);
                if (IsThemeEntry(key, name)) themeBack = true;
                try
                {
                    if (key == ArrangeKey && name == ArrangeName)
                    {
                        // SPI hem canlı ayarı hem değeri yazar; değer hiç yoktuysa sonra silinir (Windows varsayılanı: açık)
                        SetArranging(v == null || !"0".Equals(v as string));
                        if (v == null) using (var k = Registry.CurrentUser.OpenSubKey(key, true)) if (k != null) k.DeleteValue(name, false);
                        continue;
                    }
                    using (var k = Registry.CurrentUser.CreateSubKey(key))
                    {
                        if (v == null) k.DeleteValue(name, false);
                        else if (v is int) k.SetValue(name, (int)v, RegistryValueKind.DWord);
                        else k.SetValue(name, v);
                    }
                }
                catch (Exception ex) { Slider.Log("devir: " + name + " geri yazılamadı: " + ex.Message); }
            }
            IntPtr tray = FindWindow("Shell_TrayWnd", null);
            if (autoHide >= 0 && tray != IntPtr.Zero) SetAutoHideState(tray, (uint)autoHide);
            try { System.IO.File.Delete(StatePath); } catch { }
        }
        Broadcast();
        if (themeBack) BroadcastTheme();
    }

    // Explorer görev çubuğu ayarlarını WM_SETTINGCHANGE "TraySettings" ile yeniden okur; askıdaki bir pencere
    // çağıranı bekletmesin diye arkada
    static void Broadcast()
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            IntPtr r;
            SendMessageTimeout((IntPtr)0xFFFF, 0x001A, IntPtr.Zero, "TraySettings", 0x0002, 1000, out r);
        });
    }

    // ---- Windows teması (Prefs: theme, focusColor, themeSync) ----
    // Windows'un kendi pencereleri ve temaya uyan uygulamalar bizimle aynı görünsün: koyu/açık tercih Personalize'a, vurgu
    // rengi belgelenmiş kullanıcı değerlerine yazılır. DWM'in belgelenmemiş renklendirme işlevleri çağrılmaz.
    // Asıl değerler ilk yazımdan ÖNCE aynı kayıt dosyasına (shell-takeover.json) girer; devir bırakılınca ya da tema
    // eşitleme kapanınca aynen geri yazılır. Değişmeyen değer yazılmaz; ardışık değişiklikler tek yazıma birleşir
    // (son değer kazanır); WM_SETTINGCHANGE "ImmersiveColorSet" arka planda, zaman aşımlı yayınlanır.
    internal const string PersonalizeKey = @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    internal const string DwmKey = @"Software\Microsoft\Windows\DWM";
    internal const string AccentKey = @"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
    internal const int ThemeDebounceMs = 300, ThemeBroadcastTimeoutMs = 200;

    // Tema eşitlemenin dokunabileceği tüm değerler (anahtar, ad)
    internal static readonly string[][] ThemeNames =
    {
        new[] { PersonalizeKey, "AppsUseLightTheme" },
        new[] { PersonalizeKey, "SystemUsesLightTheme" },
        new[] { DwmKey, "AccentColor" },               // 0xAABBGGRR
        new[] { DwmKey, "ColorizationColor" },         // 0xAARRGGBB
        new[] { DwmKey, "ColorizationAfterglow" },     // 0xAARRGGBB
        new[] { AccentKey, "AccentColorMenu" },        // 0xAABBGGRR
    };

    internal static bool IsThemeEntry(string key, string name)
    {
        foreach (var t in ThemeNames) if (t[0] == key && t[1] == name) return true;
        return false;
    }

    // Tercihlerden Windows değerleri: her biri {anahtar, ad, int}. Geçersiz renkte yalnızca koyu/açık yazılır.
    internal static List<object[]> ThemeValues(bool light, string hex)
    {
        var list = new List<object[]>();
        int l = light ? 1 : 0;
        list.Add(new object[] { PersonalizeKey, "AppsUseLightTheme", l });
        list.Add(new object[] { PersonalizeKey, "SystemUsesLightTheme", l });
        if (hex == null || !System.Text.RegularExpressions.Regex.IsMatch(hex, "^#[0-9a-fA-F]{6}$")) return list;
        uint r = Convert.ToUInt32(hex.Substring(1, 2), 16), g = Convert.ToUInt32(hex.Substring(3, 2), 16), b = Convert.ToUInt32(hex.Substring(5, 2), 16);
        int abgr = unchecked((int)(0xFF000000u | (b << 16) | (g << 8) | r));
        int argb = unchecked((int)(0xC4000000u | (r << 16) | (g << 8) | b));
        list.Add(new object[] { DwmKey, "AccentColor", abgr });
        list.Add(new object[] { DwmKey, "ColorizationColor", argb });
        list.Add(new object[] { DwmKey, "ColorizationAfterglow", argb });
        list.Add(new object[] { AccentKey, "AccentColorMenu", abgr });
        return list;
    }

    // Gerçekten değişecek olanlar (şimdiki değer yok ya da farklı); hiçbiri değişmiyorsa boş: hiçbir şey yazılmaz
    internal static List<object[]> ThemeChanges(List<object[]> desired, Func<string, string, object> read)
    {
        var list = new List<object[]>();
        foreach (var d in desired)
        {
            object cur = read((string)d[0], (string)d[1]);
            if (!(cur is int) || (int)cur != (int)d[2]) list.Add(d);
        }
        return list;
    }

    // Tema değerlerinin şimdiki hâli (asıl değerler olarak kaydedilir)
    internal static List<Dictionary<string, object>> CaptureTheme(Func<string, string, object> read)
    {
        var list = new List<Dictionary<string, object>>();
        foreach (var t in ThemeNames) list.Add(Entry(t[0], t[1], read(t[0], t[1])));
        return list;
    }

    // Kayıtta olmayanları ekler (mevcut asıl değerler korunur); eklendiyse true
    internal static bool AddMissing(List<Dictionary<string, object>> reg, List<Dictionary<string, object>> entries)
    {
        var have = new HashSet<string>();
        foreach (var e in reg) have.Add((string)e["k"] + "\\" + (string)e["n"]);
        bool added = false;
        foreach (var e in entries)
            if (have.Add((string)e["k"] + "\\" + (string)e["n"])) { reg.Add(e); added = true; }
        return added;
    }

    internal static List<Dictionary<string, object>> ThemeEntries(List<Dictionary<string, object>> reg)
    {
        var list = new List<Dictionary<string, object>>();
        foreach (var e in reg) if (IsThemeEntry((string)e["k"], (string)e["n"])) list.Add(e);
        return list;
    }

    internal static List<Dictionary<string, object>> WithoutTheme(List<Dictionary<string, object>> reg)
    {
        var list = new List<Dictionary<string, object>>();
        foreach (var e in reg) if (!IsThemeEntry((string)e["k"], (string)e["n"])) list.Add(e);
        return list;
    }

    // Birleştirme: son istekten bu yana kaç ms daha beklenmeli (0: yazma zamanı). Tick sayacı taşsa da doğru.
    internal static int DebounceWait(int lastRequestTick, int nowTick, int ms)
    {
        int elapsed = unchecked(nowTick - lastRequestTick);
        if (elapsed < 0) elapsed = 0;
        return elapsed >= ms ? 0 : ms - elapsed;
    }

    static readonly object themeTimerGate = new object();
    static System.Threading.Timer themeTimer;
    static int themeLastRequest;

    // Tema ya da vurgu rengi ya da tercih değişti: engellemez; son istek 300 ms sakinleşince tek yazım yapılır
    public static void SyncTheme()
    {
        lock (themeTimerGate)
        {
            themeLastRequest = Environment.TickCount;
            if (themeTimer == null) themeTimer = new System.Threading.Timer(ThemeTick, null, ThemeDebounceMs, Timeout.Infinite);
            else themeTimer.Change(ThemeDebounceMs, Timeout.Infinite);
        }
    }

    static void ThemeTick(object state)
    {
        lock (themeTimerGate)
        {
            int wait = DebounceWait(themeLastRequest, Environment.TickCount, ThemeDebounceMs);
            if (wait > 0) { themeTimer.Change(wait, Timeout.Infinite); return; }
        }
        try { ApplyTheme(); }
        catch (Exception ex) { Slider.Log("devir: Windows teması eşitlenemedi: " + ex.Message); }
    }

    // Thread havuzunda çalışır; çekirdeğin arayüz / giriş thread'lerinde değil
    static void ApplyTheme()
    {
        bool changed = false, restored = false;
        lock (gate)
        {
            if (!active) return; // devir yok: Restore her şeyi zaten geri yazdı
            string existing = ReadState();
            List<Dictionary<string, object>> reg;
            int autoHide;
            if (existing == null || !TryParse(existing, out reg, out autoHide)) return; // asıl değerler bilinmiyor: dokunma
            if (!Prefs.ThemeSync)
            {
                restored = RestoreThemeEntries(reg, autoHide);
            }
            else
            {
                var todo = ThemeChanges(ThemeValues(Prefs.ThemeLight, Prefs.FocusColorHex), ReadValue);
                if (todo.Count == 0) return;
                if (AddMissing(reg, CaptureTheme(ReadValue)))
                {
                    try { WriteState(Serialize(reg, autoHide)); }
                    catch (Exception ex) { Slider.Log("devir: tema asıl değerleri kaydedilemedi, değiştirilmedi: " + ex.Message); return; }
                }
                foreach (var d in todo)
                {
                    try { using (var k = Registry.CurrentUser.CreateSubKey((string)d[0])) k.SetValue((string)d[1], (int)d[2], RegistryValueKind.DWord); changed = true; }
                    catch (Exception ex) { Slider.Log("devir: " + d[1] + " yazılamadı: " + ex.Message); }
                }
            }
        }
        if (changed || restored) BroadcastTheme();
    }

    // Tema değerlerini kayıttaki asıl hâline döndürür ve kayıttan çıkarır (gate tutulurken)
    static bool RestoreThemeEntries(List<Dictionary<string, object>> reg, int autoHide)
    {
        var mine = ThemeEntries(reg);
        if (mine.Count == 0) return false;
        foreach (var e in mine)
        {
            string key = (string)e["k"], name = (string)e["n"];
            object v = RestoreValue(e);
            try
            {
                using (var k = Registry.CurrentUser.CreateSubKey(key))
                {
                    if (v == null) k.DeleteValue(name, false);
                    else if (v is int) k.SetValue(name, (int)v, RegistryValueKind.DWord);
                    else k.SetValue(name, v);
                }
            }
            catch (Exception ex) { Slider.Log("devir: " + name + " geri yazılamadı: " + ex.Message); }
        }
        try { WriteState(Serialize(WithoutTheme(reg), autoHide)); }
        catch (Exception ex) { Slider.Log("devir: kayıt güncellenemedi: " + ex.Message); }
        return true;
    }

    // WM_SETTINGCHANGE "ImmersiveColorSet": yalnızca arka planda, zaman aşımlı (SMTO_ABORTIFHUNG); asla SendMessage değil
    static void BroadcastTheme()
    {
        ThreadPool.QueueUserWorkItem(_ =>
        {
            try
            {
                IntPtr r;
                SendMessageTimeout((IntPtr)0xFFFF, 0x001A, IntPtr.Zero, "ImmersiveColorSet", 0x0002 /*SMTO_ABORTIFHUNG*/ | 0x0000 /*SMTO_NORMAL*/, (uint)ThemeBroadcastTimeoutMs, out r);
            }
            catch (Exception ex) { Slider.Log("devir: tema bildirimi gönderilemedi: " + ex.Message); }
        });
    }

    // Asıl çekirdek açılırken (mesaj döngüsü olan bir thread'de): ayarlar + görev çubuğu
    public static void Start()
    {
        // Tercih kapandı: ayarlar ve görev çubuğu geri gelir (bildirim balonları kendi tercihine bağlı kalır)
        Prefs.TakeoverChanged += () => ThreadPool.QueueUserWorkItem(_ =>
        {
            if (!Prefs.Takeover) { Restore(); Taskbar.ShowAll(); }
            else if (ShellState.Up) Resume();
        });
        Prefs.ThemeInputsChanged += SyncTheme; // tema / vurgu rengi / eşitleme tercihi değişince (engellemez)
        Apply();
        Taskbar.Install(true);
    }

    // Bar 20 sn'dir yok ya da tercih kapandı: Windows'un parçaları geri gelir
    public static void Pause()
    {
        Restore();
        Taskbar.ShowAll();
        try { ToastBanners.Restore(); } catch (Exception ex) { Slider.Log("devir: balonlar geri açılamadı: " + ex.Message); }
    }

    // Bar geri geldi
    public static void Resume()
    {
        Apply();
        Taskbar.Sweep();
        WinNotifications.ReapplyBanners();
    }

    // Masaüstü kapanıyor / kaldırılıyor (lunge.exe --takeover-restore): her şey geri gelir, bundan sonra gizlenmez
    public static void ReleaseAll()
    {
        Taskbar.Release();
        Restore();
        try { ToastBanners.Restore(); } catch (Exception ex) { Slider.Log("devir: balonlar geri açılamadı: " + ex.Message); }
    }

    // Açılış perdesi (ayrı süreç): yalnızca görev çubuğu gizlenir, ayarlara dokunulmaz
    public static void HideTaskbarForSplash() { Taskbar.Install(false); }

    // Explorer yeniden başladı: yeni görev çubuğu yine otomatik gizlemede (asıl değeri ilk kez görülüyorsa kaydedilir)
    static void OnTaskbarCreated()
    {
        if (active) Apply();
    }

    // ---------------- görev çubuğu ----------------
    static class Taskbar
    {
        static Native.WinEventDelegate cb;
        static System.Windows.Forms.Timer timer;
        static Listener listener;
        static bool failOpen;
        static volatile bool released;

        // Mesaj döngüsü olan bir thread'den çağrılır: kanca, pencere ve zamanlayıcı o thread'de çalışır.
        // failOpen (asıl çekirdek): bizim bar'ımız yoksa Windows'un parçaları geri açılır (ShellState).
        public static void Install(bool failOpenMode)
        {
            if (cb != null) return;
            failOpen = failOpenMode;
            cb = Callback.Guard("görev çubuğu olayı", (hook, ev, h, idObject, idChild, thread, time) => { EventLag.Note("görev çubuğu", time); if (idObject == 0 && h != IntPtr.Zero) Hide(h); });
            Native.SetWinEventHook(Native.EVENT_OBJECT_SHOW, Native.EVENT_OBJECT_SHOW, IntPtr.Zero, cb, 0, 0, 0x0002);
            listener = new Listener();
            Sweep();
            if (!failOpen) return;
            // Barın ayakta olup olmadığı (görev çubuğu taranmaz: gösterilişi olayla gelir)
            timer = new System.Windows.Forms.Timer { Interval = 2000 };
            timer.Tick += (s, e) =>
            {
                if (!ShellState.Update()) return;
                if (ShellState.Up) { if (Prefs.Takeover) Resume(); }
                else Pause();
            };
            timer.Start();
        }

        public static void Release()
        {
            released = true;
            ShowAll();
            Native.EnumWindows(delegate (IntPtr h, IntPtr l)
            {
                if (Cls(h) == "NativeHWNDHost" && Native.FindWindowEx(h, IntPtr.Zero, "DirectUIHWND", null) != IntPtr.Zero)
                    Native.ShowWindowAsync(h, 9); // SW_RESTORE: gizlerken küçültülmüştü
                return true;
            }, IntPtr.Zero);
        }

        // Görev çubukları ve Başlat düğmesi yeniden görünür
        public static void ShowAll()
        {
            Native.EnumWindows(delegate (IntPtr h, IntPtr l)
            {
                string cs = Cls(h);
                if (cs == "Shell_TrayWnd" || cs == "Shell_SecondaryTrayWnd" || (cs == "Button" && Cls(Native.GetWindow(h, 4)).StartsWith("Shell_")))
                    Native.ShowWindowAsync(h, 8); // SW_SHOWNA
                return true;
            }, IntPtr.Zero);
        }

        // Yalnızca aranan sınıflar (görev çubukları, Başlat düğmesi, ses göstergesi)
        static readonly string[] sweepClasses = { "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Button", "NativeHWNDHost" };
        public static void Sweep()
        {
            foreach (var cls in sweepClasses)
            {
                IntPtr h = IntPtr.Zero;
                while ((h = Native.FindWindowEx(IntPtr.Zero, h, cls, null)) != IntPtr.Zero)
                    if (Native.IsWindowVisible(h)) Hide(h);
            }
        }

        static string Cls(IntPtr h)
        {
            var c = new StringBuilder(64);
            Native.GetClassName(h, c, 64);
            return c.ToString();
        }

        static void Hide(IntPtr h)
        {
            if (released || (failOpen && !ShellState.Up) || (failOpen && !Prefs.Takeover)) return;
            // Yalnızca üst düzey pencereler: kanca uygulamaların iç pencerelerinin gösterilişini de getirir; Görev Yöneticisi'nin
            // içerik paneli de bir NativeHWNDHost > DirectUIHWND ve küçültülünce pencere boş kalıyordu ("TaskManagerMain")
            if (Native.GetAncestor(h, 2) != h) return; // GA_ROOT
            string cs = Cls(h);
            // Başlat düğmesi: görev çubuğunun sahip olduğu ayrı bir üst pencere (Button)
            if (cs == "Shell_TrayWnd" || cs == "Shell_SecondaryTrayWnd" || (cs == "Button" && Cls(Native.GetWindow(h, 4)).StartsWith("Shell_")))
                Native.ShowWindowAsync(h, 0); // SW_HIDE; Explorer askıdaysa beklemez
            // Ses/parlaklık/medya OSD'si (NativeHWNDHost > DirectUIHWND): küçültülmüş host bir daha görünmez (HideVolumeOSD'nin yöntemi)
            else if (cs == "NativeHWNDHost" && !Native.IsIconic(h) && Native.FindWindowEx(h, IntPtr.Zero, "DirectUIHWND", null) != IntPtr.Zero)
                Native.ShowWindowAsync(h, 6); // SW_MINIMIZE
        }

        // Explorer'ın yayınları: yeniden başladı (TaskbarCreated), ekranlar değişti, uykudan dönüldü. Gizli üst düzey
        // pencere (yayınlar yalnızca mesaj penceresi olmayanlara gelir); çekirdek yönetici olarak çalışır, Explorer'ın
        // TaskbarCreated'ı ancak süzgeç izin verirse ulaşır.
        sealed class Listener : NativeWindow
        {
            [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern uint RegisterWindowMessage(string name);
            [DllImport("user32.dll")] static extern bool ChangeWindowMessageFilterEx(IntPtr hwnd, uint msg, uint action, IntPtr info);
            readonly uint taskbarCreated = RegisterWindowMessage("TaskbarCreated");

            public Listener()
            {
                CreateHandle(new CreateParams { Caption = "lunge-takeover", ExStyle = 0x80 }); // WS_EX_TOOLWINDOW, görünmez
                ChangeWindowMessageFilterEx(Handle, taskbarCreated, 1, IntPtr.Zero); // MSGFLT_ALLOW
            }

            protected override void WndProc(ref Message m)
            {
                if (taskbarCreated != 0 && (uint)m.Msg == taskbarCreated)
                {
                    Sweep();
                    if (failOpen) ThreadPool.QueueUserWorkItem(_ => OnTaskbarCreated());
                }
                else if (m.Msg == 0x007E) Sweep(); // WM_DISPLAYCHANGE: diğer ekranların görev çubukları
                else if (m.Msg == 0x0218 && (m.WParam.ToInt64() == 0x12 || m.WParam.ToInt64() == 0x7)) Sweep(); // WM_POWERBROADCAST: uykudan dönüş
                base.WndProc(ref m);
            }
        }
    }
}

// Bizim kabuk (shell'deki bar) ayakta mı. Bar 20 sn'den uzun yoksa (shell ya da tiling çöktü / açılamadı) çekirdek
// güvenli tarafa açılır: Windows görev çubuğu, Başlat menüsü ve devredilen ayarlar geri gelir (ShellTakeover.Pause),
// kullanıcı hiçbir zaman barsız, görev çubuğusuz ve Başlat'sız kalmaz. Bar dönünce hepsi yine bizim. (Kısa shell
// yeniden başlatmaları sayılmaz.)
static class ShellState
{
    static volatile bool up = true;
    // Environment.TickCount 24,8 günde eksiye geçer: "-1 = yok" bir işaret olamaz (her çağrıda baştan başlıyordu, bar
    // ölse de görev çubuğu hiç geri gelmiyordu)
    static bool missing;
    static int missingSince;
    public static bool Up { get { return up; } }

    // Durum değiştiyse true (ShellTakeover'ın 2 sn'lik zamanlayıcısından)
    public static bool Update()
    {
        bool bar = Native.FindWindowEx(IntPtr.Zero, IntPtr.Zero, null, Names.Bar) != IntPtr.Zero;
        if (bar)
        {
            missing = false;
            if (up) return false;
            up = true;
            NativeInput.SetFlag(NativeInput.SHELL_UP, true);
            Slider.Log("bar geri geldi: görev çubuğu ve Win tuşu yine kabuğun");
            return true;
        }
        if (!missing) { missing = true; missingSince = Environment.TickCount; return false; }
        if (!up || unchecked(Environment.TickCount - missingSince) < 20000) return false;
        up = false;
        NativeInput.SetFlag(NativeInput.SHELL_UP, false);
        Slider.Log("bar 20 sn'dir yok: Windows görev çubuğu, Başlat menüsü ve ayarları geri açıldı");
        return true;
    }
}
