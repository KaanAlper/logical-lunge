// Çalan medyanın albüm kapağını data URL olarak stdout'a yazar (medya kutusu için),
// Windows'un GlobalSystemMediaTransportControls'undan.
// Derleme: build.ps1 (Windows 10 SDK gerekir)
using System;
using System.IO;
using Windows.Media.Control;

static class Program
{
    // lunge-media.exe              -> kapak (data URL)
    // lunge-media.exe --seek <sn>  -> çalan medyayı o saniyeye sar (kabuğun medya API'sinde ileri sarma yok)
    static void Main(string[] args)
    {
        try
        {
            var mgr = GlobalSystemMediaTransportControlsSessionManager.RequestAsync().AsTask().Result;
            var session = mgr.GetCurrentSession();
            if (session == null) return;
            if (args.Length == 1 && args[0] == "--timeline")
            {
                var t = session.GetTimelineProperties();
                var p = session.GetPlaybackInfo();
                var timelineWriter = new StreamWriter(Console.OpenStandardOutput());
                timelineWriter.Write("{\"source\":\"" + session.SourceAppUserModelId.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\",\"positionTicks\":" + t.Position.Ticks + ",\"updatedTicks\":" + t.LastUpdatedTime.ToFileTime() + ",\"endTicks\":" + t.EndTime.Ticks + ",\"status\":\"" + p.PlaybackStatus + "\"}");
                timelineWriter.Flush();
                return;
            }
            if (args.Length == 2 && args[0] == "--seek")
            {
                double sec = double.Parse(args[1], System.Globalization.CultureInfo.InvariantCulture);
                session.TryChangePlaybackPositionAsync(TimeSpan.FromSeconds(sec).Ticks).AsTask().Wait(2000);
                return;
            }
            // A browser changes the title first and the picture a moment later (it fetches the new video's artwork):
            // the picture read at the title change was the previous video's, and the bar keeps one picture per title.
            // A picture that was given for another title is that title's: read again every quarter second until it
            // changes (at most 4 s, then it is taken as this title's own). A title that changes meanwhile gives
            // nothing; the bar asks again for the new title.
            var props = session.TryGetMediaPropertiesAsync().AsTask().Result;
            if (props == null) return;
            string title = props.Title ?? "", type;
            byte[] art = Read(props, out type);
            string lastTitle, lastHash;
            ReadLast(out lastTitle, out lastHash);
            for (int i = 0; i < 16 && (art == null || (lastTitle != title && Hash(art) == lastHash)); i++)
            {
                System.Threading.Thread.Sleep(250);
                var again = session.TryGetMediaPropertiesAsync().AsTask().Result;
                if (again == null || (again.Title ?? "") != title) return;
                art = Read(again, out type);
            }
            if (art == null) return;
            WriteLast(title, Hash(art));
            var output = new StreamWriter(Console.OpenStandardOutput());
            output.Write("data:" + type + ";base64," + Convert.ToBase64String(art));
            output.Flush();
        }
        catch { }
    }

    static byte[] Read(GlobalSystemMediaTransportControlsSessionMediaProperties props, out string type)
    {
        type = "image/png";
        if (props.Thumbnail == null) return null;
        var stream = props.Thumbnail.OpenReadAsync().AsTask().Result;
        var ms = new MemoryStream();
        stream.AsStreamForRead().CopyTo(ms);
        if (!string.IsNullOrEmpty(stream.ContentType)) type = stream.ContentType;
        return ms.ToArray();
    }

    // The last picture given and its title (%LOCALAPPDATA%\LogicalLunge\media-art-last.txt): each run is a new process
    static readonly string LastPath = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LogicalLunge", "media-art-last.txt");

    static void ReadLast(out string title, out string hash)
    {
        title = null; hash = null;
        try
        {
            var lines = File.ReadAllLines(LastPath);
            if (lines.Length == 2) { hash = lines[0]; title = lines[1]; }
        }
        catch { }
    }

    static void WriteLast(string title, string hash)
    {
        try { File.WriteAllLines(LastPath, new[] { hash, title.Replace("\r", " ").Replace("\n", " ") }); } catch { }
    }

    static string Hash(byte[] data)
    {
        using (var sha = System.Security.Cryptography.SHA256.Create()) return Convert.ToBase64String(sha.ComputeHash(data));
    }
}
