using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Web.Script.Serialization;

// Everything'in dosya dizini (Super menüsünün # araması). NTFS / ReFS birimleri Everything'in kendi değişiklik
// günlüğüyle (USN) anında güncellenir ve yeni takılan NTFS / ReFS birimleri ayarlarıyla kendiliğinden eklenir. FAT /
// exFAT (USB bellek, SD kart) birimlerinin günlüğü yoktur: Everything bunları klasör dizini olarak tarar. Kurulumun
// getirdiği Everything için bu birimler klasör dizinine eklenir ve Everything'in belgelenmiş "belirli aralıkla güncelle"
// ayarıyla (Everything.ini folder_update_*) 30 dakikada bir yeniden taranır; çıkarılan birimin satırı kaldırılır.
// Kullanıcının kendi Everything'ine (bizim kurmadığımız) dokunulmaz. Yalnızca bizim eklediğimiz satırlar yönetilir
// (state\everything-folders.json).
static class EverythingIndex
{
    const int RESCAN_MINUTES = 30;
    static System.Threading.Timer timer;
    static readonly object gate = new object();
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    static string Exe { get { return Paths.Tool(@"everything\Everything.exe"); } }
    static string Ini { get { return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), @"Everything\Everything.ini"); } }
    static string Managed { get { return Paths.State("everything-folders.json"); } }

    // İlk bakış açılıştan 1 dk sonra, sonra 5 dk'da bir (bir USB bellek takılınca en geç 5 dk sonra aranabilir)
    public static void Start()
    {
        if (!File.Exists(Exe)) return;
        timer = new System.Threading.Timer(_ => Sync(), null, 60000, 5 * 60000);
    }

    // FAT / exFAT olan hazır sabit ve çıkarılabilir birimlerin kökleri ("E:\")
    static List<string> FatRoots()
    {
        var roots = new List<string>();
        foreach (var d in DriveInfo.GetDrives())
        {
            try
            {
                if ((d.DriveType != DriveType.Fixed && d.DriveType != DriveType.Removable) || !d.IsReady) continue;
                string f = d.DriveFormat ?? "";
                if (f.Equals("NTFS", StringComparison.OrdinalIgnoreCase) || f.Equals("ReFS", StringComparison.OrdinalIgnoreCase)) continue;
                roots.Add(d.RootDirectory.FullName);
            }
            catch (Exception) { }
        }
        return roots;
    }

    public static void Sync()
    {
        lock (gate)
        {
            try
            {
                if (!File.Exists(Exe) || !File.Exists(Ini)) return;
                var want = FatRoots();
                var mine = LoadManaged();
                var ini = File.ReadAllLines(Ini, Encoding.UTF8).ToList();
                var folders = ListOf(ini, "folders");
                var add = want.Where(r => !folders.Any(f => Same(f, r))).ToList();
                var drop = mine.Where(r => !want.Any(w => Same(w, r)) && folders.Any(f => Same(f, r))).ToList();
                if (add.Count == 0 && drop.Count == 0) return;
                // Everything ayarını kapanırken yazar: önce kapanır, sonra dosya düzenlenip yeniden başlatılır
                if (!Run("-exit", 15000)) { Slider.Log("Everything kapanmadı; dizin ayarı sonraya kaldı"); return; }
                ini = File.ReadAllLines(Ini, Encoding.UTF8).ToList();
                Edit(ini, add, drop);
                File.WriteAllLines(Ini, ini, new UTF8Encoding(false));
                SaveManaged(mine.Where(m => !drop.Any(d => Same(d, m))).Concat(add).Distinct(StringComparer.OrdinalIgnoreCase).ToList());
                Relaunch();
                Slider.Log("Everything klasör dizini: +" + string.Join(" ", add) + " -" + string.Join(" ", drop));
            }
            catch (Exception ex) { Slider.Log("Everything dizini: " + ex.GetBaseException().Message); }
        }
    }

    static bool Same(string a, string b) { return string.Equals(a.TrimEnd('\\'), b.TrimEnd('\\'), StringComparison.OrdinalIgnoreCase); }

    // Paralel listeler: her klasör satırının izleme, tarama ve güncelleme ayarları aynı sıradaki elemanlardadır
    static readonly string[][] Columns =
    {
        new[] { "folder_monitor_changes", "1" },          // değişiklikleri izlemeyi dener (FAT'te çoğu zaman çalışmaz)
        new[] { "folder_buffer_size_list", "0" },
        new[] { "folder_rescan_if_full_list", "1" },
        new[] { "folder_update_types", "1" },             // 1: belirli aralıkla güncelle
        new[] { "folder_update_days", "0" },
        new[] { "folder_update_ats", "0" },
        new[] { "folder_update_intervals", RESCAN_MINUTES.ToString() },
        new[] { "folder_update_interval_types", "0" },    // 0: dakika
    };

    static void Edit(List<string> ini, List<string> add, List<string> drop)
    {
        var folders = ListOf(ini, "folders");
        var cols = Columns.Select(c => Pad(ListOf(ini, c[0]), folders.Count, c[1])).ToList();
        for (int i = folders.Count - 1; i >= 0; i--)
            if (drop.Any(d => Same(d, folders[i])))
            {
                folders.RemoveAt(i);
                foreach (var c in cols) c.RemoveAt(i);
            }
        foreach (var r in add)
        {
            folders.Add(r);
            for (int k = 0; k < cols.Count; k++) cols[k].Add(Columns[k][1]);
        }
        SetList(ini, "folders", folders);
        for (int k = 0; k < cols.Count; k++) SetList(ini, Columns[k][0], cols[k]);
    }

    static List<string> Pad(List<string> l, int n, string fill)
    {
        while (l.Count < n) l.Add(fill);
        if (l.Count > n) l.RemoveRange(n, l.Count - n);
        return l;
    }

    // INI liste değeri: virgülle ayrılmış; virgül içeren eleman çift tırnakta, tırnak içinde \ kaçış karakteri
    static List<string> ListOf(List<string> ini, string key)
    {
        string line = ini.FirstOrDefault(l => l.StartsWith(key + "=", StringComparison.Ordinal));
        var list = new List<string>();
        if (line == null) return list;
        string v = line.Substring(key.Length + 1);
        if (v.Length == 0) return list;
        var cur = new StringBuilder();
        bool quoted = false;
        for (int i = 0; i < v.Length; i++)
        {
            char c = v[i];
            if (quoted && c == '\\' && i + 1 < v.Length) { cur.Append(v[++i]); continue; }
            if (c == '"') { quoted = !quoted; continue; }
            if (c == ',' && !quoted) { list.Add(cur.ToString()); cur.Clear(); continue; }
            cur.Append(c);
        }
        list.Add(cur.ToString());
        return list;
    }

    static void SetList(List<string> ini, string key, List<string> values)
    {
        string v = string.Join(",", values.Select(x => x.IndexOf(',') >= 0 || x.IndexOf('"') >= 0
            ? "\"" + x.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"" : x));
        int i = ini.FindIndex(l => l.StartsWith(key + "=", StringComparison.Ordinal));
        if (i >= 0) ini[i] = key + "=" + v;
        else
        {
            int after = ini.FindIndex(l => l.StartsWith("folders=", StringComparison.Ordinal));
            ini.Insert(after >= 0 ? after + 1 : ini.Count, key + "=" + v);
        }
    }

    static bool Run(string arg, int waitMs)
    {
        if (!Process.GetProcessesByName("Everything").Any()) return true;
        using (var p = Process.Start(new ProcessStartInfo(Exe, arg) { UseShellExecute = false, CreateNoWindow = true }))
            p.WaitForExit(waitMs);
        var sw = Stopwatch.StartNew();
        while (Process.GetProcessesByName("Everything").Any(x => SamePath(x, Exe)) && sw.ElapsedMilliseconds < waitMs) System.Threading.Thread.Sleep(200);
        return !Process.GetProcessesByName("Everything").Any(x => SamePath(x, Exe));
    }

    static bool SamePath(Process p, string exe)
    {
        try { return string.Equals(ProcInfo.Path((uint)p.Id), exe, StringComparison.OrdinalIgnoreCase); }
        catch (Exception) { return false; }
    }

    // Kullanıcı olarak (yönetici değil) yeniden başlatılır: kurulumun LogicalLunge\Everything görevi (yalnızca istekle
    // çalışır, sınırlı yetki). Görev yoksa (eski kurulum) doğrudan başlatılır.
    static void Relaunch()
    {
        int code = -1;
        try
        {
            using (var p = Process.Start(new ProcessStartInfo("schtasks.exe", "/run /tn \"LogicalLunge\\Everything\"")
            { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true }))
            {
                p.StandardOutput.ReadToEnd(); p.StandardError.ReadToEnd();
                if (p.WaitForExit(10000)) code = p.ExitCode;
            }
        }
        catch (Exception) { }
        if (code != 0) Process.Start(new ProcessStartInfo(Exe, "-startup") { UseShellExecute = false });
    }

    static List<string> LoadManaged()
    {
        try { return File.Exists(Managed) ? json.Deserialize<List<string>>(File.ReadAllText(Managed)) ?? new List<string>() : new List<string>(); }
        catch (Exception) { return new List<string>(); }
    }

    static void SaveManaged(List<string> roots)
    {
        try { File.WriteAllText(Managed, json.Serialize(roots), new UTF8Encoding(false)); } catch (Exception) { }
    }
}
