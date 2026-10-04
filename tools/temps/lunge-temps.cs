// Logical Lunge sıcaklık okuyucu (bar'daki kullanım menüsünün "Sıcaklık" sütunu).
//   lunge-temps.exe       -> yönetici olarak (LogicalLunge\Temps zamanlanmış görevi) arka planda çalışır; LibreHardwareMonitorLib ile
//                            CPU paket / GPU çekirdek / GPU sıcak nokta sıcaklığını 2 sn'de bir C:\Users\Public\lunge-temps.json'a yazar
//   lunge-temps.exe --read -> yetkisiz: dosyayı stdout'a basar (kabuktaki bar bunu okur)
// Okuyan her şey lunge-temps.want dosyasının zamanını günceller; servis sensörleri yalnızca son 10 sn'de biri okumak
// istediyse okur (eskiden 7/24, kimse bakmazken de her 2 sn'de bütün sensörleri sürücü üzerinden güncelliyordu).
// CPU sıcaklığı için PawnIO sürücüsü gerekir (setup-temps.ps1 kurar); yoksa yalnızca GPU gelir.
using System;
using System.IO;
using System.Linq;
using System.Threading;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using LibreHardwareMonitor.Hardware;

static class Program
{
    const string OUT = @"C:\Users\Public\lunge-temps.json";
    const string WANT = @"C:\Users\Public\lunge-temps.want";
    // --read servisi en fazla dakikada bir başlatır: başlayamayan bir servis (eksik dosya) her okumada yeniden açılıp
    // çöküyordu (bir dakikada 27 kez)
    const string STARTED = @"C:\Users\Public\lunge-temps.start";
    const int WANT_FOR_SECONDS = 10, START_EVERY_SECONDS = 60;

    // Okuyanın işareti (C:\Users\Public'te oturumdaki kullanıcı her dosyayı değiştirebilir)
    static void Touch(string path)
    {
        try
        {
            if (!File.Exists(path)) File.WriteAllText(path, "");
            File.SetLastWriteTimeUtc(path, DateTime.UtcNow);
        }
        catch { }
    }

    static bool Within(string path, int seconds)
    {
        try { var fi = new FileInfo(path); return fi.Exists && (DateTime.UtcNow - fi.LastWriteTimeUtc).TotalSeconds < seconds; }
        catch { return false; }
    }
    [StructLayout(LayoutKind.Sequential)]
    struct PowerStatus { public byte ACLineStatus, BatteryFlag, BatteryLifePercent, SystemStatusFlag; public uint BatteryLifeTime, BatteryFullLifeTime; }
    [DllImport("kernel32.dll")] static extern bool GetSystemPowerStatus(out PowerStatus status);
    // Opening GPU sensors can itself wake a discrete GPU. Disable the GPU
    // subsystem while on battery, rather than returning old NVIDIA samples,
    // and while a discrete GPU sleeps (hybrid laptops): asking its
    // temperature woke it, and it ran warm and loud while the machine idled.
    static bool ReadGpu() { PowerStatus status; return GetSystemPowerStatus(out status) && status.ACLineStatus == 1 && !DiscreteGpuAsleep(); }

    [DllImport("setupapi.dll", SetLastError = true)] static extern IntPtr SetupDiGetClassDevs(ref Guid cls, IntPtr enumerator, IntPtr parent, uint flags);
    [DllImport("setupapi.dll", SetLastError = true)] static extern bool SetupDiEnumDeviceInfo(IntPtr set, uint index, ref DevInfo data);
    [DllImport("setupapi.dll", SetLastError = true)] static extern bool SetupDiGetDeviceRegistryProperty(IntPtr set, ref DevInfo data, uint property, out uint type, byte[] buffer, uint size, out uint required);
    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool SetupDiGetDeviceInstanceId(IntPtr set, ref DevInfo data, System.Text.StringBuilder id, int size, out int required);
    [DllImport("setupapi.dll")] static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);
    [StructLayout(LayoutKind.Sequential)] struct DevInfo { public uint cbSize; public Guid ClassGuid; public uint DevInst; public IntPtr Reserved; }
    static Guid DisplayClass = new Guid("4d36e968-e325-11ce-bfc1-08002be10318");

    // A discrete GPU (NVIDIA or AMD on PCI) whose current device power state is D1-D3: the state Windows keeps for it
    // (Device Manager > Power data), read without touching the card. Virtual display adapters don't count.
    static bool DiscreteGpuAsleep()
    {
        IntPtr set = SetupDiGetClassDevs(ref DisplayClass, IntPtr.Zero, IntPtr.Zero, 0x2 /*DIGCF_PRESENT*/);
        if (set == new IntPtr(-1)) return false;
        try
        {
            var data = new DevInfo { cbSize = (uint)Marshal.SizeOf(typeof(DevInfo)) };
            var power = new byte[64];
            var id = new System.Text.StringBuilder(512);
            for (uint i = 0; SetupDiEnumDeviceInfo(set, i, ref data); i++)
            {
                int idLength;
                if (!SetupDiGetDeviceInstanceId(set, ref data, id, id.Capacity, out idLength)) continue;
                string instance = id.ToString().ToUpperInvariant();
                if (!instance.StartsWith(@"PCI\VEN_10DE") && !instance.StartsWith(@"PCI\VEN_1002")) continue;
                uint type, need;
                // CM_POWER_DATA: PD_Size, then PD_MostRecentPowerState (1 = D0 ... 4 = D3)
                if (!SetupDiGetDeviceRegistryProperty(set, ref data, 0x1E /*SPDRP_DEVICE_POWER_DATA*/, out type, power, (uint)power.Length, out need) || need < 8) continue;
                int state = BitConverter.ToInt32(power, 4);
                if (state >= 2 && state <= 4) return true;
            }
        }
        finally { SetupDiDestroyDeviceInfoList(set); }
        return false;
    }

    static string Num(float? v) { return v.HasValue ? Math.Round(v.Value).ToString(CultureInfo.InvariantCulture) : "null"; }

    static float? Find(IHardware[] hw, Func<IHardware, bool> hwPick, SensorType type, params string[] names)
    {
        foreach (var h in hw.Where(hwPick))
        {
            var all = h.Sensors.Concat(h.SubHardware.SelectMany(s => s.Sensors)).Where(s => s.SensorType == type && s.Value.HasValue).ToList();
            foreach (var n in names)
            {
                var s = all.FirstOrDefault(x => x.Name == n);
                if (s != null) return s.Value;
            }
        }
        return null;
    }

    [STAThread]
    static int Main(string[] args)
    {
        if (args.Length > 0 && args[0] == "--read")
        {
            Touch(WANT);
            try
            {
                var fi = new FileInfo(OUT);
                // 15 sn'den eski veri = servis çalışmıyor
                if (fi.Exists && (DateTime.UtcNow - fi.LastWriteTimeUtc).TotalSeconds < 15) { Console.Write(File.ReadAllText(OUT)); return 0; }
                Console.Write("{\"running\":false}");
                // Görev kurulmamışsa yetkisiz başlat (yalnızca GPU okunur). ShellExecute: kabuğun soketlerini devralmasın.
                bool free;
                using (var m = new Mutex(false, @"Global\lunge-temps"))
                {
                    try { free = m.WaitOne(0); } catch (AbandonedMutexException) { free = true; }
                    if (free) m.ReleaseMutex();
                }
                if (free && !Within(STARTED, START_EVERY_SECONDS))
                {
                    Touch(STARTED);
                    System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(
                        System.Reflection.Assembly.GetExecutingAssembly().Location) { UseShellExecute = true, WorkingDirectory = AppDomain.CurrentDomain.BaseDirectory });
                }
            }
            catch { Console.Write("{\"running\":false}"); }
            return 0;
        }

        bool created;
        var mutex = new Mutex(true, @"Global\lunge-temps", out created);
        if (!created) return 0;

        Thread.CurrentThread.Priority = ThreadPriority.BelowNormal;
        // Sensör kütüphanesi Serve'de: eksikse (bozuk kurulum) burada yakalanır, süreç çökmeden çıkar
        try { Serve(); }
        catch (Exception ex)
        {
            if (!(ex is FileNotFoundException || ex is FileLoadException || ex is BadImageFormatException || ex is TypeLoadException)) throw;
            try { File.WriteAllText(OUT, "{\"running\":false,\"error\":\"" + ex.GetType().Name + "\"}"); } catch { }
            return 1;
        }
        GC.KeepAlive(mutex);
        return 0;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static void Serve()
    {
        var pc = new Computer { IsCpuEnabled = true, IsGpuEnabled = ReadGpu() };
        pc.Open();
        var tmp = OUT + ".tmp";
        while (true)
        {
            // Kimse okumuyor: sensörlere dokunma, yalnızca isteğe bak
            if (!Within(WANT, WANT_FOR_SECONDS)) { Thread.Sleep(2000); continue; }
            try
            {
                pc.IsGpuEnabled = ReadGpu();
                foreach (var h in pc.Hardware) h.Update();
                var hw = pc.Hardware.ToArray();
                Func<IHardware, bool> isCpu = h => h.HardwareType == HardwareType.Cpu;
                Func<IHardware, bool> isGpu = h => h.HardwareType == HardwareType.GpuNvidia || h.HardwareType == HardwareType.GpuAmd || h.HardwareType == HardwareType.GpuIntel;

                var cpu = Find(hw, isCpu, SensorType.Temperature, "CPU Package", "Core (Tctl/Tdie)", "Core Max", "Core Average");
                var gpu = Find(hw, isGpu, SensorType.Temperature, "GPU Core");
                var hot = Find(hw, isGpu, SensorType.Temperature, "GPU Hot Spot");
                var gpuLoad = Find(hw, isGpu, SensorType.Load, "GPU Core");
                var cpuName = hw.Where(isCpu).Select(h => h.Name).FirstOrDefault() ?? "";
                var gpuName = hw.Where(isGpu).Select(h => h.Name).FirstOrDefault() ?? "";

                var js = "{\"running\":true,\"cpu\":" + Num(cpu) + ",\"gpu\":" + Num(gpu) + ",\"gpuHot\":" + Num(hot) +
                         ",\"gpuLoad\":" + Num(gpuLoad) + ",\"cpuName\":\"" + cpuName.Replace("\"", "") + "\",\"gpuName\":\"" + gpuName.Replace("\"", "") + "\"}";
                File.WriteAllText(tmp, js);
                if (File.Exists(OUT)) File.Replace(tmp, OUT, null); else File.Move(tmp, OUT);
            }
            catch { }
            Thread.Sleep(2000);
        }
    }
}
