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
    static string Request(string method, string target, string origin) {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        try {
            using (var client = new TcpClient()) {
                client.Connect((IPEndPoint)listener.LocalEndpoint);
                using (var accepted = listener.AcceptTcpClient()) {
                    var stream = client.GetStream(); stream.ReadTimeout = 1000;
                    var bytes = Encoding.ASCII.GetBytes(method + " " + target + " HTTP/1.1\r\nHost: localhost\r\nOrigin: " + origin + "\r\nContent-Length: 0\r\n\r\n");
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
        Console.WriteLine("PASS: core routing, origin, method and release selection");
    }
}
