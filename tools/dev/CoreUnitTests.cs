// Core tests that never touch the running desktop: no window, hook, HTTP request or file of the user's.
//   powershell -NoProfile -File tools\dev\core-unit-tests.ps1
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading;

static class CoreUnitTests
{
    static int failures;

    static void Check(bool ok, string what)
    {
        if (ok) return;
        failures++;
        Console.WriteLine("FAIL " + what);
    }

    static int Main(string[] args)
    {
        string root = args.Length > 0 ? args[0] : ".";
        CallbackTests(root);
        PipeWaitTests();
        InputLatencyTests(root);
        FocusSinkTests(root);
        LogWriterTests();
        MemoryLogTests(root);
        TempsFileTests();
        Console.WriteLine(failures == 0 ? "PASS core unit tests" : failures + " failure(s)");
        return failures == 0 ? 0 : 1;
    }

    // The usage menu's temperatures come from the core without a process, and reading marks the demand.
    static void TempsFileTests()
    {
        string dir = Path.Combine(Path.GetTempPath(), "lunge-unit-temps-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        string keepPath = TempsFile.Path, keepWant = TempsFile.Want;
        TempsFile.Path = Path.Combine(dir, "lunge-temps.json");
        TempsFile.Want = Path.Combine(dir, "lunge-temps.want");
        try
        {
            File.WriteAllText(TempsFile.Path, "{\"running\":true,\"cpu\":41}");
            Check(TempsFile.Json() == "{\"running\":true,\"cpu\":41}", "a fresh temperature file was not passed on");
            Check(File.Exists(TempsFile.Want) && (DateTime.UtcNow - File.GetLastWriteTimeUtc(TempsFile.Want)).TotalSeconds < 5, "reading did not mark the demand");
            File.SetLastWriteTimeUtc(TempsFile.Path, DateTime.UtcNow.AddMinutes(-1));
            Check(TempsFile.Json() == "{\"running\":false}", "a stale temperature file was passed on as running");
            string text = File.ReadAllText(Path.Combine("core", "lunge.cs"));
            Check(text.Contains("verbless.StartsWith(\"/temps.json\")") && text.Contains("target == \"/temps.json\""), "the core does not serve /temps.json");
        }
        finally
        {
            TempsFile.Path = keepPath; TempsFile.Want = keepWant;
            try { Directory.Delete(dir, true); } catch { }
        }
    }

    // The black box writes every part's memory once an hour, also during fullscreen games (a leak shows as a growing figure).
    static void MemoryLogTests(string root)
    {
        string text = File.ReadAllText(Path.Combine(root, "core", "lunge.cs"));
        Check(text.Contains("const int MemoryEveryMs = 3600000;"), "the memory line is not hourly");
        int memory = text.IndexOf("Slider.Log(\"bellek (saatlik): \" + Parts());", StringComparison.Ordinal);
        int quiet = text.IndexOf("if (Quiet()) { slowProbes = 0; continue; }", StringComparison.Ordinal);
        Check(memory > 0 && quiet > memory, "the memory line is skipped while a fullscreen game runs");
        var parts = typeof(PerfGuard).GetMethod("Parts", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Check(parts != null && Regex.IsMatch((string)parts.Invoke(null, null) ?? "", @"dwm#\d+ özel \d+ MB"), "the parts line does not list the processes' memory");
    }

    // Logging never makes the caller wait for the disk; every line still arrives, once and in order.
    static void LogWriterTests()
    {
        string dir = Path.Combine(Path.GetTempPath(), "lunge-unit-log-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(dir);
        LogWriter.Path = Path.Combine(dir, "core.log");
        try
        {
            using (new FileStream(LogWriter.Path, FileMode.Create, FileAccess.ReadWrite, FileShare.None))
            {
                var sw = Stopwatch.StartNew();
                for (int i = 0; i < 200; i++) Slider.Log("tutulan dosya " + i);
                Check(sw.ElapsedMilliseconds < 100, "logging waited for a file someone holds: " + sw.ElapsedMilliseconds + " ms");
                Thread.Sleep(700);
            }
            var threads = Enumerable.Range(0, 4).Select(n => new Thread(() => { for (int i = 0; i < 250; i++) Slider.Log("iş " + n + " satır " + i); })).ToList();
            threads.ForEach(th => th.Start());
            threads.ForEach(th => th.Join());
            Check(LogWriter.Flush(5000), "the log did not reach the disk in time");
            string[] lines = File.ReadAllLines(LogWriter.Path);
            Check(Regex.IsMatch(lines.FirstOrDefault() ?? "", @"^---- \d{4}-\d{2}-\d{2} ----$"), "the log starts without its date line");
            var held = lines.Where(l => l.Contains(" tutulan dosya ")).Select(l => int.Parse(l.Substring(l.LastIndexOf(' ') + 1))).ToList();
            Check(held.SequenceEqual(Enumerable.Range(0, 200)), "lines written while the file was held were lost or reordered (" + held.Count + ")");
            for (int n = 0; n < 4; n++)
            {
                var mine = lines.Where(l => l.Contains(" iş " + n + " satır ")).Select(l => int.Parse(l.Substring(l.LastIndexOf(' ') + 1))).ToList();
                Check(mine.SequenceEqual(Enumerable.Range(0, 250)), "thread " + n + "'s lines were lost, doubled or reordered (" + mine.Count + ")");
            }
            Check(lines.All(l => l.StartsWith("---- ") || Regex.IsMatch(l, @"^\d\d:\d\d:\d\d\.\d{3} ")), "a line lacks its time");

            LogWriter.RotateBytes = 20000;
            for (int i = 0; i < 400; i++) Slider.Log("dönüş " + i + " " + new string('x', 80));
            LogWriter.Flush(5000);
            Slider.Log("dönüşten sonra");
            LogWriter.Flush(5000);
            Check(File.Exists(LogWriter.Path + ".old") && new FileInfo(LogWriter.Path).Length < 2 * LogWriter.RotateBytes, "the log did not turn over at its size");
        }
        finally
        {
            LogWriter.RotateBytes = 4L * 1024 * 1024;
            try { Directory.Delete(dir, true); } catch { }
        }
    }

    // The focus window lives in a process running as the user; that helper ends with the core it serves.
    static void FocusSinkTests(string root)
    {
        var sw = Stopwatch.StartNew();
        FocusSink.WaitForExit(int.MaxValue - 7, 50);
        Check(sw.ElapsedMilliseconds < 1000, "waiting for a process that does not exist did not return at once");

        var child = Process.Start(new ProcessStartInfo("cmd.exe", "/c ping -n 2 127.0.0.1 >nul") { CreateNoWindow = true, UseShellExecute = false });
        sw.Restart();
        var waiter = new Thread(() => FocusSink.WaitForExit(child.Id, 50));
        waiter.Start();
        Check(waiter.Join(10000), "the helper did not notice its core ending");
        Check(child.HasExited, "the helper stopped waiting before its core ended");

        string text = File.ReadAllText(Path.Combine(root, "core", "lunge.cs"));
        int mode = text.IndexOf("args[0] == \"--focus-sink\"", StringComparison.Ordinal);
        int mutex = text.IndexOf("new Mutex(true, \"LogicalLunge.Core\"", StringComparison.Ordinal);
        Check(mode > 0 && mutex > 0 && mode < mutex, "the --focus-sink helper would take the core's single-instance path");
        Check(Regex.IsMatch(text, @"public static void Start\(\)\s*\{\s*if \(UserLaunch\.Elevated\) \{ Current\(\); return; \}"),
            "an elevated core makes the focus window itself");
    }

    // A stalled input hook says what stopped it (garbage collection, paging), and both hooks are measured that way.
    static void InputLatencyTests(string root)
    {
        Check(InputLatency.Slow(InputLatency.Start()) == null, "a hook that returned at once was reported slow");
        var mark = InputLatency.Start();
        GC.Collect();
        Thread.Sleep((int)InputLatency.SlowMs + 50);
        string slow = InputLatency.Slow(mark);
        Check(slow != null && Regex.IsMatch(slow, @"^\d+ ms; çöp toplama 0/1/2 \+1/\+1/\+1, son ölçümden beri sayfa hatası \+\d+$"),
            "a slow hook's line does not carry its causes: " + slow);

        string text = File.ReadAllText(Path.Combine(root, "core", "lunge.cs"));
        var hooks = Regex.Matches(text, @"IntPtr Hook\(int nCode, IntPtr wParam, IntPtr lParam\)\s*\{(?<body>[^}]*)\}");
        Check(hooks.Count == 2, "expected the keyboard and the mouse hook, found " + hooks.Count);
        foreach (Match h in hooks)
            Check(h.Groups["body"].Value.Contains("InputLatency.Start()") && h.Groups["body"].Value.Contains("InputLatency.Slow(mark)"), "a hook is not measured");
        Check(Regex.Matches(text, @"InputLatency\.PrepareThread\(\);").Count == 2, "both hook threads take the input priority");
    }

    // The restart's wait for its successor: ending early (no successor in time, or its launch failed) must not leave a
    // wait behind that kills the process from an I/O thread once the pipe goes away (ObjectDisposedException).
    static void PipeWaitTests()
    {
        foreach (bool launches in new[] { true, false })
            for (int round = 0; round < 3; round++)
            {
                var pipe = new NamedPipeServerStream("lunge-unit-" + Guid.NewGuid().ToString("N"), PipeDirection.InOut, 1,
                    PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
                Check(!DesktopRestart.WaitCandidate(pipe, () => launches, () => true, 150), "a successor that never connected was accepted");
                pipe.Dispose();
            }
        Thread.Sleep(500);
        GC.Collect();
        GC.WaitForPendingFinalizers();
        Thread.Sleep(300);

        string name = "lunge-unit-" + Guid.NewGuid().ToString("N");
        using (var server = new NamedPipeServerStream(name, PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous))
        using (var client = new NamedPipeClientStream(".", name, PipeDirection.InOut, PipeOptions.Asynchronous))
        using (var said = new ManualResetEvent(false))
        {
            Func<bool> launch = () =>
            {
                ThreadPool.QueueUserWorkItem(_ =>
                {
                    try { client.Connect(2000); client.WriteByte((byte)'R'); client.Flush(); } catch (Exception) { }
                    said.Set();
                });
                return true;
            };
            Check(DesktopRestart.WaitCandidate(server, launch, () => true, 3000), "a successor that connected and said ready was refused");
            said.WaitOne(3000);
        }
    }

    // Nothing Windows calls directly lets an exception out (it killed the process without a trace).
    static void CallbackTests(string root)
    {
        var reported = new List<string>();
        Callback.Report = reported.Add;
        var key = Callback.Guard("test klavye", (Native.LowLevelKeyboardProc)((n, w, l) => { throw new InvalidOperationException("klavye"); }));
        var mouse = Callback.Guard("test fare", (Native.LowLevelMouseProc)((n, w, l) => { throw new NullReferenceException(); }));
        var ev = Callback.Guard("test olay", (Native.WinEventDelegate)((h, e, hw, o, c, t, tm) => { throw new ObjectDisposedException("olay"); }));
        bool escaped = false;
        try
        {
            for (int i = 0; i < 3; i++)
            {
                key(-1, IntPtr.Zero, IntPtr.Zero);
                mouse(-1, IntPtr.Zero, IntPtr.Zero);
                ev(IntPtr.Zero, 0, IntPtr.Zero, 0, 0, 0, 0);
            }
        }
        catch { escaped = true; }
        Check(!escaped, "an exception left a guarded callback");
        Check(reported.Count == 3, "each failing place is reported once a minute, not on every call (" + reported.Count + " reports)");
        Check(reported.All(r => r.StartsWith("geri çağrı hatası (test ") && r.Split('\n').Length > 1), "a report lacks its place or its stack trace");
        var pass = Callback.Guard("test geçiş", (Native.LowLevelKeyboardProc)((n, w, l) => (IntPtr)7));
        Check(pass(0, IntPtr.Zero, IntPtr.Zero) == (IntPtr)7, "a guarded hook changed the answer of a hook that did not fail");

        // The rule in the source: in every class, a hook or event delegate handed to Windows is assigned only through
        // Callback.Guard (a new unguarded one fails this test).
        var declared = new Regex(@"Native\.(LowLevelKeyboardProc|LowLevelMouseProc|WinEventDelegate)\s+(\w+)\s*;");
        var topLevel = new Regex(@"^(?:(?:public|internal|static|sealed|abstract|partial)\s+)*class\s+\w+", RegexOptions.Multiline);
        int fields = 0;
        foreach (string file in Directory.GetFiles(Path.Combine(root, "core"), "*.cs"))
        {
            string text = File.ReadAllText(file);
            var starts = topLevel.Matches(text).Cast<Match>().Select(m => m.Index).Concat(new[] { text.Length }).ToList();
            for (int i = 0; i + 1 < starts.Count; i++)
            {
                string cls = text.Substring(starts[i], starts[i + 1] - starts[i]);
                foreach (Match d in declared.Matches(cls))
                {
                    fields++;
                    string name = d.Groups[2].Value;
                    var assigned = new Regex(@"^\s*" + name + @"\s*=\s*(.+)$", RegexOptions.Multiline).Matches(cls);
                    Check(assigned.Count > 0, Path.GetFileName(file) + ": " + name + " is never assigned");
                    foreach (Match a in assigned)
                        Check(a.Groups[1].Value.StartsWith("Callback.Guard(") || a.Groups[1].Value.TrimEnd().TrimEnd(';') == "null",
                            Path.GetFileName(file) + ": " + name + " is handed to Windows without Callback.Guard: " + a.Value.Trim());
                }
            }
        }
        Check(fields >= 8, "the rule saw only " + fields + " hook/event fields; the source layout changed");
        Check(File.ReadAllText(Path.Combine(root, "core", "lunge.cs")).Contains("Callback.Failed(\"odak penceresi\""),
            "the focus window's procedure lets exceptions out");
    }
}
