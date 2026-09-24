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
    static void Main(string[] args)
    {
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
}
