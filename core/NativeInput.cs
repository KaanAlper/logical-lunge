// Girdi kancalarının yerel hâli (lunge_input.dll, shell/crates/lunge-input). Klavye ve fare kancaları .NET'te
// çalışırken her olay bellek ayırıyordu; çekirdeğin herhangi bir yerindeki çöp toplama ya da bellek sıkışıkken bir sayfa
// hatası kanca thread'ini durduruyor, Windows'un süre sınırı (LowLevelHooksTimeout) aşılınca tuş işlenmeden geçiyor,
// tekrarlarsa Windows kancayı sessizce söküyordu. Yerel kanca .NET'e hiç girmez (çöp toplayıcı onu durduramaz), bellek
// ayırmaz, sayfaları kilitlidir; yalnızca tuşu yutup yutmayacağına bu tablodan ve bayraklardan karar verir ve bir kayıt
// kuyruğa koyar. Bütün eylemler burada, kuyruğu boşaltan thread'de çalışır (Keys2, MouseFocus, Switcher).
// Kütüphane yüklenemezse .NET kancaları kullanılır (Keys2.Start, MouseFocus.InstallHook).
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

static class NativeInput
{
    [StructLayout(LayoutKind.Sequential)]
    public struct Ev { public ushort Kind, Vk, Mods, Flag; public int Id, X, Y; public uint Time; }

    const string Dll = "lunge_input.dll";
    [DllImport(Dll)] static extern uint li_version();
    [DllImport(Dll)] static extern int li_start();
    [DllImport(Dll)] static extern IntPtr li_event();
    [DllImport(Dll)] static extern int li_pop(out Ev ev);
    [DllImport(Dll)] static extern void li_set_flag(uint id, int value);
    [DllImport(Dll)] static extern void li_set_table(long[] entries, uint n);
    [DllImport(Dll)] static extern void li_set_capturable(uint[] bits);
    [DllImport(Dll)] static extern void li_forget();
    [DllImport(Dll)] static extern void li_reinstall(uint which);
    [DllImport(Dll)] static extern void li_ticks(out uint key, out uint mouse);
    [DllImport(Dll)] static extern void li_mouse_pos(out int x, out int y);
    [DllImport(Dll)] static extern void li_stats(uint which, out ulong calls, out ulong totalUs, out ulong maxUs);

    // Bayraklar (lib.rs > flag)
    public const uint SHELL_UP = 0, SWITCHER_ACTIVE = 1, CAPTURING = 2, SWITCHER_DEMO = 3, SINGLE_CLICK_OPEN = 5, SWITCHER_READY = 6;
    // Kayıt türleri (lib.rs > kind)
    public const ushort SW_OPEN = 1, SW_MOVE = 2, SW_COMMIT = 3, SW_CLOSE = 4, SW_ALT_UP = 5, SW_LOST_ALT = 6, SW_FULLSCREEN = 7,
        DESK_MENU_KEY = 10, DESK_OPEN_KEY = 11, WIN_UP_DOCK = 12, WIN_UP_OVERVIEW = 13, CAPTURE = 14, BIND = 15, RESERVED = 16, WM = 17, WIN_DOWN = 18, WIN_UP = 19,
        SLOW_KEY = 20, SLOW_MOUSE = 21, DESK_MENU = 30, CLICK = 31, MOVE = 32, DESK_CLICK = 33, DESK_OPEN = 34, DROPPED = 40;
    // Tablo türleri (table.rs)
    const int T_BIND = 1, T_RESERVED = 2, T_WM = 3;

    static readonly object gate = new object();
    static volatile int state;   // 0 denenmedi, 1 yerel kancalar kurulu, -1 .NET kancaları
    static volatile bool loaded; // kütüphane yüklendi (tablo ve bayraklar gönderilebilir)
    public static bool On { get { return state == 1; } }
    public static volatile Keys2 Keys;
    public static volatile MouseFocus Mouse;

    // Klavye ve fare thread'lerinin ikisi de çağırır; ilki kurar
    public static bool Start()
    {
        lock (gate)
        {
            if (state != 0) return state == 1;
            state = -1;
            try
            {
                uint v = li_version();
                if (v != 1) { Slider.Log("girdi kancaları: yerel kütüphanenin sürümü uymuyor (" + v + "), .NET kancaları kullanılıyor"); return false; }
                loaded = true;
                PushFlags();
                PushCapturable();
                PushTable();
                if (li_start() != 1) { loaded = false; Slider.Log("girdi kancaları: yerel kancalar kurulamadı, .NET kancaları kullanılıyor"); return false; }
                var wake = new AutoResetEvent(false);
                wake.SafeWaitHandle = new Microsoft.Win32.SafeHandles.SafeWaitHandle(li_event(), false);
                new Thread(() => Drain(wake)) { IsBackground = true, Name = "input-dispatch", Priority = ThreadPriority.AboveNormal }.Start();
                state = 1;
                Slider.Log("girdi kancaları yerel (lunge_input.dll, TIME_CRITICAL thread, kilitli bellek)");
                return true;
            }
            catch (Exception ex)
            {
                loaded = false;
                Slider.Log("girdi kancaları: yerel kütüphane yüklenemedi (" + ex.GetBaseException().Message + "), .NET kancaları kullanılıyor");
                return false;
            }
        }
    }

    // Kuyruk: kanca her kayıtta olayı işaretler, burada sırayla işlenir (bekleme olaya bağlı, yoklama yok)
    static void Drain(WaitHandle wake)
    {
        Ev e;
        while (true)
        {
            wake.WaitOne();
            while (li_pop(out e) == 1)
            {
                try { Dispatch(e); }
                catch (Exception ex) { Slider.Log("girdi olayı (" + e.Kind + "): " + ex.GetBaseException().Message); }
            }
        }
    }

    static void Dispatch(Ev e)
    {
        switch (e.Kind)
        {
            case SLOW_KEY: Slider.Log("klavye kancası yavaş: " + e.Id + " ms (yerel kanca, tuş 0x" + e.Vk.ToString("X") + ")"); return;
            case SLOW_MOUSE: Slider.Log("fare kancası yavaş: " + e.Id + " ms (yerel kanca)"); return;
            case DROPPED: Slider.Log("girdi kuyruğu doluydu: " + e.Id + " olay atıldı"); return;
        }
        if (e.Kind >= SW_OPEN && e.Kind <= SW_FULLSCREEN) { Switcher.FromNative(e); return; }
        if (e.Kind >= DESK_MENU) { var m = Mouse; if (m != null) m.FromNative(e); return; }
        var k = Keys;
        if (k != null) k.FromNative(e);
    }

    public static void SetFlag(uint id, bool on)
    {
        if (!loaded) return;
        try { li_set_flag(id, on ? 1 : 0); } catch (Exception ex) { Slider.Log("girdi bayrağı: " + ex.Message); }
    }

    static void PushFlags()
    {
        SetFlag(SHELL_UP, ShellState.Up);
        SetFlag(SWITCHER_ACTIVE, Switcher.Active);
        SetFlag(CAPTURING, Binds.Capturing);
        SetFlag(SWITCHER_DEMO, Switcher.IsDemo);
        SetFlag(SWITCHER_READY, Switcher.Ready);
        SetFlag(SINGLE_CLICK_OPEN, DesktopClick.SingleClickOpen());
    }

    // Düzenleyicinin adlandırabildiği tuşlar (Binds.KeyName)
    static void PushCapturable()
    {
        var bits = new uint[8];
        for (int vk = 1; vk < 256; vk++)
            if (Binds.KeyName(vk) != null) bits[vk / 32] |= 1u << (vk % 32);
        li_set_capturable(bits);
    }

    // ---- tablo: eylemler ve pencere yöneticisi komutları sayıyla gider (kanca metin tutmaz) ----
    static readonly object tableGate = new object();
    static readonly List<string> actions = new List<string>();
    static readonly Dictionary<string, int> actionIds = new Dictionary<string, int>();
    static readonly List<string[]> wmCommands = new List<string[]>();
    static readonly Dictionary<string, int> wmIds = new Dictionary<string, int>();

    // Aynı eylem hep aynı sayıyı alır: tablo yenilenirken kuyrukta bekleyen bir kayıt da doğru eylemi bulur
    static int Intern(string act)
    {
        int id;
        if (actionIds.TryGetValue(act, out id)) return id;
        actions.Add(act);
        return actionIds[act] = actions.Count - 1;
    }

    static int InternWm(string[] cmds)
    {
        string key = string.Join("\n", cmds);
        int id;
        if (wmIds.TryGetValue(key, out id)) return id;
        wmCommands.Add(cmds);
        return wmIds[key] = wmCommands.Count - 1;
    }

    public static string Action(int id) { lock (tableGate) return id >= 0 && id < actions.Count ? actions[id] : null; }
    public static string[] WmCommands(int id) { lock (tableGate) return id >= 0 && id < wmCommands.Count ? wmCommands[id] : null; }

    static long Pack(int mods, int vk, int kind, int flag, int id)
    {
        uint low = (uint)(mods & 0xFF) | (uint)(vk & 0xFF) << 8 | (uint)(kind & 0xFF) << 16 | (uint)(flag & 0xFF) << 24;
        ulong v = low | (ulong)(uint)id << 32;
        return unchecked((long)v);
    }

    // Kısayol tablolarından biri değişince (Binds.Load, WmBinds.Load, kısayol modu)
    public static void PushTable()
    {
        if (!loaded) return;
        try
        {
            var list = new List<long>();
            lock (tableGate)
            {
                foreach (var kv in Binds.Snapshot())
                {
                    int how = Keys2.Handling(kv.Value), vk = (int)(kv.Key & 0xFFFF);
                    if (how == 0 || vk > 0xFF) continue;
                    list.Add(Pack((int)(kv.Key >> 16), vk, T_BIND, how, Intern(kv.Value)));
                }
                for (int i = 0; i < Reserved.Combos.GetLength(0); i++)
                {
                    int m, vk;
                    if (!Binds.Parse(Reserved.Combos[i, 0], out m, out vk) || vk > 0xFF) continue;
                    string act = Reserved.Combos[i, 1];
                    list.Add(Pack(m, vk, T_RESERVED, 0, act.Length > 0 ? Intern(act) : -1));
                }
                foreach (var kv in WmBinds.Active())
                {
                    int vk = (int)(kv.Key & 0xFFFF);
                    if (vk > 0xFF) continue;
                    list.Add(Pack((int)(kv.Key >> 16), vk, T_WM, WmBinds.Repeats(kv.Value) ? 1 : 0, InternWm(kv.Value)));
                }
            }
            li_set_table(list.ToArray(), (uint)list.Count);
        }
        catch (Exception ex) { Slider.Log("girdi tablosu: " + ex.GetBaseException().Message); }
    }

    public static void Forget() { if (On) li_forget(); }

    // which: 1 klavye, 2 fare; force: Windows kancayı sökmüş (basılı tuş bilgisi de geçersiz)
    public static void Reinstall(uint which, bool force) { if (On) li_reinstall(which | (force ? 4u : 0u)); }

    public static void MousePos(out int x, out int y) { li_mouse_pos(out x, out y); }

    // Kanca bekçisi (HooksStale) yerel kancaların son çağrılma anlarını okur
    public static void SyncTicks()
    {
        if (!On) return;
        uint k, m;
        li_ticks(out k, out m);
        if (k != 0 && (int)k - Keys2.LastHookTick > 0) Keys2.LastHookTick = (int)k;
        if (m != 0 && (int)m - MouseFocus.LastHookTick > 0) MouseFocus.LastHookTick = (int)m;
    }

    // Sınama koşusu (LL_TEST=1) raporuna: son rapordan beri kancaların süreleri (CI'nin yük testi bunu yazdırır)
    public static string TestStats()
    {
        if (!On) return "| hooks .NET";
        ulong kc, kt, km, mc, mt, mm;
        li_stats(1, out kc, out kt, out km);
        li_stats(2, out mc, out mt, out mm);
        return "| hooks native key " + kc + " avg " + (kc > 0 ? kt / kc : 0) + "us max " + km + "us mouse " + mc + " avg " +
            (mc > 0 ? mt / mc : 0) + "us max " + mm + "us";
    }

    // Beş dakikada bir: kancaların içinde geçen süre (çağrı sayısı, ortalama, en uzun)
    public static void LogStats()
    {
        if (!On) return;
        ulong kc, kt, km, mc, mt, mm;
        li_stats(1, out kc, out kt, out km);
        li_stats(2, out mc, out mt, out mm);
        if (kc == 0 && mc == 0) return;
        Slider.Log("girdi kancaları (yerel): klavye " + kc + " olay, ort " + (kc > 0 ? kt / kc : 0) + " µs, en uzun " + km +
            " µs; fare " + mc + " olay, ort " + (mc > 0 ? mt / mc : 0) + " µs, en uzun " + mm + " µs");
    }
}
