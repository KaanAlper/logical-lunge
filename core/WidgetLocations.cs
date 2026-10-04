using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Net;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

// The two widget UIs share country/place lookup and weather data. All network work
// runs on the HTTP worker, never on a desktop/UI thread. Hosts are fixed, replies
// bounded and cached; typing cannot create unlimited concurrent upstream requests.
static class WidgetLocations
{
    const int MAX_BODY = 2 * 1024 * 1024, MAX_CACHE = 128;
    sealed class Cached { public DateTime Until; public string Body; }
    static readonly object gate = new object();
    static readonly Dictionary<string, Cached> cache = new Dictionary<string, Cached>();
    static readonly Semaphore slots = new Semaphore(3, 3);
    static readonly Dictionary<string, string> localizedNames = new Dictionary<string, string>();
    static readonly Dictionary<string, RegionInfo> regions = Regions();
    static readonly Dictionary<string, Dictionary<string, string>> countryNames = new Dictionary<string, Dictionary<string, string>>();
    static readonly CultureInfo invariant = CultureInfo.InvariantCulture;
    static JavaScriptSerializer Json() { return new JavaScriptSerializer { MaxJsonLength = MAX_BODY }; }

    static Dictionary<string, RegionInfo> Regions()
    {
        var result = new Dictionary<string, RegionInfo>(StringComparer.OrdinalIgnoreCase);
        foreach (var culture in CultureInfo.GetCultures(CultureTypes.SpecificCultures))
            try {
                var region = new RegionInfo(culture.Name);
                if (region.TwoLetterISORegionName.Length != 2) continue;
                result[region.TwoLetterISORegionName] = region;
                var name = new StringBuilder(256);
                if (GetLocaleInfoEx(culture.Name, 6, name, name.Capacity) > 0) localizedNames[region.TwoLetterISORegionName] = name.ToString();
            } catch (ArgumentException) { }
        return result;
    }
    [System.Runtime.InteropServices.DllImport("kernel32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
    static extern int GetLocaleInfoEx(string locale, uint type, StringBuilder value, int count);
    [System.Runtime.InteropServices.DllImport("icu.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    static extern IntPtr uldn_open([System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.LPStr)] string locale, int dialect, ref int status);
    [System.Runtime.InteropServices.DllImport("icu.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    static extern int uldn_regionDisplayName(IntPtr display, [System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.LPStr)] string region,
        [System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.LPWStr)] StringBuilder result, int capacity, ref int status);
    [System.Runtime.InteropServices.DllImport("icu.dll", CallingConvention = System.Runtime.InteropServices.CallingConvention.Cdecl)]
    static extern void uldn_close(IntPtr display);
    static Dictionary<string, string> Names(string language)
    {
        try { language = CultureInfo.GetCultureInfo(language).Name; } catch (ArgumentException) { language = CultureInfo.CurrentUICulture.Name; }
        lock (gate) { Dictionary<string, string> hit; if (countryNames.TryGetValue(language, out hit)) return hit; }
        var names = new Dictionary<string, string>(localizedNames);
        IntPtr display = IntPtr.Zero;
        try
        {
            int status = 0; display = uldn_open(language, 0, ref status);
            if (display != IntPtr.Zero && status <= 0)
                foreach (var region in regions.Keys)
                {
                    var name = new StringBuilder(256); status = 0;
                    if (uldn_regionDisplayName(display, region, name, name.Capacity, ref status) > 0 && status <= 0) names[region] = name.ToString();
                }
        }
        catch (DllNotFoundException) { }
        catch (EntryPointNotFoundException) { }
        finally { if (display != IntPtr.Zero) uldn_close(display); }
        lock (gate) { if (countryNames.Count >= 16) countryNames.Clear(); countryNames[language] = names; }
        return names;
    }
    public static string CountryCode(string code)
    {
        code = (code ?? "").Trim().ToUpperInvariant();
        return regions.ContainsKey(code) ? code : null;
    }
    public static string Fold(string text)
    {
        var result = new StringBuilder();
        foreach (char c in (text ?? "").Replace('İ', 'I').Replace('ı', 'i').Normalize(NormalizationForm.FormD))
            if (CharUnicodeInfo.GetUnicodeCategory(c) != UnicodeCategory.NonSpacingMark) result.Append(char.ToLowerInvariant(c));
        return result.ToString().Normalize(NormalizationForm.FormC);
    }
    public static List<Dictionary<string, object>> Countries(string query, string language)
    {
        string prefix = Fold(query.Trim());
        var names = Names(language ?? "");
        var results = new List<Dictionary<string, object>>();
        foreach (var entry in regions)
        {
            var r = entry.Value;
            string name;
            if (!names.TryGetValue(entry.Key, out name)) name = r.DisplayName;
            if ((language ?? "").StartsWith("en")) name = r.EnglishName;
            // ICU follows the selected app language; NLS is a fallback on older
            // Windows. Native/English names and ISO codes remain searchable.
            if (entry.Key == "TR" && (language ?? "").StartsWith("tr")) name = "Türkiye";
            if (prefix.Length == 0 || Fold(name).StartsWith(prefix) || Fold(r.EnglishName).StartsWith(prefix)
                || Fold(r.NativeName).StartsWith(prefix) || Fold(entry.Key).StartsWith(prefix))
                results.Add(new Dictionary<string, object> { { "code", entry.Key }, { "name", name }, { "englishName", r.EnglishName } });
        }
        results.Sort((a, b) => string.Compare((string)a["name"], (string)b["name"], StringComparison.CurrentCultureIgnoreCase));
        return results;
    }
    static Dictionary<string, string> Query(string target)
    {
        var result = new Dictionary<string, string>(StringComparer.Ordinal);
        int start = target.IndexOf('?');
        if (start < 0) return result;
        foreach (var part in target.Substring(start + 1).Split('&'))
        {
            int eq = part.IndexOf('=');
            if (eq > 0) result[part.Substring(0, eq)] = Uri.UnescapeDataString(part.Substring(eq + 1).Replace('+', ' '));
        }
        return result;
    }
    static string Get(Dictionary<string, string> q, string key) { string value; return q.TryGetValue(key, out value) ? value.Trim() : ""; }
    static string Text(Dictionary<string, object> v, string key) { object value; return v != null && v.TryGetValue(key, out value) ? value as string ?? "" : ""; }
    static object Value(Dictionary<string, object> v, string key) { object value; return v != null && v.TryGetValue(key, out value) ? value : null; }
    static object[] Array(object value)
    {
        var list = value as System.Collections.IList;
        if (list == null) return new object[0];
        var result = new object[list.Count]; list.CopyTo(result, 0); return result;
    }
    static double Number(object value)
    {
        if (value == null || value is bool) return double.NaN;
        double result;
        return double.TryParse(Convert.ToString(value, invariant), NumberStyles.Float, invariant, out result) ? result : double.NaN;
    }
    public static bool ValidCoordinates(string latitude, string longitude) { return ValidCoordinates(Number(latitude), Number(longitude)); }
    static bool ValidCoordinates(double latitude, double longitude) { return latitude >= -90 && latitude <= 90 && longitude >= -180 && longitude <= 180; }
    static Dictionary<string, object> Place(string name, string countryCode, string country, string city, string district, double lat, double lon)
    {
        return new Dictionary<string, object> {
            { "name", name }, { "countryCode", countryCode }, { "country", country }, { "city", city }, { "district", district },
            { "latitude", lat }, { "longitude", lon }, { "label", name + (city.Length > 0 && Fold(city) != Fold(name) ? " · " + city : "") + " · " + country }
        };
    }
    public static List<Dictionary<string, object>> ParsePhoton(string body, string countryCode, string city, bool district)
    {
        var root = Json().Deserialize<Dictionary<string, object>>(body);
        var results = new List<Dictionary<string, object>>();
        var seen = new HashSet<string>();
        foreach (var item in Array(Value(root, "features")))
        {
            var f = item as Dictionary<string, object>;
            var p = Value(f, "properties") as Dictionary<string, object>;
            var g = Value(f, "geometry") as Dictionary<string, object>;
            var xy = Array(Value(g, "coordinates"));
            string code = Text(p, "countrycode").ToUpperInvariant(), name = Text(p, "name");
            if (code != countryCode || name.Length == 0 || xy == null || xy.Length < 2) continue;
            string keyType = Text(p, "osm_key"), placeType = Text(p, "osm_value");
            if (keyType.Length > 0 && keyType != "place" && keyType != "boundary") continue;
            if (placeType == "square" || placeType == "archipelago") continue;
            double lon = Number(xy[0]), lat = Number(xy[1]);
            if (!ValidCoordinates(lat, lon)) continue;
            if (district && !(Fold(Text(p, "city")) == Fold(city) || Fold(Text(p, "state")) == Fold(city) || Fold(Text(p, "county")) == Fold(city))) continue;
            if (district && Fold(name) == Fold(city)) continue;
            string key = Fold(name) + "/" + Fold(Text(p, "state"));
            if (!seen.Add(key)) continue;
            string country = Text(p, "country");
            if (country.Length == 0) country = regions[countryCode].DisplayName;
            results.Add(Place(name, code, country, district ? city : name, district ? name : "", lat, lon));
            if (results.Count == 10) break;
        }
        return results;
    }
    public static List<Dictionary<string, object>> ParseMeteo(string body, string countryCode, string city, bool district)
    {
        var root = Json().Deserialize<Dictionary<string, object>>(body);
        var results = new List<Dictionary<string, object>>();
        foreach (var item in Array(Value(root, "results")))
        {
            var p = item as Dictionary<string, object>;
            string name = Text(p, "name"), code = Text(p, "country_code");
            double lat = Number(Value(p, "latitude")), lon = Number(Value(p, "longitude"));
            if (code != countryCode || name.Length == 0 || !ValidCoordinates(lat, lon)) continue;
            if (district && Fold(Text(p, "admin1")) != Fold(city) && Fold(Text(p, "admin2")) != Fold(city)) continue;
            if (district && Fold(name) == Fold(city)) continue;
            results.Add(Place(name, code, Text(p, "country"), district ? city : name, district ? name : "", lat, lon));
            if (results.Count == 10) break;
        }
        return results;
    }
    static string Download(string url, int timeout)
    {
        ServicePointManager.SecurityProtocol |= (SecurityProtocolType)3072;
        var request = (HttpWebRequest)WebRequest.Create(url);
        request.Timeout = timeout; request.ReadWriteTimeout = timeout;
        request.UserAgent = "LogicalLunge/1.2 (desktop weather location picker)";
        // .NET Framework 4 predates the named TLS 1.2 enum, available on our Win10 runtime.
        request.ProtocolVersion = HttpVersion.Version11;
        // One deadline includes connection AND body reads; a slow stream cannot
        // reset the timeout indefinitely and outlive the picker's request budget.
        using (var deadline = new Timer(_ => request.Abort(), null, timeout, Timeout.Infinite))
        using (var response = request.GetResponse())
        using (var stream = response.GetResponseStream())
        using (var output = new MemoryStream())
        {
            var buffer = new byte[8192]; int read;
            while ((read = stream.Read(buffer, 0, buffer.Length)) > 0)
            {
                if (output.Length + read > MAX_BODY) throw new InvalidDataException("Konum yanıtı çok büyük");
                output.Write(buffer, 0, read);
            }
            return Encoding.UTF8.GetString(output.ToArray());
        }
    }
    static List<Dictionary<string, object>> Search(Dictionary<string, string> q)
    {
        string term = Get(q, "q"), code = CountryCode(Get(q, "countryCode")), city = Get(q, "city");
        bool district = Get(q, "kind") == "district";
        if (code == null || term.Length > 80 || city.Length > 80 || (district && city.Length == 0)) throw new ArgumentException("Ülke ve şehir seçimini kontrol edin");
        if (term.Length == 0) return new List<Dictionary<string, object>>();
        string path = "https://photon.komoot.io/api/?q=" + Uri.EscapeDataString(term)
            + "&countrycode=" + code + "&limit=30" + (district ? "&layer=district&layer=locality&layer=county&layer=city" : "&layer=city&layer=state");
        if (district && ValidCoordinates(Get(q, "latitude"), Get(q, "longitude"))) path += "&lat=" + Uri.EscapeDataString(Get(q, "latitude")) + "&lon=" + Uri.EscapeDataString(Get(q, "longitude")) + "&location_bias_scale=0.05";
        try { return ParsePhoton(Download(path, 7000), code, city, district); }
        catch (WebException) { if (term.Length < 2) throw; }
        string language = Get(q, "language").Split('-')[0];
        string geo = Download("https://geocoding-api.open-meteo.com/v1/search?name=" + Uri.EscapeDataString(term)
            + "&count=30&countryCode=" + code + "&language=" + Uri.EscapeDataString(language.Length == 0 ? "en" : language) + "&format=json", 6000);
        return ParseMeteo(geo, code, city, district);
    }
    public static Dictionary<string, object> ParseForecast(string body, string place, bool fahrenheit)
    {
        var root = Json().Deserialize<Dictionary<string, object>>(body);
        var current = Value(root, "current") as Dictionary<string, object>;
        var daily = Value(root, "daily") as Dictionary<string, object>;
        double temp = Number(Value(current, "temperature_2m"));
        var high = Array(Value(daily, "temperature_2m_max"));
        var low = Array(Value(daily, "temperature_2m_min"));
        if (double.IsNaN(temp) || double.IsInfinity(temp) || high.Length == 0 || low.Length == 0
            || double.IsNaN(Number(high[0])) || double.IsNaN(Number(low[0]))) throw new InvalidDataException("Hava durumu yanıtı okunamadı");
        return new Dictionary<string, object> { { "place", place }, { "temp", temp }, { "high", Number(high[0]) }, { "low", Number(low[0]) },
            { "code", Number(Value(current, "weather_code")) }, { "day", Number(Value(current, "is_day")) != 0 }, { "fahrenheit", fahrenheit } };
    }
    static Dictionary<string, object> Weather(Dictionary<string, string> q)
    {
        string lat = Get(q, "latitude"), lon = Get(q, "longitude"), place = Get(q, "place");
        bool fahrenheit = Get(q, "fahrenheit") == "1";
        if (lat.Length > 0 || lon.Length > 0)
        {
            if (!ValidCoordinates(lat, lon)) throw new ArgumentException("Geçersiz konum");
        }
        else
        {
            string name = Get(q, "city");
            if (name.Length == 0)
            {
                // Same Windows-zone city fallback as the native widget.
                string zone = TimeZoneInfo.Local.Id;
                if (zone == "Turkey Standard Time") name = "Istanbul";
                else
                {
                    var match = System.Text.RegularExpressions.Regex.Match(TimeZoneInfo.Local.DisplayName, @"\)\s*([^,;]+)");
                    name = match.Success ? match.Groups[1].Value.Trim() : TimeZoneInfo.Local.StandardName;
                }
            }
            if (name.Length > 120) throw new ArgumentException("Konum adı çok uzun");
            var geo = Json().Deserialize<Dictionary<string, object>>(Download("https://geocoding-api.open-meteo.com/v1/search?name=" + Uri.EscapeDataString(name) + "&count=1&format=json", 5000));
            var results = Array(Value(geo, "results"));
            var selected = results != null && results.Length > 0 ? results[0] as Dictionary<string, object> : null;
            if (selected == null) throw new ArgumentException("Konum bulunamadı");
            lat = Number(Value(selected, "latitude")).ToString(invariant); lon = Number(Value(selected, "longitude")).ToString(invariant);
            place = Text(selected, "name");
            if (!ValidCoordinates(lat, lon)) throw new InvalidDataException("Geçersiz hava durumu konumu");
        }
        if (place.Length > 160) throw new ArgumentException("Konum adı çok uzun");
        string path = "https://api.open-meteo.com/v1/forecast?latitude=" + Uri.EscapeDataString(lat) + "&longitude=" + Uri.EscapeDataString(lon)
            + "&current=temperature_2m,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=1" + (fahrenheit ? "&temperature_unit=fahrenheit" : "");
        return ParseForecast(Download(path, 8000), place, fahrenheit);
    }
    public static string Handle(string target)
    {
        try
        {
            var q = Query(target);
            string route = target.Split('?')[0];
            if (route == "/widgets/countries") return Json().Serialize(new Dictionary<string, object> { { "results", Countries(Get(q, "q"), Get(q, "language")) } });
            if (route != "/widgets/places" && route != "/widgets/weather") throw new ArgumentException("Bilinmeyen widget isteği");
            var key = new StringBuilder(route).Append('?');
            foreach (var e in new SortedDictionary<string, string>(q))
                if (e.Key != "refresh") key.Append(Uri.EscapeDataString(e.Key)).Append('=').Append(Uri.EscapeDataString(e.Value)).Append('&');
            string cacheKey = key.ToString();
            bool force = route == "/widgets/weather" && Get(q, "refresh") == "1";
            lock (gate) { Cached hit; if (!force && cache.TryGetValue(cacheKey, out hit) && hit.Until > DateTime.UtcNow) return hit.Body; }
            if (!slots.WaitOne(0)) return Json().Serialize(new Dictionary<string, object> { { "error", "Konum araması meşgul, tekrar deneyin" } });
            try
            {
                object answer = route == "/widgets/places" ? (object)new Dictionary<string, object> { { "results", Search(q) } } : Weather(q);
                string body = Json().Serialize(answer);
                lock (gate)
                {
                    if (cache.Count >= MAX_CACHE) { string oldest = null; DateTime until = DateTime.MaxValue; foreach (var e in cache) if (e.Value.Until < until) { oldest = e.Key; until = e.Value.Until; } if (oldest != null) cache.Remove(oldest); }
                    cache[cacheKey] = new Cached { Until = DateTime.UtcNow.AddMinutes(route == "/widgets/places" ? 360 : 15), Body = body };
                }
                return body;
            }
            finally { slots.Release(); }
        }
        catch (ArgumentException ex) { return Json().Serialize(new Dictionary<string, object> { { "error", ex.Message } }); }
        catch (Exception ex)
        {
            // Do not send provider URLs/transport details into the product UI.
            Slider.Log("widget location: " + ex.GetType().Name);
            return Json().Serialize(new Dictionary<string, object> { { "error", "Konum verisi alınamadı. Bağlantıyı kontrol edip tekrar deneyin." } });
        }
    }
}
