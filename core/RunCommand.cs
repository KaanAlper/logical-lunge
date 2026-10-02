using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Text;
using System.Web.Script.Serialization;

// Super menüsünün komut / web eylemleri (lunge.exe --run <mod> <metin>; eskiden scripts\run.ps1). Hiçbir zaman konsol
// penceresi açmaz; sonucu JSON olarak yazar, menü bunu bildirim olarak gösterir. Program açıldıysa çıktı yoktur.
//   run  "<metin>" -> Win+R gibi: program / dosya / klasör / adres / "program argüman"; konsol komutları gizli
//                     çalışır, çıktısı bildirimde görünür
//   term "<komut>" -> ($ öneki) doğrudan gizli cmd'de çalıştırır
//   url  "<adres>" -> varsayılan tarayıcıda açar
// Kabuktan (kullanıcı haklarıyla) başlatılan ayrı bir lunge.exe'de çalışır, yönetici çekirdekte değil.
static class RunCommand
{
    const int WAIT_MS = 8000;
    const int MAX_LINES = 8;

    static string Result(string kind, string title, string body, string icon)
    {
        return new JavaScriptSerializer().Serialize(new Dictionary<string, object>
        {
            { "kind", kind }, { "title", title }, { "body", body }, { "icon", icon }
        });
    }

    // null: açıldı, söylenecek bir şey yok
    public static string Run(string mode, string text)
    {
        text = (text ?? "").Trim();
        if (text.Length == 0) return null;
        if (mode == "url")
        {
            try { Shell(text, null); } catch { return Result("error", "Açılamadı", text, "link_off"); }
            return null;
        }
        if (mode == "term") return Hidden(text);

        // ---- run: Win+R mantığı ----
        string expanded = Environment.ExpandEnvironmentVariables(text);
        string[] parts = new System.Text.RegularExpressions.Regex(@"\s+").Split(expanded, 2);
        string first = parts[0];
        string rest = parts.Length > 1 && parts[1].Length > 0 ? parts[1] : null;

        // PATH'te bulunan program: konsolsa gizli çalıştır, pencereliyse normal aç
        string app = FindApplication(first);
        if (app != null)
        {
            if (IsConsoleExe(app)) return Hidden(text);
            try { Shell(app, rest); } catch { }
            return null;
        }
        // Dosya, klasör, adres, App Paths (ör. "chrome"), shell: yolları
        try { Shell(expanded, null); return null; } catch { }
        if (rest != null)
        {
            try { Shell(first, rest); return null; } catch { }
        }
        // cmd yerleşik komutları (dir, echo, set...) gizli; o da yoksa "bulunamadı" bildirimi
        return Hidden(text);
    }

    static void Shell(string file, string args)
    {
        var psi = new ProcessStartInfo(file) { UseShellExecute = true, WorkingDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) };
        if (args != null) psi.Arguments = args;
        using (Process.Start(psi)) { }
    }

    // Get-Command -CommandType Application: yol verildiyse o dosya, yoksa PATH klasörlerinde ad ve PATHEXT uzantıları
    static string FindApplication(string name)
    {
        if (name.Length == 0 || name.IndexOfAny(System.IO.Path.GetInvalidPathChars()) >= 0) return null;
        var exts = (Environment.GetEnvironmentVariable("PATHEXT") ?? ".COM;.EXE;.BAT;.CMD")
            .Split(new[] { ';' }, StringSplitOptions.RemoveEmptyEntries);
        bool hasExt = System.IO.Path.HasExtension(name)
            && exts.Any(e => string.Equals(e, System.IO.Path.GetExtension(name), StringComparison.OrdinalIgnoreCase));
        Func<string, string> probe = basePath =>
        {
            if (hasExt) return System.IO.File.Exists(basePath) ? basePath : null;
            foreach (var e in exts) if (System.IO.File.Exists(basePath + e)) return basePath + e;
            return null;
        };
        try
        {
            if (name.IndexOf('\\') >= 0 || name.IndexOf('/') >= 0) return probe(System.IO.Path.GetFullPath(name));
            foreach (var dir in (Environment.GetEnvironmentVariable("PATH") ?? "").Split(new[] { ';' }, StringSplitOptions.RemoveEmptyEntries))
            {
                string d = dir.Trim().Trim('"');
                if (d.Length == 0) continue;
                string found = probe(System.IO.Path.Combine(d, name));
                if (found != null) return found;
            }
        }
        catch (Exception) { }
        return null;
    }

    // PE başlığından alt sistem: 2 = pencereli (GUI), 3 = konsol
    static bool IsConsoleExe(string path)
    {
        try
        {
            using (var fs = System.IO.File.OpenRead(path))
            using (var br = new System.IO.BinaryReader(fs))
            {
                fs.Position = 0x3C; int pe = br.ReadInt32();
                fs.Position = pe + 0x5C;
                return br.ReadUInt16() == 3;
            }
        }
        catch { return false; }
    }

    static string Hidden(string cmd)
    {
        // Konsol programları OEM kod sayfasıyla yazar (Türkçe: 857)
        var oem = Encoding.GetEncoding(System.Globalization.CultureInfo.CurrentCulture.TextInfo.OEMCodePage);
        var psi = new ProcessStartInfo(System.IO.Path.Combine(Environment.GetEnvironmentVariable("SystemRoot") ?? @"C:\Windows", @"System32\cmd.exe"), "/d /s /c \"" + cmd + "\"")
        {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
            StandardOutputEncoding = oem, StandardErrorEncoding = oem,
            WorkingDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
        };
        using (var p = Process.Start(psi))
        {
            var outTask = p.StandardOutput.ReadToEndAsync();
            var errTask = p.StandardError.ReadToEndAsync();
            if (!p.WaitForExit(WAIT_MS)) return Result("info", "Arka planda çalışıyor", cmd, "hourglass_top");
            string output = outTask.Result.Trim(), err = errTask.Result.Trim();
            if (p.ExitCode == 9009 || System.Text.RegularExpressions.Regex.IsMatch(err, "tanınmıyor|is not recognized"))
            {
                string first = System.Text.RegularExpressions.Regex.Split(cmd, @"\s+")[0];
                return Result("error", "Komut bulunamadı", "'" + first + "' diye bir program ya da komut yok.", "search_off");
            }
            string text = p.ExitCode != 0 && err.Length > 0 ? err : output.Length > 0 ? output : err;
            var lines = System.Text.RegularExpressions.Regex.Split(text, "\r?\n").Where(l => l.Trim().Length > 0).ToList();
            string body = string.Join("\n", lines.Take(MAX_LINES));
            if (lines.Count > MAX_LINES) body += "\n… (+" + (lines.Count - MAX_LINES) + " satır)";
            if (p.ExitCode != 0) return Result("error", "Hata (" + p.ExitCode + ")", body.Length > 0 ? body : cmd, "error");
            return Result("ok", cmd, body.Length > 0 ? body : "Tamamlandı", "terminal");
        }
    }
}
