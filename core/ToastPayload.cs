using System;
using System.Collections.Generic;
using System.Xml;

// Bir Windows bildiriminin içeriği (Bildirim Merkezi veritabanındaki toast XML'i): başlık, metin, gönderenin görseli,
// senaryo. Saf fonksiyonlar: çekirdek testleri bunları Windows'suz da çalıştırır.
sealed class ToastPayload
{
    public string Title = "", Body = "";
    // Body click context is application data, never a command line.
    public string Launch = "", ActivationType = "foreground";
    // appLogoOverride görseli (ör. mesajı gönderenin fotoğrafı): dosya yolu, ms-appdata:/// ya da http(s) adresi
    public string Logo;
    // reminder | alarm | incomingCall | urgent; Windows bunları kullanıcı kapatana kadar ekranda tutar
    public string Scenario = "";
    // Düğmesi ya da yanıt kutusu var: Logical Lunge kartı bunları çalıştıramaz, Bildirim Merkezi'ne yönlendirir
    public bool Interactive;

    const int MAX_XML = 64 * 1024;

    public bool Urgent
    {
        get { return Scenario == "reminder" || Scenario == "alarm" || Scenario == "incomingCall" || Scenario == "urgent"; }
    }

    public static ToastPayload Parse(string xml)
    {
        var p = new ToastPayload();
        if (string.IsNullOrEmpty(xml) || xml.Length > MAX_XML) return p;
        var doc = new XmlDocument { XmlResolver = null };
        try
        {
            // Uygulamanın yazdığı XML: DTD ve dış varlıklar kapalı
            var settings = new XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit, XmlResolver = null };
            using (var r = XmlReader.Create(new System.IO.StringReader(xml), settings)) doc.Load(r);
        }
        catch (XmlException) { return p; }
        var toast = doc.DocumentElement;
        if (toast == null || toast.Name != "toast") return p;

        p.Scenario = toast.GetAttribute("scenario");
        p.Launch = toast.GetAttribute("launch");
        if (toast.HasAttribute("activationType")) p.ActivationType = toast.GetAttribute("activationType");
        var texts = new List<string>();
        foreach (XmlNode n in toast.SelectNodes("visual//text"))
        {
            string t = n.InnerText.Trim();
            if (t.Length > 0) texts.Add(t);
        }
        if (texts.Count > 0) p.Title = texts[0];
        if (texts.Count > 1) p.Body = string.Join("\n", texts.GetRange(1, texts.Count - 1));
        foreach (XmlElement img in toast.SelectNodes("visual//image"))
            if (img.GetAttribute("placement") == "appLogoOverride") { p.Logo = img.GetAttribute("src"); break; }
        p.Interactive = toast.SelectSingleNode("actions/action[not(@placement='contextMenu')] | actions/input") != null;
        return p;
    }

    // Görsel kaynağının yerel dosya yolu: file:///, düz yol ya da paketli uygulamanın ms-appdata:///local|roaming|temp/
    // klasörü (AUMID = <paket aile adı>!<uygulama>). Çözülemeyen (ms-appx:///, http) null.
    public static string LocalImagePath(string src, string aumid, string localAppData)
    {
        if (string.IsNullOrEmpty(src)) return null;
        const string APPDATA = "ms-appdata:///";
        if (src.StartsWith(APPDATA, StringComparison.OrdinalIgnoreCase))
        {
            int bang = aumid == null ? -1 : aumid.IndexOf('!');
            if (bang <= 0) return null;
            string family = aumid.Substring(0, bang);
            if (family.IndexOfAny(new[] { '\\', '/', ':' }) >= 0 || family.Contains("..")) return null;
            string rest = src.Substring(APPDATA.Length);
            int slash = rest.IndexOf('/');
            if (slash <= 0) return null;
            string kind = rest.Substring(0, slash).ToLowerInvariant();
            string folder = kind == "local" ? "LocalState" : kind == "roaming" ? "RoamingState" : kind == "temp" ? "TempState" : null;
            string rel = Uri.UnescapeDataString(rest.Substring(slash + 1)).Replace('/', '\\');
            // "ms-appdata:///local///sunucu/paylaşım" köklü bir yol verir: Path.Combine paket klasörünü atar, ağa çıkar
            if (folder == null || rel.Length == 0 || rel.Contains("..") || rel.Contains(":") || rel[0] == '\\') return null;
            string root = System.IO.Path.Combine(localAppData, "Packages", family, folder);
            string full = System.IO.Path.GetFullPath(System.IO.Path.Combine(root, rel));
            return full.StartsWith(root + System.IO.Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase) ? full : null;
        }
        Uri u;
        if (Uri.TryCreate(src, UriKind.Absolute, out u) && u.IsFile && !u.IsUnc) return u.LocalPath;
        return null;
    }

    // Uygulama listesinde ya da kayıt defterinde adı bulunamayan gönderenin okunur adı:
    // "Microsoft.WindowsStore_8wekyb3d8bbwe!App" -> "WindowsStore", "com.squirrel.Discord.Discord" -> "Discord",
    // "{GUID}\Mozilla Firefox\firefox.exe" -> "firefox"
    public static string NameFromAumid(string aumid)
    {
        if (string.IsNullOrEmpty(aumid)) return "";
        string s = aumid;
        int bang = s.IndexOf('!');
        if (bang > 0)
        {
            s = s.Substring(0, bang);
            int us = s.LastIndexOf('_');
            if (us > 0) s = s.Substring(0, us);
        }
        int slash = s.LastIndexOf('\\');
        if (slash >= 0) s = s.Substring(slash + 1);
        if (s.EndsWith(".exe", StringComparison.OrdinalIgnoreCase)) s = s.Substring(0, s.Length - 4);
        int dot = s.LastIndexOf('.');
        if (dot >= 0 && dot < s.Length - 1) s = s.Substring(dot + 1);
        return s.Length > 0 ? s : aumid;
    }

    // Veritabanındaki varış zamanı (FILETIME, 100 ns / 1601) -> Unix milisaniye
    public static long UnixMs(long fileTime)
    {
        return (fileTime - 116444736000000000L) / 10000;
    }
}
