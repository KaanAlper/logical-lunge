// Logical Lunge CI diagnostics: who makes a process's Windows thread pool grow.
//
// Reads a trace recorded with ll-threadpool.wprp (kernel thread/image events with the creating stack of every new
// thread, and Microsoft-Windows-ThreadPool's callback events with the stack of whoever queued the work) and writes,
// for the named process:
//   - the thread pool's workers: alive at the start, created, retired, the peak alive and alive at the end;
//   - every new thread grouped by its start routine and the stack that created it;
//   - every callback queued to the thread pool grouped by the callback and the stack that queued it, and how long the
//     callbacks ran (a worker busy in a callback is what makes the pool add another).
// summary.txt is short enough for a job log; details.txt has every group with its full stack.
//
//   ThreadPoolTrace <trace.etl> <process name> <symbol dirs, ';'-separated> <out dir>
using System.Text;
using Microsoft.Diagnostics.Symbols;
using Microsoft.Diagnostics.Tracing;
using Microsoft.Diagnostics.Tracing.Etlx;
using Microsoft.Diagnostics.Tracing.Parsers;

static class Program
{
    static readonly Guid ThreadPoolProvider = new Guid("c861d0e2-a2c1-4d36-9f9c-970bab943a12");

    sealed class NewThread { public int Pid, Tid; public double Start, End = double.NaN; public ulong Addr; public CallStackIndex Stack; public bool AtStart; }
    sealed class PoolEvent { public string Name; public int Pid, Tid; public double At; public ulong Callback, Pool; public CallStackIndex Stack; }

    static int Main(string[] args)
    {
        if (args.Length < 4) { Console.Error.WriteLine("usage: ThreadPoolTrace <trace.etl> <process name> <symbol dirs> <out dir>"); return 2; }
        string etl = args[0], name = args[1], outDir = args[3];
        Directory.CreateDirectory(outDir);
        // the converted trace is twice the .etl: next to it in the temp folder, not in the uploaded folder
        var etlx = TraceLog.CreateFromEventTraceLogFile(etl, Path.Combine(Path.GetTempPath(), Path.GetFileNameWithoutExtension(etl) + ".etlx"),
            new TraceLogOptions { ContinueOnError = true, ConversionLog = TextWriter.Null });
        using var log = new TraceLog(etlx);

        var procs = log.Processes.Where(p => string.Equals(p.Name, name, StringComparison.OrdinalIgnoreCase)).ToList();
        if (procs.Count == 0) { Console.WriteLine($"no process named {name} in the trace"); return 1; }
        var pids = new HashSet<int>(procs.Select(p => p.ProcessID));

        string cache = Path.Combine(Path.GetTempPath(), "ll-symbols");
        string symPath = string.Join(";", args[2].Split(';', StringSplitOptions.RemoveEmptyEntries).Append($"SRV*{cache}*https://msdl.microsoft.com/download/symbols"));
        var sym = new SymbolReader(TextWriter.Null, symPath) { SecurityCheck = _ => true };

        // ------------------------------------------------------------------ read the events
        var threads = new Dictionary<(int, int), NewThread>();
        var pool = new List<PoolEvent>();
        var payloadNames = new Dictionary<string, string>();
        var src = log.Events.GetSource();
        var registered = new RegisteredTraceEventParser(src);
        src.Kernel.ThreadDCStart += d => { if (pids.Contains(d.ProcessID)) threads[(d.ProcessID, d.ThreadID)] = new NewThread { Pid = d.ProcessID, Tid = d.ThreadID, Start = d.TimeStampRelativeMSec, Addr = d.Win32StartAddr, Stack = CallStackIndex.Invalid, AtStart = true }; };
        src.Kernel.ThreadStart += d => { if (pids.Contains(d.ProcessID)) threads[(d.ProcessID, d.ThreadID)] = new NewThread { Pid = d.ProcessID, Tid = d.ThreadID, Start = d.TimeStampRelativeMSec, Addr = d.Win32StartAddr, Stack = d.CallStackIndex() }; };
        src.Kernel.ThreadStop += d => { if (threads.TryGetValue((d.ProcessID, d.ThreadID), out var t)) t.End = d.TimeStampRelativeMSec; };
        registered.All += e =>
        {
            if (e.ProviderGuid != ThreadPoolProvider || !pids.Contains(e.ProcessID)) return;
            string ev = string.IsNullOrEmpty(e.EventName) ? "event " + (int)e.ID : e.EventName;
            if (!payloadNames.ContainsKey(ev)) payloadNames[ev] = string.Join(",", e.PayloadNames ?? Array.Empty<string>());
            pool.Add(new PoolEvent { Name = ev, Pid = e.ProcessID, Tid = e.ThreadID, At = e.TimeStampRelativeMSec,
                Callback = Field(e, "Callback", "CallbackFunction", "Function"), Pool = Field(e, "PoolId", "Pool"), Stack = e.CallStackIndex() });
        };
        src.Process();

        // ------------------------------------------------------------------ symbols
        // the modules any stack or address of the process lands in, looked up once
        var wanted = new HashSet<ModuleFileIndex>();
        void Want(CallStackIndex s) { for (; s != CallStackIndex.Invalid; s = log.CallStacks.Caller(s)) { var m = log.CodeAddresses.ModuleFileIndex(log.CallStacks.CodeAddressIndex(s)); if (m != ModuleFileIndex.Invalid) wanted.Add(m); } }
        foreach (var t in threads.Values) Want(t.Stack);
        foreach (var p in pool) Want(p.Stack);
        foreach (var m in wanted) { try { log.CodeAddresses.LookupSymbolsForModule(sym, log.ModuleFiles[m]); } catch { } }
        var raw = new Dictionary<ulong, string>();
        var opened = new Dictionary<string, NativeSymbolModule>();
        string At(int pid, ulong a)
        {
            if (a == 0) return "-";
            if (raw.TryGetValue(a, out var s)) return s;
            s = "0x" + a.ToString("x");
            var proc = procs.FirstOrDefault(p => p.ProcessID == pid);
            var lm = proc?.LoadedModules.FirstOrDefault(m => a >= m.ImageBase && a < m.ImageBase + (ulong)m.ModuleFile.ImageSize);
            if (lm != null)
            {
                var mf = lm.ModuleFile;
                s = mf.Name + "+0x" + (a - lm.ImageBase).ToString("x");
                try
                {
                    if (!opened.TryGetValue(mf.FilePath, out var nm))
                    {
                        string pdb = mf.PdbName != null ? sym.FindSymbolFilePath(mf.PdbName, mf.PdbSignature, mf.PdbAge, mf.FilePath) : null;
                        nm = pdb != null ? sym.OpenNativeSymbolFile(pdb) : null;
                        opened[mf.FilePath] = nm;
                    }
                    string fn = nm?.FindNameForRva((uint)(a - lm.ImageBase));
                    if (!string.IsNullOrEmpty(fn)) s = mf.Name + "!" + fn;
                }
                catch { }
            }
            return raw[a] = s;
        }
        List<string> Frames(CallStackIndex s)
        {
            var r = new List<string>();
            for (; s != CallStackIndex.Invalid && r.Count < 60; s = log.CallStacks.Caller(s))
            {
                var ca = log.CallStacks.CodeAddressIndex(s);
                var mf = log.CodeAddresses.ModuleFile(ca);
                var mi = log.CodeAddresses.MethodIndex(ca);
                string fn = mi != MethodIndex.Invalid ? log.CodeAddresses.Methods.FullMethodName(mi) : null;
                r.Add((mf?.Name ?? "?") + "!" + (string.IsNullOrEmpty(fn) ? "0x" + log.CodeAddresses.Address(ca).ToString("x") : fn));
            }
            return r;
        }
        // ------------------------------------------------------------------ report
        var summary = new StringBuilder();
        var details = new StringBuilder();
        double end = log.SessionEndTimeRelativeMSec;
        summary.AppendLine($"trace {etl}: {end / 1000:0.0} s; process {name}: pid {string.Join(", ", pids)}");

        bool IsWorker(NewThread t) => At(t.Pid, t.Addr).Contains("TppWorkerThread", StringComparison.OrdinalIgnoreCase);
        var workers = threads.Values.Where(IsWorker).ToList();
        var steps = new List<(double At, int D)>();
        foreach (var w in workers) { steps.Add((w.AtStart ? 0 : w.Start, 1)); if (!double.IsNaN(w.End)) steps.Add((w.End, -1)); }
        int alive = 0, peak = 0; double peakAt = 0;
        foreach (var s in steps.OrderBy(x => x.At).ThenBy(x => -x.D)) { alive += s.D; if (alive > peak) { peak = alive; peakAt = s.At; } }
        summary.AppendLine($"thread pool workers: {workers.Count(w => w.AtStart)} at the start, {workers.Count(w => !w.AtStart)} created, {workers.Count(w => !double.IsNaN(w.End))} retired, peak {peak} at {peakAt / 1000:0.0} s, {alive} alive at the end");

        var created = threads.Values.Where(t => !t.AtStart).ToList();
        var lived = created.Where(t => !double.IsNaN(t.End)).Select(t => t.End - t.Start).OrderBy(x => x).ToList();
        summary.AppendLine($"new threads: {created.Count} ({created.Count / Math.Max(end / 1000, 1):0.0}/s), {lived.Count} ended in the trace" +
            (lived.Count > 0 ? $" after a median {lived[lived.Count / 2]:0} ms" : "") + "; by start routine:");
        foreach (var g in created.GroupBy(t => At(t.Pid, t.Addr)).OrderByDescending(g => g.Count()))
            summary.AppendLine($"  {g.Count(),5}  {g.Key}");
        Group(details, "new threads by creating stack", created.Select(t => (Key: At(t.Pid, t.Addr), Stack: Frames(t.Stack))), summary, 4);

        summary.AppendLine("thread pool events: " + string.Join(", ", pool.GroupBy(p => p.Name).OrderByDescending(g => g.Count()).Select(g => $"{g.Key} {g.Count()}")));
        foreach (var kv in payloadNames) details.AppendLine($"payload of {kv.Key}: {kv.Value}");
        var queued = pool.Where(p => IsAny(p.Name, "CBEnqueue", "CallbackEnqueue")).ToList();
        Group(details, "callbacks queued, by callback and queuing stack", queued.Select(p => (Key: At(p.Pid, p.Callback), Stack: Frames(p.Stack))), summary, 8);

        // how long each callback ran on its worker (start .. stop on the same thread)
        var open = new Dictionary<(int, int), PoolEvent>();
        var ran = new Dictionary<string, (int N, double Ms, double Max)>();
        foreach (var p in pool.OrderBy(p => p.At))
        {
            // ThreadPool/CBStart and CBStop (CallbackStart/Stop on some builds)
            if (IsAny(p.Name, "CBStart", "CallbackStart")) open[(p.Pid, p.Tid)] = p;
            else if (IsAny(p.Name, "CBStop", "CallbackStop") && open.Remove((p.Pid, p.Tid), out var st))
            {
                string k = At(st.Pid, st.Callback); double ms = p.At - st.At;
                ran.TryGetValue(k, out var v); ran[k] = (v.N + 1, v.Ms + ms, Math.Max(v.Max, ms));
            }
        }
        summary.AppendLine("callbacks run (count, total ms, longest ms):");
        foreach (var kv in ran.OrderByDescending(kv => kv.Value.Ms).Take(15))
            summary.AppendLine($"  {kv.Value.N,5} {kv.Value.Ms,9:0.0} {kv.Value.Max,8:0.0}  {kv.Key}");
        var pools = pool.Where(p => p.Name.Contains("PoolCreate", StringComparison.OrdinalIgnoreCase)).ToList();
        if (pools.Count > 0) Group(details, "pools created", pools.Select(p => (Key: "pool", Stack: Frames(p.Stack))), summary, 3);

        File.WriteAllText(Path.Combine(outDir, "summary.txt"), summary.ToString());
        File.WriteAllText(Path.Combine(outDir, "details.txt"), summary + "\n" + details);
        Console.Write(summary);
        return 0;
    }

    // groups by (key, the first frames past the plumbing): the summary gets the top groups with a few frames each,
    // the details every group with its full stack
    static void Group(StringBuilder details, string title, IEnumerable<(string Key, List<string> Stack)> items, StringBuilder summary, int top)
    {
        var groups = items.Select(i => (i.Key, i.Stack, Who: WhoOf(i.Stack)))
            .GroupBy(i => i.Key + " <- " + string.Join(" <- ", i.Who.Take(12)))
            .OrderByDescending(g => g.Count()).ToList();
        summary.AppendLine($"{title}: {groups.Sum(g => g.Count())} in {groups.Count} groups");
        details.AppendLine($"=== {title}");
        int n = 0;
        foreach (var g in groups)
        {
            var first = g.First();
            details.AppendLine($"--- {g.Count()} x {first.Key}");
            foreach (var f in first.Stack) details.AppendLine("      " + f);
            if (n++ < top)
            {
                summary.AppendLine($"  {g.Count(),5}  {first.Key}");
                foreach (var f in first.Who.Take(10)) summary.AppendLine("           " + f);
            }
        }
    }

    // the frames that say who: past the thread pool's, the system calls' and the kernel's own frames at the top
    static List<string> WhoOf(List<string> fs)
    {
        int i = 0;
        while (i < fs.Count - 1 && (fs[i].StartsWith("ntoskrnl", StringComparison.OrdinalIgnoreCase) || fs[i].StartsWith("?!") ||
               fs[i].StartsWith("ntdll!Tp", StringComparison.OrdinalIgnoreCase) || fs[i].StartsWith("ntdll!Nt", StringComparison.OrdinalIgnoreCase) ||
               fs[i].StartsWith("ntdll!Zw", StringComparison.OrdinalIgnoreCase) || fs[i].StartsWith("ntdll!Etw", StringComparison.OrdinalIgnoreCase)))
            i++;
        return fs.Skip(i).ToList();
    }

    static bool IsAny(string name, params string[] parts) => parts.Any(x => name.Contains(x, StringComparison.OrdinalIgnoreCase));

    static ulong Field(TraceEvent e, params string[] names)
    {
        var all = e.PayloadNames ?? Array.Empty<string>();
        foreach (var want in names)
        {
            int i = Array.FindIndex(all, n => string.Equals(n, want, StringComparison.OrdinalIgnoreCase));
            if (i < 0) continue;
            try
            {
                object v = e.PayloadValue(i);
                return v switch { ulong u => u, long l => (ulong)l, uint ui => ui, int ii => (ulong)ii, string s when s.StartsWith("0x") => Convert.ToUInt64(s, 16), _ => Convert.ToUInt64(v) };
            }
            catch { return 0; }
        }
        return 0;
    }
}
