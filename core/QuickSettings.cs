using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using Microsoft.Win32;

// Sağ panelin hızlı ayarları (Wi-Fi / Bluetooth radyoları, Ethernet, eşleşmiş Bluetooth cihazları, uyanık tut, tema):
// eskiden panel her açıldığında dört PowerShell betiği başlıyordu (radios/eth/bt/status.ps1, her biri ~1 sn ve
// onlarca MB), uyanık tut da arkada sonsuza dek çalışan bir PowerShell'di. Hepsi çekirdeğin içinde, /qs/ yollarıyla.
static class QuickSettings
{
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    // /qs/... (yalnızca POST; çağıran Accept'te doğrulandı). false: bilinmeyen yol.
    public static bool Http(string target, out string status, out string body)
    {
        status = "200 OK"; body = "";
        var m = System.Text.RegularExpressions.Regex.Match(target, @"^/qs/radio\?kind=(wifi|bluetooth)&state=(On|Off)$");
        if (m.Success)
        {
            SetRadio(m.Groups[1].Value, m.Groups[2].Value);
            status = "204 No Content";
            return true;
        }
        m = System.Text.RegularExpressions.Regex.Match(target, @"^/qs/awake\?v=([01])$");
        if (m.Success)
        {
            SetAwake(m.Groups[1].Value == "1");
            status = "204 No Content";
            return true;
        }
        m = System.Text.RegularExpressions.Regex.Match(target, @"^/qs/wifi-connect\?ssid=([^&]+)(?:&pw=([^&]*))?$");
        if (m.Success)
        {
            body = Wifi.Connect(Uri.UnescapeDataString(m.Groups[1].Value), m.Groups[2].Success ? Uri.UnescapeDataString(m.Groups[2].Value) : null);
            return true;
        }
        switch (target)
        {
            case "/qs/wifi": body = Wifi.List(); return true;
            case "/qs/wifi-disconnect": body = Wifi.Disconnect(); return true;
            case "/qs/radios": body = RadiosJson(); return true;
            case "/qs/eth": body = EthJson(); return true;
            case "/qs/eth-toggle": body = EthToggle(); return true;
            case "/qs/bt": body = BtJson(); return true;
            case "/qs/status": body = StatusJson(); return true;
        }
        if (target.StartsWith("/qs/radio") || target.StartsWith("/qs/awake") || target.StartsWith("/qs/wifi")) { status = "400 Bad Request"; return true; }
        return false;
    }

    // ---------------- Radyolar (WinRT Windows.Devices.Radios) ----------------
    // .NET 4.8'den WinRT: türler çalışma anında Windows meta verisinden, IAsyncOperation System.Runtime.WindowsRuntime'ın
    // AsTask'iyle beklenir (PowerShell betiğinin yaptığının aynısı). Derleme Windows.winmd'ye bağlı kalmaz.
    const int WINRT_TIMEOUT_MS = 5000;
    static MethodInfo asTask;
    static Type radioType;

    static Type RadioType
    {
        get
        {
            if (radioType == null) radioType = Type.GetType("Windows.Devices.Radios.Radio, Windows.System.Devices, ContentType=WindowsRuntime", true);
            return radioType;
        }
    }

    // WinRT yöntemini çağırıp IAsyncOperation<T> sonucunu bekler; T yöntemin dönüş türünden (çalışma anındaki nesnenin
    // türü yalnızca __ComObject olabilir)
    static object Await(MethodInfo method, object target, params object[] args)
    {
        if (asTask == null)
        {
            var asm = Assembly.Load("System.Runtime.WindowsRuntime, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089");
            asTask = asm.GetType("System.WindowsRuntimeSystemExtensions", true).GetMethods().First(x =>
                x.Name == "AsTask" && x.IsGenericMethodDefinition && x.GetParameters().Length == 1
                && x.GetParameters()[0].ParameterType.Name == "IAsyncOperation`1");
        }
        var op = method.Invoke(target, args);
        var task = (Task)asTask.MakeGenericMethod(method.ReturnType.GetGenericArguments()[0]).Invoke(null, new[] { op });
        if (!task.Wait(WINRT_TIMEOUT_MS)) throw new TimeoutException("WinRT");
        return task.GetType().GetProperty("Result").GetValue(task, null);
    }

    static List<object> Radios()
    {
        var t = RadioType;
        Await(t.GetMethod("RequestAccessAsync"), null);
        var list = new List<object>();
        foreach (var r in (IEnumerable)Await(t.GetMethod("GetRadiosAsync"), null)) list.Add(r);
        return list;
    }

    static string Prop(object o, string name)
    {
        var v = RadioType.GetProperty(name).GetValue(o, null);
        return v == null ? null : v.ToString();
    }

    // {"wifi":"On","bluetooth":"Off"}; adaptörü olmayan null
    public static string RadiosJson()
    {
        string wifi = null, bt = null;
        try
        {
            foreach (var r in Radios())
            {
                string kind = Prop(r, "Kind");
                if (kind == "WiFi") wifi = Prop(r, "State");
                if (kind == "Bluetooth") bt = Prop(r, "State");
            }
        }
        catch (Exception ex) { Slider.Log("radyolar okunamadı: " + ex.GetBaseException().Message); }
        return json.Serialize(new Dictionary<string, object> { { "wifi", wifi }, { "bluetooth", bt } });
    }

    static void SetRadio(string kind, string state)
    {
        string want = kind == "wifi" ? "WiFi" : "Bluetooth";
        try
        {
            foreach (var r in Radios())
            {
                if (Prop(r, "Kind") != want) continue;
                var set = RadioType.GetMethod("SetStateAsync");
                Await(set, r, Enum.Parse(set.GetParameters()[0].ParameterType, state));
                return;
            }
        }
        catch (Exception ex) { Slider.Log("radyo ayarlanamadı (" + kind + " " + state + "): " + ex.GetBaseException().Message); }
    }

    // ---------------- Ethernet ----------------
    // Fiziksel kablolu kart: donanım arabirimi, takılı bir bağlantı noktası ve fiziksel ortamı 802.3 (MSFT_NetAdapter;
    // Get-NetAdapter de bu sınıfı okur). Sanal kartlar (VPN, sanal makine, tünel) bu üç özellikle ayrılır, adlarıyla değil.

    sealed class Adapter { public string Name, Desc; public int Index; public bool Disabled, Up; public ulong Speed; }

    static Adapter Wired()
    {
        var scope = new System.Management.ManagementScope(@"root\StandardCimv2");
        var q = new System.Management.ObjectQuery("SELECT Name, InterfaceDescription, InterfaceIndex, State, InterfaceOperationalStatus, NdisPhysicalMedium, Speed, Virtual, HardwareInterface, ConnectorPresent FROM MSFT_NetAdapter");
        using (var s = new System.Management.ManagementObjectSearcher(scope, q, new System.Management.EnumerationOptions { Timeout = TimeSpan.FromSeconds(5) }))
        using (var all = s.Get())
            foreach (System.Management.ManagementObject a in all)
                using (a)
                {
                    bool physical = !Flag(a["Virtual"]) && Flag(a["HardwareInterface"]) && Flag(a["ConnectorPresent"]);
                    string desc = (a["InterfaceDescription"] as string) ?? "";
                    if (!physical || Convert.ToUInt32(a["NdisPhysicalMedium"] ?? 0u) != 14) continue; // 14: NdisPhysicalMedium802_3
                    return new Adapter
                    {
                        Name = a["Name"] as string, Desc = desc, Index = Convert.ToInt32(a["InterfaceIndex"] ?? 0),
                        Disabled = Convert.ToUInt32(a["State"] ?? 0u) == 3,
                        Up = Convert.ToUInt32(a["InterfaceOperationalStatus"] ?? 0u) == 1,
                        Speed = Convert.ToUInt64(a["Speed"] ?? 0ul)
                    };
                }
        return null;
    }

    static bool Flag(object v) { return v is bool && (bool)v; }

    // Get-NetAdapter'ın LinkSpeed yazımı: "1 Gbps", "2.5 Gbps", "100 Mbps"
    static string Speed(ulong bps)
    {
        var ci = System.Globalization.CultureInfo.InvariantCulture;
        if (bps >= 1000000000UL) return (bps / 1e9).ToString("0.#", ci) + " Gbps";
        if (bps >= 1000000UL) return (bps / 1e6).ToString("0.#", ci) + " Mbps";
        if (bps >= 1000UL) return (bps / 1e3).ToString("0.#", ci) + " Kbps";
        return bps + " bps";
    }

    static string IPv4(int index)
    {
        foreach (var ni in System.Net.NetworkInformation.NetworkInterface.GetAllNetworkInterfaces())
        {
            try
            {
                var p = ni.GetIPProperties();
                var v4 = p.GetIPv4Properties();
                if (v4 == null || v4.Index != index) continue;
                foreach (var u in p.UnicastAddresses)
                    if (u.Address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork) return u.Address.ToString();
            }
            catch (System.Net.NetworkInformation.NetworkInformationException) { }
        }
        return null;
    }

    // {"state":"up|disconnected|disabled|none","name","desc","speed","ip"}
    public static string EthJson()
    {
        Adapter a;
        try { a = Wired(); }
        catch (Exception ex) { Slider.Log("ethernet okunamadı: " + ex.GetBaseException().Message); a = null; }
        if (a == null) return "{\"state\":\"none\"}";
        return json.Serialize(new Dictionary<string, object>
        {
            { "state", a.Disabled ? "disabled" : a.Up ? "up" : "disconnected" },
            { "name", a.Name }, { "desc", a.Desc }, { "speed", Speed(a.Speed) }, { "ip", a.Disabled ? null : IPv4(a.Index) }
        });
    }

    // Kartı açıp kapatmak yönetici ister. Çekirdek oturum açılışındaki görevden yönetici olarak başlar ve bunu kendisi
    // yapar; yönetici değilse kurulumun görevleri (LogicalLunge\Ethernet-On / -Off: lunge.exe --eth) izin sormaz.
    public static string EthToggle()
    {
        Adapter a;
        try { a = Wired(); }
        catch (Exception ex) { Slider.Log("ethernet okunamadı: " + ex.GetBaseException().Message); a = null; }
        if (a == null) return "{\"ok\":false}";
        if (UserLaunch.Elevated) return SetEth(a.Disabled) ? "{\"ok\":true,\"needSetup\":false}" : "{\"ok\":false,\"needSetup\":false}";
        string task = a.Disabled ? @"LogicalLunge\Ethernet-On" : @"LogicalLunge\Ethernet-Off";
        int code = -1;
        try
        {
            using (var p = Process.Start(new ProcessStartInfo("schtasks.exe", "/run /tn \"" + task + "\"")
            { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true }))
            {
                p.StandardOutput.ReadToEnd(); p.StandardError.ReadToEnd();
                if (p.WaitForExit(10000)) code = p.ExitCode;
            }
        }
        catch (Exception ex) { Slider.Log("ethernet görevi başlatılamadı: " + ex.Message); }
        return code == 0 ? "{\"ok\":true,\"needSetup\":false}" : "{\"ok\":false,\"needSetup\":true}";
    }

    // lunge.exe --eth enable|disable (yönetici): fiziksel kablolu kartı MSFT_NetAdapter Enable/Disable ile açar / kapatır
    public static bool SetEth(bool enable)
    {
        try
        {
            var scope = new System.Management.ManagementScope(@"root\StandardCimv2");
            var q = new System.Management.ObjectQuery("SELECT * FROM MSFT_NetAdapter");
            bool any = false;
            using (var s = new System.Management.ManagementObjectSearcher(scope, q, new System.Management.EnumerationOptions { Timeout = TimeSpan.FromSeconds(5) }))
            using (var all = s.Get())
                foreach (System.Management.ManagementObject a in all)
                    using (a)
                    {
                        bool physical = !Flag(a["Virtual"]) && Flag(a["HardwareInterface"]) && Flag(a["ConnectorPresent"]);
                        if (!physical || Convert.ToUInt32(a["NdisPhysicalMedium"] ?? 0u) != 14) continue;
                        var r = a.InvokeMethod(enable ? "Enable" : "Disable", null);
                        if (Convert.ToUInt32(r ?? 1u) == 0) any = true;
                    }
            return any;
        }
        catch (Exception ex) { Slider.Log("ethernet " + (enable ? "açılamadı" : "kapatılamadı") + ": " + ex.GetBaseException().Message); return false; }
    }

    // ---------------- Bluetooth cihazları ----------------
    // Eşleşmiş cihazlar SetupAPI'den (Get-PnpDevice -Class Bluetooth'un okuduğu yer); PnP'nin "OK" durumu yalnızca
    // eşleşmiş/yüklü demek, gerçek bağlantı DEVPKEY_Bluetooth_IsConnected.
    static readonly Guid GUID_DEVCLASS_BLUETOOTH = new Guid("e0cbf06c-cd8b-4647-bb8a-263b43f0f974");
    static readonly System.Text.RegularExpressions.Regex btSkip = new System.Text.RegularExpressions.Regex(
        "enumerator|adapter|radio|microsoft|generic|service|protocol|transport|rfcomm|avrcp|hizmet|ağ geçidi|erişim",
        System.Text.RegularExpressions.RegexOptions.IgnoreCase);

    [StructLayout(LayoutKind.Sequential)]
    struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public uint DevInst; public IntPtr Reserved; }
    [StructLayout(LayoutKind.Sequential)]
    struct DEVPROPKEY { public Guid fmtid; public uint pid; }

    [DllImport("setupapi.dll", SetLastError = true)]
    static extern IntPtr SetupDiGetClassDevsW(ref Guid cls, IntPtr enumerator, IntPtr parent, uint flags);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiEnumDeviceInfo(IntPtr set, uint index, ref SP_DEVINFO_DATA data);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetupDiGetDeviceInstanceIdW(IntPtr set, ref SP_DEVINFO_DATA data, StringBuilder id, int size, out int need);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetupDiGetDeviceRegistryPropertyW(IntPtr set, ref SP_DEVINFO_DATA data, uint prop, out uint type, byte[] buf, int size, out int need);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiGetDevicePropertyW(IntPtr set, ref SP_DEVINFO_DATA data, ref DEVPROPKEY key, out uint type, byte[] buf, int size, out int need, uint flags);
    [DllImport("setupapi.dll")]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);

    const uint SPDRP_DEVICEDESC = 0x0, SPDRP_FRIENDLYNAME = 0xC;

    static string RegString(IntPtr set, ref SP_DEVINFO_DATA d, uint prop)
    {
        var buf = new byte[1024]; uint type; int need;
        if (!SetupDiGetDeviceRegistryPropertyW(set, ref d, prop, out type, buf, buf.Length, out need)) return null;
        string s = Encoding.Unicode.GetString(buf, 0, Math.Max(0, Math.Min(need, buf.Length))).TrimEnd('\0');
        return s.Length > 0 ? s : null;
    }

    static bool IsConnected(IntPtr set, ref SP_DEVINFO_DATA d)
    {
        var key = new DEVPROPKEY { fmtid = new Guid("83DA6326-97A6-4088-9453-A1923F573B29"), pid = 15 };
        var buf = new byte[4]; uint type; int need;
        return SetupDiGetDevicePropertyW(set, ref d, ref key, out type, buf, buf.Length, out need, 0) && type == 0x11 && buf[0] != 0; // DEVPROP_TYPE_BOOLEAN
    }

    static string Kind(string n)
    {
        Func<string, bool> has = p => System.Text.RegularExpressions.Regex.IsMatch(n, p, System.Text.RegularExpressions.RegexOptions.IgnoreCase);
        if (has("buds|air|headphone|headset|kulaklık|wh-|wf-")) return "headphones";
        if (has("mouse|fare")) return "mouse";
        if (has("keyboard|klavye")) return "keyboard";
        if (has("phone|galaxy|iphone|pixel|redmi|xiaomi")) return "smartphone";
        if (has("controller|gamepad|xbox|dualsense")) return "sports_esports";
        return "bluetooth";
    }

    // {"adapter":true,"on":true,"devices":[{"name","connected","kind"}]}
    public static string BtJson()
    {
        object radio = null;
        try { foreach (var r in Radios()) if (Prop(r, "Kind") == "Bluetooth") { radio = r; break; } }
        catch (Exception ex) { Slider.Log("bluetooth radyosu okunamadı: " + ex.GetBaseException().Message); }

        var names = new List<string>();
        var connected = new HashSet<string>(StringComparer.CurrentCultureIgnoreCase);
        Guid cls = GUID_DEVCLASS_BLUETOOTH;
        IntPtr set = SetupDiGetClassDevsW(ref cls, IntPtr.Zero, IntPtr.Zero, 0); // takılı olmayanlar da (eşleşmiş ama kapalı)
        if (set != new IntPtr(-1))
        {
            try
            {
                var d = new SP_DEVINFO_DATA { cbSize = Marshal.SizeOf(typeof(SP_DEVINFO_DATA)) };
                for (uint i = 0; SetupDiEnumDeviceInfo(set, i, ref d); i++)
                {
                    var id = new StringBuilder(512); int need;
                    if (!SetupDiGetDeviceInstanceIdW(set, ref d, id, id.Capacity, out need) || !id.ToString().StartsWith("BTH", StringComparison.OrdinalIgnoreCase)) continue;
                    string name = RegString(set, ref d, SPDRP_FRIENDLYNAME) ?? RegString(set, ref d, SPDRP_DEVICEDESC);
                    if (name == null || btSkip.IsMatch(name)) continue;
                    names.Add(name);
                    if (IsConnected(set, ref d)) connected.Add(name);
                }
            }
            finally { SetupDiDestroyDeviceInfoList(set); }
        }
        var devices = new List<object>();
        var seen = new HashSet<string>(StringComparer.CurrentCultureIgnoreCase);
        foreach (var n in names.OrderBy(x => x, StringComparer.CurrentCultureIgnoreCase))
        {
            if (!seen.Add(n)) continue;
            devices.Add(new Dictionary<string, object> { { "name", n }, { "connected", connected.Contains(n) }, { "kind", Kind(n) } });
        }
        bool on = false;
        try { on = radio != null && Prop(radio, "State") == "On"; } catch { }
        return json.Serialize(new Dictionary<string, object> { { "adapter", radio != null }, { "on", on }, { "devices", devices } });
    }

    // ---------------- Uyanık tut ----------------
    // Ekran ve sistem uykuya geçmez: durum, onu isteyen iş parçacığı yaşadıkça geçerli (SetThreadExecutionState).
    [DllImport("kernel32.dll")]
    static extern uint SetThreadExecutionState(uint flags);
    const uint ES_CONTINUOUS = 0x80000000, ES_SYSTEM_REQUIRED = 0x1, ES_DISPLAY_REQUIRED = 0x2;

    static readonly object awakeGate = new object();
    static ManualResetEvent awakeStop;
    static int legacySwept;

    public static bool Awake { get { lock (awakeGate) return awakeStop != null; } }

    public static void SetAwake(bool on)
    {
        SweepLegacyAwake();
        lock (awakeGate)
        {
            if (on == (awakeStop != null)) return;
            if (!on) { awakeStop.Set(); awakeStop = null; return; }
            var stop = new ManualResetEvent(false);
            awakeStop = stop;
            new Thread(() =>
            {
                SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
                stop.WaitOne();
                SetThreadExecutionState(ES_CONTINUOUS);
                stop.Dispose();
            }) { IsBackground = true, Name = "keep-awake" }.Start();
        }
    }

    // Eski sürümden kalan keep-awake.ps1 (arkada sonsuza dek çalışan PowerShell): güncellemeden sonra bir kez kapatılır,
    // açık bırakılmışsa panel uyanık tutmayı çekirdekte yeniden açar
    static void SweepLegacyAwake()
    {
        if (Interlocked.Exchange(ref legacySwept, 1) != 0) return;
        try
        {
            using (var s = new System.Management.ManagementObjectSearcher("SELECT ProcessId, CommandLine FROM Win32_Process WHERE Name='powershell.exe'"))
            using (var all = s.Get())
                foreach (System.Management.ManagementObject p in all)
                    using (p)
                    {
                        string cl = p["CommandLine"] as string;
                        if (cl == null || cl.IndexOf("keep-awake.ps1", StringComparison.OrdinalIgnoreCase) < 0) continue;
                        try { using (var proc = Process.GetProcessById(Convert.ToInt32(p["ProcessId"]))) proc.Kill(); } catch { }
                    }
        }
        catch (Exception ex) { Slider.Log("eski uyanık tut betiği aranamadı: " + ex.Message); }
    }

    // {"awake":true,"light":false}
    public static string StatusJson()
    {
        bool light = false;
        try
        {
            using (var k = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"))
            {
                object v = k == null ? null : k.GetValue("AppsUseLightTheme");
                light = v is int && (int)v == 1;
            }
        }
        catch { }
        ThreadPool.QueueUserWorkItem(_ => SweepLegacyAwake());
        return "{\"awake\":" + (Awake ? "true" : "false") + ",\"light\":" + (light ? "true" : "false") + "}";
    }
}
