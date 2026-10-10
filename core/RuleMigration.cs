using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;

// Pencere kuralları (window_rules) için BindMigration'ın karşılığı: kullanıcı config.yaml'ını tuttuğundan sürümle gelen
// iki değişiklik mevcut kullanıcılara ulaşmazdı.
//  1) Yeni varsayılan kurallar (ör. SndVol yüzen+ortalı): varsayılan config'teki (config.default.yaml) her "match" girdisi
//     (komut listesiyle birlikte) kullanıcıda yoksa ve daha önce sunulmadıysa, eksik girdiler config'in sonuna yeni bir kural
//     olarak eklenir. Sunulan girdi state\offered-rules.json'a yazılır; kullanıcı silerse geri gelmez.
//  2) Düzenli ifadeler artık tüm değeri eşler (Hyprland gibi). Önceki sürümlerin gönderdiği kuralların kısmi eşleşmeye
//     dayanan ifadeleri, kullanıcıda AYNEN duruyorsa, aynı anlama gelen tam eşleşme biçimine (.*...*.) çevrilir.
//     Kullanıcının yazdığı ya da değiştirdiği hiçbir kurala dokunulmaz.
static class RuleMigration
{
    static string OfferedPath { get { return Paths.State("offered-rules.json"); } }
    static string DefaultPath { get { return Paths.In("config.default.yaml"); } }

    internal sealed class Item { public int Start, End; public string[] Lines; public string Key; }
    internal sealed class Rule { public string Cmds; public List<string> CmdList = new List<string>(); public List<Item> Items = new List<Item>(); }

    // Önceki sürümlerin gönderdiği, kısmi eşleşmeye dayanan girdiler (config.yaml geçmişi) ve tam eşleşme karşılıkları.
    // Her satır: komut anahtarı, eski girdi satırları, yeni girdi satırları (ilk satır "- " olmadan).
    static readonly string[][][] Upgrades = new string[][][] {
        new[] { new[] { "ignore" },
            new[] { "window_title: { regex: '[Pp]icture.in.[Pp]icture' }", "window_class: { regex: 'Chrome_WidgetWin_1|MozillaDialogClass' }" },
            new[] { "window_title: { regex: '.*[Pp]icture.in.[Pp]icture.*' }", "window_class: { regex: '.*(?:Chrome_WidgetWin_1|MozillaDialogClass).*' }" } },
        new[] { new[] { "ignore" },
            new[] { "window_process: { equals: 'PowerToys' }", "window_class: { regex: 'HwndWrapper\\[PowerToys\\.PowerAccent.*?\\]' }" },
            new[] { "window_process: { equals: 'PowerToys' }", "window_class: { regex: '.*HwndWrapper\\[PowerToys\\.PowerAccent.*?\\].*' }" } },
        new[] { new[] { "ignore" },
            new[] { "window_process: { equals: 'PowerToys' }", "window_title: { regex: '.*? - Peek' }" },
            new[] { "window_process: { equals: 'PowerToys' }", "window_title: { regex: '.*? - Peek.*' }" } },
        new[] { new[] { "ignore" },
            new[] { "window_process: { equals: 'Lively' }", "window_class: { regex: 'HwndWrapper' }" },
            new[] { "window_process: { equals: 'Lively' }", "window_class: { regex: '.*HwndWrapper.*' }" } },
        new[] { new[] { "ignore" },
            new[] { "window_process: { equals: 'EXCEL' }", "window_class: { not_regex: 'XLMAIN' }" },
            new[] { "window_process: { equals: 'EXCEL' }", "window_class: { not_regex: '.*XLMAIN.*' }" } },
        new[] { new[] { "ignore" },
            new[] { "window_process: { equals: 'WINWORD' }", "window_class: { not_regex: 'OpusApp' }" },
            new[] { "window_process: { equals: 'WINWORD' }", "window_class: { not_regex: '.*OpusApp.*' }" } },
        new[] { new[] { "ignore" },
            new[] { "window_process: { equals: 'POWERPNT' }", "window_class: { not_regex: 'PPTFrameClass' }" },
            new[] { "window_process: { equals: 'POWERPNT' }", "window_class: { not_regex: '.*PPTFrameClass.*' }" } },
        // '^(Open|Save|...).*' tam eşlemede de aynı şeyi eşler: çevrilecek bir şey yok
    };

    static string Norm(string line)
    {
        string s = Regex.Replace(line.Trim().Replace('"', '\''), @"\s+", " ");
        return Regex.Replace(Regex.Replace(s, @"\{\s*", "{"), @"\s*\}", "}");
    }

    static string ItemKey(IEnumerable<string> lines) { return string.Join(" & ", lines.Select(Norm).OrderBy(x => x, StringComparer.Ordinal)); }

    static int Indent(string l) { return l.Length - l.TrimStart().Length; }

    // window_rules bölümü: [first, end) satır aralığı ("window_rules:" satırından sonrası); bölüm yoksa ya da satır içi ise false
    static bool Section(string[] lines, out int first, out int end)
    {
        first = -1; end = lines.Length;
        for (int i = 0; i < lines.Length; i++)
        {
            string l = lines[i];
            if (first < 0) { if (Regex.IsMatch(l, @"^window_rules:\s*(#.*)?$")) first = i + 1; continue; }
            if (l.Length > 0 && !char.IsWhiteSpace(l[0])) { end = i; break; }
        }
        return first >= 0;
    }

    internal static List<Rule> Read(string[] lines, int first, int end)
    {
        var rules = new List<Rule>();
        Rule cur = null; bool inMatch = false; int dash = -1;
        Item item = null;
        Action closeItem = () => { if (item != null) { cur.Items.Add(item); item = null; } };
        for (int i = first; i < end; i++)
        {
            string l = lines[i];
            var c = Regex.Match(l, @"^\s*- commands:\s*\[(.*)\]\s*$");
            if (c.Success)
            {
                closeItem();
                cur = new Rule(); rules.Add(cur); inMatch = false;
                foreach (Match q in Regex.Matches(c.Groups[1].Value, @"'([^']*)'|""([^""]*)"""))
                    cur.CmdList.Add(q.Groups[1].Success ? q.Groups[1].Value : q.Groups[2].Value);
                cur.Cmds = BindMigration.Key(cur.CmdList);
                continue;
            }
            if (cur == null) continue;
            if (Regex.IsMatch(l, @"^\s*match:\s*(#.*)?$")) { closeItem(); inMatch = true; continue; }
            if (!inMatch || l.Trim().Length == 0 || l.TrimStart().StartsWith("#")) continue;
            var d = Regex.Match(l, @"^(\s*)- (.*)$");
            if (d.Success && (item == null || d.Groups[1].Length <= dash))
            {
                closeItem();
                dash = d.Groups[1].Length;
                item = new Item { Start = i, End = i, Lines = new[] { d.Groups[2].Value.Trim() } };
                continue;
            }
            if (item != null && Indent(l) > dash) { item.End = i; item.Lines = item.Lines.Concat(new[] { l.Trim() }).ToArray(); continue; }
            closeItem(); inMatch = false; // match: altında olmayan başka bir anahtar
        }
        closeItem();
        foreach (var r in rules) foreach (var it in r.Items) it.Key = ItemKey(it.Lines);
        return rules;
    }

    // 2) eski varsayılan girdileri tam eşleşme biçimine çevir
    static string Upgrade(string user, string eol, out int upgraded)
    {
        upgraded = 0;
        string[] lines = user.Replace("\r\n", "\n").Split('\n');
        int first, end;
        if (!Section(lines, out first, out end)) return user;
        var old = new Dictionary<string, string[]>(StringComparer.Ordinal);
        foreach (var u in Upgrades) old[BindMigration.Key(u[0]) + " => " + ItemKey(u[1])] = u[2];
        var edits = new List<KeyValuePair<Item, string[]>>();
        foreach (var r in Read(lines, first, end))
            foreach (var it in r.Items)
            {
                string[] repl;
                if (old.TryGetValue(r.Cmds + " => " + it.Key, out repl)) edits.Add(new KeyValuePair<Item, string[]>(it, repl));
            }
        if (edits.Count == 0) return user;
        var outLines = new List<string>(lines);
        foreach (var e in edits.OrderByDescending(x => x.Key.Start))
        {
            var it = e.Key;
            string dashIndent = lines[it.Start].Substring(0, Indent(lines[it.Start]));
            string contIndent = it.End > it.Start ? lines[it.Start + 1].Substring(0, Indent(lines[it.Start + 1])) : dashIndent + "  ";
            var block = new List<string>();
            for (int k = 0; k < e.Value.Length; k++) block.Add(k == 0 ? dashIndent + "- " + e.Value[k] : contIndent + e.Value[k]);
            outLines.RemoveRange(it.Start, it.End - it.Start + 1);
            outLines.InsertRange(it.Start, block);
            upgraded++;
        }
        return string.Join(eol, outLines);
    }

    // Saf birleştirme. user/def: config.yaml metinleri, offered: daha önce sunulmuş girdi anahtarları. Dönüş: yeni metin
    // (değişmediyse aynı nesne); newOffered: güncel küme; added: eklenen girdi sayısı; upgraded: tam eşlemeye çevrilen girdi sayısı.
    internal static string Merge(string user, string def, ISet<string> offered, out HashSet<string> newOffered, out int added, out int upgraded)
    {
        added = 0; upgraded = 0;
        newOffered = new HashSet<string>(offered ?? new HashSet<string>(), StringComparer.Ordinal);
        string eol = user.Contains("\r\n") ? "\r\n" : "\n";
        int uf, ue, df, de;
        string[] probe = user.Replace("\r\n", "\n").Split('\n');
        string[] defLines = def.Replace("\r\n", "\n").Split('\n');
        if (!Section(probe, out uf, out ue) || !Section(defLines, out df, out de)) return user; // bölüm yok ya da satır içi ("window_rules: []"): elle yazılmış, dokunma

        string text = Upgrade(user, eol, out upgraded);
        string[] lines = text.Replace("\r\n", "\n").Split('\n');
        Section(lines, out uf, out ue);
        var userRules = Read(lines, uf, ue);
        var have = new HashSet<string>(StringComparer.Ordinal);
        foreach (var r in userRules) foreach (var it in r.Items) have.Add(r.Cmds + " => " + it.Key);

        var block = new List<string>();
        string dashIndent = userRules.Count > 0 ? lines[FirstRuleLine(lines, uf, ue)].Substring(0, Indent(lines[FirstRuleLine(lines, uf, ue)])) : "  ";
        foreach (var r in Read(defLines, df, de))
        {
            var fresh = new List<Item>();
            foreach (var it in r.Items)
            {
                string key = r.Cmds + " => " + it.Key;
                if (newOffered.Contains(key)) continue;
                newOffered.Add(key);
                if (!have.Contains(key)) { fresh.Add(it); have.Add(key); }
            }
            if (fresh.Count == 0) continue;
            block.Add(dashIndent + "- commands: [" + string.Join(", ", r.CmdList.Select(c => "'" + c.Replace("'", "") + "'")) + "]");
            block.Add(dashIndent + "  match:");
            foreach (var it in fresh)
                for (int k = 0; k < it.Lines.Length; k++)
                    block.Add(k == 0 ? dashIndent + "    - " + it.Lines[k] : dashIndent + "      " + it.Lines[k]);
            added += fresh.Count;
        }
        if (added == 0) return upgraded > 0 ? text : user;
        // bölümün son dolu satırının altına
        int at = ue;
        while (at > uf && lines[at - 1].Trim().Length == 0) at--;
        var outLines = new List<string>(lines);
        outLines.Insert(at, "");
        outLines.Insert(at + 1, dashIndent + "# yeni varsayılan pencere kuralları (güncellemeyle eklendi)");
        outLines.InsertRange(at + 2, block);
        return string.Join(eol, outLines);
    }

    static int FirstRuleLine(string[] lines, int first, int end)
    {
        for (int i = first; i < end; i++) if (Regex.IsMatch(lines[i], @"^\s*- commands:")) return i;
        return first;
    }

    static bool ReadOffered(out HashSet<string> set)
    {
        set = new HashSet<string>(StringComparer.Ordinal);
        if (!System.IO.File.Exists(OfferedPath)) return true;
        try
        {
            var o = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(OfferedPath));
            object list;
            if (o == null || !o.TryGetValue("offered", out list)) return false;
            foreach (object x in (System.Collections.IEnumerable)list) set.Add((string)x);
            return true;
        }
        catch (Exception ex) { Slider.Log("kural göçü: durum dosyası okunamadı, atlandı: " + ex.Message); return false; }
    }

    // BindMigration'dan sonra aynı iş parçacığında çalışır (ikisi de config.yaml'ı yazar)
    public static void Run()
    {
        try
        {
            if (!System.IO.File.Exists(DefaultPath) || !System.IO.File.Exists(Paths.ConfigFile)) return;
            HashSet<string> offered;
            if (!ReadOffered(out offered)) return;
            string user = System.IO.File.ReadAllText(Paths.ConfigFile);
            string def = System.IO.File.ReadAllText(DefaultPath);
            HashSet<string> now; int added, upgraded;
            string next = Merge(user, def, offered, out now, out added, out upgraded);
            bool changed = !ReferenceEquals(next, user) && next != user;
            if (changed)
            {
                if (!Files.WriteAtomic(Paths.ConfigFile, next)) { Slider.Log("kural göçü: config.yaml yazılamadı"); return; }
                try
                {
                    string shaFile = Paths.State("config.sha256");
                    if (System.IO.File.Exists(shaFile) && System.IO.File.ReadAllText(shaFile).Trim().Equals(BindMigration.Sha(user), StringComparison.OrdinalIgnoreCase))
                        Files.WriteAtomic(shaFile, BindMigration.Sha(next) + Environment.NewLine);
                }
                catch (Exception ex) { Slider.Log("kural göçü: config.sha256 güncellenemedi: " + ex.Message); }
            }
            if (now.Count != offered.Count)
                if (!Files.WriteAtomic(OfferedPath, new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "offered", now.OrderBy(k => k, StringComparer.Ordinal).ToList() } })))
                    Slider.Log("kural göçü: durum dosyası yazılamadı");
            if (changed)
            {
                Slider.Log("kural göçü: " + added + " yeni varsayılan kural girdisi eklendi, " + upgraded + " eski girdi tam eşlemeye çevrildi");
                try { new TilingClient().Command("wm-reload-config"); } catch { }
            }
        }
        catch (Exception ex) { Slider.Log("kural göçü: " + ex.Message); }
    }
}
