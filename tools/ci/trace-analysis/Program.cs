// Stress test ETW analysis: where a long frame of the core's slide went. For every long wait of a lunge.exe thread in
// DwmFlush (or any wait that dwm.exe ended), it prints how long the thread waited, how long it then sat ready without
// a CPU, who woke it, and what the machine and dwm.exe did meanwhile (CPU per process, dwm threads' own waits and how
// long they sat ready). Usage: TraceAnalysis <trace.etl> [min wait ms] [symbol minutes]
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using Microsoft.Windows.EventTracing;
using Microsoft.Windows.EventTracing.Cpu;
using Microsoft.Windows.EventTracing.Processes;
using Microsoft.Windows.EventTracing.Symbols;

static class Program
{
    static string Frames(IStackSnapshot s, int max)
    {
        if (s == null || s.Frames == null) return "(yığın yok)";
        var parts = new List<string>();
        foreach (var f in s.Frames)
        {
            string img = f.Image?.FileName ?? "?";
            // the kernel part (context switch, ETW stack walk) is the same for every wait; the user part says what was waited on
            if (img.Equals("ntoskrnl.exe", StringComparison.OrdinalIgnoreCase) || img.Equals("hal.dll", StringComparison.OrdinalIgnoreCase)) continue;
            string fn = f.Symbol?.FunctionName;
            string p = fn != null ? img + "!" + fn : img;
            if (parts.Count == 0 || parts[parts.Count - 1] != p) parts.Add(p);
            if (parts.Count >= max) break;
        }
        return string.Join(" < ", parts);
    }

    static string Name(IProcess p) { return p == null ? "?" : (p.ImageName ?? "?") + "#" + p.Id; }
    static double Ms(TimeSpan t) { return t.TotalMilliseconds; }
    static double D(TraceDuration? d) { return d.HasValue ? (double)d.Value.Duration.TotalMilliseconds : 0; }
    static double T(TraceTimestamp ts) { return (double)ts.RelativeTimestamp.TotalMilliseconds; }

    static int Main(string[] args)
    {
        string etl = args[0];
        double minWait = args.Length > 1 ? double.Parse(args[1]) : 100;
        int symMinutes = args.Length > 2 ? int.Parse(args[2]) : 6;
        using var t = TraceProcessor.Create(etl);
        var sched = t.UseCpuSchedulingData();
        var symbols = t.UseSymbols();
        t.Process();
        try
        {
            var load = symbols.Result.LoadSymbolsAsync(SymCachePath.Automatic, SymbolPath.Automatic);
            if (!load.Wait(TimeSpan.FromMinutes(symMinutes))) Console.WriteLine("(semboller " + symMinutes + " dakikada yüklenemedi; görüntü adlarıyla devam)");
        }
        catch (Exception e) { Console.WriteLine("(semboller yüklenemedi: " + e.Message + ")"); }

        var acts = sched.Result.ThreadActivity;
        var slices = sched.Result.CpuTimeSlices;
        var ours = new List<ICpuThreadActivity>();
        foreach (var a in acts)
        {
            var p = a.SwitchIn.Process;
            if (p == null || !string.Equals(p.ImageName, "lunge.exe", StringComparison.OrdinalIgnoreCase)) continue;
            double wait = D(a.WaitingDuration);
            double ready = D(a.ReadyDuration);
            if (wait + ready < minWait) continue;
            string stack = Frames(a.SwitchIn.Stack, 60);
            // waits inside dwmapi (DwmFlush, thumbnail calls) or in win32u calls that DWM answers; a timer wait readied from a DPC
            // that happened to interrupt dwm.exe is not one of them
            bool dwmWait = stack.IndexOf("dwmapi", StringComparison.OrdinalIgnoreCase) >= 0
                || stack.IndexOf("NtDwm", StringComparison.OrdinalIgnoreCase) >= 0 || stack.IndexOf("dcomp", StringComparison.OrdinalIgnoreCase) >= 0;
            if (dwmWait) ours.Add(a);
        }
        Console.WriteLine("lunge.exe: DWM'i bekleyen " + ours.Count + " uzun bekleme (>= " + minWait + " ms)");
        foreach (var a in ours.OrderByDescending(x => D(x.WaitingDuration) + D(x.ReadyDuration)).Take(8))
        {
            var sw = a.SwitchIn.ContextSwitch.Timestamp;
            double wait = D(a.WaitingDuration);
            double ready = D(a.ReadyDuration);
            double end = T(sw);
            double start = end - (wait + ready);
            var sb = new StringBuilder();
            sb.AppendLine();
            sb.AppendFormat("== {0:HH:mm:ss.fff} {1} thread {2}: bekledi {3:0.0} ms, sonra hazır ama işlemcisiz {4:0.0} ms; uyandıran {5} thread {6}; öncelik {7}; bekleme nedeni {8}",
                sw.DateTimeOffset, Name(a.SwitchIn.Process), a.SwitchIn.ThreadId, wait, ready, Name(a.ReadyingProcess), a.ReadyingThread?.Id,
                a.SwitchIn.Priority, a.PreviousActivity?.SwitchOut.WaitReason);
            sb.AppendLine();
            sb.AppendLine("   bekleme yığını: " + Frames(a.SwitchIn.Stack, 14));
            if (a.ReadyThreadStack != null) sb.AppendLine("   uyandıran yığın: " + Frames(a.ReadyThreadStack, 14));
            // CPU per process in the interval
            var cpu = new Dictionary<string, double>();
            foreach (var s in slices)
            {
                double s0 = T(s.StartTime), s1 = T(s.StopTime);
                if (s1 <= start || s0 >= end) continue;
                double o = Math.Min(s1, end) - Math.Max(s0, start);
                string key = s.Process?.ImageName ?? "Idle";
                cpu[key] = (cpu.TryGetValue(key, out var v) ? v : 0) + o;
            }
            sb.AppendLine("   aralıkta işlemci (ms): " + string.Join(" · ", cpu.OrderByDescending(kv => kv.Value).Take(8).Select(kv => kv.Key + " " + kv.Value.ToString("0"))));
            // dwm.exe threads: their own waits and ready (starved) time inside the interval
            var dwm = acts.Where(x => string.Equals(x.SwitchIn.Process?.ImageName, "dwm.exe", StringComparison.OrdinalIgnoreCase)
                && T(x.SwitchIn.ContextSwitch.Timestamp) > start && T(x.SwitchIn.ContextSwitch.Timestamp) <= end + 50).ToList();
            double readySum = 0, readyMax = 0;
            foreach (var x in dwm) { double r = D(x.ReadyDuration); readySum += r; readyMax = Math.Max(readyMax, r); }
            sb.AppendFormat("   dwm.exe: {0} çalışmaya geçiş, hazır-bekleme toplam {1:0.0} ms, en uzun {2:0.0} ms", dwm.Count, readySum, readyMax);
            sb.AppendLine();
            foreach (var x in dwm.OrderByDescending(x => D(x.WaitingDuration) + D(x.ReadyDuration)).Take(4))
            {
                double w2 = D(x.WaitingDuration), r2 = D(x.ReadyDuration);
                sb.AppendFormat("   dwm thread {0}: bekledi {1:0.0} ms + hazır {2:0.0} ms, öncelik {3}, uyandıran {4}; yığın: {5}",
                    x.SwitchIn.ThreadId, w2, r2, x.SwitchIn.Priority, Name(x.ReadyingProcess), Frames(x.SwitchIn.Stack, 10));
                sb.AppendLine();
            }
            Console.Write(sb.ToString());
        }
        // whole trace: the longest ready (runnable but not running) delays per process, the starvation picture
        Console.WriteLine();
        Console.WriteLine("en uzun hazır bekleyişler (işlemci sırası), süreç başına:");
        foreach (var g in acts.Where(a => a.ReadyDuration.HasValue && a.SwitchIn.Process != null)
                     .GroupBy(a => a.SwitchIn.Process.ImageName)
                     .Select(g => new { g.Key, Max = g.Max(a => D(a.ReadyDuration)), Over50 = g.Count(a => D(a.ReadyDuration) > 50) })
                     .OrderByDescending(x => x.Max).Take(12))
            Console.WriteLine("   " + g.Key + ": en uzun " + g.Max.ToString("0.0") + " ms, 50 ms'den uzun " + g.Over50 + " kez");
        return 0;
    }
}
