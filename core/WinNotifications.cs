using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using Microsoft.Win32;

// Windows bildirimleri: uygulamaların Windows'a gönderdiği bildirimler Logical Lunge kartı olarak çıkar ve sağ paneldeki
// listeye girer. Kaynak Bildirim Merkezi'nin veritabanı (wpndatabase.db); Windows'un kendi SQLite'ı (winsqlite3.dll) ile
// süreç içinde okunur. Eskiden panel açıkken 5 sn'de bir PowerShell + Scoop'tan sqlite3.exe başlatılıyordu (yoksa liste
// boş kalıyordu) ve açılır kart hiç yoktu.
// Değişiklik, SQLite'ın paylaşılan bellek dosyasının (-shm) başlığından anlaşılır: her yazmada artan sayaç orada. Saniyede
// bir 48 baytlık okuma; veritabanı yalnızca değişince açılır.
static class WinNotifications
{
    const int LIST_MAX = 150;
    const int POLL_MS = 1000;
    const long FRESH_MS = 120000; // bundan eski bildirim (listeye sonradan kayan) kart olarak gösterilmez
    const int IMAGE_MAX = 128 * 1024; // kart olay akışından geçer: görsel küçük kalsın

    sealed class Item
    {
        public long Id, Arrival, Time;
        public string Aumid, App, Icon, Open, Title, Body;
    }

    sealed class AppInfo
    {
        public string Name, Icon, Open;
    }

    static readonly object gate = new object();
    static List<Item> list = new List<Item>(); // en yeni önce
    static readonly Dictionary<long, Item> items = new Dictionary<long, Item>();
    static readonly Dictionary<string, AppInfo> apps = new Dictionary<string, AppInfo>(StringComparer.OrdinalIgnoreCase);
    static Timer poll;
    static int busy;
    static bool loaded; // ilk okuma yapıldı (öncekiler kart olarak gösterilmez)
    static string stamp;
    static bool copyMode; // veritabanı doğrudan açılamadı: kopyası okunur (en çok 5 sn'de bir)
    static DateTime lastCopy, lastWarn;

    static string Dir
    {
        get { return System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), @"Microsoft\Windows\Notifications"); }
    }

    public static void Start()
    {
        Prefs.WinToastsChanged += () => ThreadPool.QueueUserWorkItem(_ => ApplyBanners());
        poll = new Timer(_ => Tick(), null, 2000, POLL_MS);
    }

    // Sağ panel: {"items":[{id,time,app,aumid,title,body}], "icons":{aumid: data:}} (simge uygulama başına bir kez)
    public static string Json()
    {
        var outItems = new List<object>();
        var icons = new Dictionary<string, object>();
        lock (gate)
        {
            foreach (var it in list)
            {
                outItems.Add(new Dictionary<string, object> { { "id", it.Id }, { "time", it.Time }, { "app", it.App }, { "aumid", it.Aumid }, { "title", it.Title }, { "body", it.Body } });
                if (it.Icon != null && !icons.ContainsKey(it.Aumid)) icons[it.Aumid] = it.Icon;
            }
        }
        return new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.Serialize(new Dictionary<string, object> { { "items", outItems }, { "icons", icons } });
    }

    // /notification-open?id=: bildirimi gönderen uygulamayı açar (Super menüsündeki gibi). false: bildirim ya da uygulaması yok.
    public static bool Open(long id)
    {
        Item it;
        lock (gate) items.TryGetValue(id, out it);
        return it != null && it.Open != null && UserLaunch.Start(it.Open, "", Paths.Home);
    }

    static void Tick()
    {
        if (Interlocked.Exchange(ref busy, 1) == 1) return;
        try
        {
            string now = Stamp();
            if (now == null || now == stamp) return;
            if (copyMode && (DateTime.UtcNow - lastCopy).TotalSeconds < 5) return; // değişiklik kalır, sonra okunur
            if (Read()) stamp = now;
        }
        catch (Exception ex) { Warn(ex.GetBaseException().Message); }
        finally { Interlocked.Exchange(ref busy, 0); }
    }

    // Bu Windows'ta okunamaz (winsqlite3.dll yok): yoklama durur
    static void Disable(string why)
    {
        Warn(why);
        var t = poll;
        if (t != null) t.Change(Timeout.Infinite, Timeout.Infinite);
    }

    // -shm başlığı (wal-index: işlem sayacı, son kare, tuzlar) + dosya boyları. Okuma tanıtıcıyla: başka sürecin açık
    // tuttuğu dosyanın klasör kaydındaki boyu / zamanı gecikebiliyor.
    static string Stamp()
    {
        string db = System.IO.Path.Combine(Dir, "wpndatabase.db");
        if (!System.IO.File.Exists(db)) return null;
        var sb = new StringBuilder();
        foreach (var name in new[] { "wpndatabase.db-shm", "wpndatabase.db-wal", "wpndatabase.db" })
        {
            try
            {
                using (var fs = new System.IO.FileStream(System.IO.Path.Combine(Dir, name), System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
                {
                    sb.Append(fs.Length).Append(':');
                    if (name.EndsWith("-shm"))
                    {
                        var head = new byte[48];
                        int n = fs.Read(head, 0, head.Length);
                        sb.Append(BitConverter.ToString(head, 0, n));
                    }
                }
            }
            catch (System.IO.FileNotFoundException) { sb.Append('-'); }
            catch (System.IO.IOException) { sb.Append('?'); }
            sb.Append('|');
        }
        return sb.ToString();
    }

    // Son LIST_MAX bildirimin kimlik + varış zamanı; yeni (ya da kimliği yeniden kullanılmış) olanların içeriği okunur,
    // bilinenler önbellekte. false: okunamadı.
    static bool Read()
    {
        List<KeyValuePair<long, long>> ids = null;
        var fresh = new Dictionary<long, KeyValuePair<Item, ToastPayload>>();
        var handlers = new List<string>();
        bool firstLoad = !loaded;
        lock (gate) if (apps.Count > 200) apps.Clear();
        // Doğrudan okuma yarıda kalıp kopyaya düşülürse iş baştan çalışır
        Func<IntPtr, bool> work = db =>
        {
            ids = new List<KeyValuePair<long, long>>();
            fresh.Clear();
            handlers.Clear();
            WinSqlite.Each(db, "SELECT Id, ArrivalTime FROM Notification WHERE Type = 'toast' ORDER BY ArrivalTime DESC, Id DESC LIMIT " + LIST_MAX, null,
                st => ids.Add(new KeyValuePair<long, long>(WinSqlite.sqlite3_column_int64(st, 0), WinSqlite.sqlite3_column_int64(st, 1))));
            var unknown = new List<long>();
            lock (gate)
                foreach (var kv in ids)
                {
                    Item known;
                    if (!items.TryGetValue(kv.Key, out known) || known.Arrival != kv.Value) unknown.Add(kv.Key);
                }
            foreach (var id in unknown)
                WinSqlite.Each(db, "SELECT n.ArrivalTime, h.PrimaryId, CAST(n.Payload AS TEXT) FROM Notification n JOIN NotificationHandler h ON n.HandlerId = h.RecordId WHERE n.Id = ?1", id, st =>
                {
                    var p = ToastPayload.Parse(WinSqlite.Text(st, 2));
                    long arrival = WinSqlite.sqlite3_column_int64(st, 0);
                    var it = new Item { Id = id, Arrival = arrival, Time = ToastPayload.UnixMs(arrival), Aumid = WinSqlite.Text(st, 1) ?? "", Title = p.Title, Body = p.Body };
                    fresh[id] = new KeyValuePair<Item, ToastPayload>(it, p);
                });
            if (firstLoad) WinSqlite.Each(db, "SELECT PrimaryId FROM NotificationHandler", null, st => { var a = WinSqlite.Text(st, 0); if (!string.IsNullOrEmpty(a)) handlers.Add(a); });
            return true;
        };
        if (!WithDatabase(work)) return false;

        // Gönderen adı / simgesi (uygulama listesi, kayıt defteri); kart yalnızca ilk okumadan sonra gelen yeni bildirime
        List<Dictionary<string, object>> appList = null;
        long now = ToastPayload.UnixMs(DateTime.UtcNow.ToFileTimeUtc());
        var cards = new List<KeyValuePair<Item, ToastPayload>>();
        foreach (var kv in fresh.Values)
        {
            var app = App(kv.Key.Aumid, ref appList);
            kv.Key.App = app.Name;
            kv.Key.Icon = app.Icon;
            kv.Key.Open = app.Open;
            if (!firstLoad && now - kv.Key.Time < FRESH_MS) cards.Add(kv);
        }
        bool changed;
        lock (gate)
        {
            foreach (var kv in fresh.Values) items[kv.Key.Id] = kv.Key;
            var next = new List<Item>(ids.Count);
            foreach (var kv in ids) { Item it; if (items.TryGetValue(kv.Key, out it)) next.Add(it); }
            // Silinen (Windows'ta kapatılan, uygulamanın geri aldığı) bildirimler önbellekten de çıkar
            var keep = new HashSet<long>();
            foreach (var kv in ids) keep.Add(kv.Key);
            foreach (var id in new List<long>(items.Keys)) if (!keep.Contains(id)) items.Remove(id);
            changed = next.Count != list.Count;
            for (int i = 0; !changed && i < next.Count; i++) changed = !ReferenceEquals(next[i], list[i]);
            list = next;
        }
        loaded = true;

        if (firstLoad) ApplyBanners(handlers);
        else if (fresh.Count > 0)
        {
            var senders = new List<string>();
            foreach (var kv in fresh.Values) senders.Add(kv.Key.Aumid);
            try { ToastBanners.Suppress(senders); }
            catch (Exception ex) { Warn("bildirim balonları: " + ex.GetBaseException().Message); }
        }
        cards.Sort((a, b) => a.Key.Arrival.CompareTo(b.Key.Arrival));
        if (Prefs.WinToasts) foreach (var kv in cards) Card(kv.Key, kv.Value);
        if (changed) Toasts.Emit("ll:notifications");
        return true;
    }

    // Veritabanını salt okunur açar; açılamazsa (kilitli) kopyasını okur
    static bool WithDatabase(Func<IntPtr, bool> work)
    {
        string db = System.IO.Path.Combine(Dir, "wpndatabase.db");
        Exception direct;
        try { if (WinSqlite.Open(db, false, work)) { copyMode = false; return true; } direct = null; }
        catch (DllNotFoundException ex) { Disable("winsqlite3.dll yok: " + ex.Message); return false; }
        catch (EntryPointNotFoundException ex) { Disable("winsqlite3.dll: " + ex.Message); return false; }
        catch (Exception ex) { direct = ex; }
        copyMode = true;
        lastCopy = DateTime.UtcNow;
        // Her kopya için yeni, rastgele adlı klasör: yönetici haklarıyla çalışan çekirdek, kullanıcının %TEMP%'ine önceden
        // konmuş bir bağlantının (junction) gösterdiği yere yazmasın
        string tmp = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "ll-wpn-" + Guid.NewGuid().ToString("N"));
        try
        {
            System.IO.Directory.CreateDirectory(tmp);
            string copy = System.IO.Path.Combine(tmp, "wpndatabase.db");
            CopyShared(db, copy);
            string wal = db + "-wal";
            if (System.IO.File.Exists(wal)) CopyShared(wal, copy + "-wal");
            // -shm kopyalanmaz: SQLite dizini -wal'dan yeniden kurar (yazılırken alınmış yarım kopya bozmasın). Kopya
            // bizim: yazılabilir açılır, yoksa salt okunur bağlantı -shm'yi kuramayabiliyor.
            return WinSqlite.Open(copy, true, work);
        }
        catch (Exception ex)
        {
            Warn("okunamadı: " + (direct != null ? direct.Message + " / " : "") + ex.GetBaseException().Message);
            return false;
        }
        finally { try { System.IO.Directory.Delete(tmp, true); } catch { } }
    }

    static void CopyShared(string from, string to)
    {
        using (var src = new System.IO.FileStream(from, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
        using (var dst = new System.IO.FileStream(to, System.IO.FileMode.CreateNew, System.IO.FileAccess.Write))
            src.CopyTo(dst);
    }

    static void Card(Item it, ToastPayload p)
    {
        var card = new Dictionary<string, object>
        {
            { "kind", "info" }, { "icon", "notifications" }, { "app", it.App },
            { "title", p.Title.Length > 0 ? p.Title : it.App }, { "body", p.Body },
        };
        string image = Image(p.Logo, it.Aumid) ?? it.Icon;
        if (image != null) card["image"] = image;
        // Karta tıklayınca uygulama açılır: kart yalnızca kimliği taşır, hedefi çekirdek kendi listesinden bulur
        if (it.Open != null) card["notification"] = it.Id;
        // Yanıt / erteleme düğmeleri ve arama, alarm gibi bekleyen bildirimler Windows'un Bildirim Merkezi'nde yanıtlanır
        if (p.Interactive || p.Urgent)
            card["actions"] = new object[] { new Dictionary<string, object> { { "label", "Bildirim merkezi" }, { "url", "ms-actioncenter:" } } };
        if (p.Urgent) card["timeout"] = 15000;
        Toasts.Card(card);
    }

    // Gönderenin görseli: http(s) adresi yeniden yazılmış haliyle (tırnak, boşluk kaçışlı), küçük yerel resim data: olarak
    static string Image(string src, string aumid)
    {
        if (string.IsNullOrEmpty(src)) return null;
        Uri u;
        if (Uri.TryCreate(src, UriKind.Absolute, out u) && (u.Scheme == Uri.UriSchemeHttps || u.Scheme == Uri.UriSchemeHttp)) return u.AbsoluteUri;
        string path = ToastPayload.LocalImagePath(src, aumid, Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData));
        return path == null ? null : ImageFile(path);
    }

    static readonly Dictionary<string, string> imageTypes = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
    {
        { ".png", "image/png" }, { ".jpg", "image/jpeg" }, { ".jpeg", "image/jpeg" }, { ".gif", "image/gif" },
        { ".bmp", "image/bmp" }, { ".ico", "image/x-icon" }, { ".webp", "image/webp" }, { ".svg", "image/svg+xml" },
    };

    static string ImageFile(string path)
    {
        try
        {
            // Yalnızca yerel diskteki tam yol: ağ yolu (\\sunucu, \\?\UNC) yönetici haklarıyla kimlik doğrulaması açar ve
            // yoklamayı bekletir
            if (string.IsNullOrEmpty(path) || path.StartsWith(@"\\") || path.StartsWith("//") || !System.IO.Path.IsPathRooted(path) || path.IndexOf(':') != 1) return null;
            string type;
            if (!imageTypes.TryGetValue(System.IO.Path.GetExtension(path), out type)) return null;
            var info = new System.IO.FileInfo(path);
            if (!info.Exists || info.Length == 0 || info.Length > IMAGE_MAX) return null;
            return "data:" + type + ";base64," + Convert.ToBase64String(System.IO.File.ReadAllBytes(path));
        }
        catch { return null; }
    }

    // Gönderen: Super menüsünün uygulama listesi (shell:AppsFolder\<AUMID>), yoksa kayıt defterindeki bildirim kimliği
    // (Software\Classes\AppUserModelId\<AUMID>: DisplayName, IconUri), yoksa AUMID'den okunur ad
    static AppInfo App(string aumid, ref List<Dictionary<string, object>> appList)
    {
        lock (gate) { AppInfo cached; if (apps.TryGetValue(aumid, out cached)) return cached; }
        var info = new AppInfo();
        if (appList == null) appList = ReadApps();
        string path = @"shell:AppsFolder\" + aumid;
        foreach (var a in appList)
        {
            object p, n, i;
            if (!a.TryGetValue("path", out p) || !path.Equals(p as string, StringComparison.OrdinalIgnoreCase)) continue;
            info.Name = a.TryGetValue("name", out n) ? n as string : null;
            info.Icon = a.TryGetValue("icon", out i) ? i as string : null;
            info.Open = path;
            break;
        }
        if (info.Name == null && aumid.Length > 0 && aumid.Length <= 255 && aumid.IndexOf('\\') < 0)
            foreach (var root in new[] { Registry.CurrentUser, Registry.LocalMachine })
            {
                try
                {
                    using (var k = root.OpenSubKey(@"Software\Classes\AppUserModelId\" + aumid))
                    {
                        if (k == null) continue;
                        info.Name = Indirect(k.GetValue("DisplayName") as string);
                        string icon = k.GetValue("IconUri") as string;
                        if (info.Icon == null && !string.IsNullOrEmpty(icon)) info.Icon = ImageFile(Environment.ExpandEnvironmentVariables(icon));
                        if (info.Name != null) break;
                    }
                }
                catch { }
            }
        if (string.IsNullOrEmpty(info.Name)) info.Name = ToastPayload.NameFromAumid(aumid);
        lock (gate) apps[aumid] = info;
        return info;
    }

    static List<Dictionary<string, object>> ReadApps()
    {
        var result = new List<Dictionary<string, object>>();
        try
        {
            var arr = new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.DeserializeObject(System.IO.File.ReadAllText(Paths.AppsJson)) as object[];
            if (arr != null) foreach (var o in arr) { var d = o as Dictionary<string, object>; if (d != null) result.Add(d); }
        }
        catch { }
        return result;
    }

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)]
    static extern int SHLoadIndirectString(string source, StringBuilder buffer, int size, IntPtr reserved);

    // "@%SystemRoot%\system32\x.dll,-123" gibi dolaylı adlar
    static string Indirect(string s)
    {
        if (string.IsNullOrEmpty(s) || s[0] != '@') return string.IsNullOrEmpty(s) ? null : s;
        var sb = new StringBuilder(512);
        return SHLoadIndirectString(s, sb, sb.Capacity, IntPtr.Zero) == 0 && sb.Length > 0 ? sb.ToString() : null;
    }

    // İlk okumada: bilinen tüm göndericiler; ayar değişince açık / kapalı
    static List<string> knownHandlers = new List<string>();
    static void ApplyBanners(List<string> handlers)
    {
        lock (gate) knownHandlers = handlers;
        ApplyBanners();
    }

    static void ApplyBanners()
    {
        try
        {
            if (!Prefs.WinToasts) { ToastBanners.Restore(); return; }
            List<string> all;
            lock (gate) all = new List<string>(knownHandlers);
            all.AddRange(ToastBanners.Configured());
            ToastBanners.Suppress(all);
        }
        catch (Exception ex) { Warn("bildirim balonları: " + ex.GetBaseException().Message); }
    }

    // 24 saat açık kabukta günlüğü doldurmasın: en çok 10 dakikada bir
    static void Warn(string m)
    {
        if ((DateTime.UtcNow - lastWarn).TotalMinutes < 10) return;
        lastWarn = DateTime.UtcNow;
        Slider.Log("windows bildirimleri: " + m);
    }
}

// Windows'un kendi bildirim balonları. Logical Lunge bildirimleri kendi kartıyla gösterirken Windows'un balonu da çıkınca
// her bildirim iki kez görünüyordu. Uygulama başına "Bildirim başlıklarını göster" (Ayarlar > Bildirimler'deki aynı ayar:
// ...\Notifications\Settings\<AUMID>\ShowBanner) kapatılır; bildirimler Bildirim Merkezi'nde kalır (Win+N).
// Eski değerler state\toast-banners.json'a yazılır: ayar kapanınca ya da kaldırırken (lunge.exe --restore-banners)
// her uygulama eski haline döner. Kullanıcının sonradan Windows'tan değiştirdiği balona dokunulmaz.
static class ToastBanners
{
    const string SETTINGS = @"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings";
    static readonly object gate = new object();
    static string BackupFile { get { return Paths.State("toast-banners.json"); } }

    // aumid -> eski ShowBanner değeri (null: değer yoktu, Windows varsayılanı açık). Dosya yoksa boş; var ama okunamıyor /
    // bozuksa null: o zaman hiçbir şeye dokunulmaz (yoksa kullanıcının eski değerleri bizim koyduğumuz 0'larla ezilirdi).
    static Dictionary<string, object> ReadBackup()
    {
        var d = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        if (!System.IO.File.Exists(BackupFile)) return d;
        try
        {
            var read = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(System.IO.File.ReadAllText(BackupFile));
            if (read == null) return null;
            foreach (var kv in read) d[kv.Key] = kv.Value;
            return d;
        }
        catch { return null; }
    }

    // Kayıt defteri alt anahtarı olabilecek kimlik (ters bölü alt anahtar açardı)
    static bool Usable(string aumid)
    {
        if (string.IsNullOrEmpty(aumid) || aumid.Length > 255 || aumid.IndexOf('\\') >= 0) return false;
        foreach (char c in aumid) if (c < ' ') return false;
        return true;
    }

    // Windows'un ayarını tuttuğu uygulamalar
    public static List<string> Configured()
    {
        var names = new List<string>();
        using (var root = Registry.CurrentUser.OpenSubKey(SETTINGS)) if (root != null) names.AddRange(root.GetSubKeyNames());
        return names;
    }

    public static void Suppress(IEnumerable<string> aumids)
    {
        lock (gate)
        {
            // Ayar kapatıldıysa (geri yükleme sıradaysa) yeniden kapatma
            if (!Prefs.WinToasts) return;
            var backup = ReadBackup();
            if (backup == null) return;
            var todo = new List<string>();
            foreach (var a in aumids)
                if (Usable(a) && !backup.ContainsKey(a) && !todo.Exists(t => t.Equals(a, StringComparison.OrdinalIgnoreCase))) todo.Add(a);
            if (todo.Count == 0) return;
            using (var root = Registry.CurrentUser.CreateSubKey(SETTINGS))
            {
                foreach (var a in todo)
                    using (var k = root.OpenSubKey(a))
                    {
                        object v = k == null ? null : k.GetValue("ShowBanner");
                        backup[a] = v is int ? v : null;
                    }
                // Önce yedek: yazma yarıda kalırsa eski değerler kaybolmasın
                if (!Files.WriteAtomic(BackupFile, new JavaScriptSerializer().Serialize(backup))) return;
                foreach (var a in todo)
                    using (var k = root.CreateSubKey(a)) k.SetValue("ShowBanner", 0, RegistryValueKind.DWord);
            }
        }
    }

    // Yedekteki her uygulama eski haline; yalnızca hâlâ bizim koyduğumuz 0'da duranlar (kullanıcının sonradan açtığına
    // dokunulmaz). Geri yüklenemeyenler yedekte kalır.
    public static void Restore()
    {
        lock (gate)
        {
            var backup = ReadBackup();
            if (backup == null || backup.Count == 0) return;
            var left = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
            using (var root = Registry.CurrentUser.OpenSubKey(SETTINGS, true))
            {
                if (root == null) { TryDelete(); return; }
                foreach (var kv in backup)
                {
                    if (!Usable(kv.Key)) continue;
                    try
                    {
                        using (var k = root.OpenSubKey(kv.Key, true))
                        {
                            if (k == null) continue;
                            object now = k.GetValue("ShowBanner");
                            if (now is int && (int)now == 0)
                            {
                                if (kv.Value is int) k.SetValue("ShowBanner", (int)kv.Value, RegistryValueKind.DWord);
                                else k.DeleteValue("ShowBanner", false);
                            }
                        }
                        // Yalnızca bizim açtığımız, boş kalan anahtar kalkar
                        if (kv.Value == null)
                            using (var k = root.OpenSubKey(kv.Key))
                                if (k != null && k.ValueCount == 0 && k.SubKeyCount == 0) { k.Close(); root.DeleteSubKey(kv.Key, false); }
                    }
                    catch { left[kv.Key] = kv.Value; }
                }
            }
            if (left.Count == 0) TryDelete();
            else Files.WriteAtomic(BackupFile, new JavaScriptSerializer().Serialize(left));
        }
    }

    static void TryDelete()
    {
        try { System.IO.File.Delete(BackupFile); } catch { }
    }
}

// Windows'un kendi SQLite'ı (System32\winsqlite3.dll, Windows 10'dan beri her sürümde): yalnızca okuma için gerekenler
static class WinSqlite
{
    const string DLL = "winsqlite3.dll";
    const int SQLITE_OPEN_READONLY = 0x1, SQLITE_OPEN_READWRITE = 0x2, SQLITE_ROW = 100, SQLITE_DONE = 101;

    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern int sqlite3_open_v2(byte[] filename, out IntPtr db, int flags, IntPtr vfs);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern int sqlite3_close_v2(IntPtr db);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern int sqlite3_busy_timeout(IntPtr db, int ms);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall, CharSet = CharSet.Unicode)] static extern int sqlite3_prepare16_v2(IntPtr db, string sql, int bytes, out IntPtr stmt, IntPtr tail);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern int sqlite3_step(IntPtr stmt);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern int sqlite3_finalize(IntPtr stmt);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern int sqlite3_bind_int64(IntPtr stmt, int index, long value);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] public static extern long sqlite3_column_int64(IntPtr stmt, int col);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern IntPtr sqlite3_column_text16(IntPtr stmt, int col);
    [DllImport(DLL, CallingConvention = CallingConvention.StdCall)] static extern IntPtr sqlite3_errmsg16(IntPtr db);

    public static bool Open(string path, bool writable, Func<IntPtr, bool> work)
    {
        IntPtr db;
        var name = Encoding.UTF8.GetBytes(path + "\0");
        int rc = sqlite3_open_v2(name, out db, writable ? SQLITE_OPEN_READWRITE : SQLITE_OPEN_READONLY, IntPtr.Zero);
        try
        {
            if (rc != 0) throw new InvalidOperationException("açılamadı (" + rc + "): " + Error(db));
            sqlite3_busy_timeout(db, 250);
            return work(db);
        }
        finally { if (db != IntPtr.Zero) sqlite3_close_v2(db); }
    }

    // Sorguyu çalıştırır, her satırda row çağrılır; arg: ?1 parametresi
    public static void Each(IntPtr db, string sql, long? arg, Action<IntPtr> row)
    {
        IntPtr st;
        if (sqlite3_prepare16_v2(db, sql, -1, out st, IntPtr.Zero) != 0) throw new InvalidOperationException(Error(db));
        try
        {
            if (arg.HasValue) sqlite3_bind_int64(st, 1, arg.Value);
            int rc;
            while ((rc = sqlite3_step(st)) == SQLITE_ROW) row(st);
            if (rc != SQLITE_DONE) throw new InvalidOperationException(Error(db));
        }
        finally { sqlite3_finalize(st); }
    }

    public static string Text(IntPtr st, int col)
    {
        IntPtr p = sqlite3_column_text16(st, col);
        return p == IntPtr.Zero ? null : Marshal.PtrToStringUni(p);
    }

    static string Error(IntPtr db)
    {
        IntPtr p = db == IntPtr.Zero ? IntPtr.Zero : sqlite3_errmsg16(db);
        return p == IntPtr.Zero ? "SQLite hatası" : Marshal.PtrToStringUni(p);
    }
}
