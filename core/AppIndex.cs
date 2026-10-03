using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;

// Super menüsünün uygulama listesi (lunge.exe --build-apps; eskiden scripts\build-apps.ps1): Windows'un uygulama listesi
// (shell:AppsFolder, Başlat menüsünün kullandığı liste) yerelleştirilmiş adları ("Ekran Klavyesi" gibi) ve gerçek
// simgeleriyle state\apps.json'a yazılır. Belgeler / yardım / kaldırma kısayolları elenir. Shell.Application bir STA
// nesnesi: Main [STAThread].
static class AppIndex
{
    // Windows'un Çalıştır penceresi (Win+R de bunu açar)
    public const string RunDialog = @"shell:AppsFolder\Microsoft.Windows.Shell.RunDialog";

    const int ICON_SIZE = 48;
    static readonly Regex skip = new Regex(@"(uninstall|kaldır|readme|beni oku|help|yardım|documentation|belgeler|release notes|license|lisans|website|web sitesi|manual|kılavuz|changelog|what's new)", RegexOptions.IgnoreCase);
    static readonly Regex document = new Regex(@"\.(txt|pdf|html?|chm|md|rtf)$", RegexOptions.IgnoreCase);
    // Bir .url kısayolu web sayfası açıyorsa elenir; steam://, com.epicgames.launcher:// gibi bir programı başlatanlar
    // (Steam / Epic / itch oyunları ve uygulamaları Başlat menüsüne .lnk değil .url koyar) listede kalır.
    static readonly Regex webTarget = new Regex(@"^(https?|ftp|file|mailto):", RegexOptions.IgnoreCase);
    static readonly Regex exeName = new Regex(@"\\([^\\]+)\.exe$", RegexOptions.IgnoreCase);
    // Görünmez karakterler (bazı oyun adlarında sıfır genişlikli boşluk var: "4<ZWSP>42"; aranınca bulunmuyordu)
    static readonly Regex invisible = new Regex("[­​-‏⁠-⁤﻿]");

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)]
    static extern int SHLoadIndirectString(string src, StringBuilder buf, int cch, IntPtr reserved);

    // "@%SystemRoot%\system32\Taskmgr.exe,-32420" -> "Görev Yöneticisi" (kullanıcının arayüz dilinde)
    static string Indirect(string s)
    {
        var sb = new StringBuilder(512);
        return SHLoadIndirectString(Environment.ExpandEnvironmentVariables(s), sb, sb.Capacity, IntPtr.Zero) == 0 && sb.Length > 0 ? sb.ToString() : null;
    }

    [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellItemImageFactory { void GetImage(SIZE size, int flags, out IntPtr phbm); }
    [StructLayout(LayoutKind.Sequential)] struct SIZE { public int cx, cy; }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr o);

    static string Png(string parsingName, int size)
    {
        try
        {
            IShellItemImageFactory f;
            SHCreateItemFromParsingName(parsingName, IntPtr.Zero, typeof(IShellItemImageFactory).GUID, out f);
            IntPtr hbm;
            f.GetImage(new SIZE { cx = size, cy = size }, 0x1 /*BIGGERSIZEOK*/ | 0x4 /*ICONONLY*/, out hbm);
            try
            {
                // Alfa kanalını koruyarak kopyala (Image.FromHbitmap alfayı atar)
                using (var src = Image.FromHbitmap(hbm))
                using (var bmp = new Bitmap(src.Width, src.Height, PixelFormat.Format32bppArgb))
                {
                    var data = src.LockBits(new Rectangle(0, 0, src.Width, src.Height), ImageLockMode.ReadOnly, src.PixelFormat);
                    try
                    {
                        using (var dst = new Bitmap(src.Width, src.Height, data.Stride, PixelFormat.Format32bppArgb, data.Scan0))
                        using (var g = Graphics.FromImage(bmp)) g.DrawImage(dst, 0, 0);
                    }
                    finally { src.UnlockBits(data); }
                    using (var ms = new System.IO.MemoryStream()) { bmp.Save(ms, ImageFormat.Png); return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray()); }
                }
            }
            finally { DeleteObject(hbm); }
        }
        catch { return null; }
    }

    // Başlat menüsü kısayollarının yerelleştirilmiş adları (klasörlerindeki desktop.ini [LocalizedFileNames]). AppsFolder bazı
    // sistem kısayollarını dosya adıyla veriyor ("Task Manager"), Başlat menüsü ise bu kaynaktan çözüp Türkçe gösteriyor.
    static Dictionary<string, string> LocalizedNames()
    {
        var map = new Dictionary<string, string>();
        var roots = new[]
        {
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), @"Microsoft\Windows\Start Menu\Programs"),
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), @"Microsoft\Windows\Start Menu\Programs")
        };
        foreach (var root in roots)
            foreach (var ini in Files(root, "desktop.ini"))
            {
                string[] lines;
                try { lines = System.IO.File.ReadAllLines(ini); } catch { continue; }
                bool inFiles = false;
                foreach (var line in lines)
                {
                    var sec = Regex.Match(line, @"^\s*\[(.+)\]");
                    if (sec.Success) { inFiles = sec.Groups[1].Value.Equals("LocalizedFileNames", StringComparison.OrdinalIgnoreCase); continue; }
                    var m = Regex.Match(line, @"^(.+?)\.lnk=(.+)$", RegexOptions.IgnoreCase);
                    if (!inFiles || !m.Success) continue;
                    string val = m.Groups[2].Value.Trim();
                    string nm = val.StartsWith("@") ? Indirect(val) : val;
                    if (!string.IsNullOrEmpty(nm)) map[m.Groups[1].Value.Trim().ToLower()] = nm;
                }
            }
        return map;
    }

    // Get-ChildItem -Recurse -Force: erişilemeyen klasörler atlanır, bağlantılara (junction) girilmez
    static IEnumerable<string> Files(string dir, string name)
    {
        var stack = new Stack<string>();
        stack.Push(dir);
        while (stack.Count > 0)
        {
            string d = stack.Pop();
            string[] files = new string[0], dirs = new string[0];
            try
            {
                files = System.IO.Directory.GetFiles(d, name);
                dirs = System.IO.Directory.GetDirectories(d);
            }
            catch (Exception) { }
            foreach (var f in files) yield return f;
            foreach (var sub in dirs)
            {
                try { if ((System.IO.File.GetAttributes(sub) & System.IO.FileAttributes.ReparsePoint) != 0) continue; } catch { continue; }
                stack.Push(sub);
            }
        }
    }

    [DllImport("shell32.dll")] static extern int SHGetKnownFolderPath(ref Guid id, uint flags, IntPtr token, out IntPtr path);
    static readonly Regex knownFolder = new Regex(@"^\{([0-9A-Fa-f-]{36})\}(\\.*)?$");

    // AppsFolder kimliğinden kısayol dosyasının yolu ("{bilinen klasör}\alt\ad.url" ya da tam yol)
    static string PathOf(string id)
    {
        var m = knownFolder.Match(id);
        if (m.Success)
        {
            var guid = new Guid(m.Groups[1].Value);
            IntPtr p;
            if (SHGetKnownFolderPath(ref guid, 0, IntPtr.Zero, out p) != 0) return null;
            try { return Marshal.PtrToStringUni(p) + m.Groups[2].Value; } finally { Marshal.FreeCoTaskMem(p); }
        }
        return id.Length > 3 && id[1] == ':' && id[2] == '\\' ? id : null;
    }

    // Bir İnternet kısayolunun ([InternetShortcut] URL=) açtığı adres; okunamazsa null
    static string UrlOf(string id)
    {
        try
        {
            string path = PathOf(id);
            if (path == null || !System.IO.File.Exists(path)) return null;
            foreach (var line in System.IO.File.ReadAllLines(path))
                if (line.StartsWith("URL=", StringComparison.OrdinalIgnoreCase)) return line.Substring(4).Trim();
        }
        catch (Exception) { }
        return null;
    }

    // Uygulamanın dosyası (Super menüsünün "Dosya konumunu aç" maddesi): kısayolun hedefi, ya da kimliğin kendisi bir yol
    // ("{bilinen klasör}\alt\uygulama.exe" ya da "C:\...\uygulama.exe"). Mağaza uygulamalarında yok.
    static string FileOf(object it, string id)
    {
        try
        {
            var target = Convert.ToString(Call(it, "ExtendedProperty", "System.Link.TargetParsingPath"));
            if (!string.IsNullOrEmpty(target) && System.IO.Path.IsPathRooted(target) && System.IO.File.Exists(target)) return target;
        }
        catch (Exception) { }
        string path = null;
        var m = knownFolder.Match(id);
        if (m.Success)
        {
            var guid = new Guid(m.Groups[1].Value);
            IntPtr p;
            if (SHGetKnownFolderPath(ref guid, 0, IntPtr.Zero, out p) == 0)
            {
                try { path = Marshal.PtrToStringUni(p) + m.Groups[2].Value; } finally { Marshal.FreeCoTaskMem(p); }
            }
        }
        else if (id.Length > 3 && id[1] == ':' && id[2] == '\\') path = id;
        return path != null && System.IO.File.Exists(path) ? path : null;
    }

    static object Call(object o, string name, params object[] args)
    {
        return o.GetType().InvokeMember(name, BindingFlags.InvokeMethod, null, o, args);
    }

    static object Get(object o, string name)
    {
        return o.GetType().InvokeMember(name, BindingFlags.GetProperty, null, o, null);
    }

    // Uygulama sayısı; liste boşsa hata (önceki liste kalır)
    public static int Build(string outPath)
    {
        System.IO.Directory.CreateDirectory(System.IO.Path.GetDirectoryName(outPath));
        var localized = LocalizedNames();
        object shell = Activator.CreateInstance(Type.GetTypeFromProgID("Shell.Application", true));
        var apps = new List<Dictionary<string, object>>();
        try
        {
            object folder = Call(shell, "NameSpace", "shell:AppsFolder");
            if (folder == null) throw new InvalidOperationException("Windows AppsFolder could not be opened.");
            object items = Call(folder, "Items");
            int count = (int)Get(items, "Count");
            var seen = new HashSet<string>();
            for (int i = 0; i < count; i++)
            {
                object it = Call(items, "Item", i);
                if (it == null) continue;
                try
                {
                    string name = Get(it, "Name") as string;
                    if (name != null) name = invisible.Replace(name, "");
                    string id = (Get(it, "Path") as string) ?? "";
                    string also = null;
                    // Sistem dilindeki ad (Windows'un arayüz dili); dosya adı farklıysa onunla da aranır
                    string loc;
                    if (!string.IsNullOrEmpty(name) && localized.TryGetValue(name.ToLower(), out loc) && loc != name) { also = name; name = loc; }
                    if (string.IsNullOrEmpty(name) || skip.IsMatch(name)) continue;
                    if (Regex.IsMatch(id, "^https?:", RegexOptions.IgnoreCase) || document.IsMatch(id)) continue;
                    if (id.EndsWith(".url", StringComparison.OrdinalIgnoreCase))
                    {
                        string url = UrlOf(id);
                        if (string.IsNullOrEmpty(url) || webTarget.IsMatch(url)) continue;
                    }
                    if (!seen.Add(name.ToLower())) continue;
                    string exe = null;
                    var m = exeName.Match(id);
                    if (m.Success) exe = m.Groups[1].Value.ToLower();
                    else
                    {
                        // Kendi kimliğiyle kayıtlı masaüstü uygulaması (Chrome, Discord...): kısayolunun hedefi. Dock çalışan
                        // pencereyi süreç adıyla bu exe'ye bağlar, Super menüsünün "Dock'ta tut" maddesi de bununla çıkar.
                        try
                        {
                            var target = Convert.ToString(Call(it, "ExtendedProperty", "System.Link.TargetParsingPath"));
                            var t = exeName.Match(target ?? "");
                            if (t.Success) exe = t.Groups[1].Value.ToLower();
                        }
                        catch (Exception) { }
                    }
                    // Win+R penceresi: 'run' yazınca da bulunsun
                    string alias = null;
                    if (@"shell:AppsFolder\" + id == RunDialog) { name = name + " (Run)"; alias = "run"; }
                    apps.Add(new Dictionary<string, object>
                    {
                        { "name", name }, { "path", @"shell:AppsFolder\" + id }, { "exe", exe }, { "alias", alias }, { "also", also },
                        { "icon", Png(@"shell:AppsFolder\" + id, ICON_SIZE) }
                    });
                }
                finally { Marshal.FinalReleaseComObject(it); }
            }
            Marshal.FinalReleaseComObject(items);
            Marshal.FinalReleaseComObject(folder);
        }
        finally { Marshal.FinalReleaseComObject(shell); }

        if (apps.Count == 0) throw new InvalidOperationException("Windows AppsFolder returned no applications; the previous index was kept.");
        var sorted = apps.OrderBy(a => (string)a["name"], StringComparer.CurrentCultureIgnoreCase).ToList();
        string temp = outPath + "." + System.Diagnostics.Process.GetCurrentProcess().Id + ".tmp";
        try
        {
            System.IO.File.WriteAllText(temp, new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.Serialize(sorted), new UTF8Encoding(false));
            // Okuyanlar ya eski tam listeyi ya da yeni tam listeyi görür
            if (System.IO.File.Exists(outPath)) System.IO.File.Replace(temp, outPath, null);
            else System.IO.File.Move(temp, outPath);
        }
        finally { if (System.IO.File.Exists(temp)) System.IO.File.Delete(temp); }
        return apps.Count;
    }

    // ---- Yenileme: listeyi ayrı bir süreç (lunge.exe --build-apps, STA ve COM) yazar. Aynı anda tek tarama; tarama
    // sürerken gelen istek bittiğinde bir kez daha tarar.
    static readonly object rebuildLock = new object();
    static bool scanning, again;

    public static void RebuildInBackground(string reason)
    {
        lock (rebuildLock)
        {
            if (scanning) { again = true; return; }
            scanning = true;
        }
        System.Threading.ThreadPool.QueueUserWorkItem(_ =>
        {
            while (true)
            {
                try
                {
                    var psi = new System.Diagnostics.ProcessStartInfo(System.Windows.Forms.Application.ExecutablePath, "--build-apps")
                    {
                        UseShellExecute = false, CreateNoWindow = true, RedirectStandardError = true,
                        WorkingDirectory = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
                    };
                    using (var scan = System.Diagnostics.Process.Start(psi))
                    {
                        try { scan.PriorityClass = System.Diagnostics.ProcessPriorityClass.BelowNormal; } catch (Exception) { }
                        string error = scan.StandardError.ReadToEnd();
                        scan.WaitForExit();
                        string apps = Paths.AppsJson;
                        if (scan.ExitCode != 0 || !System.IO.File.Exists(apps) || new System.IO.FileInfo(apps).Length <= 2)
                            Slider.Log("apps index failed (" + reason + ", exit " + scan.ExitCode + "): " + (error.Length > 300 ? error.Substring(0, 300) : error));
                        else Toasts.Emit("ll:apps"); // kabuk listeyi hemen yeniden okur
                    }
                }
                catch (Exception ex) { Slider.Log("apps index (" + reason + "): " + ex.Message); }
                lock (rebuildLock)
                {
                    if (!again) { scanning = false; return; }
                    again = false;
                }
            }
        });
    }

    // Başlat menüsü klasörleri izlenir: bir uygulama kurulunca / kaldırılınca liste birkaç saniye sonra kendiliğinden
    // yenilenir (eskiden yalnızca açılışta ve günde bir; yeni kurulan uygulama ertesi güne kadar aranamıyordu). Bir
    // kurulum art arda çok dosya yazar: son değişiklikten 4 sn sonra bir kez taranır.
    static readonly List<System.IO.FileSystemWatcher> watchers = new List<System.IO.FileSystemWatcher>();
    static System.Threading.Timer settle;

    public static void WatchStartMenu()
    {
        settle = new System.Threading.Timer(_ => RebuildInBackground("start menu changed"), null, System.Threading.Timeout.Infinite, System.Threading.Timeout.Infinite);
        foreach (var dir in new[] {
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), @"Microsoft\Windows\Start Menu\Programs"),
            System.IO.Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), @"Microsoft\Windows\Start Menu\Programs") })
        {
            if (!System.IO.Directory.Exists(dir)) continue;
            try
            {
                var w = new System.IO.FileSystemWatcher(dir)
                {
                    IncludeSubdirectories = true,
                    NotifyFilter = System.IO.NotifyFilters.FileName | System.IO.NotifyFilters.DirectoryName | System.IO.NotifyFilters.LastWrite
                };
                System.IO.FileSystemEventHandler changed = (o, e) => settle.Change(4000, System.Threading.Timeout.Infinite);
                w.Created += changed; w.Deleted += changed; w.Changed += changed;
                w.Renamed += (o, e) => settle.Change(4000, System.Threading.Timeout.Infinite);
                // arabellek taştıysa (çok büyük kurulum) yine bir kez taranır
                w.Error += (o, e) => settle.Change(4000, System.Threading.Timeout.Infinite);
                w.EnableRaisingEvents = true;
                watchers.Add(w);
            }
            catch (Exception ex) { Slider.Log("apps index: cannot watch " + dir + ": " + ex.Message); }
        }
    }
}
