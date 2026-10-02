using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32;

// Başlatmadan önce: dosya / program / adres açılabilir mi? Çekirdek yönetici olarak çalıştığı için kullanıcının
// programlarını Gezgin'e başlattırır (UserLaunch); Gezgin bulamadığı ya da açacak uygulaması olmayan bir şeyde kendi
// kutusunu gösterir. Bu yüzden yol, PATH, "App Paths" ve dosya türünün / adres şemasının ilişkilendirmesi önce burada
// denetlenir; açılamayacaksa Windows kutusu yerine bizim kartımız çıkar (ilişkisi olmayan dosyada "Birlikte aç" ile).
static class Launcher
{
    public const int OK = 0, NOT_FOUND = 2, PATH_NOT_FOUND = 3, NO_ASSOCIATION = 1155;

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)]
    static extern int AssocQueryString(int flags, int str, string assoc, string extra, StringBuilder outStr, ref uint outLen);
    const int ASSOCF_INIT_IGNOREUNKNOWN = 0x400, ASSOCSTR_COMMAND = 1;

    // Kendi kendine çalışan türler (ilişkilendirme gerekmez)
    static readonly string[] RUNNABLE = { ".exe", ".com", ".bat", ".cmd", ".lnk", ".url", ".msc", ".cpl", ".scr", ".pif", ".appref-ms", ".msi", ".ps1", ".vbs", ".js" };

    // "C:" sürücü harfi değil, en az iki harfli bir şema: https:, ms-settings:, spotify:, shell:
    static string SchemeOf(string s)
    {
        int c = s.IndexOf(':');
        if (c < 2) return null;
        for (int i = 0; i < c; i++)
        {
            char ch = s[i];
            if (!(char.IsLetterOrDigit(ch) || ch == '+' || ch == '-' || ch == '.')) return null;
        }
        return s.Substring(0, c).ToLowerInvariant();
    }

    // Dosya türünün ya da şemanın açacak bir komutu var mı (Windows'un kendi ilişkilendirmesi)
    public static bool HasHandler(string assoc)
    {
        uint len = 0;
        int hr = AssocQueryString(ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, assoc, null, null, ref len);
        return (hr == 0 || hr == 1 /* S_FALSE: boyut döndü */) && len > 0;
    }

    static bool InPathOrAppPaths(string name)
    {
        if (name.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0) return false;
        var exts = new List<string> { "" };
        exts.AddRange((Environment.GetEnvironmentVariable("PATHEXT") ?? ".COM;.EXE;.BAT;.CMD").Split(';'));
        foreach (var dir in (Environment.GetEnvironmentVariable("PATH") ?? "").Split(';'))
        {
            if (dir.Trim().Length == 0) continue;
            foreach (var ext in exts)
                try { if (File.Exists(Path.Combine(Environment.ExpandEnvironmentVariables(dir.Trim()), name + ext))) return true; } catch { }
        }
        string key = name.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) ? name : name + ".exe";
        foreach (var root in new[] { Registry.CurrentUser, Registry.LocalMachine })
            try { using (var k = root.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\App Paths\" + key)) if (k != null) return true; } catch { }
        return false;
    }

    // 0: açılabilir; yoksa Win32 hata kodu (2 bulunamadı, 3 yol yok, 1155 açacak uygulama yok)
    public static int Check(string file)
    {
        if (string.IsNullOrWhiteSpace(file)) return NOT_FOUND;
        string f = Environment.ExpandEnvironmentVariables(file.Trim().Trim('"'));
        string scheme = SchemeOf(f);
        if (scheme != null)
        {
            if (scheme == "shell") return OK; // shell:AppsFolder\..., shell:Downloads
            return HasHandler(scheme + ":") || HasHandler(scheme) ? OK : NO_ASSOCIATION;
        }
        bool rooted;
        try { rooted = Path.IsPathRooted(f); } catch { return NOT_FOUND; }
        if (!rooted && f.IndexOf('\\') < 0 && f.IndexOf('/') < 0)
            return InPathOrAppPaths(f) ? OK : NOT_FOUND;
        if (Directory.Exists(f)) return OK;
        if (!File.Exists(f))
        {
            string dir = null;
            try { dir = Path.GetDirectoryName(f); } catch { }
            return dir != null && dir.Length > 0 && !Directory.Exists(dir) ? PATH_NOT_FOUND : NOT_FOUND;
        }
        string ext = Path.GetExtension(f).ToLowerInvariant();
        if (Array.IndexOf(RUNNABLE, ext) >= 0) return OK;
        return ext.Length > 0 && HasHandler(ext) ? OK : NO_ASSOCIATION;
    }

    // Windows'un kendi (yerelleştirilmiş) hata metni
    public static string Describe(int code)
    {
        return new System.ComponentModel.Win32Exception(code).Message;
    }

    // Açılamayanın kartı: Windows kutusu yerine; açacak uygulaması yoksa "Birlikte aç"
    public static void Report(string file, int code)
    {
        string name = file;
        try { name = Path.GetFileName(file.Trim().Trim('"').TrimEnd('\\')); if (name.Length == 0) name = file; } catch { }
        Slider.Log("açılamadı (" + code + "): " + file);
        var card = new Dictionary<string, object>
        {
            { "kind", "error" }, { "icon", "error" }, { "title", "Açılamadı: " + name }, { "body", Describe(code) },
        };
        if (code == NO_ASSOCIATION && File.Exists(file.Trim().Trim('"')))
            card["actions"] = new object[] { new Dictionary<string, object> { { "label", "Birlikte aç" }, { "url", "openwith:" + file.Trim().Trim('"') } } };
        Toasts.Card(card);
    }
}
