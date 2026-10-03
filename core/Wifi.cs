using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

// Wi-Fi ağları ve bağlanma (eskiden scripts\wifi.ps1, netsh wlan çıktısını dile göre ayrıştırıyordu): Windows'un Native
// Wifi API'si (wlanapi.dll). Çıktılar betiğinkiyle aynı JSON:
//   List()                -> {"connected":"SSID","networks":[{"ssid","signal","secure","known"}]}
//   Connect(ssid, pw)     -> {"ok":true} | {"ok":false,"needPassword":true} | {"ok":false,"error":"..."}
//   Disconnect()          -> {"ok":true}
static class Wifi
{
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    [DllImport("wlanapi.dll")] static extern int WlanOpenHandle(uint version, IntPtr reserved, out uint negotiated, out IntPtr handle);
    [DllImport("wlanapi.dll")] static extern int WlanCloseHandle(IntPtr handle, IntPtr reserved);
    [DllImport("wlanapi.dll")] static extern int WlanEnumInterfaces(IntPtr handle, IntPtr reserved, out IntPtr list);
    [DllImport("wlanapi.dll")] static extern int WlanScan(IntPtr handle, ref Guid iface, IntPtr ssid, IntPtr ie, IntPtr reserved);
    [DllImport("wlanapi.dll")] static extern int WlanGetAvailableNetworkList(IntPtr handle, ref Guid iface, uint flags, IntPtr reserved, out IntPtr list);
    [DllImport("wlanapi.dll", CharSet = CharSet.Unicode)] static extern int WlanSetProfile(IntPtr handle, ref Guid iface, uint flags, string xml, string security, bool overwrite, IntPtr reserved, out uint reason);
    [DllImport("wlanapi.dll", CharSet = CharSet.Unicode)] static extern int WlanDeleteProfile(IntPtr handle, ref Guid iface, string name, IntPtr reserved);
    [DllImport("wlanapi.dll")] static extern int WlanConnect(IntPtr handle, ref Guid iface, ref ConnectionParameters p, IntPtr reserved);
    [DllImport("wlanapi.dll")] static extern int WlanDisconnect(IntPtr handle, ref Guid iface, IntPtr reserved);
    [DllImport("wlanapi.dll")] static extern int WlanRegisterNotification(IntPtr handle, uint source, bool ignoreDuplicate, NotificationCallback cb, IntPtr context, IntPtr reserved, out uint previous);
    [DllImport("wlanapi.dll")] static extern void WlanFreeMemory(IntPtr p);

    [StructLayout(LayoutKind.Sequential)]
    struct NotificationData { public uint Source; public uint Code; public Guid Iface; public uint Size; public IntPtr Data; }
    delegate void NotificationCallback(ref NotificationData data, IntPtr context);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct ConnectionParameters
    {
        public int Mode;                                       // 0: wlan_connection_mode_profile
        [MarshalAs(UnmanagedType.LPWStr)] public string Profile;
        public IntPtr Ssid, BssidList;
        public int BssType;                                    // 1: infrastructure
        public uint Flags;
    }

    const uint SOURCE_ACM = 0x8, ACM_SCAN_COMPLETE = 7, ACM_SCAN_FAIL = 8;
    const uint PROFILE_USER = 2;                              // yalnızca bu kullanıcının profili (netsh user=current)
    const int NET_CONNECTED = 1, NET_HAS_PROFILE = 2;
    const int ITEM = 628;                                     // WLAN_AVAILABLE_NETWORK

    sealed class Net { public string Ssid, Profile; public int Signal, Auth, Cipher; public bool Secure, Known, Connected; }

    static T WithHandle<T>(Func<IntPtr, Guid, T> f, T none)
    {
        IntPtr h; uint v;
        if (WlanOpenHandle(2, IntPtr.Zero, out v, out h) != 0) return none;
        try
        {
            IntPtr list;
            if (WlanEnumInterfaces(h, IntPtr.Zero, out list) != 0) return none;
            Guid iface;
            try
            {
                if (Marshal.ReadInt32(list) == 0) return none;     // dwNumberOfItems: Wi-Fi kartı yok
                // WLAN_INTERFACE_INFO_LIST { uint count; uint index; WLAN_INTERFACE_INFO[] } -> ilk kartın Guid'i
                var b = new byte[16];
                Marshal.Copy(list + 8, b, 0, 16);
                iface = new Guid(b);
            }
            finally { WlanFreeMemory(list); }
            return f(h, iface);
        }
        finally { WlanCloseHandle(h, IntPtr.Zero); }
    }

    // Tarama ister ve bitmesini (en fazla 4 sn) bekler; tarama olmasa da önbellekteki liste döner
    static void Scan(IntPtr h, Guid iface)
    {
        using (var done = new ManualResetEvent(false))
        {
            NotificationCallback cb = (ref NotificationData d, IntPtr c) =>
            {
                if (d.Source == SOURCE_ACM && (d.Code == ACM_SCAN_COMPLETE || d.Code == ACM_SCAN_FAIL)) try { done.Set(); } catch (ObjectDisposedException) { }
            };
            uint prev;
            bool registered = WlanRegisterNotification(h, SOURCE_ACM, true, cb, IntPtr.Zero, IntPtr.Zero, out prev) == 0;
            if (WlanScan(h, ref iface, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero) == 0 && registered) done.WaitOne(4000);
            if (registered) WlanRegisterNotification(h, 0, true, null, IntPtr.Zero, IntPtr.Zero, out prev);
            GC.KeepAlive(cb);
        }
    }

    static List<Net> Networks(IntPtr h, Guid iface)
    {
        var nets = new Dictionary<string, Net>();
        IntPtr list;
        if (WlanGetAvailableNetworkList(h, ref iface, 0, IntPtr.Zero, out list) != 0) return new List<Net>();
        try
        {
            int count = Marshal.ReadInt32(list);
            for (int i = 0; i < count; i++)
            {
                IntPtr e = list + 8 + i * ITEM;
                int len = Math.Min(32, Marshal.ReadInt32(e, 512));
                if (len <= 0) continue;                            // gizli ağ
                var raw = new byte[len];
                Marshal.Copy(e + 516, raw, 0, len);
                string ssid = Encoding.UTF8.GetString(raw);
                int flags = Marshal.ReadInt32(e, 620);
                var n = new Net
                {
                    Ssid = ssid,
                    Profile = Marshal.PtrToStringUni(e),
                    Signal = Marshal.ReadInt32(e, 604),
                    Secure = Marshal.ReadInt32(e, 608) != 0,
                    Auth = Marshal.ReadInt32(e, 612),
                    Cipher = Marshal.ReadInt32(e, 616),
                    Known = (flags & NET_HAS_PROFILE) != 0,
                    Connected = (flags & NET_CONNECTED) != 0
                };
                Net have;
                if (!nets.TryGetValue(ssid, out have)) { nets[ssid] = n; continue; }
                // aynı ağ profilli ve profilsiz iki satır olarak gelir: birleştirilir
                have.Signal = Math.Max(have.Signal, n.Signal);
                have.Connected |= n.Connected;
                if (n.Known && !have.Known) { have.Known = true; have.Profile = n.Profile; }
            }
        }
        finally { WlanFreeMemory(list); }
        return nets.Values.OrderByDescending(n => n.Signal).ToList();
    }

    public static string List()
    {
        return WithHandle((h, iface) =>
        {
            Scan(h, iface);
            var nets = Networks(h, iface);
            var con = nets.FirstOrDefault(n => n.Connected);
            return json.Serialize(new Dictionary<string, object>
            {
                { "connected", con != null ? con.Ssid : "" },
                { "networks", nets.Select(n => new Dictionary<string, object> { { "ssid", n.Ssid }, { "signal", n.Signal }, { "secure", n.Secure }, { "known", n.Known } }).ToList() }
            });
        }, "{\"connected\":\"\",\"networks\":[]}");
    }

    public static string Disconnect()
    {
        return WithHandle((h, iface) => WlanDisconnect(h, ref iface, IntPtr.Zero) == 0 ? "{\"ok\":true}" : "{\"ok\":false}", "{\"ok\":false}");
    }

    public static string Connect(string ssid, string password)
    {
        if (string.IsNullOrEmpty(ssid) || ssid.Length > 32) return Fail("Geçersiz ağ adı");
        return WithHandle((h, iface) =>
        {
            var net = Networks(h, iface).FirstOrDefault(n => n.Ssid == ssid);
            string profile = net != null && net.Known ? net.Profile : null;
            bool created = false;
            if (profile == null)
            {
                bool secure = net == null || net.Secure;
                if (secure && string.IsNullOrEmpty(password)) return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "needPassword", true } });
                string xml = ProfileXml(ssid, net, password);
                if (xml == null) return Fail("Bu ağın güvenlik türü desteklenmiyor (kurumsal ağ)");
                uint reason;
                if (WlanSetProfile(h, ref iface, PROFILE_USER, xml, null, true, IntPtr.Zero, out reason) != 0) return Fail("Ağ profili kaydedilemedi");
                profile = ssid; created = true;
            }
            var p = new ConnectionParameters { Mode = 0, Profile = profile, BssType = 1 };
            if (WlanConnect(h, ref iface, ref p, IntPtr.Zero) == 0)
                for (int i = 0; i < 30; i++)                       // en fazla ~15 sn
                {
                    Thread.Sleep(500);
                    var now = Networks(h, iface).FirstOrDefault(n => n.Ssid == ssid);
                    if (now != null && now.Connected) return "{\"ok\":true}";
                }
            // yanlış şifreyle oluşturulan profil kalmasın (sonraki denemede yine şifre sorulsun)
            if (created) WlanDeleteProfile(h, ref iface, profile, IntPtr.Zero);
            return Fail("Bağlanılamadı (şifre yanlış olabilir)");
        }, Fail("Wi-Fi kartı bulunamadı"));
    }

    static string Fail(string error) { return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", error } }); }

    // Ağın kendi bildirdiği güvenlik türüyle kişisel profil; kurumsal (802.1X) ağlar için null
    static string ProfileXml(string ssid, Net net, string password)
    {
        int auth = net != null ? net.Auth : 7, cipher = net != null ? net.Cipher : 4;
        string a, e;
        switch (auth)
        {
            case 1: a = "open"; break;          // DOT11_AUTH_ALGO_80211_OPEN
            case 4: a = "WPAPSK"; break;        // WPA_PSK
            case 7: a = "WPA2PSK"; break;       // RSNA_PSK
            case 9: a = "WPA3SAE"; break;       // WPA3_SAE
            default: return null;
        }
        e = a == "open" ? "none" : cipher == 2 ? "TKIP" : "AES";
        string name = SecurityElement.Escape(ssid);
        string hex = string.Concat(Encoding.UTF8.GetBytes(ssid).Select(b => b.ToString("X2")));
        string key = a == "open" ? "" :
            "<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>" + SecurityElement.Escape(password ?? "") + "</keyMaterial></sharedKey>";
        return "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>" + name +
            "</name><SSIDConfig><SSID><hex>" + hex + "</hex><name>" + name + "</name></SSID></SSIDConfig><connectionType>ESS</connectionType>" +
            "<connectionMode>auto</connectionMode><MSM><security><authEncryption><authentication>" + a + "</authentication><encryption>" + e +
            "</encryption><useOneX>false</useOneX></authEncryption>" + key + "</security></MSM></WLANProfile>";
    }
}
