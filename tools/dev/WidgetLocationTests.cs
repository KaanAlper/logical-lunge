using System;
using System.Collections.Generic;
using System.Web.Script.Serialization;

static class WidgetLocationTests
{
    static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }
    static string Request(string target, string extraHeaders, string method = "POST")
    {
        // An isolated ephemeral port tests the real HTTP parser/routes without
        // starting the desktop or touching the installed core's port.
        var listener = new System.Net.Sockets.TcpListener(System.Net.IPAddress.Loopback, 0);
        listener.Start();
        int port = ((System.Net.IPEndPoint)listener.LocalEndpoint).Port;
        var server = new System.Threading.Thread(() => {
            using (var accepted = listener.AcceptTcpClient())
                typeof(Toasts).GetMethod("Accept", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static).Invoke(null, new object[] { accepted });
        });
        server.IsBackground = true; server.Start();
        try
        {
            using (var client = new System.Net.Sockets.TcpClient("127.0.0.1", port))
            {
                client.ReceiveTimeout = 3000;
                using (var stream = client.GetStream())
                {
                    var bytes = System.Text.Encoding.ASCII.GetBytes(method + " " + target + " HTTP/1.1\r\n" + extraHeaders + "Content-Length: 0\r\nConnection: close\r\n\r\n");
                    stream.Write(bytes, 0, bytes.Length);
                    using (var reader = new System.IO.StreamReader(stream)) return reader.ReadToEnd();
                }
            }
        }
        finally { listener.Stop(); server.Join(3000); }
    }
    static void Main(string[] args)
    {
        var json = new JavaScriptSerializer();
        var countries = WidgetLocations.Countries("b", "tr");
        Check(countries.Count > 0, "Single letter country completion is empty");
        Check(WidgetLocations.Countries("Alm", "tr").Exists(c => (string)c["code"] == "DE"), "Localized country prefix is missing");
        Check(WidgetLocations.Countries("Alle", "fr").Exists(c => (string)c["code"] == "DE" && (string)c["name"] == "Allemagne"), "Country labels ignore chosen UI language");
        Check(WidgetLocations.CountryCode("tr") == "TR" && WidgetLocations.CountryCode("XX") == null, "Invalid country accepted");
        Check(WidgetLocations.Fold("İSTANBUL") == WidgetLocations.Fold("istanbul"), "Turkish dotted I does not match");
        string fixture = "{\"features\":[{\"properties\":{\"name\":\"Beşiktaş\",\"countrycode\":\"TR\",\"country\":\"Türkiye\",\"city\":\"İstanbul\",\"state\":\"İstanbul\"},\"geometry\":{\"coordinates\":[29.01,41.04]}},{\"properties\":{\"name\":\"Bursa\",\"countrycode\":\"TR\",\"state\":\"Bursa\"},\"geometry\":{\"coordinates\":[29.1,40.2]}},{\"properties\":{\"name\":\"Berlin\",\"countrycode\":\"DE\"},\"geometry\":{\"coordinates\":[13.4,52.5]}}]}";
        var result = WidgetLocations.ParsePhoton(fixture, "TR", "İstanbul", true);
        Check(result.Count == 1 && (string)result[0]["district"] == "Beşiktaş", "District escaped country/city scope");
        Check((double)result[0]["latitude"] == 41.04 && (double)result[0]["longitude"] == 29.01, "GeoJSON coordinates reversed");
        Check(WidgetLocations.ParsePhoton("{\"features\":[{\"properties\":{\"name\":\"Bad\",\"countrycode\":\"TR\"},\"geometry\":{\"coordinates\":[999,999]}}]}", "TR", "", false).Count == 0, "Impossible coordinates accepted");
        Check(WidgetLocations.ValidCoordinates("41.01", "29.02") && !WidgetLocations.ValidCoordinates("NaN", "29") && !WidgetLocations.ValidCoordinates("91", "29"), "Coordinate validation failed");
        Check(WidgetLocations.ParseMeteo("{\"results\":[{\"name\":\"Paris\",\"country_code\":\"FR\",\"latitude\":48.8,\"longitude\":2.3},{\"name\":\"Bolu\",\"country_code\":\"TR\",\"latitude\":40.7,\"longitude\":31.6}]}", "TR", "", false).Count == 1, "Fallback lost country scope");
        var weather = WidgetLocations.ParseForecast("{\"current\":{\"temperature_2m\":18,\"weather_code\":2,\"is_day\":1},\"daily\":{\"temperature_2m_max\":[20],\"temperature_2m_min\":[12]}}", "Bolu", false);
        Check((double)weather["temp"] == 18 && (string)weather["place"] == "Bolu", "Forecast incorrectly decoded");
        Check(json.Serialize(weather).Contains("\"day\":true"), "Forecast day is not boolean");
        string countryReply = Request("/widgets/countries?q=Alm&language=tr", "Host: localhost:6131\r\nOrigin: http://127.0.0.1:6124\r\n");
        Check(countryReply.StartsWith("HTTP/1.1 200") && countryReply.Contains("\"code\":\"DE\""), "Country route was not dispatched to JSON API");
        Check(Request("/widgets/countries", "Host: localhost:6131\r\nOrigin: https://example.com\r\n").StartsWith("HTTP/1.1 403"), "Widget API accepted foreign browser origin");
        Check(Request("/widgets/countries", "Host: example.com:6131\r\n").StartsWith("HTTP/1.1 403"), "Widget API accepted DNS rebinding host");
        Check(Request("/widgets/countries", "Host: localhost:6131\r\n", "GET").StartsWith("HTTP/1.1 405"), "An external img/link GET can trigger widget upstream work");
        Check(Request("/widgets/places?q=b&countryCode=XX&kind=city", "Host: localhost:6131\r\n").Contains("\"error\""), "Invalid country did not return an actionable JSON error");
        if (args.Length > 0 && args[0] == "--live")
        {
            var live = json.Deserialize<Dictionary<string, object>>(WidgetLocations.Handle("/widgets/places?q=b&countryCode=TR&kind=city"));
            Check(!live.ContainsKey("error"), "Live single-letter lookup failed: " + json.Serialize(live));
            Check(((System.Collections.IList)live["results"]).Count > 0, "Live single-letter lookup empty");
            var district = json.Deserialize<Dictionary<string, object>>(WidgetLocations.Handle("/widgets/places?q=b&countryCode=TR&kind=district&city=Istanbul&latitude=41.01&longitude=29.02"));
            Check(!district.ContainsKey("error") && ((System.Collections.IList)district["results"]).Count > 0, "Live single-letter district lookup failed");
            var report = json.Deserialize<Dictionary<string, object>>(WidgetLocations.Handle("/widgets/weather?latitude=41.01&longitude=29.02&place=Istanbul"));
            Check(!report.ContainsKey("error") && report.ContainsKey("temp"), "Live weather coordinates failed");
            // Poison the actual cached response: ordinary polls should reuse it,
            // but explicit Refresh must fetch and replace that same cache entry.
            var flags = System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static;
            var cache = (System.Collections.IDictionary)typeof(WidgetLocations).GetField("cache", flags).GetValue(null);
            string key = "/widgets/weather?latitude=41.01&longitude=29.02&place=Istanbul&";
            object cached = cache[key];
            Check(cached != null, "Weather response did not populate canonical cache key");
            cached.GetType().GetField("Body").SetValue(cached, "{\"temp\":-999}");
            string reordered = "/widgets/weather?place=Istanbul&longitude=29.02&latitude=41.01";
            Check(WidgetLocations.Handle(reordered).Contains("-999"), "Query order bypassed ordinary weather cache");
            string refreshed = WidgetLocations.Handle(reordered + "&refresh=1");
            var refreshedReport = json.Deserialize<Dictionary<string, object>>(refreshed);
            Check(!refreshedReport.ContainsKey("error") && refreshedReport.ContainsKey("temp") && !refreshed.Contains("-999"), "Manual refresh reused stale weather cache");
            Check(WidgetLocations.Handle(reordered) == refreshed, "Manual refresh failed to replace the ordinary poll cache");
        }
        Console.WriteLine("Widget location tests passed");
    }
}
