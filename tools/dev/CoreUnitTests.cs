// Core tests that never touch the running desktop: no window, hook, HTTP request or file of the user's.
//   powershell -NoProfile -File tools\dev\core-unit-tests.ps1
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;

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
        Console.WriteLine(failures == 0 ? "PASS core unit tests" : failures + " failure(s)");
        return failures == 0 ? 0 : 1;
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
