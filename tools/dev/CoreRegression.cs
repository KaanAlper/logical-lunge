// Compiled with core/lunge.cs and /main:CoreRegression. Never starts the desktop.
using System;
using System.Collections.Generic;
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
    static void Main() {
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
        string notes = Request("POST", "/notifications", "http://127.0.0.1:6124");
        Check(notes.Contains("200 OK") && notes.Contains("\"items\"") && notes.Contains("\"icons\""), "Notification list was not served");
        Check(Request("POST", "/notifications", "https://example.invalid").Contains("403"), "Notification list accepted a foreign origin");
        string dir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-core-test-" + Guid.NewGuid().ToString("N"));
        System.IO.Directory.CreateDirectory(dir);
        try {
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
        Console.WriteLine("PASS: core routing, origin, method, release selection, settings file updates and Windows notifications");
    }
}
