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
        Check(Launcher.Describe(Launcher.NOT_FOUND).Length > 0, "Windows' error text must come back");
        Check(Request("GET", "/launch?file=cmd.exe", "http://127.0.0.1:6124").Contains("405") && Request("POST", "/launch?file=cmd.exe", "https://example.invalid").Contains("403"), "Launches must be local POSTs");
        Check(Request("POST", "/launch?file=", "http://127.0.0.1:6124").Contains("400") && Request("POST", "/launch?file=a&verb=delete", "http://127.0.0.1:6124").Contains("400"), "A bad launch must be refused");
        Check(Request("POST", "/launch?file=" + Uri.EscapeDataString(System.IO.Path.Combine(missingDir, "a.exe")), "http://127.0.0.1:6124").Contains("422"), "A launch that cannot open must answer 422 (our card)");
    }

    static void Main() {
        TakeoverTests();
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
        Check(Request("GET", "/qs/bt", "http://127.0.0.1:6124").Contains("405"), "The Bluetooth device list must require POST");
        Check(Request("POST", "/qs/radio?kind=wifi&state=Maybe", "http://127.0.0.1:6124").Contains("400"), "A radio accepted an invalid state");
        Check(Request("POST", "/qs/awake?v=1", "https://example.invalid").Contains("403"), "Keep-awake accepted a foreign origin");
        Check(Request("POST", "/qs/nope", "http://127.0.0.1:6124").Contains("404"), "An unknown quick setting was served");
        string radios = Request("POST", "/qs/radios", "http://127.0.0.1:6124");
        Check(radios.Contains("200 OK") && radios.Contains("\"wifi\"") && radios.Contains("\"bluetooth\""), "Radios were not served");
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
}
