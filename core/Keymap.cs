// Kısayol haritası: çekirdeğin kısayolları (keybinds.json, Binds), kullanıcının eklediği uygulama kısayolları,
// pencere yöneticisinin kısayolları (config.yaml > keybindings, WmBinds) ve Windows'un bırakmadığı kombinasyonlar
// (Reserved) tek yerde. Düzenleyiciler (sağ panel) modeli buradan okur, çakışmaları buraya sordurur ve kaydı buradan yapar.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Web.Script.Serialization;

// Windows'un hiçbir kancaya bırakmadığı ya da bırakmaması gereken kombinasyonlar. Eylemi olan (Super+L) çekirdekte o
// eylemi çalıştırır: Win tuşu Windows'a hiç iletilmediği için kilidi Windows'un kendisi göremez.
static class Reserved
{
    public static readonly string[,] Combos = {
        { "Super+L", "lock", "Bilgisayarı kilitle" },
        { "Ctrl+Alt+Delete", "", "Windows güvenlik ekranı" },
    };

    public static string Action(int mods, int vk)
    {
        for (int i = 0; i < Combos.GetLength(0); i++)
        {
            int m, k;
            if (Binds.Parse(Combos[i, 0], out m, out k) && m == mods && k == vk) return Combos[i, 1];
        }
        return null;
    }
}

// Pencere yöneticisinin kısayolları (config.yaml > keybindings). Win tuşu Windows'a hiç iletilmediği için pencere
// yöneticisi Super'li kısayollarını kendisi göremez: çekirdek onları bu tablodan bulup komutlarını IPC ile gönderir.
// Super'sizler (ctrl+alt+t) ve kısayol modlarının (binding_modes, örn. boyutlandırma) tuşları da buradan: pencere
// yöneticisinin klavye kancası yok, sistemde tek klavye kancası çekirdeğinki.
static class WmBinds
{
    internal sealed class Entry { public List<string> Commands = new List<string>(); public List<string> Bindings = new List<string>(); public int Line = -1; }

    static readonly object gate = new object();
    static Dictionary<long, string[]> table = new Dictionary<long, string[]>();
    // binding_modes: ad -> tablo; açık mod pencere yöneticisinden (WmModes)
    static Dictionary<string, Dictionary<long, string[]>> modes = new Dictionary<string, Dictionary<long, string[]>>();
    static string mode;
    static string OrigPath { get { return Paths.State("tiling-keybindings.default.json"); } }

    static readonly Dictionary<string, string> toUi = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase) {
        { "lwin", "Super" }, { "rwin", "Super" }, { "ctrl", "Ctrl" }, { "control", "Ctrl" }, { "shift", "Shift" }, { "alt", "Alt" }, { "menu", "Alt" },
        { "page_up", "PageUp" }, { "page_down", "PageDown" }, { "oem_1", ";" }, { "oem_7", "'" }, { "oem_comma", "Comma" }, { "oem_period", "Period" },
        { "oem_minus", "Minus" }, { "oem_plus", "Plus" },
    };

    public static string ToUi(string binding)
    {
        return string.Join("+", binding.Split('+').Select(p =>
        {
            string v;
            if (toUi.TryGetValue(p, out v)) return v;
            if (p.Length == 1) return p.ToUpperInvariant();
            return p.Length == 0 ? p : char.ToUpperInvariant(p[0]) + p.Substring(1);
        }));
    }

    public static string ToWm(string combo)
    {
        return string.Join("+", combo.Split('+').Select(p =>
        {
            foreach (var kv in toUi) if (kv.Value == p && kv.Key != "rwin" && kv.Key != "control" && kv.Key != "menu") return kv.Key;
            return p.ToLowerInvariant();
        }));
    }

    static readonly Regex quoted = new Regex("'([^']*)'");

    // Üst düzey "keybindings:" listesi: "- commands: [...]" ardından "bindings: [...]" (binding_modes ayrı, dokunulmaz)
    internal static List<Entry> Read(string[] lines)
    {
        var list = new List<Entry>();
        bool inKb = false;
        Entry pending = null;
        for (int i = 0; i < lines.Length; i++)
        {
            string l = lines[i];
            if (l.StartsWith("keybindings:")) { inKb = true; continue; }
            if (inKb && l.Length > 0 && !char.IsWhiteSpace(l[0]) && !l.StartsWith("#")) break;
            if (!inKb) continue;
            var c = Regex.Match(l, @"^\s*- commands:\s*\[(.*)\]\s*$");
            if (c.Success) { pending = new Entry(); foreach (Match m in quoted.Matches(c.Groups[1].Value)) pending.Commands.Add(m.Groups[1].Value); continue; }
            var b = Regex.Match(l, @"^\s*bindings:\s*\[(.*)\]\s*$");
            if (pending != null && b.Success)
            {
                foreach (Match m in quoted.Matches(b.Groups[1].Value)) pending.Bindings.Add(m.Groups[1].Value);
                pending.Line = i;
                list.Add(pending);
                pending = null;
            }
        }
        return list;
    }

    static string[] Lines()
    {
        try { return System.IO.File.Exists(Paths.ConfigFile) ? System.IO.File.ReadAllLines(Paths.ConfigFile) : new string[0]; }
        catch (Exception ex) { Slider.Log("pencere yöneticisi kısayolları okunamadı: " + ex.Message); return null; }
    }

    // Kancanın tablosu: pencere yöneticisinin bütün kısayolları ve kısayol modları
    public static void Load()
    {
        var lines = Lines();
        if (lines == null) return;
        var t = Table(Read(lines));
        var ms = new Dictionary<string, Dictionary<long, string[]>>();
        foreach (var kv in ReadModes(lines)) ms[kv.Key] = Table(kv.Value);
        lock (gate) { table = t; modes = ms; }
        NativeInput.PushTable();
    }

    static Dictionary<long, string[]> Table(List<Entry> entries)
    {
        var t = new Dictionary<long, string[]>();
        foreach (var e in entries)
            foreach (var b in e.Bindings)
            {
                int m, vk;
                if (!Binds.Parse(ToUi(b), out m, out vk)) continue;
                long key = ((long)m << 16) | (uint)vk;
                if (!t.ContainsKey(key)) t[key] = e.Commands.ToArray();
            }
        return t;
    }

    // Açık bir kısayol modunda pencere yöneticisi gibi: yalnızca modun tuşları; Super'li kısayollar yine çalışır
    // (Win'i pencere yöneticisi hiç görmediği için onları hep çekirdek gönderiyordu)
    public static string[] Lookup(int mods, int vk)
    {
        long key = ((long)mods << 16) | (uint)vk;
        string[] c;
        Dictionary<long, string[]> m;
        lock (gate)
        {
            if (mode != null && modes.TryGetValue(mode, out m))
            {
                if (m.TryGetValue(key, out c)) return c;
                if ((mods & Binds.SUPER) == 0) return null;
            }
            return table.TryGetValue(key, out c) ? c : null;
        }
    }

    // Yerel kancanın tablosu: Lookup'ın o anki karşılığı
    public static List<KeyValuePair<long, string[]>> Active()
    {
        lock (gate)
        {
            Dictionary<long, string[]> m;
            if (mode == null || !modes.TryGetValue(mode, out m)) return new List<KeyValuePair<long, string[]>>(table);
            var list = new List<KeyValuePair<long, string[]>>(m);
            foreach (var kv in table)
                if (((kv.Key >> 16) & Binds.SUPER) != 0 && !m.ContainsKey(kv.Key)) list.Add(kv);
            return list;
        }
    }

    // Pencere yöneticisinin açık kısayol modu (WmModes; yoksa null)
    public static void SetMode(string name)
    {
        lock (gate)
        {
            if (name == mode) return;
            mode = name;
        }
        Slider.Log("pencere yöneticisi kısayol modu: " + (name ?? "yok"));
        NativeInput.PushTable();
    }

    // binding_modes: "- name: '...'" ve altında "keybindings:" listesi
    static Dictionary<string, List<Entry>> ReadModes(string[] lines)
    {
        var result = new Dictionary<string, List<Entry>>();
        bool inModes = false;
        string name = null;
        Entry pending = null;
        foreach (string l in lines)
        {
            if (l.StartsWith("binding_modes:")) { inModes = true; continue; }
            if (inModes && l.Length > 0 && !char.IsWhiteSpace(l[0]) && !l.StartsWith("#")) break;
            if (!inModes) continue;
            var n = Regex.Match(l, @"^\s*- name:\s*'([^']*)'");
            if (n.Success) { name = n.Groups[1].Value; result[name] = new List<Entry>(); pending = null; continue; }
            if (name == null) continue;
            var c = Regex.Match(l, @"^\s*- commands:\s*\[(.*)\]\s*$");
            if (c.Success) { pending = new Entry(); foreach (Match m in quoted.Matches(c.Groups[1].Value)) pending.Commands.Add(m.Groups[1].Value); continue; }
            var b = Regex.Match(l, @"^\s*bindings:\s*\[(.*)\]\s*$");
            if (pending != null && b.Success)
            {
                foreach (Match m in quoted.Matches(b.Groups[1].Value)) pending.Bindings.Add(m.Groups[1].Value);
                result[name].Add(pending);
                pending = null;
            }
        }
        return result;
    }

    // Basılı tutunca tekrar eden kısayol mu: yalnızca bölme oranı ve boyutlandırma (Hyprland/ii'de "binde"); geçiş,
    // odak ve durum komutları basış başına bir kez
    public static bool Repeats(string[] commands)
    {
        if (commands == null || commands.Length == 0) return false;
        foreach (var cmd in commands)
            if (!(cmd.StartsWith("split-ratio ") || cmd.StartsWith("resize "))) return false;
        return true;
    }

    // Düzenleyici için: [{"index","commands","bindings"(Super+F biçiminde)}]
    public static List<Dictionary<string, object>> List()
    {
        var list = new List<Dictionary<string, object>>();
        var lines = Lines() ?? new string[0];
        var entries = Read(lines);
        for (int i = 0; i < entries.Count; i++)
            list.Add(new Dictionary<string, object> {
                { "index", i }, { "commands", entries[i].Commands }, { "bindings", entries[i].Bindings.Select(ToUi).ToList() },
            });
        return list;
    }

    // Değişen girdilerin kısayollarını yazar (index -> Super+F biçiminde liste). İlk değişiklikten önce özgün
    // kısayollar bir kez saklanır (sıfırlama için).
    public static bool Write(Dictionary<int, List<string>> changes)
    {
        if (changes.Count == 0) return true;
        var lines = Lines();
        if (lines == null) return false;
        var entries = Read(lines);
        if (!System.IO.File.Exists(OrigPath))
        {
            var orig = entries.Select(e => e.Bindings).ToList();
            if (!Files.WriteAtomic(OrigPath, new JavaScriptSerializer().Serialize(orig))) return false;
        }
        foreach (var kv in changes)
        {
            if (kv.Key < 0 || kv.Key >= entries.Count) return false;
            lines[entries[kv.Key].Line] = BindingsLine(lines[entries[kv.Key].Line], kv.Value.Select(ToWm));
        }
        return Save(lines);
    }

    static string BindingsLine(string old, IEnumerable<string> bindings)
    {
        string indent = Regex.Match(old, @"^\s*").Value;
        return indent + "bindings: [" + string.Join(", ", bindings.Select(b => "'" + b.Replace("'", "") + "'")) + "]";
    }

    public static bool Reset()
    {
        if (!System.IO.File.Exists(OrigPath)) return true;
        var lines = Lines();
        if (lines == null) return false;
        var entries = Read(lines);
        try
        {
            var saved = new JavaScriptSerializer().Deserialize<List<List<string>>>(System.IO.File.ReadAllText(OrigPath));
            for (int i = 0; i < entries.Count && saved != null && i < saved.Count; i++)
                if (saved[i] != null) lines[entries[i].Line] = BindingsLine(lines[entries[i].Line], saved[i]);
        }
        catch (Exception ex) { Slider.Log("pencere yöneticisi kısayolları sıfırlanamadı: " + ex.Message); return false; }
        if (!Save(lines)) return false;
        try { System.IO.File.Delete(OrigPath); } catch { }
        return true;
    }

    static bool Save(string[] lines)
    {
        if (!Files.WriteAtomic(Paths.ConfigFile, string.Join("\r\n", lines) + "\r\n")) return false;
        Load();
        try { new TilingClient().Command("wm-reload-config"); } catch { }
        return true;
    }
}

// Kısayol modeli ve kaydı (düzenleyiciler: lunge.exe --keybinds-model / --keybinds-check / --keybinds-save / --keybinds-reset)
static class Keymap
{
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    // Düzenleyicinin satır anahtarları: "ll:<id>", "tiling:<index>", "reserved:<combo>"
    sealed class Item { public string Key, Combo, Label; }

    // Aynı tuşlara bağlı birden fazla satır ya da Windows'un bırakmadığı bir kombinasyon: [{"combo","keys":[...],"reserved"}]
    public static List<Dictionary<string, object>> Conflicts(Dictionary<string, string> core, Dictionary<int, List<string>> tiling)
    {
        var items = new List<Item>();
        foreach (var kv in core) items.Add(new Item { Key = "ll:" + kv.Key, Combo = kv.Value });
        foreach (var kv in tiling) foreach (var b in kv.Value) items.Add(new Item { Key = "tiling:" + kv.Key, Combo = b });
        for (int i = 0; i < Reserved.Combos.GetLength(0); i++)
            items.Add(new Item { Key = "reserved:" + Reserved.Combos[i, 0], Combo = Reserved.Combos[i, 0], Label = Reserved.Combos[i, 2] });
        var groups = new Dictionary<long, List<Item>>();
        foreach (var it in items)
        {
            int m, vk;
            if (string.IsNullOrEmpty(it.Combo) || !Binds.Parse(it.Combo, out m, out vk)) continue;
            long k = ((long)m << 16) | (uint)vk;
            List<Item> g;
            if (!groups.TryGetValue(k, out g)) groups[k] = g = new List<Item>();
            if (!g.Any(x => x.Key == it.Key)) g.Add(it);
        }
        var list = new List<Dictionary<string, object>>();
        foreach (var g in groups.Values)
        {
            var real = g.Where(x => !x.Key.StartsWith("reserved:")).ToList();
            var reserved = g.FirstOrDefault(x => x.Key.StartsWith("reserved:"));
            if (real.Count + (reserved != null ? 1 : 0) < 2) continue;
            list.Add(new Dictionary<string, object> {
                { "combo", Binds.Canonical(g[0].Combo) }, { "keys", real.Select(x => x.Key).ToList() },
                { "reserved", reserved != null ? reserved.Label : null },
            });
        }
        return list;
    }

    static Dictionary<int, List<string>> TilingNow()
    {
        var d = new Dictionary<int, List<string>>();
        foreach (var e in WmBinds.List()) d[(int)e["index"]] = (List<string>)e["bindings"];
        return d;
    }

    public static string Model()
    {
        var state = Binds.ReadUser();
        var eff = Binds.Effective();
        var core = new List<Dictionary<string, object>>();
        for (int i = 0; i < Binds.Defaults.GetLength(0); i++)
        {
            string id = Binds.Defaults[i, 0];
            core.Add(new Dictionary<string, object> {
                { "id", id }, { "combo", eff[id] }, { "default", Binds.Defaults[i, 1] }, { "app", Binds.IsApp(id) },
                { "custom", false }, { "removed", state.Removed.Contains(id) },
            });
        }
        foreach (var a in state.Apps)
            core.Add(new Dictionary<string, object> {
                { "id", a.Id }, { "combo", a.Combo }, { "default", "" }, { "app", true }, { "custom", true }, { "removed", false },
                { "name", a.Name }, { "path", a.Path },
            });
        var reserved = new List<Dictionary<string, object>>();
        for (int i = 0; i < Reserved.Combos.GetLength(0); i++)
            reserved.Add(new Dictionary<string, object> { { "combo", Reserved.Combos[i, 0] }, { "label", Reserved.Combos[i, 2] } });
        var active = new Dictionary<string, string>();
        foreach (var kv in eff) if (!state.Removed.Contains(kv.Key)) active[kv.Key] = kv.Value;
        foreach (var a in state.Apps) active[a.Id] = a.Combo;
        return json.Serialize(new Dictionary<string, object> {
            { "core", core }, { "tiling", WmBinds.List() }, { "reserved", reserved }, { "conflicts", Conflicts(active, TilingNow()) },
        });
    }

    // Düzenleyicinin bekleyen durumu: {"core":{id:combo}, "apps":[{id,name,path,combo}], "removed":[id], "tiling":{index:[combo]}}
    sealed class Staged
    {
        public Dictionary<string, string> Core = new Dictionary<string, string>();
        public List<Binds.CustomApp> Apps = new List<Binds.CustomApp>();
        public HashSet<string> Removed = new HashSet<string>();
        public Dictionary<int, List<string>> Tiling = new Dictionary<int, List<string>>();
    }

    static Staged Parse(string text, out string error)
    {
        error = null;
        var s = new Staged();
        Dictionary<string, object> d;
        try { d = json.DeserializeObject(text) as Dictionary<string, object>; }
        catch { error = "json"; return null; }
        if (d == null) { error = "json"; return null; }
        object v;
        if (d.TryGetValue("core", out v) && v is Dictionary<string, object>)
            foreach (var kv in (Dictionary<string, object>)v)
            {
                string combo = kv.Value as string ?? "";
                if (!Binds.IsDefault(kv.Key) && !kv.Key.StartsWith("app:")) { error = "id " + kv.Key; return null; }
                if (combo != "" && Binds.Canonical(combo) == null) { error = "combo " + combo; return null; }
                s.Core[kv.Key] = combo == "" ? "" : Binds.Canonical(combo);
            }
        if (d.TryGetValue("apps", out v) && List(v) != null)
            foreach (var o in List(v))
            {
                var a = o as Dictionary<string, object>;
                if (a == null) { error = "app"; return null; }
                var app = new Binds.CustomApp { Id = Str(a, "id"), Name = Str(a, "name"), Path = Str(a, "path"), Combo = Str(a, "combo") };
                string why = Binds.ValidateApp(app);
                if (why != null) { error = why; return null; }
                if (app.Combo != "") app.Combo = Binds.Canonical(app.Combo);
                if (s.Apps.Any(x => x.Id == app.Id)) { error = "app id " + app.Id; return null; }
                s.Apps.Add(app);
            }
        if (d.TryGetValue("removed", out v) && List(v) != null)
            foreach (var o in List(v))
            {
                string id = o as string;
                if (id == null || !Binds.IsApp(id) || !Binds.IsDefault(id)) { error = "removed " + id; return null; }
                s.Removed.Add(id);
            }
        if (d.TryGetValue("tiling", out v) && v is Dictionary<string, object>)
            foreach (var kv in (Dictionary<string, object>)v)
            {
                int index;
                var arr = List(kv.Value);
                if (!int.TryParse(kv.Key, out index) || arr == null) { error = "tiling " + kv.Key; return null; }
                var list = new List<string>();
                foreach (var b in arr)
                {
                    string c = b as string;
                    if (string.IsNullOrEmpty(c) || Binds.Canonical(c) == null) { error = "combo " + c; return null; }
                    list.Add(Binds.Canonical(c));
                }
                s.Tiling[index] = list;
            }
        return s;
    }

    // JSON dizisi: DeserializeObject object[], Deserialize<...> ArrayList verir; ikisi de kabul (dizi değilse null)
    static List<object> List(object v)
    {
        if (v == null || v is string) return null;
        var e = v as System.Collections.IEnumerable;
        if (e == null || v is Dictionary<string, object>) return null;
        var list = new List<object>();
        foreach (var o in e) list.Add(o);
        return list;
    }

    static string Str(Dictionary<string, object> d, string k) { object v; return d.TryGetValue(k, out v) && v is string ? (string)v : ""; }

    // Bekleyen durumun tamamı (kaydedilmemiş değişiklikler + kalan her şey) ve çakışmaları
    static List<Dictionary<string, object>> ConflictsOf(Staged s)
    {
        var eff = Binds.Effective();
        var active = new Dictionary<string, string>();
        foreach (var kv in eff) if (!s.Removed.Contains(kv.Key)) active[kv.Key] = s.Core.ContainsKey(kv.Key) ? s.Core[kv.Key] : kv.Value;
        foreach (var a in s.Apps) active[a.Id] = a.Combo;
        var tiling = TilingNow();
        foreach (var kv in s.Tiling) tiling[kv.Key] = kv.Value;
        return Conflicts(active, tiling);
    }

    public static string Check(string text)
    {
        string error;
        var s = Parse(text, out error);
        if (s == null) return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", error } });
        return json.Serialize(new Dictionary<string, object> { { "ok", true }, { "conflicts", ConflictsOf(s) } });
    }

    public static string Save(string text)
    {
        string error;
        var s = Parse(text, out error);
        if (s == null) return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", error } });
        var conflicts = ConflictsOf(s);
        if (conflicts.Count > 0) return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", "conflict" }, { "conflicts", conflicts } });
        if (!Binds.Write(s.Core, s.Apps, s.Removed)) return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", "keybinds" } });
        if (!WmBinds.Write(s.Tiling)) return json.Serialize(new Dictionary<string, object> { { "ok", false }, { "error", "config" } });
        return json.Serialize(new Dictionary<string, object> { { "ok", true } });
    }

    // Düzenleyicinin "Gözat"ı: {"path","name"} ya da vazgeçildiyse {}
    public static string PickApp()
    {
        bool tr = System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName == "tr";
        using (var d = new System.Windows.Forms.OpenFileDialog
        {
            Title = tr ? "Uygulama seç" : "Choose an app",
            Filter = (tr ? "Uygulamalar" : "Apps") + "|*.exe;*.lnk;*.url;*.appref-ms;*.bat;*.cmd",
            InitialDirectory = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles),
        })
        {
            if (d.ShowDialog() != System.Windows.Forms.DialogResult.OK) return "{}";
            string name = System.IO.Path.GetFileNameWithoutExtension(d.FileName);
            try
            {
                var info = System.Diagnostics.FileVersionInfo.GetVersionInfo(d.FileName);
                if (!string.IsNullOrWhiteSpace(info.FileDescription)) name = info.FileDescription.Trim();
            }
            catch { }
            return json.Serialize(new Dictionary<string, object> { { "path", d.FileName }, { "name", name } });
        }
    }

    // Her şey varsayılana; apps: kullanıcının eklediği uygulama kısayolları da silinir
    public static string Reset(bool apps)
    {
        // kalan uygulamalar tuşlarını korur (varsayılanları yok): çakışırlarsa düzenleyici kırmızı gösterir
        var keep = apps ? new List<Binds.CustomApp>() : Binds.ReadUser().Apps;
        bool ok = Binds.Write(new Dictionary<string, string>(), keep, new HashSet<string>(), true) && WmBinds.Reset();
        return json.Serialize(new Dictionary<string, object> { { "ok", ok } });
    }
}

// Pencere yöneticisinin açık kısayol modu: kendi kancası olmadığı için modun tuşlarını (boyutlandırmada oklar, Esc) çekirdek
// yakalar; mod pencere yöneticisinde değişir (kısayolla, barın mod göstergesine tıklayınca, komut satırından). Bir abonelikle
// izlenir; bağlantı koparsa (pencere yöneticisi yeniden başladı: modlar da sıfırlandı) mod kapanır, pencere yöneticisi
// bekçisinin döngüsü aboneliği yeniden açar.
static class WmModes
{
    static int running;

    public static void Ensure()
    {
        if (Interlocked.CompareExchange(ref running, 1, 0) != 0) return;
        new Thread(Run) { IsBackground = true, Name = "wm-modes" }.Start();
    }

    static void Run()
    {
        try
        {
            using (var ws = new System.Net.WebSockets.ClientWebSocket())
            {
                ws.Options.Proxy = null;
                if (!ws.ConnectAsync(new Uri("ws://127.0.0.1:6123"), CancellationToken.None).Wait(3000)) return;
                // önce abonelik, sonra o anki durum: arada bir değişim kaçmasın
                Send(ws, "sub -e binding_modes_changed");
                Send(ws, "query binding-modes");
                var json = new JavaScriptSerializer { MaxJsonLength = int.MaxValue };
                var buf = new byte[1 << 16];
                while (true)
                {
                    string text = Receive(ws, buf);
                    if (text == null) return;
                    var msg = json.DeserializeObject(text) as Dictionary<string, object>;
                    object data;
                    if (msg == null || !msg.TryGetValue("data", out data)) continue;
                    var d = data as Dictionary<string, object>;
                    if (d == null) continue;
                    object list;
                    if (d.TryGetValue("newBindingModes", out list) || d.TryGetValue("bindingModes", out list))
                        WmBinds.SetMode(First(list));
                }
            }
        }
        catch (Exception ex) { Slider.Log("kısayol modu aboneliği: " + ex.GetBaseException().Message); }
        finally
        {
            WmBinds.SetMode(null);
            Interlocked.Exchange(ref running, 0);
        }
    }

    static string First(object list)
    {
        var arr = list as object[];
        if (arr == null && list is System.Collections.ArrayList) arr = ((System.Collections.ArrayList)list).ToArray();
        if (arr == null || arr.Length == 0) return null;
        var m = arr[0] as Dictionary<string, object>;
        object name;
        return m != null && m.TryGetValue("name", out name) ? name as string : null;
    }

    static void Send(System.Net.WebSockets.ClientWebSocket ws, string message)
    {
        var bytes = Encoding.UTF8.GetBytes(message);
        if (!ws.SendAsync(new ArraySegment<byte>(bytes), System.Net.WebSockets.WebSocketMessageType.Text, true, CancellationToken.None).Wait(3000))
            throw new TimeoutException("gönderilemedi");
    }

    // Abonelik boşta günlerce bekleyebilir: süre sınırı yok, bağlantı kapanınca null
    static string Receive(System.Net.WebSockets.ClientWebSocket ws, byte[] buf)
    {
        var sb = new StringBuilder();
        while (true)
        {
            var r = ws.ReceiveAsync(new ArraySegment<byte>(buf), CancellationToken.None).Result;
            if (r.MessageType == System.Net.WebSockets.WebSocketMessageType.Close) return null;
            sb.Append(Encoding.UTF8.GetString(buf, 0, r.Count));
            if (r.EndOfMessage) return sb.ToString();
        }
    }
}
