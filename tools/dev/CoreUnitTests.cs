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
        NotificationActivationTests();
        DesktopPerformancePolicyTests();
        BlackboxAttachmentTests();
        CallbackTests(root);
        PipeWaitTests();
        InputLatencyTests(root);
        FocusSinkTests(root);
        LogWriterTests();
        MemoryLogTests(root);
        TempsFileTests();
        RounderRegionTests();
        WinIconTests();
        UninstallerTests(root);
        StartupCoverTests(root);
        Console.WriteLine(failures == 0 ? "PASS core unit tests" : failures + " failure(s)");
        return failures == 0 ? 0 : 1;
    }

    static void DesktopPerformancePolicyTests()
    {
        var cache = typeof(Dwindle).GetMethod("CacheWaitMs", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Check(cache != null, "connected cache has an event-driven wait policy");
        if (cache != null)
        {
            Check((int)cache.Invoke(null, new object[] { true, true }) == 30000, "connected idle cache queries at most once per 30 seconds");
            Check((int)cache.Invoke(null, new object[] { false, true }) == 2000, "disconnected cache retains recovery polling");
            Check((int)cache.Invoke(null, new object[] { true, false }) == 2000, "failed query recovers quickly despite a live event socket");
        }
        var snapshot = typeof(Dwindle).GetMethod("SnapshotEvent", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Check(snapshot != null, "focus bookkeeping does not trigger a geometry snapshot");
        if (snapshot != null)
        {
            Check(!(bool)snapshot.Invoke(null, new object[] { "focus_changed" }), "focus-only changes keep the existing geometry cache");
            foreach (var ev in new[] { "window_managed", "window_unmanaged", "focused_container_moved", "workspace_activated", "workspace_updated", "monitor_updated" })
                Check((bool)snapshot.Invoke(null, new object[] { ev }), "layout cache responds immediately to " + ev);
        }
        var pool = typeof(Slider).GetMethod("RingPoolTarget", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Check(pool != null, "ring reserve follows actual demand rather than twelve windows per monitor");
        if (pool != null)
        {
            Check((int)pool.Invoke(null, new object[] { false, 0 }) == 2 && (int)pool.Invoke(null, new object[] { true, 0 }) == 2, "idle monitor reserves two inactive sets and focus handoff sets");
            Check((int)pool.Invoke(null, new object[] { false, 7 }) == 8, "crowded workspace reserves an arriving window before reveal");
            Check((int)pool.Invoke(null, new object[] { false, 100 }) == 12 && (int)pool.Invoke(null, new object[] { true, 100 }) == 2, "reserve growth remains bounded after crowded workspaces");
        }
        var clip = typeof(Rounder).GetMethod("NeedsTileClip", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Check(clip != null, "tile clipping is distinct from optional corner rounding");
        if (clip != null)
        {
            var tile = new Native.RECT { Left=964, Top=45, Right=1915, Bottom=1075 };
            var full = new Native.RECT { Left=0, Top=0, Right=1920, Bottom=1080 };
            Check((bool)clip.Invoke(null,new object[]{ full,tile,true }), "monitor-sized video still needs clipping after decorative rounding gives up");
            Check(!(bool)clip.Invoke(null,new object[]{ tile,tile,true }), "fitting tiles do not repeatedly fight an app's optional rounding");
            Check(!(bool)clip.Invoke(null,new object[]{ full,tile,false }), "genuine fullscreen without a tile slot stays fullscreen");
        }
        var region = typeof(Rounder).GetMethod("RegionMatches", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        var due = typeof(Rounder).GetMethod("ClipRepairDue", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static);
        Check(region != null && due != null, "clip repair validates geometry and bounds callback bursts");
        if (region != null && due != null)
        {
            var expected = new Native.RECT { Left=10, Top=20, Right=110, Bottom=120 };
            var wrong = new Native.RECT { Left=0, Top=0, Right=1920, Bottom=1080 };
            Check(!(bool)region.Invoke(null,new object[]{2,wrong,2,expected}), "an app's full-window replacement region cannot bypass the tile clip");
            Check(!(bool)region.Invoke(null,new object[]{1,expected,1,expected}), "empty region is not a successful clip");
            Check((bool)region.Invoke(null,new object[]{2,expected,2,expected}), "valid rectangular clip is retained");
            Check(!(bool)due.Invoke(null,new object[]{101,100}) && (bool)due.Invoke(null,new object[]{116,100}), "repeated callbacks coalesce while repair resumes next frame");
            Check((bool)due.Invoke(null,new object[]{unchecked(int.MinValue+20), int.MaxValue-10}), "repair deadline survives tick count wrap");
        }
    }

    static void BlackboxAttachmentTests()
    {
        var saved = BugReports.CaptureBlackbox;
        try
        {
            string captured = "KARA KUTU: hata raporu istendi\n  işlemci: test\n  gpu: 3D";
            BugReports.CaptureBlackbox = why => captured;
            var result = new System.Web.Script.Serialization.JavaScriptSerializer().Deserialize<Dictionary<string, object>>(BugReports.File("blackbox"));
            Check((bool)result["ok"] && (string)result["text"] == captured, "blackbox attachment uses the complete capture, without racing an asynchronous or rotated log file");
            BugReports.CaptureBlackbox = why => null;
            result = new System.Web.Script.Serialization.JavaScriptSerializer().Deserialize<Dictionary<string, object>>(BugReports.File("blackbox"));
            Check(!(bool)result["ok"] && result.ContainsKey("error"), "a failed blackbox capture stays visible as an attachment error");
        }
        finally { BugReports.CaptureBlackbox = saved; }
    }

    static void NotificationActivationTests()
    {
        var protocol = ToastPayload.Parse("<toast activationType=\"protocol\" launch=\"nvidiaapp://route/#nvapp/rewards\"><visual><binding><text>Reward</text></binding></visual></toast>");
        var launch = typeof(ToastPayload).GetField("Launch");
        var type = typeof(ToastPayload).GetField("ActivationType");
        Check(launch != null && (string)launch.GetValue(protocol) == "nvidiaapp://route/#nvapp/rewards", "notification-specific launch target is preserved");
        Check(type != null && (string)type.GetValue(protocol) == "protocol", "notification activation contract is preserved");
        var conversation = ToastPayload.Parse("<toast launch=\"type=click&amp;tag=14317607570838471237\"/>");
        Check(launch != null && (string)launch.GetValue(conversation) == "type=click&tag=14317607570838471237", "opaque conversation click arguments are preserved without treating them as a URL");
        string delivered = null;
        Check(ToastActivation.Dispatch(protocol, "com.vendor.app", url => { delivered = url; return true; }, (id, context) => { throw new Exception("protocol routed as native activation"); }, id => { throw new Exception("protocol lost its destination"); })
            && delivered == "nvidiaapp://route/#nvapp/rewards", "protocol click opens the supplied destination with its fragment");
        Check(ToastActivation.Dispatch(conversation, "Vendor.App!App", url => { throw new Exception("opaque argument launched as URL"); }, (id, context) => { delivered = context; return true; }, id => false)
            && delivered == "type=click&tag=14317607570838471237", "native activation receives the exact conversation context");
        int calls = 0;
        Check(!ToastActivation.Dispatch(protocol, "com.vendor.app", url => false, (id, context) => false, id => { calls++; return true; }) && calls == 0, "failed deep link does not silently launch the app's unrelated home page");
        foreach (string target in new[] { "file:///C:/x.exe", "C:\\x.exe", "shell:AppsFolder\\x", "javascript:alert(1)", "data:text/plain,x", "https://example.com/\nrun", "cmd.exe /c start x" })
            Check(ToastActivation.ProtocolTarget(ToastPayload.Parse("<toast activationType=\"protocol\" launch=\"" + System.Security.SecurityElement.Escape(target).Replace("\n", "&#10;") + "\"/>")) == null, "unsafe notification target rejected: " + target);
        Check(!ToastActivation.Dispatch(ToastPayload.Parse("<toast activationType=\"background\" launch=\"delete=1\"/>"), "vendor.app", url => { calls++; return true; }, (id, context) => { calls++; return true; }, id => { calls++; return true; }) && calls == 0, "body click cannot replay background actions");
        Check(!ToastActivation.ValidSender(@"a\b") && !ToastActivation.ValidSender("x\0y") && ToastActivation.ValidSender("Vendor.App!App"), "sender IDs cannot alter shell or registry paths");
        string callback = "{3F3C2DFD-1EDC-4F47-8CDE-7567D9CE7C92}";
        Check(ToastActivation.ShortcutActivator("vendor.app", "Vendor.App", callback) == new Guid(callback), "matching shortcut supplies the app's notification callback");
        Check(ToastActivation.ShortcutActivator("vendor.app", "other.app", callback) == Guid.Empty, "another shortcut cannot receive this notification's context");
        Check(ToastActivation.ShortcutActivator("vendor.app", "vendor.app", "not-a-guid") == Guid.Empty, "invalid shortcut activator is ignored");
    }

    // Rounded corners: the region the rounder sets must count as its own on the next event. It did not (a rounded region's
    // box is a pixel smaller than its rectangle, a small window's region is not even rounded), so every event set it again,
    // and SetWindowRgn raises another location event: thousands of rounds per window per second, a core while idle.
    // The window lives on a desktop of its own: the running desktop's hooks and window manager never see it.
    [System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
    static extern IntPtr CreateDesktop(string name, IntPtr device, IntPtr mode, uint flags, uint access, IntPtr security);
    [System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true)] static extern bool SetThreadDesktop(IntPtr desk);
    [System.Runtime.InteropServices.DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desk);
    [System.Runtime.InteropServices.DllImport("gdi32.dll")] static extern int GetRgnBox(IntPtr rgn, out Native.RECT box);
    [System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)] static extern bool SetProp(IntPtr h, string name, IntPtr value);

    sealed class RegionProbe : System.Windows.Forms.Form
    {
        public int PosChanged;
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override void WndProc(ref System.Windows.Forms.Message m)
        {
            if (m.Msg == 0x0083 && m.WParam != IntPtr.Zero) { m.Result = IntPtr.Zero; return; } // WM_NCCALCSIZE: draws its own title bar
            if (m.Msg == 0x0047) PosChanged++;                                                    // WM_WINDOWPOSCHANGED: SetWindowRgn sends it
            base.WndProc(ref m);
        }
    }

    static void RounderRegionTests()
    {
        var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static;
        var make = typeof(Rounder).GetMethod("MakeRegion", flags);
        var shape = typeof(Rounder).GetMethod("RegionShape", flags);
        var matches = typeof(Rounder).GetMethod("RegionMatches", flags);
        var apply = typeof(Rounder).GetMethod("Apply", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
        Check(make != null && shape != null && matches != null && apply != null, "the rounder's region helpers exist");
        if (make == null || shape == null || matches == null || apply == null) return;

        // Every size from a sliver to a wide window, both shapes: Windows reports our region the way the rounder expects it.
        var sizes = Enumerable.Range(1, 64).Concat(new[] { 100, 333, 1000, 1919, 2560 }).ToArray();
        int unrecognised = 0, emptied = 0;
        foreach (bool square in new[] { true, false })
            foreach (int w in sizes)
                foreach (int hgt in sizes)
                {
                    IntPtr rgn = (IntPtr)make.Invoke(null, new object[] { square, 3, 5, 3 + w, 5 + hgt });
                    Native.RECT box;
                    int kind = GetRgnBox(rgn, out box);
                    Native.DeleteObject(rgn);
                    var args = new object[] { square, 3, 5, 3 + w, 5 + hgt, null };
                    int expected = (int)shape.Invoke(null, args);
                    if (kind <= 1) emptied++;
                    if (!(bool)matches.Invoke(null, new object[] { kind, box, expected, args[5] })) unrecognised++;
                }
        Check(unrecognised == 0, "the rounder does not recognise its own region at " + unrecognised + " sizes");
        Check(emptied == 0, "the rounder would hide " + emptied + " window sizes behind an empty region");

        string error = null, log = null;
        int first = -1, repeated = -1, repairs = -1, firstKind = 0, lastKind = 0, transient = -1, lasting = -1, lastingKind = 0;
        Native.RECT lastingBox = new Native.RECT();
        Native.RECT last = new Native.RECT();
        string oldLog = LogWriter.Path, tmp = Path.Combine(Path.GetTempPath(), "ll-rounder-test-" + Guid.NewGuid().ToString("N") + ".log");
        LogWriter.Path = tmp;
        var thread = new Thread(() =>
        {
            IntPtr desk = CreateDesktop("LLRounderTest" + Guid.NewGuid().ToString("N"), IntPtr.Zero, IntPtr.Zero, 0, 0x10000000 /*GENERIC_ALL*/, IntPtr.Zero);
            if (desk == IntPtr.Zero || !SetThreadDesktop(desk)) { error = "no test desktop: " + System.Runtime.InteropServices.Marshal.GetLastWin32Error(); return; }
            try
            {
                using (var f = new RegionProbe { StartPosition = System.Windows.Forms.FormStartPosition.Manual, Bounds = new System.Drawing.Rectangle(200, 150, 640, 420) })
                {
                    f.Show();
                    System.Windows.Forms.Application.DoEvents();
                    var rounder = new Rounder();
                    Action run = () => { apply.Invoke(rounder, new object[] { f.Handle }); System.Windows.Forms.Application.DoEvents(); };
                    Native.RECT box;
                    int start = f.PosChanged;
                    run();
                    first = f.PosChanged - start;
                    firstKind = Native.GetWindowRgnBox(f.Handle, out box);
                    // the location events its own region raises, and the 0.7 s sweep
                    start = f.PosChanged;
                    for (int i = 0; i < 20; i++) run();
                    repeated = f.PosChanged - start;
                    // an app that puts its own region back after every change: a few repairs, then it is left alone
                    repairs = 0;
                    for (int i = 0; i < 12; i++)
                    {
                        Native.SetWindowRgn(f.Handle, Native.CreateRectRgn(0, 0, 300, 200), true);
                        System.Windows.Forms.Application.DoEvents();
                        start = f.PosChanged;
                        run();
                        if (f.PosChanged > start) repairs++;
                    }
                    lastKind = Native.GetWindowRgnBox(f.Handle, out last);

                    // An app's own resize past its tile (the slot the window manager wrote): no clip while it comes back
                    // within the grace, the clip once it stays out
                    var tiler = new Rounder();
                    Action<System.Drawing.Rectangle> slot = s =>
                    {
                        SetProp(f.Handle, "LungeSlotLT", new IntPtr(unchecked((long)(((ulong)(uint)(s.Left + 0x40000000) << 32) | (uint)(s.Top + 0x40000000)))));
                        SetProp(f.Handle, "LungeSlotRB", new IntPtr(unchecked((long)(((ulong)(uint)(s.Right + 0x40000000) << 32) | (uint)(s.Bottom + 0x40000000)))));
                    };
                    Action tile = () => { apply.Invoke(tiler, new object[] { f.Handle }); System.Windows.Forms.Application.DoEvents(); };
                    Action<int> wait = ms => { var w = Stopwatch.StartNew(); while (w.ElapsedMilliseconds < ms) { System.Windows.Forms.Application.DoEvents(); Thread.Sleep(5); } };
                    Native.SetWindowRgn(f.Handle, IntPtr.Zero, true);
                    var whole = f.Bounds;
                    var smaller = new System.Drawing.Rectangle(whole.Left, whole.Top, whole.Width / 2, whole.Height / 2);
                    slot(whole); tile();
                    start = f.PosChanged;
                    slot(smaller); tile();      // past its tile
                    slot(whole); tile();        // back within a frame
                    wait(150);
                    transient = f.PosChanged - start;
                    start = f.PosChanged;
                    slot(smaller); tile();      // past its tile, and it stays
                    wait(150);
                    lasting = f.PosChanged - start;
                    lastingKind = Native.GetWindowRgnBox(f.Handle, out lastingBox);
                    f.Close();
                }
            }
            catch (Exception ex) { error = ex.GetBaseException().Message; }
            finally { CloseDesktop(desk); }
        });
        thread.SetApartmentState(ApartmentState.MTA); // an STA thread already owns a COM window: it could not change desktops
        thread.Start();
        bool done = thread.Join(20000);
        LogWriter.Flush(3000);
        try { log = File.Exists(tmp) ? File.ReadAllText(tmp) : ""; File.Delete(tmp); } catch { }
        LogWriter.Path = oldLog;
        Check(done, "the rounder's window test did not finish");
        Check(error == null, "the rounder's window test failed: " + error);
        if (!done || error != null) return;
        Check(first > 0 && firstKind == 3, "a window with its own title bar was not rounded (" + first + " moves, region kind " + firstKind + ")");
        Check(repeated == 0, "the rounder set the same region again " + repeated + " times: every location event would loop");
        Check(repairs >= 1 && repairs <= 5, "an app replacing the region was repaired " + repairs + " times out of 12 (want a few, then stop)");
        Check(lastKind == 2 && last.Right == 300 && last.Bottom == 200, "the rounder took away the app's own region after giving up");
        Check(log != null && log.Contains("gave up rounding"), "giving up was not logged");
        Check(transient == 0, "a window back in its tile within the grace was clipped anyway (" + transient + " region changes): it vanishes for a frame");
        Check(lasting > 0 && lastingKind > 1 && lastingBox.Right <= 640 / 2 + 1 && lastingBox.Bottom <= 420 / 2 + 1, "a window staying past its tile was not clipped to it (" + lasting + " changes, region kind " + lastingKind + ", box " + lastingBox.Right + "x" + lastingBox.Bottom + ")");
    }

    // Window icons follow the app's identity (AppUserModelID), not its exe: every Store app runs in
    // ApplicationFrameHost.exe, and the icon cached for the first one (Calculator) was given to all of them (Roblox).
    // Two hidden windows of this one process, each with another app's identity, must get their own apps' icons.
    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)] struct TestKey { public Guid fmtid; public uint pid; }
    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)] sealed class TestVariant { public ushort vt; public ushort r1, r2, r3; public IntPtr p; public IntPtr p2; }
    [System.Runtime.InteropServices.ComImport, System.Runtime.InteropServices.Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), System.Runtime.InteropServices.InterfaceType(System.Runtime.InteropServices.ComInterfaceType.InterfaceIsIUnknown)]
    interface ITestPropertyStore
    {
        [System.Runtime.InteropServices.PreserveSig] int GetCount(out uint count);
        [System.Runtime.InteropServices.PreserveSig] int GetAt(uint index, out TestKey key);
        [System.Runtime.InteropServices.PreserveSig] int GetValue(ref TestKey key, [System.Runtime.InteropServices.Out] TestVariant value);
        [System.Runtime.InteropServices.PreserveSig] int SetValue(ref TestKey key, [System.Runtime.InteropServices.In] TestVariant value);
        [System.Runtime.InteropServices.PreserveSig] int Commit();
    }
    [System.Runtime.InteropServices.DllImport("shell32.dll")] static extern int SHGetPropertyStoreForWindow(IntPtr h, ref Guid iid, [System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.Interface)] out ITestPropertyStore store);

    static void SetAppId(IntPtr h, string id)
    {
        var iid = new Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99");
        ITestPropertyStore store;
        if (SHGetPropertyStoreForWindow(h, ref iid, out store) != 0) return;
        var key = new TestKey { fmtid = new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), pid = 5 };
        var v = new TestVariant { vt = (ushort)(id == null ? 0 : 31), p = id == null ? IntPtr.Zero : System.Runtime.InteropServices.Marshal.StringToCoTaskMemUni(id) };
        store.SetValue(ref key, v);
        store.Commit();
        if (v.p != IntPtr.Zero) System.Runtime.InteropServices.Marshal.FreeCoTaskMem(v.p);
        System.Runtime.InteropServices.Marshal.ReleaseComObject(store);
    }

    static void WinIconTests()
    {
        const string settings = "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel", explorer = "Microsoft.Windows.Explorer";
        string a = WinIcons.AppIcon(settings), b = WinIcons.AppIcon(explorer);
        Check(a != null && b != null && a != b, "apps' own icons (Settings, File Explorer) were not found or are the same");
        using (var one = new System.Windows.Forms.Form())
        using (var two = new System.Windows.Forms.Form())
        {
            SetAppId(one.Handle, settings);
            SetAppId(two.Handle, explorer);
            Check(WinIcons.AppId(one.Handle) == settings, "a window's app identity is not read");
            string first = WinIcons.For(one.Handle), second = WinIcons.For(two.Handle);
            Check(first == a && second == b, "two apps of one exe got " + (first == second ? "the same icon" : "wrong icons"));
            SetAppId(one.Handle, null);
            SetAppId(two.Handle, null);
        }
    }

    // The uninstaller: which extras it offers comes from the install record, its steps from uninstall.ps1's steps.txt,
    // its pages are drawn (for a look), and its window repair touches only what Logical Lunge left on other windows.
    [System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateWindowEx(int ex, string cls, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
    [System.Runtime.InteropServices.DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
    [System.Runtime.InteropServices.DllImport("user32.dll")] static extern bool GetLayeredWindowAttributes(IntPtr h, out uint key, out byte alpha, out uint flags);
    [System.Runtime.InteropServices.DllImport("gdi32.dll")] static extern IntPtr CreateEllipticRgn(int l, int t, int r, int b);

    static void UninstallerTests(string root)
    {
        var extras = Uninstaller.InstalledExtras(@"{""installed"":[""path:C:\\Program Files\\LogicalLunge\\tools\\bin"",""terminal"",""runtime-shell-settings"",""everything"",""terminal""]}");
        Check(string.Join(",", extras.ToArray()) == "terminal,everything", "extras from the install record: " + string.Join(",", extras.ToArray()));
        Check(Uninstaller.InstalledExtras("not json").Count == 0 && Uninstaller.InstalledExtras(null).Count == 0, "an unreadable install record offers no extras");
        string result, message;
        var steps = Uninstaller.ReadSteps("\uFEFFshell run\r\ntaskbar run\ntaskbar done\nmessage some files are in use\nresult done\n", out result, out message);
        Check(steps["taskbar"] == "done" && steps["shell"] == "run" && result == "done" && message == "some files are in use", "steps.txt: the last state of each step, the result and its message");

        string shots = Path.Combine(root, @"build\tests\uninstall-shots");
        UninstallCard.Shots(shots);
        foreach (var name in new[] { "1-confirm", "2-confirm-keep", "3-confirm-no-extras", "4-progress", "5-done" })
            Check(File.Exists(Path.Combine(shots, name + ".png")), "the uninstaller's " + name + " page was not drawn");

        string error = null;
        bool llRemoved = false, appKept = false, slotRemoved = false, moved = false, onScreenKept = false, revealed = false;
        var thread = new Thread(() =>
        {
            IntPtr desk = CreateDesktop("LLRepairTest" + Guid.NewGuid().ToString("N"), IntPtr.Zero, IntPtr.Zero, 0, 0x10000000, IntPtr.Zero);
            if (desk == IntPtr.Zero || !SetThreadDesktop(desk)) { error = "no test desktop"; return; }
            try
            {
                using (var a = new RegionProbe { StartPosition = System.Windows.Forms.FormStartPosition.Manual, Bounds = new System.Drawing.Rectangle(100, 100, 500, 400) })
                using (var b = new RegionProbe { StartPosition = System.Windows.Forms.FormStartPosition.Manual, Bounds = new System.Drawing.Rectangle(150, 150, 500, 400) })
                using (var c = new RegionProbe { StartPosition = System.Windows.Forms.FormStartPosition.Manual, Bounds = new System.Drawing.Rectangle(200, 200, 500, 400) })
                {
                    a.Show(); b.Show(); c.Show();
                    System.Windows.Forms.Application.DoEvents();
                    Native.RECT box;
                    // the rounder's own region (set by the rounder) on a window it no longer tiles: goes
                    typeof(Rounder).GetMethod("Apply", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance).Invoke(new Rounder(), new object[] { a.Handle });
                    bool rounded = Native.GetWindowRgnBox(a.Handle, out box) > 1;
                    WindowRepair.Region(a.Handle, false);
                    llRemoved = rounded && Native.GetWindowRgnBox(a.Handle, out box) == 0;
                    // an app's own shape: stays
                    Native.SetWindowRgn(b.Handle, CreateEllipticRgn(0, 0, 500, 400), true);
                    WindowRepair.Region(b.Handle, false);
                    appKept = Native.GetWindowRgnBox(b.Handle, out box) > 1;
                    // a tiled window (slot mark) with any region: goes
                    Native.SetWindowRgn(c.Handle, Native.CreateRectRgn(0, 0, 250, 200), true);
                    SetProp(c.Handle, "LungeSlotLT", new IntPtr(0x4000000040000000));
                    WindowRepair.Region(c.Handle, true);
                    slotRemoved = Native.GetWindowRgnBox(c.Handle, out box) == 0;
                    // a tiled window left off every screen comes into view; one already on a screen stays
                    onScreenKept = !WindowRepair.IntoView(c.Handle);
                    Native.SetWindowPos(c.Handle, IntPtr.Zero, -20000, -20000, 500, 400, 0x0004 | 0x0010);
                    moved = WindowRepair.IntoView(c.Handle);
                    Native.RECT r; Native.GetWindowRect(c.Handle, out r);
                    moved = moved && r.Left > -20000 && System.Windows.Forms.Screen.FromRectangle(System.Drawing.Rectangle.FromLTRB(r.Left, r.Top, r.Right, r.Bottom)).WorkingArea.Contains(r.Left + 10, r.Top + 10);
                    a.Close(); b.Close(); c.Close();
                }
                // a dialog box left fully transparent while it was being caught: shown again
                IntPtr dlg = CreateWindowEx(0x00080000 /*LAYERED*/, "#32770", "test", 0x90000000 /*POPUP|VISIBLE*/, 100, 100, 300, 200, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
                if (dlg != IntPtr.Zero)
                {
                    Native.SetLayeredWindowAttributes(dlg, 0, 0, 0x2);
                    WindowRepair.Reveal(dlg);
                    uint key, flags; byte alpha;
                    revealed = GetLayeredWindowAttributes(dlg, out key, out alpha, out flags) && alpha == 255;
                    DestroyWindow(dlg);
                }
                else error = "no dialog window: " + System.Runtime.InteropServices.Marshal.GetLastWin32Error();
            }
            catch (Exception ex) { error = ex.GetBaseException().Message; }
            finally { CloseDesktop(desk); }
        });
        thread.SetApartmentState(ApartmentState.MTA);
        thread.Start();
        Check(thread.Join(20000), "the window repair test did not finish");
        Check(error == null, "the window repair test failed: " + error);
        Check(llRemoved, "the rounder's region on a window was not removed");
        Check(appKept, "an app's own window region was removed");
        Check(slotRemoved, "a tiled window's region was not removed");
        Check(onScreenKept, "a tiled window on a screen was moved");
        Check(moved, "a tiled window off every screen was not brought into view");
        Check(revealed, "a transparent dialog box was not shown again");
    }

    // The startup cover: its first frame comes before anything slow, it leaves only when the core says every part is
    // there, it has its own logon task, and the wallpaper doesn't count it as hiding the desktop.
    static void StartupCoverTests(string root)
    {
        var spinner = typeof(Splash).GetNestedType("Cover", System.Reflection.BindingFlags.NonPublic).GetMethod("Spinner");
        float lastStart = -1; int moved = 0;
        for (double ms = 0; ms < 8000; ms += 16)
        {
            var args = new object[] { ms, 0f, 0f };
            spinner.Invoke(null, args);
            float start = (float)args[1], sweep = (float)args[2];
            if (!(sweep >= 18 && sweep <= 268.01 && start >= 0 && start < 360)) { Check(false, "the spinner's arc went out of shape at " + ms + " ms: " + start + "/" + sweep); break; }
            if (lastStart >= 0 && Math.Abs(start - lastStart) > 0.01) moved++;
            lastStart = start;
        }
        Check(moved > 400, "the spinner does not turn");

        string xml = SplashTask.Xml("S-1-5-21-1", @"C:\Program Files\LogicalLunge\lunge.exe", @"C:\Users\x");
        foreach (var part in new[] { "<LogonTrigger>", "<UserId>S-1-5-21-1</UserId>", "<RunLevel>LeastPrivilege</RunLevel>", "<Priority>1</Priority>",
                                      "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>", "<Arguments>--splash</Arguments>" })
            Check(xml.Contains(part), "the splash task lacks " + part);
        try { new System.Xml.XmlDocument().LoadXml(xml.Substring(xml.IndexOf("<Task", StringComparison.Ordinal))); }
        catch (Exception ex) { Check(false, "the splash task is not valid XML: " + ex.Message); }

        string core = File.ReadAllText(Path.Combine(root, "core", "lunge.cs"));
        int run = core.IndexOf("public static void Run()\n    {\n        long sinceLogon", StringComparison.Ordinal);
        if (run < 0) run = core.IndexOf("public static void Run()\r\n    {\r\n        long sinceLogon", StringComparison.Ordinal);
        Check(run > 0, "the cover's Run() was not found");
        int show = core.IndexOf("f.Show();", run, StringComparison.Ordinal), wall = core.IndexOf("Wallpaper();", run, StringComparison.Ordinal);
        int lang = core.IndexOf("I18n.T(", run, StringComparison.Ordinal);
        Check(show > 0 && show < wall && show < lang, "the cover waits for the wallpaper or the language before its first frame");
        Check(core.Contains("state.Ready = CoreSaysReady();") && core.Contains("target == \"/desktop-ready\""), "the cover does not ask the core whether the desktop is ready");
        Check(core.Contains("Text = Names.StartupCover;"), "the cover windows are not named");
        // The time since logon decides "starting" or "restarting" and goes into the log: this session's, from Windows
        long sinceLogon = (long)typeof(Splash).GetMethod("SinceLogonMs", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static).Invoke(null, null);
        long explorer = -1;
        foreach (var p in Process.GetProcessesByName("explorer"))
            try { if (p.SessionId == Process.GetCurrentProcess().SessionId) explorer = Math.Max(explorer, (long)(DateTime.Now - p.StartTime).TotalMilliseconds); } catch { }
        Check(sinceLogon > 0 && (explorer < 0 || sinceLogon >= explorer - 5000), "the time since logon is wrong: " + sinceLogon + " ms (Explorer runs for " + explorer + " ms)");
        string ui = File.ReadAllText(Path.Combine(root, "shell", "crates", "live-wallpaper", "src", "ui.rs"));
        var named = Regex.Match(core, "StartupCover = \"([^\"]+)\"").Groups[1].Value;
        Check(named.Length > 0 && ui.Contains("const STARTUP_COVER: &str = \"" + named + "\";"), "the wallpaper skips a different title than the cover's");
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
