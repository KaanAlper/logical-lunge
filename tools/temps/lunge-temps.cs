// Logical Lunge sıcaklık okuyucu (bar'daki kullanım menüsünün "Sıcaklık" sütunu).
//   ll-temps.exe          -> yönetici olarak (LL\Temps zamanlanmış görevi) arka planda çalışır; LibreHardwareMonitorLib ile
//                            CPU paket / GPU çekirdek / GPU sıcak nokta sıcaklığını 2 sn'de bir C:\Users\Public\ll-temps.json'a yazar
//   ll-temps.exe --read   -> yetkisiz: dosyayı stdout'a basar (Zebar bunu okur)
// CPU sıcaklığı için PawnIO sürücüsü gerekir (setup-temps.ps1 kurar); yoksa yalnızca GPU gelir.
using System;
using System.IO;
using System.Linq;
using System.Threading;
using System.Globalization;
using LibreHardwareMonitor.Hardware;

static class Program
{
    const string OUT = @"C:\Users\Public\ll-temps.json";

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
            try
            {
                var fi = new FileInfo(OUT);
                // 15 sn'den eski veri = servis çalışmıyor
                if (fi.Exists && (DateTime.UtcNow - fi.LastWriteTimeUtc).TotalSeconds < 15) { Console.Write(File.ReadAllText(OUT)); return 0; }
                Console.Write("{\"running\":false}");
                // Görev kurulmamışsa yetkisiz başlat (yalnızca GPU okunur). ShellExecute: Zebar'ın soketlerini devralmasın.
                bool free;
                using (var m = new Mutex(false, @"Global\ll-temps"))
                {
                    try { free = m.WaitOne(0); } catch (AbandonedMutexException) { free = true; }
                    if (free) m.ReleaseMutex();
                }
                if (free)
                    System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(
                        System.Reflection.Assembly.GetExecutingAssembly().Location) { UseShellExecute = true, WorkingDirectory = AppDomain.CurrentDomain.BaseDirectory });
            }
            catch { Console.Write("{\"running\":false}"); }
            return 0;
        }

        bool created;
        var mutex = new Mutex(true, @"Global\ll-temps", out created);
        if (!created) return 0;

        Thread.CurrentThread.Priority = ThreadPriority.BelowNormal;
        var pc = new Computer { IsCpuEnabled = true, IsGpuEnabled = true };
        pc.Open();
        var tmp = OUT + ".tmp";
        while (true)
        {
            try
            {
                foreach (var h in pc.Hardware) h.Update();
                var hw = pc.Hardware.ToArray();
                Func<IHardware, bool> isCpu = h => h.HardwareType == HardwareType.Cpu;
                Func<IHardware, bool> isGpu = h => h.HardwareType == HardwareType.GpuNvidia || h.HardwareType == HardwareType.GpuAmd || h.HardwareType == HardwareType.GpuIntel;
                // Harici GPU varsa onu seç
                var dGpu = hw.FirstOrDefault(h => h.HardwareType == HardwareType.GpuNvidia || h.HardwareType == HardwareType.GpuAmd);
                if (dGpu != null) isGpu = h => h == dGpu;

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
