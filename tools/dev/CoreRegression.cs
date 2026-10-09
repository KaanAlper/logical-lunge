// Compiled with core/lunge.cs and /main:CoreRegression. Never starts the desktop.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Web.Script.Serialization;

static class CoreRegression
{
    static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }
    static object Call(Type type, string name, params object[] args) {
        return type.GetMethod(name, BindingFlags.NonPublic | BindingFlags.Static).Invoke(null, args);
    }
    static string Request(string method, string target, string origin, string host = "localhost") {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        try {
            using (var client = new TcpClient()) {
                client.Connect((IPEndPoint)listener.LocalEndpoint);
                using (var accepted = listener.AcceptTcpClient()) {
                    var stream = client.GetStream(); stream.ReadTimeout = 1000;
                    var bytes = Encoding.ASCII.GetBytes(method + " " + target + " HTTP/1.1\r\nHost: " + host + "\r\n" + (origin == null ? "" : "Origin: " + origin + "\r\n") + "Content-Length: 0\r\n\r\n");
                    stream.Write(bytes, 0, bytes.Length);
                    Call(typeof(Toasts), "Accept", accepted);
                    var buffer = new byte[4096];
                    int n = stream.Read(buffer, 0, buffer.Length);
                    return Encoding.UTF8.GetString(buffer, 0, n);
                }
            }
        } finally { listener.Stop(); }
    }
    // Windows kabuğunun devri: asıl değerler kaydedilir, ikinci uygulama onları ezmez, geri yükleme olmayanı siler
    static void TakeoverTests() {
        var reg = new Dictionary<string, object> {
            { @"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\SnapAssist", 1 },
            { ShellTakeover.ArrangeKey + "\\" + ShellTakeover.ArrangeName, "1" },
        };
        Func<string, string, object> read = (k, n) => { object v; return reg.TryGetValue(k + "\\" + n, out v) ? v : null; };
        List<Dictionary<string, object>> saved; int autoHide;
        string first = ShellTakeover.Originals(null, read, out saved, out autoHide);
        Dictionary<string, object> snap = null, da = null, arrange = null;
        foreach (var e in saved) {
            if ((string)e["n"] == "SnapAssist") snap = e;
            if ((string)e["n"] == "TaskbarDa") da = e;
            if ((string)e["n"] == ShellTakeover.ArrangeName) arrange = e;
        }
        Check(snap != null && (int)ShellTakeover.RestoreValue(snap) == 1, "Takeover did not keep an existing value to restore");
        Check(da != null && ShellTakeover.RestoreValue(da) == null, "Takeover must delete a value that did not exist before");
        Check(arrange != null && (string)ShellTakeover.RestoreValue(arrange) == "1", "Aero Snap's original string was not kept");
        Check(autoHide == -1, "Auto-hide must stay unknown until the taskbar is seen");
        // a crashed run left its record: the values it changed are not the originals
        reg[@"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\SnapAssist"] = 0;
        string again = ShellTakeover.Originals(first, read, out saved, out autoHide);
        Check(again == first, "A second apply overwrote the saved originals");
        foreach (var e in saved) if ((string)e["n"] == "SnapAssist") Check((int)ShellTakeover.RestoreValue(e) == 1, "Original value lost after a crash");
        Check(!ShellTakeover.TryParse("{\"reg\":[{\"k\":1}]}", out saved, out autoHide), "A broken record was accepted");
        string withAh = ShellTakeover.Serialize(new List<Dictionary<string, object>>(), 3);
        Check(ShellTakeover.TryParse(withAh, out saved, out autoHide) && autoHide == 3, "Auto-hide state did not survive the record");
    }

    static void FullscreenLayerTests() {
        // the workspace slide's layer covers the bar when a window covers its monitor (a game, a video)
        Check(Slider.CoversMonitor(new Native.RECT { Left = 0, Top = 0, Right = 1920, Bottom = 1080 }, 0, 0, 1920, 1080), "A monitor-sized window was no fullscreen");
        Check(Slider.CoversMonitor(new Native.RECT { Left = -1, Top = -1, Right = 1921, Bottom = 1079 }, 0, 0, 1920, 1080), "A pixel off made a fullscreen window a normal one");
        Check(!Slider.CoversMonitor(new Native.RECT { Left = 5, Top = 45, Right = 1915, Bottom = 1075 }, 0, 0, 1920, 1080), "A maximized window under the bar counted as fullscreen");
        Check(!Slider.CoversMonitor(new Native.RECT { Left = 1920, Top = 0, Right = 3840, Bottom = 1080 }, 0, 0, 1920, 1080), "A window on the next monitor counted as fullscreen here");
        // state changes animate through a freeze; other commands do not
        Check(Dwindle.ChangesState(new[] { "toggle-fullscreen" }) && Dwindle.ChangesState(new[] { "toggle-fullscreen --maximized" })
            && Dwindle.ChangesState(new[] { "toggle-floating --centered" }) && Dwindle.ChangesState(new[] { "toggle-fullscreen-spoof" }), "A state toggle was not animated");
        Check(!Dwindle.ChangesState(new[] { "focus --workspace 2" }) && !Dwindle.ChangesState(new[] { "toggle-tiling-direction" }), "A non-state command went through the state animation");
        // the window manager's own workspace keys slide like ours
        int sd; string st;
        Check(Keys2.SwitchesWorkspace(new[] { "focus --next-workspace" }, out sd, out st) && sd == 1 && st == null, "Super+PageDown did not slide right");
        Check(Keys2.SwitchesWorkspace(new[] { "focus --prev-active-workspace" }, out sd, out st) && sd == -1, "Super+Ctrl+Alt+Left did not slide left");
        Check(Keys2.SwitchesWorkspace(new[] { "move --next-workspace", "focus --next-workspace" }, out sd, out st) && sd == 1, "Super+Shift+PageDown did not carry the window along");
        Check(Keys2.SwitchesWorkspace(new[] { "focus --workspace 4" }, out sd, out st) && sd == 0 && st == "4", "A named workspace lost its name");
        Check(!Keys2.SwitchesWorkspace(new[] { "move --workspace 3" }, out sd, out st) && !Keys2.SwitchesWorkspace(new[] { "toggle-fullscreen" }, out sd, out st)
            && !Keys2.SwitchesWorkspace(new[] { "focus --direction left" }, out sd, out st), "A command that switches no workspace went through the slide");
    }

    static void UiScaleTests() {
        Check(UiScale.Valid(100) && UiScale.Valid(125) && !UiScale.Valid(101) && !UiScale.Valid(0), "uiScale steps were misjudged");
        Check(UiScale.TopGap(100) == 45 && UiScale.TopGap(125) == 55 && UiScale.TopGap(85) == 39, "Top gap must be the bar's height plus 5");
        string yaml = "gaps:\n  scale_with_dpi: true\n  inner_gap: '8px'\n  outer_gap:\n    top: '45px'\n    right: '5px'\n";
        string next = UiScale.WithTopGap(yaml, 55);
        Check(next.Contains("    top: '55px'") && next.Contains("right: '5px'") && next.Contains("inner_gap: '8px'"), "outer_gap top was not rewritten alone: " + next);
        string reordered = "  outer_gap:\r\n    left: '5px'\r\n    top: 45px\r\n";
        Check(UiScale.WithTopGap(reordered, 60).Contains("top: '60px'"), "top after another side was missed");
        string none = "gaps:\n  inner_gap: '8px'\n";
        Check(UiScale.WithTopGap(none, 60) == none, "A config without outer_gap must stay as it is");
        Check(UiScale.Percent(new Dictionary<string, object> { { "uiScale", 125 } }) == 125 && UiScale.Percent(new Dictionary<string, object> { { "uiScale", 7 } }) == 100, "uiScale pref read wrongly");
    }

    static void DialogTests() {
        string err;
        var d = Dialogs.Parse("kind=question&title=Silinsin%20mi%3F&body=a+b&buttons=Sil|Vazge%C3%A7&default=1&cancel=1&check=Bir%20daha%20sorma&checked=1", out err);
        Check(d != null && d.Title == "Silinsin mi?" && d.Body == "a b" && d.Buttons.Length == 2 && d.Buttons[1] == "Vazgeç" && d.Default == 1 && d.Cancel == 1 && d.Check == "Bir daha sorma" && d.Checked, "Dialog question was misread: " + err);
        d = Dialogs.Parse("kind=error&title=x&buttons=Tamam", out err);
        Check(d != null && d.Cancel == 0 && d.Default == 0, "A one-button dialog must cancel with that button");
        d = Dialogs.Parse("title=x&buttons=a|b&default=9&cancel=-1", out err);
        Check(d != null && d.Default == 0 && d.Cancel == -1 && d.Kind == "question", "Out-of-range dialog indexes must fall back");
        Check(Dialogs.Parse("title=x&buttons=", out err) == null && Dialogs.Parse("title=x&buttons=a|b|c|d", out err) == null, "Dialogs need one to three buttons");
        Check(Dialogs.Parse("kind=shout&title=x&buttons=a", out err) == null && Dialogs.Parse("buttons=a", out err) == null, "Dialog kind and text must be checked");
        Check(Dialogs.Parse("title=x&buttons=" + new string('a', 41), out err) == null, "Dialog button labels have a length limit");
        Check(Dialogs.ParseNotice("kind=warning&title=WM&body=x", out err) != null && Dialogs.ParseNotice("kind=shout&title=x", out err) == null && Dialogs.ParseNotice("kind=error&title=", out err) == null, "Notices must be checked");
        Check(Request("GET", "/notify?kind=error&title=x", "http://127.0.0.1:6124").Contains("405") && Request("POST", "/notify?kind=error&title=x", "https://example.invalid").Contains("403"), "Notices must be local POSTs");
        Check(Request("POST", "/notify?kind=error&title=x&body=y", "http://127.0.0.1:6124").Contains("204") && Request("POST", "/notify?kind=x&title=x", "http://127.0.0.1:6124").Contains("400"), "A notice must become a card, a bad one refused");
        Check(Request("GET", "/dialog?title=x&buttons=a", "http://127.0.0.1:6124").Contains("405"), "Dialogs must require POST");
        Check(Request("POST", "/dialog?title=x&buttons=a", "https://example.invalid").Contains("403"), "Dialogs accepted a foreign origin");
        Check(Request("POST", "/dialog?title=x&buttons=", "http://127.0.0.1:6124").Contains("400"), "An invalid dialog must be refused");
        Check(Request("POST", "/dialog-answer?id=987654&b=0&c=0", "http://127.0.0.1:6124").Contains("404"), "An answer to an unknown dialog must be refused");
        Check(Request("POST", "/dialog-shown?id=x", "http://127.0.0.1:6124").Contains("400") && Request("POST", "/dialog-answer?id=1", "http://127.0.0.1:6124").Contains("400"), "Malformed dialog answers must be refused");
        // no shell in the test: the question comes back unanswered instead of waiting
        Dialogs.ShowWaitMs = 300;
        string r = Dialogs.Ask(Dialogs.Parse("kind=info&title=x&buttons=Tamam", out err));
        Check(r.Contains("\"button\":-1") && r.Contains("no-ui"), "A dialog without a shell must answer no-ui: " + r);
        Check(Request("POST", "/dialog?kind=info&title=x&buttons=Tamam", "http://127.0.0.1:6124").StartsWith("HTTP/1.1 200"), "A valid dialog must be asked");
    }

    static void LauncherTests() {
        string missingDir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-no-dir-" + Guid.NewGuid().ToString("N"));
        Check(Launcher.Check(System.IO.Path.Combine(missingDir, "a.txt")) == Launcher.PATH_NOT_FOUND, "A file in a missing folder must be PATH_NOT_FOUND");
        Check(Launcher.Check(System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-missing-" + Guid.NewGuid().ToString("N") + ".txt")) == Launcher.NOT_FOUND, "A missing file must be NOT_FOUND");
        Check(Launcher.Check(Environment.GetFolderPath(Environment.SpecialFolder.Windows)) == Launcher.OK, "A folder must open");
        Check(Launcher.Check("cmd.exe") == Launcher.OK && Launcher.Check("notepad") == Launcher.OK, "Programs on PATH must be found");
        Check(Launcher.Check("ll-not-a-program-" + Guid.NewGuid().ToString("N")) == Launcher.NOT_FOUND, "An unknown program must be NOT_FOUND");
        Check(Launcher.Check("llnoscheme" + Guid.NewGuid().ToString("N").Substring(0, 8) + ":x") == Launcher.NO_ASSOCIATION, "An unregistered address scheme must be NO_ASSOCIATION");
        Check(Launcher.Check("https://example.invalid/") == Launcher.OK && Launcher.Check(@"shell:AppsFolder\x!App") == Launcher.OK, "https and shell: addresses must open");
        string odd = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-" + Guid.NewGuid().ToString("N") + ".llnoassoc" + Guid.NewGuid().ToString("N").Substring(0, 6));
        System.IO.File.WriteAllText(odd, "x");
        try { Check(Launcher.Check(odd) == Launcher.NO_ASSOCIATION, "A file no app opens must be NO_ASSOCIATION"); }
        finally { System.IO.File.Delete(odd); }
        // a shortcut whose target is gone (desktop double-click): our card, not Explorer's "shortcut problem" box
        string tmp = System.IO.Path.GetTempPath(), gone = System.IO.Path.Combine(tmp, "ll-gone-" + Guid.NewGuid().ToString("N") + ".exe");
        string lnkGone = System.IO.Path.Combine(tmp, "ll-" + Guid.NewGuid().ToString("N") + ".lnk"), lnkOk = System.IO.Path.Combine(tmp, "ll-" + Guid.NewGuid().ToString("N") + ".lnk");
        object wsh = Activator.CreateInstance(Type.GetTypeFromProgID("WScript.Shell"));
        foreach (var pair in new[] { new[] { lnkGone, gone }, new[] { lnkOk, Environment.GetFolderPath(Environment.SpecialFolder.Windows) } })
        {
            object sc = wsh.GetType().InvokeMember("CreateShortcut", System.Reflection.BindingFlags.InvokeMethod, null, wsh, new object[] { pair[0] });
            sc.GetType().InvokeMember("TargetPath", System.Reflection.BindingFlags.SetProperty, null, sc, new object[] { pair[1] });
            sc.GetType().InvokeMember("Save", System.Reflection.BindingFlags.InvokeMethod, null, sc, null);
        }
        try
        {
            Check(Launcher.Check(lnkGone) == Launcher.NOT_FOUND, "A shortcut whose target is gone must be NOT_FOUND");
            Check(Launcher.Check(lnkOk) == Launcher.OK, "A shortcut to an existing folder must open");
        }
        finally { System.IO.File.Delete(lnkGone); System.IO.File.Delete(lnkOk); }
        string urlOdd = System.IO.Path.Combine(tmp, "ll-" + Guid.NewGuid().ToString("N") + ".url");
        System.IO.File.WriteAllText(urlOdd, "[InternetShortcut]\r\nURL=llnoscheme" + Guid.NewGuid().ToString("N").Substring(0, 8) + ":x\r\n");
        try { Check(Launcher.Check(urlOdd) == Launcher.NO_ASSOCIATION, "An internet shortcut nothing opens must be NO_ASSOCIATION"); }
        finally { System.IO.File.Delete(urlOdd); }
        Check(Launcher.Describe(Launcher.NOT_FOUND).Length > 0, "Windows' error text must come back");
        Check(Request("GET", "/launch?file=cmd.exe", "http://127.0.0.1:6124").Contains("405") && Request("POST", "/launch?file=cmd.exe", "https://example.invalid").Contains("403"), "Launches must be local POSTs");
        Check(Request("POST", "/launch?file=", "http://127.0.0.1:6124").Contains("400") && Request("POST", "/launch?file=a&verb=delete", "http://127.0.0.1:6124").Contains("400"), "A bad launch must be refused");
        Check(Request("POST", "/launch?file=" + Uri.EscapeDataString(System.IO.Path.Combine(missingDir, "a.exe")), "http://127.0.0.1:6124").Contains("422"), "A launch that cannot open must answer 422 (our card)");
    }

    static void RestartTests() {
        var type = typeof(Supervisor).Assembly.GetType("DesktopRestart");
        Check(type != null, "Restart has no readiness gate: the desktop can be stopped before an elevated successor exists");
        var handoff = type.GetMethod("Handoff", BindingFlags.Static | BindingFlags.Public);
        Func<Func<bool>, Action, Func<bool>, Action, bool> run = (prepare, stop, commit, rollback) =>
            (bool)handoff.Invoke(null, new object[] { prepare, stop, commit, rollback });
        var events = new List<string>();
        Action stopDesktop = () => events.Add("stop");
        Action restore = () => events.Add("restore");
        Check(!run(() => false, stopDesktop, () => true, restore) && events.Count == 0,
            "Failed preparation changed the running desktop");
        Check(!run(() => { throw new Exception("launch failed"); }, stopDesktop, () => true, restore) && events.Count == 0,
            "A launch exception changed the running desktop");
        Check(run(() => { events.Add("ready"); return true; }, stopDesktop,
            () => { events.Add("commit"); return true; }, restore) && string.Join(",", events) == "ready,stop,commit",
            "Restart stopped the desktop before readiness or did not commit the successor");
        events.Clear();
        Check(!run(() => true, stopDesktop, () => false, restore) && string.Join(",", events) == "stop,restore",
            "A failed commit did not restore desktop availability");
        events.Clear();
        Check(!run(() => true, () => { events.Add("partial-stop"); throw new Exception("cannot stop core"); },
            () => { events.Add("commit"); return true; }, restore) && string.Join(",", events) == "partial-stop,restore",
            "Partial shutdown committed a successor or skipped recovery");
        events.Clear();
        Check(!run(() => true, stopDesktop, () => { throw new Exception("pipe broke"); }, restore) &&
            string.Join(",", events) == "stop,restore", "A broken commit pipe skipped recovery");
        bool inherited = false;
        Check(!DesktopRestart.PrepareRoutes(false, () => false, () => { inherited = true; return true; }) && !inherited,
            "A medium coordinator fell back to a medium successor");
        Check(DesktopRestart.PrepareRoutes(true, () => false, () => { inherited = true; return true; }) && inherited,
            "An elevated coordinator could not preserve its token when the task was unavailable");
        Check(DesktopRestart.PrepareRoutes(false, () => true, () => { throw new Exception("unexpected direct launch"); }),
            "A prepared scheduled candidate was bypassed");
        Check(!DesktopRestart.PrepareRoutes(true, () => false, () => false), "Two failed launch routes were treated as success");
        Check(DesktopRestart.CanBrokerLaunch(5, true), "Access-denied breakaway could not use the elevated own-token route");
        Check(!DesktopRestart.CanBrokerLaunch(5, false), "A medium caller was allowed to use the own-token fallback");
        foreach (int error in new[] { 0, 2, 87, 1314 })
            Check(!DesktopRestart.CanBrokerLaunch(error, true), "An unrelated launch error used the broker fallback");

        string xml = "<Task xmlns='http://schemas.microsoft.com/windows/2004/02/mit/task'><Principals>" +
            "<Principal id='Author'><UserId>S-1-5-21-123-1001</UserId><LogonType>InteractiveToken</LogonType>" +
            "<RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>" +
            "</Settings><Actions Context='Author'><Exec><Command>C:\\Program Files\\LogicalLunge\\lunge.exe</Command>" +
            "<WorkingDirectory>C:\\Users\\tester</WorkingDirectory></Exec></Actions></Task>";
        Func<string, int, bool> allowed = (doc, count) => DesktopRestart.CanRunTask(doc,
            @"C:\Program Files\LogicalLunge\lunge.exe", @"C:\Users\tester", "S-1-5-21-123-1001", @"PC\tester", count);
        Check(allowed(xml, 0), "The installed highest interactive task was rejected");
        Check(!allowed(xml, 1) && !allowed(xml.Replace("IgnoreNew", "Queue"), 1), "A busy task was treated as a ready replacement");
        Check(!allowed(xml.Replace("IgnoreNew", "StopExisting"), 0), "A task could kill the desktop before candidate readiness");
        Check(!allowed(xml.Replace("HighestAvailable", "LeastPrivilege"), 0), "A limited task could replace the elevated core");
        Check(!allowed(xml.Replace("InteractiveToken", "Password"), 0), "A task in a non-interactive logon could replace the desktop");
        Check(!allowed(xml.Replace("123-1001", "123-1002"), 0), "A task for a different user could replace the desktop");
        Check(!allowed(xml.Replace("lunge.exe", "other.exe"), 0), "A different executable could replace the desktop");
        Check(!allowed(xml.Replace("</Exec>", "<Arguments>--shutdown</Arguments></Exec>"), 0), "A task with destructive arguments was accepted");
        Check(!allowed(xml.Replace("<Settings>", "<Settings><Enabled>false</Enabled>"), 0) &&
            !allowed(xml.Replace("<Settings>", "<Settings><AllowStartOnDemand>false</AllowStartOnDemand>"), 0), "A disabled task was accepted");
        Check(!allowed(xml.Replace("</Actions>", "<Exec><Command>other.exe</Command></Exec></Actions>"), 0) && !allowed("broken", 0),
            "Extra task actions or malformed XML were accepted");
        Check(!allowed(xml.Replace("Context='Author'", "Context='SomeoneElse'"), 0), "The action used an unvalidated principal");

        // Exercise the real wire protocol on an isolated pipe, never the installed restart pipe.
        string pipeName = "ll-restart-test-" + Guid.NewGuid().ToString("N");
        using (var server = new System.IO.Pipes.NamedPipeServerStream(pipeName, System.IO.Pipes.PipeDirection.InOut, 1,
            System.IO.Pipes.PipeTransmissionMode.Byte, System.IO.Pipes.PipeOptions.Asynchronous))
        using (var client = new System.IO.Pipes.NamedPipeClientStream(".", pipeName, System.IO.Pipes.PipeDirection.InOut,
            System.IO.Pipes.PipeOptions.Asynchronous))
        {
            var connected = server.BeginWaitForConnection(null, null);
            client.Connect(1000); server.EndWaitForConnection(connected);
            var peer = System.Threading.Tasks.Task.Run(() => {
                Check(DesktopRestart.WriteSignal(client, (byte)'R', 1000), "Candidate readiness write timed out");
                Check(DesktopRestart.ReadSignal(client, (byte)'C', 1000), "Commit was not received");
                Check(DesktopRestart.WatchStartup(client, () => true, () => false, () => { }, () => {
                    throw new Exception("Ready startup was aborted");
                }, 1000).Wait(1500), "Startup acknowledgement did not finish");
            });
            Check(DesktopRestart.ReadSignal(server, (byte)'R', 1000), "Candidate readiness was not received");
            Check(DesktopRestart.WriteSignal(server, (byte)'C', 1000), "Commit write timed out");
            Check(DesktopRestart.ReadSignal(server, (byte)'S', 1000), "Startup acknowledgement was not received");
            Check(DesktopRestart.WriteSignal(server, (byte)'A', 1000), "Coordinator acceptance was not sent");
            Check(peer.Wait(1500), "Candidate protocol did not finish");
        }
        using (var empty = new System.IO.MemoryStream())
            Check(!DesktopRestart.ReadSignal(empty, (byte)'C', 1000), "Coordinator EOF was interpreted as commit");
        using (var wrong = new System.IO.MemoryStream(new byte[] { (byte)'R' }))
            Check(!DesktopRestart.ReadSignal(wrong, (byte)'C', 1000), "Readiness was interpreted as commit");
        string timeoutPipe = "ll-restart-timeout-" + Guid.NewGuid().ToString("N");
        using (var server = new System.IO.Pipes.NamedPipeServerStream(timeoutPipe, System.IO.Pipes.PipeDirection.InOut, 1,
            System.IO.Pipes.PipeTransmissionMode.Byte, System.IO.Pipes.PipeOptions.Asynchronous))
        using (var client = new System.IO.Pipes.NamedPipeClientStream(".", timeoutPipe, System.IO.Pipes.PipeDirection.InOut,
            System.IO.Pipes.PipeOptions.Asynchronous))
        {
            var connected = server.BeginWaitForConnection(null, null); client.Connect(1000); server.EndWaitForConnection(connected);
            Check(!DesktopRestart.ReadSignal(server, (byte)'R', 50), "A silent candidate had no readiness deadline");
            Check(!DesktopRestart.WriteSignal(server, (byte)'C', 50), "An unread commit had no write deadline");
        }
        RestartReviewTests();
        Console.WriteLine("PASS: restart readiness, elevation fallback, task contract, pipe protocol/deadlines, commit order and rollback");
    }

    sealed class FakeStopProcess : DesktopRestart.StopProcess {
        readonly Func<bool> exit;
        public bool Disposed;
        public FakeStopProcess(Func<bool> action) { exit = action; }
        public bool Exit(int timeout) { return exit(); }
        public void Dispose() { Disposed = true; }
    }

    static void RestartPipe(Action<System.IO.Pipes.NamedPipeServerStream, System.IO.Pipes.NamedPipeClientStream> test) {
        string name = "ll-restart-review-" + Guid.NewGuid().ToString("N");
        using (var server = new System.IO.Pipes.NamedPipeServerStream(name, System.IO.Pipes.PipeDirection.InOut, 1,
            System.IO.Pipes.PipeTransmissionMode.Byte, System.IO.Pipes.PipeOptions.Asynchronous))
        using (var client = new System.IO.Pipes.NamedPipeClientStream(".", name, System.IO.Pipes.PipeDirection.InOut,
            System.IO.Pipes.PipeOptions.Asynchronous)) {
            var connection = server.BeginWaitForConnection(null, null);
            client.Connect(1000); server.EndWaitForConnection(connection);
            test(server, client);
        }
    }

    static void RestartReviewTests() {
        // Core termination tests use a private mutex and fake handles; no installed process is touched.
        string name = "ll-core-stop-test-" + Guid.NewGuid().ToString("N");
        using (var held = new System.Threading.ManualResetEventSlim())
        using (var release = new System.Threading.ManualResetEventSlim()) {
            var owner = System.Threading.Tasks.Task.Run(() => {
                using (var mutex = new System.Threading.Mutex(false, name)) {
                    mutex.WaitOne(); held.Set(); release.Wait(); mutex.ReleaseMutex();
                }
            });
            Check(held.Wait(1000), "Test mutex owner did not start");
            try {
                foreach (string why in new[] { "missing PID", "unreadable PID", "stale PID", "termination access denied" }) {
                    bool launched = false, mutated = false, recovery = false;
                    Check(!DesktopRestart.Handoff(() => {
                        using (var guard = new DesktopRestart.CoreStop(name, () => { throw new InvalidOperationException(why); })) {
                            launched = true; return true;
                        }
                    }, () => mutated = true, () => true, () => recovery = true) &&
                        !launched && !mutated && !recovery, "Unsafe preflight changed the desktop: " + why);
                }
                var denied = new FakeStopProcess(() => false);
                using (var guard = new DesktopRestart.CoreStop(name, () => denied)) {
                    bool mutated = false;
                    try { guard.ExitAndReserve(); mutated = true; } catch (InvalidOperationException) { }
                    Check(!mutated && !guard.Exited, "Failed termination allowed wallpaper/splash/teardown mutation");
                }
                Check(denied.Disposed, "Failed termination leaked the retained handle");
                using (var guard = new DesktopRestart.CoreStop(name, () => new FakeStopProcess(() => true))) {
                    bool mutated = false;
                    try { guard.ExitAndReserve(); mutated = true; } catch (InvalidOperationException) { }
                    Check(!mutated, "An exit result without a free core mutex allowed teardown");
                }
                using (var guard = new DesktopRestart.CoreStop(name, () => new FakeStopProcess(() => {
                    release.Set(); return owner.Wait(1000);
                }))) {
                    guard.ExitAndReserve();
                    Check(guard.Exited, "Verified exit did not reserve the core mutex");
                    bool competitor = System.Threading.Tasks.Task.Run(() => {
                        using (var mutex = new System.Threading.Mutex(false, name)) {
                            bool got = mutex.WaitOne(0); if (got) mutex.ReleaseMutex(); return got;
                        }
                    }).Result;
                    Check(!competitor, "Another startup acquired the core mutex during teardown");
                    guard.Release();
                    guard.ExitAndReserve(); // rollback reacquires after a released handoff
                    Check(guard.Exited, "Rollback could not reacquire the free core mutex");
                }
                using (var guard = new DesktopRestart.CoreStop(name, () => { throw new Exception("PID should not be read"); }))
                    Check(guard.Exited, "A genuinely free core mutex still required a PID file");
            } finally { release.Set(); Check(owner.Wait(1500), "Test mutex owner did not finish"); }
        }

        // Readiness includes BringUp, a live UI pump, initialized watchdogs and HTTP binding.
        RestartPipe((server, client) => {
            int flags = 0, accepted = 0, aborted = 0;
            var worker = DesktopRestart.WatchStartup(client, () => System.Threading.Volatile.Read(ref flags) == 15,
                () => false, () => System.Threading.Interlocked.Increment(ref accepted),
                () => System.Threading.Interlocked.Increment(ref aborted), 2000);
            var signal = new byte[1]; var read = server.ReadAsync(signal, 0, 1);
            for (int i = 0; i < 4; i++) {
                Check(!read.Wait(40), "S was sent before all startup prerequisites were ready");
                System.Threading.Volatile.Write(ref flags, (1 << (i + 1)) - 1);
            }
            Check(read.Wait(1000) && read.Result == 1 && signal[0] == (byte)'S', "Full readiness did not send S");
            Check(accepted == 0, "Startup discarded rollback before coordinator acceptance");
            server.Dispose(); // coordinator failed after S: the candidate must still abort
            Check(worker.Wait(1500) && aborted == 1 && accepted == 0, "EOF after S did not abort startup");
        });
        foreach (bool explicitFailure in new[] { false, true }) RestartPipe((server, client) => {
            int accepted = 0, aborted = 0;
            var worker = DesktopRestart.WatchStartup(client, () => false, () => explicitFailure,
                () => accepted++, () => aborted++, 120);
            Check(DesktopRestart.ReadSignal(server, (byte)'F', 1500), "Startup failure/timeout did not report F");
            Check(worker.Wait(1500) && accepted == 0 && aborted == 1, "Startup failure/timeout did not abort");
        });
        RestartPipe((server, client) => {
            int accepted = 0, aborted = 0;
            var worker = DesktopRestart.WatchStartup(client, () => false, () => false,
                () => accepted++, () => aborted++, 1000);
            Check(DesktopRestart.WriteSignal(server, (byte)'A', 1000), "Early acceptance test could not send A");
            Check(worker.Wait(1500) && accepted == 0 && aborted == 1, "Early acceptance bypassed readiness");
        });

        // The first (medium duplicate) client must not consume the legitimate candidate's launch attempt.
        string race = "ll-restart-race-" + Guid.NewGuid().ToString("N");
        using (var server = new System.IO.Pipes.NamedPipeServerStream(race, System.IO.Pipes.PipeDirection.InOut, 1,
            System.IO.Pipes.PipeTransmissionMode.Byte, System.IO.Pipes.PipeOptions.Asynchronous)) {
            int peers = 0, launches = 0;
            System.Threading.Tasks.Task clients = null;
            Check(DesktopRestart.WaitCandidate(server, () => {
                launches++;
                clients = System.Threading.Tasks.Task.Run(() => {
                    using (var wrong = new System.IO.Pipes.NamedPipeClientStream(".", race,
                        System.IO.Pipes.PipeDirection.InOut, System.IO.Pipes.PipeOptions.Asynchronous)) {
                        wrong.Connect(1000);
                        Check(wrong.ReadByte() == -1, "Wrong peer was not disconnected");
                    }
                    using (var right = new System.IO.Pipes.NamedPipeClientStream(".", race,
                        System.IO.Pipes.PipeDirection.InOut, System.IO.Pipes.PipeOptions.Asynchronous)) {
                        right.Connect(1000);
                        Check(DesktopRestart.WriteSignal(right, (byte)'R', 1000), "Correct candidate could not signal R");
                    }
                });
                return true;
            }, () => ++peers == 2, 2000), "A wrong first client prevented the correct candidate from preparing");
            Check(launches == 1 && peers == 2 && clients.Wait(1500), "Client rejection relaunched or lost the candidate");
        }
        Console.WriteLine("PASS: termination preflight, missing/stale PID mutex guard, startup readiness/failure/rollback and wrong-peer race");
    }

    static void DesktopWidgetTests() {
        var eligible = typeof(Slider).GetMethod("DesktopWidgetEligible", BindingFlags.NonPublic | BindingFlags.Static);
        var attach = typeof(Slider).GetMethod("AddDesktopThumbnails", BindingFlags.NonPublic | BindingFlags.Static);
        Check(eligible != null && attach != null, "Desktop widgets have no stationary layer in the animation scene");
        for (int bits = 0; bits < 64; bits++) {
            bool identity = (bits & 7) == 7, visible = (bits & 8) != 0, cloaked = (bits & 16) != 0, topmost = (bits & 32) != 0;
            bool actual = (bool)eligible.Invoke(null, new object[] {
                (bits & 1) != 0 ? "Logical Lunge · widget" : "another surface",
                (bits & 2) != 0 ? "LungeNativeBar" : "another class",
                (bits & 4) != 0 ? Names.Shell : "another process", visible, cloaked, topmost });
            Check(actual == (identity && visible && !cloaked && !topmost), "Wrong desktop thumbnail eligibility at " + bits);
        }
        var background = new Slider.Thumb { Src = new IntPtr(1) };
        var scene = new List<Slider.Thumb> { background };
        var sources = new List<KeyValuePair<IntPtr, Native.RECT>> {
            new KeyValuePair<IntPtr, Native.RECT>(new IntPtr(2), new Native.RECT { Left = -1900, Top = 100, Right = -1800, Bottom = 180 }),
            new KeyValuePair<IntPtr, Native.RECT>(new IntPtr(3), new Native.RECT { Left = -1850, Top = 110, Right = -1750, Bottom = 210 }),
        };
        Func<IntPtr, Native.RECT, Slider.Thumb> register = (h, dest) => new Slider.Thumb { Src = h, Dest = dest };
        attach.Invoke(null, new object[] { scene, sources, -1920, 40, register });
        scene.Add(new Slider.Thumb { Src = new IntPtr(4), IsWin = true });
        Check(scene.Count == 4 && scene[0] == background && scene[1].Src == new IntPtr(3) && scene[2].Src == new IntPtr(2) && scene[3].Src == new IntPtr(4),
            "Widget previews must keep z-order between wallpaper and animated apps");
        var r = scene[2].Dest;
        Check(!scene[2].IsWin && r.Left == 20 && r.Top == 60 && r.Right == 120 && r.Bottom == 140,
            "Desktop previews moved with apps or changed their size/monitor coordinates");
        attach.Invoke(null, new object[] { scene, sources, 0, 0, new Func<IntPtr, Native.RECT, Slider.Thumb>((h, dest) => null) });
        Check(scene.Count == 4, "A failed thumbnail registration corrupted the scene");
        Console.WriteLine("PASS: stationary desktop thumbnails, visibility, ownership, z-order and monitor coordinates");
    }

    static void Main(string[] args) {
        WorkspaceOutlineTests();
        if (args.Length == 1 && args[0] == "--workspace-outline-only") return;
        DesktopWidgetTests();
        RestartTests();
        if (args.Length == 1 && args[0] == "--restart-only") return;
        TakeoverTests();
        UiScaleTests();
        FullscreenLayerTests();
        DialogTests();
        LauncherTests();
        string response = Request("POST", "/focus-color?v=invalid", "http://127.0.0.1:6124");
        Check(!response.Contains("text/event-stream") && response.Contains("\"ok\":false"), "Focus-color request was routed into SSE instead of returning a JSON result");
        Check(Request("GET", "/focus-color?v=invalid", "http://127.0.0.1:6124").Contains("405"), "Color writes must require POST");
        Check(Request("POST", "/focus-color?v=invalid", "https://example.invalid").Contains("403"), "Foreign origin was accepted");
        var releases = new List<object>();
        foreach (string tag in new[] { "v0.2.9-native-ui", "v0.2.10-native-ui", "v9.0.0-web-ui" }) {
            string edition = tag.EndsWith("web-ui") ? "web-ui" : "native-ui";
            string version = tag.Substring(1).Split('-')[0];
            string name = "LogicalLunge-" + edition + "-" + version + ".zip";
            releases.Add(new Dictionary<string, object> {
                {"tag_name",tag}, {"assets", new object[] {
                    new Dictionary<string, object>{{"name",name},{"browser_download_url","https://example.invalid/"+name},{"size",10}},
                    new Dictionary<string, object>{{"name",name+".sha256"},{"browser_download_url","https://example.invalid/"+name+".sha256"}}
                }}
            });
        }
        var rel = Call(typeof(Updater), "SelectRelease", releases, "native-ui");
        Check((string)rel.GetType().GetField("Tag").GetValue(rel) == "v0.2.10-native-ui", "Updater switched editions or compared versions as text");
        string display = Uri.EscapeDataString(@"\\.\DISPLAY1");
        Check(Request("POST", "/brightness?dev=" + Uri.EscapeDataString(@"C:\x"), "http://127.0.0.1:6124").Contains("400"), "Brightness accepted a device that is not a display");
        Check(Request("GET", "/brightness?dev=" + display + "&v=50", "http://127.0.0.1:6124").Contains("405"), "Brightness writes must require POST");
        Check(Request("POST", "/brightness?dev=" + display + "&v=50", "https://example.invalid").Contains("403"), "Brightness accepted a foreign origin");
        Check((string)Call(typeof(MonitorFriendlyNames), "Key", @"\\?\DISPLAY#BOE0812#4&2a3b5c7d&0&UID8388688#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}") == @"DISPLAY\BOE0812\4&2a3b5c7d&0&UID8388688", "Monitor interface ID was not reduced to its WMI key");
        Check(MonitorFriendlyNames.IsInstanceOf(@"DISPLAY\BOE0812\4&2a3b5c7d&0&UID8388688_0", @"DISPLAY\BOE0812\4&2a3b5c7d&0&UID8388688"), "The panel's WMI row was not matched to its monitor");
        Check(!MonitorFriendlyNames.IsInstanceOf(@"DISPLAY\BOE0812\4&2a3b5c7d&0&UID8388689_0", @"DISPLAY\BOE0812\4&2a3b5c7d&0&UID8388688"), "Another monitor's WMI row was matched");
        var toast = ToastPayload.Parse("<toast scenario=\"reminder\"><visual><binding template=\"ToastGeneric\"><text>Ayşe</text><text>Yarın 10:00</text><text>Toplantı</text><image placement=\"appLogoOverride\" src=\"file:///C:/x/a.png\"/></binding></visual><actions><action content=\"Ertele\" arguments=\"s\"/></actions></toast>");
        Check(toast.Title == "Ayşe" && toast.Body == "Yarın 10:00\nToplantı" && toast.Logo == "file:///C:/x/a.png", "Toast text or sender image was not read");
        Check(toast.Interactive && toast.Urgent, "A reminder with buttons must point to the notification centre");
        var legacy = ToastPayload.Parse("<toast><visual><binding template=\"ToastText02\"><text id=\"1\">Başlık</text><text id=\"2\"> </text></binding></visual><actions><action content=\"Kopyala\" arguments=\"c\" placement=\"contextMenu\"/></actions></toast>");
        Check(legacy.Title == "Başlık" && legacy.Body == "" && !legacy.Interactive && !legacy.Urgent, "Legacy toast template or context-menu action was misread");
        Check(ToastPayload.Parse("<!DOCTYPE toast [<!ENTITY x SYSTEM \"file:///C:/Windows/win.ini\">]><toast><visual><binding><text>&x;</text></binding></visual></toast>").Title == "", "Toast XML with a DTD must be rejected");
        Check(ToastPayload.Parse("<tile><visual><binding><text>x</text></binding></visual></tile>").Title == "", "Only toast payloads are read");
        string local = @"C:\Users\u\AppData\Local";
        Check(ToastPayload.LocalImagePath("file:///C:/x/a.png", "", local) == @"C:\x\a.png", "file:/// image path was not resolved");
        Check(ToastPayload.LocalImagePath("ms-appdata:///local/img/a%20b.png", "Fam.App_8wekyb3d8bbwe!App", local) == local + @"\Packages\Fam.App_8wekyb3d8bbwe\LocalState\img\a b.png", "ms-appdata image was not resolved to the package folder");
        Check(ToastPayload.LocalImagePath("ms-appdata:///local/../../x.png", "Fam.App_8wekyb3d8bbwe!App", local) == null, "ms-appdata path left the package folder");
        Check(ToastPayload.LocalImagePath("file://server/share/a.png", "", local) == null && ToastPayload.LocalImagePath("ms-appx:///Assets/a.png", "a!b", local) == null, "Network or package-internal images must be skipped");
        Check(ToastPayload.LocalImagePath("ms-appdata:///local///server/share/a.png", "Fam.App_8wekyb3d8bbwe!App", local) == null && ToastPayload.LocalImagePath("ms-appdata:///local/a.png", @"\\host\share!x", local) == null, "ms-appdata path reached a network share");
        Check(ToastPayload.NameFromAumid("Microsoft.WindowsStore_8wekyb3d8bbwe!App") == "WindowsStore" && ToastPayload.NameFromAumid("com.squirrel.Discord.Discord") == "Discord" && ToastPayload.NameFromAumid(@"{6D809377-6AF0-444B-8957-A3773F02200E}\Mozilla Firefox\firefox.exe") == "firefox", "Sender name fallback is wrong");
        Check(ToastPayload.UnixMs(116444736000000000L) == 0 && ToastPayload.UnixMs(116444736000000000L + 10000) == 1, "Arrival time conversion is wrong");
        Check(Request("POST", "/prefs.json", null, "rebound.example:6131").Contains("403"), "A request for another host name (DNS rebinding) was served");
        Check(Request("GET", "/events", "https://example.invalid").Contains("403"), "The event stream accepted a browser origin");
        Check(Request("GET", "/gamma?dev=" + Uri.EscapeDataString(@"C:\x"), "http://127.0.0.1:6124").Contains("400"), "Gamma accepted a device that is not a display");
        Check(Request("GET", "/gamma?dev=" + display + "&v=50", "http://127.0.0.1:6124").Contains("405"), "Gamma writes must require POST");
        Check(Request("GET", "/dock-pin?id=chrome&on=1", "http://127.0.0.1:6124").Contains("405"), "Dock pin writes must require POST");
        Check(Request("POST", "/dock-pin?id=chrome&on=2", "http://127.0.0.1:6124").Contains("400"), "Dock pin accepted an invalid state");
        Check(Request("POST", "/dock-pin?id=chrome&on=1", "https://example.invalid").Contains("403"), "Dock pin accepted a foreign origin");
        Check(Request("GET", "/notifications", "http://127.0.0.1:6124").Contains("405"), "The notification list must require POST");
        // A web page's <img>/link GET carries no Origin: it must not reach anything that changes state
        Check(Request("GET", "/cmd?a=stop-desktop", null).Contains("405"), "A plain GET reached a desktop command");
        Check(Request("GET", "/pref?k=theme&v=light", null).Contains("405"), "A plain GET wrote a preference");
        Check(Request("GET", "/dock-pins?v=%5B%5D", null).Contains("405"), "A plain GET overwrote the Dock pins");
        Check(Request("GET", "/widget?w=osk&v=1", null).Contains("405"), "A plain GET showed a widget");
        Check(Request("POST", "/pref?k=nope&v=1", null).Contains("400"), "A POST without Origin from a local client must still be served");
        string notes = Request("POST", "/notifications", "http://127.0.0.1:6124");
        Check(notes.Contains("200 OK") && notes.Contains("\"items\"") && notes.Contains("\"icons\""), "Notification list was not served");
        Check(Request("POST", "/notifications", "https://example.invalid").Contains("403"), "Notification list accepted a foreign origin");
        // Quick settings moved from PowerShell into the core: reads and writes are POST-only and answer JSON
        Check(Request("GET", "/qs/radio?kind=wifi&state=Off", null).Contains("405"), "A plain GET switched a radio");
        Check(Request("GET", "/qs/awake?v=1", null).Contains("405"), "A plain GET changed keep-awake");
        Check(Request("GET", "/qs/eth-toggle", null).Contains("405"), "A plain GET toggled Ethernet");
        Check(Request("GET", "/qs/wifi-disconnect", null).Contains("405"), "A plain GET disconnected Wi-Fi");
        Check(Request("GET", "/qs/wifi-connect?ssid=x&pw=y", null).Contains("405"), "A plain GET joined a Wi-Fi network");
        Check(Request("POST", "/qs/wifi-connect?pw=y", "http://127.0.0.1:6124").Contains("400"), "Joining Wi-Fi without a network name was accepted");
        Check(Request("POST", "/qs/wifi-disconnect", "https://example.invalid").Contains("403"), "Wi-Fi accepted a foreign origin");
        Check(Request("GET", "/qs/bt", "http://127.0.0.1:6124").Contains("405"), "The Bluetooth device list must require POST");
        Check(Request("POST", "/qs/radio?kind=wifi&state=Maybe", "http://127.0.0.1:6124").Contains("400"), "A radio accepted an invalid state");
        Check(Request("POST", "/qs/awake?v=1", "https://example.invalid").Contains("403"), "Keep-awake accepted a foreign origin");
        Check(Request("POST", "/qs/nope", "http://127.0.0.1:6124").Contains("404"), "An unknown quick setting was served");
        // Gallery "remove from library": only inside the library folders, POST only, never Windows' own screen savers
        string shell = "http://127.0.0.1:6124";
        Func<string, string, string> remove = (kind, path) => "/library-remove?kind=" + kind + "&path=" + Uri.EscapeDataString(path);
        Check(Request("GET", remove("wall", @"C:\x.png"), shell).Contains("405"), "Library removal must require POST");
        Check(Request("POST", remove("wall", @"C:\x.png"), "https://example.invalid").Contains("403"), "Library removal accepted a foreign origin");
        Check(Request("POST", "/library-remove?kind=nope&path=x", shell).Contains("400"), "Library removal accepted an unknown kind");
        Check(Request("POST", remove("wall", Environment.GetFolderPath(Environment.SpecialFolder.Windows) + @"\win.ini"), shell).Contains("400"), "Library removal reached a file outside the library");
        Check(Request("POST", remove("saver", Environment.GetFolderPath(Environment.SpecialFolder.System) + @"\scrnsave.scr"), shell).Contains("400"), "Library removal reached one of Windows' screen savers");
        Check(Request("POST", remove("live", LiveWallpaper.Dir + @"\..\..\x.mp4"), shell).Contains("400"), "Library removal followed .. out of the library");
        Check(Request("POST", remove("wall", "relative.png"), shell).Contains("400"), "Library removal accepted a relative path");
        string wallFile = System.IO.Path.Combine(Wallpaper.Dir, "ll-test-remove.png");
        System.IO.File.WriteAllText(wallFile, "x");
        Check(Request("POST", remove("wall", wallFile), shell).Contains("204") && !System.IO.File.Exists(wallFile), "A library wallpaper was not removed");
        Check(Request("POST", remove("wall", wallFile), shell).Contains("404"), "Removing a missing wallpaper must say so");
        string packDir = System.IO.Path.Combine(ScreenSavers.Dir, "ll-test-pack");
        System.IO.Directory.CreateDirectory(packDir);
        string packFile = System.IO.Path.Combine(packDir, "a.scr");
        System.IO.File.WriteAllText(packFile, "x");
        Check(Request("POST", remove("saver", packFile), shell).Contains("204") && !System.IO.Directory.Exists(packDir), "An imported screen saver (and its emptied pack folder) was not removed");
        string radios = Request("POST", "/qs/radios", "http://127.0.0.1:6124");
        Check(radios.Contains("200 OK") && radios.Contains("\"wifi\"") && radios.Contains("\"bluetooth\""), "Radios were not served");
        string wifi = Request("POST", "/qs/wifi", "http://127.0.0.1:6124");
        Check(wifi.Contains("200 OK") && wifi.Contains("\"connected\"") && wifi.Contains("\"networks\""), "Wi-Fi networks were not served");
        string eth = Request("POST", "/qs/eth", "http://127.0.0.1:6124");
        Check(eth.Contains("200 OK") && eth.Contains("\"state\""), "Ethernet state was not served");
        string bt = Request("POST", "/qs/bt", "http://127.0.0.1:6124");
        Check(bt.Contains("200 OK") && bt.Contains("\"adapter\"") && bt.Contains("\"devices\""), "Bluetooth devices were not served");
        Check(Request("POST", "/qs/awake?v=1", "http://127.0.0.1:6124").Contains("204"), "Keep-awake did not turn on");
        Check(Request("POST", "/qs/status", "http://127.0.0.1:6124").Contains("\"awake\":true"), "Keep-awake is not reported as on");
        Check(Request("POST", "/qs/awake?v=0", "http://127.0.0.1:6124").Contains("204"), "Keep-awake did not turn off");
        Check(Request("POST", "/qs/status", "http://127.0.0.1:6124").Contains("\"awake\":false"), "Keep-awake is not reported as off");
        // The Super menu's command runner (was scripts\run.ps1)
        Check(RunCommand.Run("run", "   ") == null, "An empty command must do nothing");
        string echo = RunCommand.Run("term", "echo ll-run-test");
        Check(echo != null && echo.Contains("\"kind\":\"ok\"") && echo.Contains("ll-run-test"), "A hidden command's output was not returned: " + echo);
        string missing = RunCommand.Run("term", "ll-no-such-command-xyz");
        Check(missing != null && missing.Contains("\"kind\":\"error\""), "A missing command was not reported: " + missing);
        string dir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-core-test-" + Guid.NewGuid().ToString("N"));
        System.IO.Directory.CreateDirectory(dir);
        try {
            // The Super menu's app list (was scripts\build-apps.ps1): Shell.Application wants an STA thread
            string appsPath = System.IO.Path.Combine(dir, "apps.json");
            int appCount = 0; Exception appError = null;
            var sta = new System.Threading.Thread(() => { try { appCount = AppIndex.Build(appsPath); } catch (Exception ex) { appError = ex; } });
            sta.SetApartmentState(System.Threading.ApartmentState.STA);
            sta.Start(); sta.Join();
            Check(appError == null && appCount > 0, "The app list was not built: " + (appError == null ? "empty" : appError.GetBaseException().Message));
            var appList = new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.Deserialize<List<Dictionary<string, object>>>(System.IO.File.ReadAllText(appsPath));
            Check(appList.Count == appCount && appList[0].ContainsKey("path") && appList[0].ContainsKey("icon"), "The app list file is not the expected JSON");
            Func<string, Dictionary<string, object>> parse = t => new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(t);
            Func<Dictionary<string, object>> empty = () => new Dictionary<string, object>();
            Dictionary<string, object> d;
            string path = System.IO.Path.Combine(dir, "keybinds.json");
            Check(SettingsFile.TryReadForUpdate(path, parse, empty, out d) && d.Count == 0, "Missing settings file must start empty");
            System.IO.File.WriteAllText(path, "{\"terminal\":\"super+t\"}");
            Check(SettingsFile.TryReadForUpdate(path, parse, empty, out d) && (string)d["terminal"] == "super+t", "Settings file was not read");
            using (new System.IO.FileStream(path, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.None))
                Check(!SettingsFile.TryReadForUpdate(path, parse, empty, out d), "An unreadable settings file must block the write");
            System.IO.File.WriteAllText(path, "{not json");
            Check(SettingsFile.TryReadForUpdate(path, parse, empty, out d) && d.Count == 0 && System.IO.File.Exists(path + ".bad"), "A corrupt settings file must be backed up before starting over");
        } finally { try { System.IO.Directory.Delete(dir, true); } catch { } }
        // Canlı duvar kağıdı kayıtları: herkese bir video, tek monitör kapalı, monitörün kendi videosu, hepsi kapalı
        var none = new List<KeyValuePair<string, string>>();
        var forAll = (List<KeyValuePair<string, string>>)Call(typeof(LiveWallpaper), "WithVideo", none, "all", @"C:\v\a.mp4");
        Check((string)Call(typeof(LiveWallpaper), "FileFor", forAll, "M1") == @"C:\v\a.mp4", "A video for every monitor did not reach a monitor");
        var oneOff = (List<KeyValuePair<string, string>>)Call(typeof(LiveWallpaper), "WithoutVideo", forAll, "M2");
        Check((string)Call(typeof(LiveWallpaper), "FileFor", oneOff, "m2") == "" && (string)Call(typeof(LiveWallpaper), "FileFor", oneOff, "M1") == @"C:\v\a.mp4", "Turning one monitor off changed the others");
        var own = (List<KeyValuePair<string, string>>)Call(typeof(LiveWallpaper), "WithVideo", oneOff, "M2", @"C:\v\b.mp4");
        Check(own.Count == 2 && (string)Call(typeof(LiveWallpaper), "FileFor", own, "M2") == @"C:\v\b.mp4", "A monitor's own video did not replace its off entry");
        Check(((List<KeyValuePair<string, string>>)Call(typeof(LiveWallpaper), "WithoutVideo", own, "span")).Count == 0, "Turning every monitor off left entries");
        var single = (List<KeyValuePair<string, string>>)Call(typeof(LiveWallpaper), "WithVideo", none, "M1", @"C:\v\c.mp4");
        Check(((List<KeyValuePair<string, string>>)Call(typeof(LiveWallpaper), "WithoutVideo", single, "M1")).Count == 0 && none.Count == 0 && forAll.Count == 1, "The last video off must clear the state, and inputs must stay unchanged");
        Check((string)Call(typeof(LiveWallpaper), "Slug", "a/../b:c") == "a____b_c" && (string)Call(typeof(LiveWallpaper), "Slug", "CON") == "_CON", "Store names must become plain folder names");
        // The video screen saver plays only videos from the live wallpaper library
        Func<string, bool> refused = v => { try { SaverVideo.InLibrary(v); return false; } catch (ArgumentException) { return true; } };
        string saverDir = System.IO.Path.Combine(LiveWallpaper.Dir, "__regression__");
        System.IO.Directory.CreateDirectory(saverDir);
        string saverVideo = System.IO.Path.Combine(saverDir, "a.mp4"), saverText = System.IO.Path.Combine(saverDir, "a.txt");
        System.IO.File.WriteAllText(saverVideo, "x"); System.IO.File.WriteAllText(saverText, "x");
        try
        {
            Check(SaverVideo.InLibrary(saverVideo) == saverVideo, "A library video was not accepted for the screen saver");
            Check(refused(System.IO.Path.Combine(saverDir, @"..\..\..\a.mp4")), "A path leaving the library was accepted for the screen saver");
            Check(refused(@"C:\Windows\Media\a.mp4") && refused(saverText) && refused(System.IO.Path.Combine(saverDir, "missing.mp4")) && refused(""), "A file outside the library, not a video or missing was accepted for the screen saver");
            Check(refused(LiveWallpaper.Dir + "-other\\a.mp4"), "A sibling folder sharing the library's name prefix was accepted");
        }
        finally { System.IO.Directory.Delete(saverDir, true); }
        // Kısayollar: pencere yöneticisi biçimi, çakışmalar, uygulama kısayollarının doğrulaması
        Check(WmBinds.ToUi("lwin+shift+oem_1") == "Super+Shift+;" && WmBinds.ToWm("Super+Shift+;") == "lwin+shift+oem_1", "Window manager key names did not round-trip");
        Check(WmBinds.ToWm("Super+PageUp") == "lwin+page_up" && WmBinds.ToUi("lwin+ctrl+page_down") == "Super+Ctrl+PageDown", "Page keys did not convert");
        Check(Binds.Canonical("super+shift+s") == "Super+Shift+S" && Binds.Canonical("ctrl+shift+escape") == "Ctrl+Shift+Escape" && Binds.Canonical("Super+Banana") == null, "Combos were not normalised");
        var conflicts = Keymap.Conflicts(new Dictionary<string, string> { { "browser", "Super+W" }, { "app:x", "super+w" }, { "files", "Super+L" }, { "code", "" } },
            new Dictionary<int, List<string>> { { 0, new List<string> { "Super+F" } }, { 1, new List<string> { "Super+F", "Super+G" } } });
        Func<string, Dictionary<string, object>> conflictFor = c => conflicts.Find(x => (string)x["combo"] == c);
        Check(conflicts.Count == 3, "Expected three conflicts, got " + conflicts.Count);
        Check(conflictFor("Super+W") != null && ((List<string>)conflictFor("Super+W")["keys"]).Count == 2, "Two shortcuts on Super+W were not reported");
        Check(conflictFor("Super+L") != null && conflictFor("Super+L")["reserved"] != null, "A shortcut on Windows' lock combo was not reported");
        Check(conflictFor("Super+F") != null, "Two window manager entries on Super+F were not reported");
        Check(Keymap.Conflicts(new Dictionary<string, string> { { "browser", "Super+W" } }, new Dictionary<int, List<string>>()).Count == 0, "A lone shortcut was reported as a conflict");
        Func<string, string, string> keyAppError = (id, path) => Binds.ValidateApp(new Binds.CustomApp { Id = id, Name = "x", Path = path });
        string exe = System.Diagnostics.Process.GetCurrentProcess().MainModule.FileName;
        Check(keyAppError("app:ok-1", exe) == null, "A valid app shortcut was refused");
        Check(keyAppError("browser", exe) != null && keyAppError("app:../x", exe) != null, "An app shortcut with a bad id was accepted");
        Check(keyAppError("app:x", @"C:\no\such.exe") != null && keyAppError("app:x", "relative.exe") != null, "A missing or relative app path was accepted");
        Check(keyAppError("app:x", System.IO.Path.Combine(System.IO.Path.GetTempPath(), "x.txt")) != null, "A non-launchable file type was accepted");
        Check(keyAppError("app:x", @"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App") == null, "A store app from the app list was refused");
        Check(keyAppError("app:x", "shell:AppsFolder\\a\" x") != null, "A store app id with a quote was accepted");
        Check(Keymap.Check("{\"core\":{\"nope\":\"Super+Z\"}}").Contains("\"ok\":false"), "An unknown shortcut id was accepted");
        Check(Keymap.Check("{\"core\":{\"browser\":\"Super+Banana\"}}").Contains("\"ok\":false"), "An unknown key name was accepted");
        Check(Keymap.Check("{\"removed\":[\"ws-1\"]}").Contains("\"ok\":false"), "A non-app shortcut could be removed");
        Check(Reserved.Action(Binds.SUPER, 0x4C) == "lock" && Reserved.Action(Binds.SUPER, 0x4B) == null, "Super+L is not the lock action");
        Console.WriteLine("PASS: core routing, origin, method, release selection, settings file updates, Windows notifications and live wallpaper entries");
    }

    static void WorkspaceOutlineTests() {
        var method = typeof(Slider).GetMethod("WorkspaceFocusHandle", BindingFlags.NonPublic | BindingFlags.Static);
        Check(method != null, "Workspace preview must select the arriving focus before the WM changes foreground");
        Func<string, IntPtr> focus = text => (IntPtr)method.Invoke(null, new object[] { new JavaScriptSerializer().DeserializeObject(text) });
        Check(focus(@"{""children"":[{""id"":""a"",""type"":""window"",""handle"":41},{""id"":""b"",""type"":""window"",""handle"":42}],""childFocusOrder"":[""b"",""a""]}") == new IntPtr(42), "Incoming focus followed layout order rather than workspace MRU");
        Check(focus(@"{""children"":[{""id"":""split"",""children"":[{""id"":""a"",""type"":""window"",""handle"":41},{""id"":""b"",""type"":""window"",""handle"":42}],""childFocusOrder"":[""b"",""a""]},{""id"":""c"",""type"":""window"",""handle"":43}],""childFocusOrder"":[""split"",""c""]}") == new IntPtr(42), "Incoming focus did not traverse the focused split");
        Check(focus(@"{""children"":[{""id"":""a"",""type"":""window"",""handle"":41}],""childFocusOrder"":[""removed"",""a""]}") == new IntPtr(41), "Removed focus-order IDs hid the incoming cue");
        Check(focus(@"{""children"":[{""id"":""a"",""type"":""window"",""handle"":41,""state"":{""type"":""minimized""}}],""childFocusOrder"":[""a""]}") == IntPtr.Zero, "A minimized preview must not get an outline");
        Check(focus(@"{""children"":[],""childFocusOrder"":[]}") == IntPtr.Zero, "Empty workspace borrowed the departing window's outline");
        Check((IntPtr)method.Invoke(null, new object[] { null }) == IntPtr.Zero, "Missing target workspace did not yield an empty preview");
        Check(focus(@"{""children"":[{""id"":""a"",""type"":""window"",""handle"":41}]}") == new IntPtr(41), "Missing focus order lost the available preview");
        var preview = typeof(Slider).GetMethod("WorkspacePreviewFocus", BindingFlags.NonPublic | BindingFlags.Static);
        Check(preview != null, "Swipe focus must follow the incoming workspace before commit");
        Func<double, bool, bool, IntPtr> swipeFocus = (progress, prev, next) => (IntPtr)preview.Invoke(null,
            new object[] { progress, new IntPtr(41), new IntPtr(42), new IntPtr(43), prev, next });
        Check(swipeFocus(0.01, true, true) == new IntPtr(43), "Next workspace cue appeared only at swipe completion");
        Check(swipeFocus(-0.01, true, true) == new IntPtr(42), "Reversing swipe direction kept the wrong cue");
        Check(swipeFocus(0, true, true) == new IntPtr(41), "Cancelled swipe did not restore the original cue");
        Check(swipeFocus(0.03, true, false) == new IntPtr(41), "Rubber band at the last workspace lost the original cue");
        Check(swipeFocus(-0.03, false, true) == new IntPtr(41), "Rubber band at the first workspace lost the original cue");
        Console.WriteLine("PASS: arriving workspace focus outline (MRU, nested splits, stale IDs, empty and minimized)");
    }
}
