// Çalan medyanın albüm kapağını data URL olarak stdout'a yazar (ii medya kutusu için).
// ii kapağı MPRIS'ten alır; Windows karşılığı GlobalSystemMediaTransportControls.
// Derleme: helper klasöründe build-media-art.ps1
using System;
using System.IO;
using Windows.Media.Control;

static class Program
{
    // media-art.exe               -> kapak (data URL)
    // media-art.exe --seek <sn>    -> çalan medyayı o saniyeye sar (Zebar'ın medya API'sinde ileri sarma yok)
    // media-art.exe --watch        -> native bar (ll-helper --shell) için: değişince stdout'a bir satır JSON
    //                                 {"title","artist","playing","pos","end"}, şarkı değişince {"art":"data:..."};
    //                                 stdin'den komut: toggle | next | prev | stop | seek <sn>
    static void Main(string[] args)
    {
        if (args.Length == 1 && args[0] == "--watch") { Watch(); return; }
        try
        {
            var mgr = GlobalSystemMediaTransportControlsSessionManager.RequestAsync().AsTask().Result;
            var session = mgr.GetCurrentSession();
            if (session == null) return;
            if (args.Length == 2 && args[0] == "--seek")
            {
                double sec = double.Parse(args[1], System.Globalization.CultureInfo.InvariantCulture);
                session.TryChangePlaybackPositionAsync(TimeSpan.FromSeconds(sec).Ticks).AsTask().Wait(2000);
                return;
            }
            var props = session.TryGetMediaPropertiesAsync().AsTask().Result;
            if (props == null || props.Thumbnail == null) return;
            var stream = props.Thumbnail.OpenReadAsync().AsTask().Result;
            var ms = new MemoryStream();
            stream.AsStreamForRead().CopyTo(ms);
            string type = string.IsNullOrEmpty(stream.ContentType) ? "image/png" : stream.ContentType;
            var output = new StreamWriter(Console.OpenStandardOutput());
            output.Write("data:" + type + ";base64," + Convert.ToBase64String(ms.ToArray()));
            output.Flush();
        }
        catch { }
    }

    static string Esc(string s)
    {
        var sb = new System.Text.StringBuilder("\"");
        foreach (char c in s ?? "")
        {
            if (c == '"' || c == '\\') sb.Append('\\').Append(c);
            else if (c < 0x20) sb.Append("\\u").Append(((int)c).ToString("x4"));
            else sb.Append(c);
        }
        return sb.Append('"').ToString();
    }

    static void Watch()
    {
        var output = new StreamWriter(Console.OpenStandardOutput(), new System.Text.UTF8Encoding(false)) { AutoFlush = true };
        GlobalSystemMediaTransportControlsSessionManager mgr = null;
        var gate = new object();
        // Komutlar: bar'dan tıklama ve (kabuk Explorer değilken) klavyenin medya tuşları
        new System.Threading.Thread(() =>
        {
            string line;
            while ((line = Console.In.ReadLine()) != null)
            {
                try
                {
                    GlobalSystemMediaTransportControlsSession s;
                    lock (gate) s = mgr == null ? null : mgr.GetCurrentSession();
                    if (s == null) continue;
                    line = line.Trim();
                    if (line == "toggle") s.TryTogglePlayPauseAsync().AsTask().Wait(2000);
                    else if (line == "next") s.TrySkipNextAsync().AsTask().Wait(2000);
                    else if (line == "prev") s.TrySkipPreviousAsync().AsTask().Wait(2000);
                    else if (line == "stop") s.TryStopAsync().AsTask().Wait(2000);
                    else if (line.StartsWith("seek "))
                    {
                        double sec = double.Parse(line.Substring(5), System.Globalization.CultureInfo.InvariantCulture);
                        s.TryChangePlaybackPositionAsync(TimeSpan.FromSeconds(sec).Ticks).AsTask().Wait(2000);
                    }
                }
                catch { }
            }
            Environment.Exit(0); // kabuk kapandı
        }) { IsBackground = true }.Start();

        string last = null, lastArtKey = null;
        while (true)
        {
            try
            {
                lock (gate) if (mgr == null) mgr = GlobalSystemMediaTransportControlsSessionManager.RequestAsync().AsTask().Result;
                GlobalSystemMediaTransportControlsSession session;
                lock (gate) session = mgr.GetCurrentSession();
                string json;
                GlobalSystemMediaTransportControlsSessionMediaProperties props = null;
                if (session == null) json = "{\"title\":\"\",\"artist\":\"\",\"playing\":false,\"pos\":0,\"end\":0}";
                else
                {
                    props = session.TryGetMediaPropertiesAsync().AsTask().Result;
                    var pb = session.GetPlaybackInfo();
                    var tl = session.GetTimelineProperties();
                    bool playing = pb != null && pb.PlaybackStatus == GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing;
                    double pos = tl.Position.TotalSeconds, end = (tl.EndTime - tl.StartTime).TotalSeconds;
                    // Konum yalnızca uygulama bildirince güncellenir: çalarken geçen süreyi ekle (bar kendisi de sayar)
                    if (playing) pos += (DateTimeOffset.Now - tl.LastUpdatedTime).TotalSeconds;
                    if (end > 0) pos = Math.Max(0, Math.Min(end, pos));
                    var inv = System.Globalization.CultureInfo.InvariantCulture;
                    json = "{\"title\":" + Esc(props == null ? "" : props.Title) + ",\"artist\":" + Esc(props == null ? "" : props.Artist)
                        + ",\"playing\":" + (playing ? "true" : "false") + ",\"pos\":" + Math.Round(pos).ToString(inv) + ",\"end\":" + Math.Round(end).ToString(inv) + "}";
                }
                if (json != last) { output.WriteLine(json); last = json; }
                string artKey = props == null ? "" : props.Title + "|" + props.Artist;
                if (artKey != lastArtKey)
                {
                    lastArtKey = artKey;
                    string url = "";
                    if (props != null && props.Thumbnail != null)
                    {
                        try
                        {
                            var stream = props.Thumbnail.OpenReadAsync().AsTask().Result;
                            var ms = new MemoryStream();
                            stream.AsStreamForRead().CopyTo(ms);
                            string type = string.IsNullOrEmpty(stream.ContentType) ? "image/png" : stream.ContentType;
                            url = "data:" + type + ";base64," + Convert.ToBase64String(ms.ToArray());
                        }
                        catch { lastArtKey = null; } // kapak bazen biraz sonra gelir: tekrar dene
                    }
                    output.WriteLine("{\"art\":" + Esc(url) + "}");
                }
            }
            catch (IOException) { return; }
            catch { lock (gate) mgr = null; }
            System.Threading.Thread.Sleep(1000);
        }
    }
}
