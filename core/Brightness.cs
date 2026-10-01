using System;
using System.Collections.Generic;
using System.Management;
using System.Runtime.InteropServices;
using System.Threading;

// Parlaklık: dahili ekran WMI ile (WmiMonitorBrightness), harici monitör DDC/CI ile (Windows'un monitör yapılandırma
// API'si, dxva2). Bar her tekerlek adımında HTTP /brightness ile buraya gelir: eskiden her adım bir PowerShell süreci ve
// iki WMI sorgusu açıyordu, dizüstünde parlaklık bir saniyeye yakın gecikiyordu. Yazmalar arka planda, monitör başına
// "son değer kazanır" ile uygulanır. WMI satırı monitörüne MonitorFriendlyNames'in eşlemesiyle bağlanır: betik, dizüstüne
// harici monitör takılıyken onun barında da dahili ekranı değiştiriyordu.
static class Brightness
{
    // device: monitörün GDI adı ("\\.\DISPLAY1"). null: okunamadı ya da bu monitörün parlaklığı ayarlanamıyor (o zaman
    // bar yalnızca gamayı değiştirir).
    public static int? Get(string device)
    {
        int? wmi = WmiGet(device);
        if (wmi.HasValue) return wmi;
        return WithPhysicalMonitor(device, h =>
        {
            uint min, cur, max;
            if (!GetMonitorBrightness(h, out min, out cur, out max) || max <= min) return null;
            return (int?)(int)Math.Round((cur - min) * 100.0 / (max - min));
        });
    }

    static readonly object gate = new object();
    static readonly Dictionary<string, int> pending = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
    static readonly AutoResetEvent wake = new AutoResetEvent(false);
    static Thread worker;

    public static void Set(string device, int value)
    {
        lock (gate)
        {
            pending[device] = Math.Max(0, Math.Min(100, value));
            if (worker == null) { worker = new Thread(Work) { IsBackground = true, Name = "brightness" }; worker.Start(); }
        }
        wake.Set();
    }

    static void Work()
    {
        while (true)
        {
            wake.WaitOne();
            while (true)
            {
                string device = null;
                int value = 0;
                lock (gate)
                {
                    foreach (var kv in pending) { device = kv.Key; value = kv.Value; break; }
                    if (device == null) break;
                    pending.Remove(device);
                }
                try
                {
                    if (!WmiSet(device, value))
                        WithPhysicalMonitor(device, h =>
                        {
                            uint min, cur, max;
                            if (GetMonitorBrightness(h, out min, out cur, out max) && max > min)
                                SetMonitorBrightness(h, min + (uint)Math.Round((max - min) * value / 100.0));
                            return null;
                        });
                }
                catch (Exception ex) { Slider.Log("parlaklık " + device + ": " + ex.Message); }
            }
        }
    }

    // WMI parlaklık sınıfı yalnızca dahili ekranlarda vardır; masaüstünde sorgu ManagementException verir.
    static int? WmiGet(string device)
    {
        var keys = MonitorFriendlyNames.InstanceKeys(device);
        if (keys.Count == 0) return null;
        try
        {
            using (var search = new ManagementObjectSearcher("root\\wmi", "SELECT InstanceName, CurrentBrightness FROM WmiMonitorBrightness WHERE Active = True"))
            using (var rows = search.Get())
                foreach (ManagementBaseObject row in rows)
                    if (Matches(row["InstanceName"] as string, keys)) return Convert.ToInt32(row["CurrentBrightness"]);
        }
        catch (ManagementException) { }
        return null;
    }

    static bool WmiSet(string device, int value)
    {
        var keys = MonitorFriendlyNames.InstanceKeys(device);
        if (keys.Count == 0) return false;
        try
        {
            using (var search = new ManagementObjectSearcher("root\\wmi", "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active = True"))
            using (var rows = search.Get())
                foreach (ManagementObject row in rows)
                    if (Matches(row["InstanceName"] as string, keys))
                    {
                        row.InvokeMethod("WmiSetBrightness", new object[] { (uint)1, (byte)value });
                        return true;
                    }
        }
        catch (ManagementException) { }
        return false;
    }

    static bool Matches(string instanceName, List<string> keys)
    {
        foreach (var key in keys) if (MonitorFriendlyNames.IsInstanceOf(instanceName, key)) return true;
        return false;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct MonitorInfoEx
    {
        public int cbSize;
        public Native.RECT rcMonitor, rcWork;
        public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szDevice;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct PhysicalMonitor
    {
        public IntPtr hPhysicalMonitor;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string szPhysicalMonitorDescription;
    }

    delegate bool MonitorEnumProc(IntPtr monitor, IntPtr hdc, IntPtr rect, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnumProc proc, IntPtr data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfoEx info);
    [DllImport("dxva2.dll")] static extern bool GetNumberOfPhysicalMonitorsFromHMONITOR(IntPtr monitor, out uint count);
    [DllImport("dxva2.dll")] static extern bool GetPhysicalMonitorsFromHMONITOR(IntPtr monitor, uint count, [Out] PhysicalMonitor[] monitors);
    [DllImport("dxva2.dll")] static extern bool DestroyPhysicalMonitors(uint count, PhysicalMonitor[] monitors);
    [DllImport("dxva2.dll")] static extern bool GetMonitorBrightness(IntPtr monitor, out uint min, out uint current, out uint max);
    [DllImport("dxva2.dll")] static extern bool SetMonitorBrightness(IntPtr monitor, uint value);

    // DDC/CI: GDI monitörünün arkasındaki ilk fiziksel monitör (yansıtılmış ekranda birden çok olur, aynı görüntüyü
    // paylaşırlar). DDC/CI desteklemeyen monitörde null.
    static int? WithPhysicalMonitor(string device, Func<IntPtr, int?> use)
    {
        IntPtr target = IntPtr.Zero;
        MonitorEnumProc find = (monitor, hdc, rect, data) =>
        {
            var info = new MonitorInfoEx { cbSize = Marshal.SizeOf(typeof(MonitorInfoEx)) };
            if (GetMonitorInfo(monitor, ref info) && string.Equals(info.szDevice, device, StringComparison.OrdinalIgnoreCase))
            {
                target = monitor;
                return false;
            }
            return true;
        };
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, find, IntPtr.Zero);
        GC.KeepAlive(find);
        uint count;
        if (target == IntPtr.Zero || !GetNumberOfPhysicalMonitorsFromHMONITOR(target, out count) || count == 0) return null;
        var monitors = new PhysicalMonitor[count];
        if (!GetPhysicalMonitorsFromHMONITOR(target, count, monitors)) return null;
        try { return use(monitors[0].hPhysicalMonitor); }
        finally { DestroyPhysicalMonitors(count, monitors); }
    }
}
