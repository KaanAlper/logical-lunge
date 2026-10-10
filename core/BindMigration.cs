using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;

// Yeni varsayılan pencere yöneticisi kısayolları mevcut kullanıcılara da ulaşsın diye: kullanıcı kendi config.yaml'ını
// tuttuğundan (kurucu düzenlenmiş dosyaya dokunmaz) sürümle gelen yeni kısayollar yalnızca yeni kurulumlarda olurdu.
// Çekirdek açılırken gönderilen varsayılan config'i (config.default.yaml) kullanıcınınkiyle karşılaştırır: komutu olmayan
// ve tuş kombinasyonu hiçbir yerde kullanılmayan her varsayılan kısayolu ekler. Bir kez değerlendirilen varsayılan
// state\offered-binds.json'a yazılır; kullanıcı sonradan silerse geri gelmez. Var olan kısayollara dokunulmaz.
static class BindMigration
{
    static string OfferedPath { get { return Paths.State("offered-binds.json"); } }
    static string DefaultPath { get { return Paths.In("config.default.yaml"); } }

    // Bir varsayılan girdinin kimliği: komutları (boşluk ve büyük/küçük harf farkı yok)
    internal static string Key(IEnumerable<string> commands)
    {
        return string.Join(" ; ", commands.Select(c => Regex.Replace(c.Trim(), @"\s+", " ").ToLowerInvariant()));
    }

    // "lwin+ctrl+Left", "Super+Ctrl+Left", "rwin+left" aynı kombinasyonu aynı biçime getirir: değiştiriciler sıralı, tuş sonda
    internal static string Combo(string combo)
    {
        var mods = new SortedSet<string>(StringComparer.Ordinal);
        string key = "";
        foreach (string raw in combo.Split('+'))
        {
            string p = raw.Trim().ToLowerInvariant();
            if (p == "super" || p == "win" || p == "rwin") p = "lwin";
            else if (p == "control") p = "ctrl";
            else if (p == "menu") p = "alt";
            else if (p == "pageup") p = "page_up";
            else if (p == "pagedown") p = "page_down";
            if (p == "lwin" || p == "ctrl" || p == "alt" || p == "shift") mods.Add(p); else key = p;
        }
        return string.Join("+", mods.Concat(new[] { key }));
    }

    // Saf birleştirme. user/def: config.yaml metinleri, offered: daha önce değerlendirilmiş anahtarlar, taken: yaml dışında
    // kullanılan kombinasyonlar (çekirdeğin kendi kısayolları, Windows'un ayırdıkları). Dönüş: yeni metin (değişmediyse
    // aynı nesne); newOffered: güncel değerlendirilmiş kümesi; added: eklenen girdi sayısı.
    internal static string Merge(string user, string def, ISet<string> offered, ISet<string> taken, out HashSet<string> newOffered, out int added)
    {
        added = 0;
        newOffered = new HashSet<string>(offered ?? new HashSet<string>(), StringComparer.Ordinal);
        string eol = user.Contains("\r\n") ? "\r\n" : "\n";
        string[] userLines = user.Replace("\r\n", "\n").Split('\n');
        string[] defLines = def.Replace("\r\n", "\n").Split('\n');
        var userEntries = WmBinds.Read(userLines);
        var defEntries = WmBinds.Read(defLines);
        if (userEntries.Count == 0 || defEntries.Count == 0) return user; // kısayol bölümü yoksa elle yazılmış bir dosya: dokunma

        // yaml'daki her bağlamadaki kombinasyonlar (binding_modes dahil: tutucu olalım) + yaml dışındakiler
        var used = new HashSet<string>(StringComparer.Ordinal);
        foreach (string l in userLines)
        {
            var m = Regex.Match(l, @"^\s*bindings:\s*\[(.*)\]\s*$");
            if (m.Success) foreach (Match q in Regex.Matches(m.Groups[1].Value, "'([^']*)'")) used.Add(Combo(q.Groups[1].Value));
        }
        if (taken != null) foreach (string t in taken) used.Add(Combo(t));
        var have = new HashSet<string>(userEntries.Select(e => Key(e.Commands)), StringComparer.Ordinal);

        // son kısayol girdisinin altına, onun girintisiyle
        var last = userEntries[userEntries.Count - 1];
        string bindIndent = Regex.Match(userLines[last.Line], @"^\s*").Value;
        string dashIndent = bindIndent.Length >= 2 ? bindIndent.Substring(2) : "";
        var block = new List<string> { dashIndent + "# yeni varsayılan kısayollar (güncellemeyle eklendi)" };
        foreach (var e in defEntries)
        {
            string key = Key(e.Commands);
            if (offered != null && offered.Contains(key)) continue;
            newOffered.Add(key);
            if (have.Contains(key)) continue;
            var free = new List<string>();
            foreach (string b in e.Bindings)
                if (used.Add(Combo(b))) free.Add(b); // aynı turda iki varsayılan aynı tuşu istemesin
            if (free.Count == 0) continue;
            block.Add(dashIndent + "- commands: [" + string.Join(", ", e.Commands.Select(c => "'" + c.Replace("'", "") + "'")) + "]");
            block.Add(bindIndent + "bindings: [" + string.Join(", ", free.Select(b => "'" + b.Replace("'", "") + "'")) + "]");
            added++;
        }
        if (added == 0) return user;
        var outLines = new List<string>(userLines);
        outLines.InsertRange(last.Line + 1, block);
        return string.Join(eol, outLines);
    }

    // kullanıcıdan gelen kombinasyonlar dışında kalan, çekirdeğin ve Windows'un tuşları
    static HashSet<string> CoreCombos()
    {
        var set = new HashSet<string>(StringComparer.Ordinal);
        try { foreach (var v in Binds.Effective().Values) if (!string.IsNullOrEmpty(v)) set.Add(Combo(v)); } catch (Exception ex) { Slider.Log("kısayol göçü: çekirdek tuşları okunamadı: " + ex.Message); }
        for (int i = 0; i < Reserved.Combos.GetLength(0); i++) set.Add(Combo(Reserved.Combos[i, 0]));
        return set;
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
        catch (Exception ex) { Slider.Log("kısayol göçü: durum dosyası okunamadı, atlandı: " + ex.Message); return false; }
    }

    internal static string Sha(string text)
    {
        using (var sha = System.Security.Cryptography.SHA256.Create())
            return BitConverter.ToString(sha.ComputeHash(Encoding.UTF8.GetBytes(text))).Replace("-", "");
    }

    // Çekirdek açılışında (güncellemeden sonra çekirdek yeniden başladığı için o da kapsanır) bir kez
    public static void Run()
    {
        try
        {
            if (!System.IO.File.Exists(DefaultPath) || !System.IO.File.Exists(Paths.ConfigFile)) return;
            HashSet<string> offered;
            if (!ReadOffered(out offered)) return;
            string user = System.IO.File.ReadAllText(Paths.ConfigFile);
            string def = System.IO.File.ReadAllText(DefaultPath);
            HashSet<string> now; int added;
            string next = Merge(user, def, offered, CoreCombos(), out now, out added);
            if (added > 0)
            {
                if (!Files.WriteAtomic(Paths.ConfigFile, next)) { Slider.Log("kısayol göçü: config.yaml yazılamadı"); return; }
                // kurucunun "dokunulmamış config" işareti: dosya onun yazdığıysa öyle kalsın (sonraki güncelleme tümünü yenileyebilsin)
                try
                {
                    string shaFile = Paths.State("config.sha256");
                    if (System.IO.File.Exists(shaFile) && System.IO.File.ReadAllText(shaFile).Trim().Equals(Sha(user), StringComparison.OrdinalIgnoreCase))
                        Files.WriteAtomic(shaFile, Sha(next) + Environment.NewLine);
                }
                catch (Exception ex) { Slider.Log("kısayol göçü: config.sha256 güncellenemedi: " + ex.Message); }
            }
            if (now.Count != offered.Count)
                if (!Files.WriteAtomic(OfferedPath, new JavaScriptSerializer().Serialize(new Dictionary<string, object> { { "offered", now.OrderBy(k => k, StringComparer.Ordinal).ToList() } })))
                    Slider.Log("kısayol göçü: durum dosyası yazılamadı");
            if (added > 0)
            {
                Slider.Log("kısayol göçü: " + added + " yeni varsayılan kısayol eklendi");
                try { new TilingClient().Command("wm-reload-config"); } catch { }
            }
        }
        catch (Exception ex) { Slider.Log("kısayol göçü: " + ex.Message); }
    }
}
