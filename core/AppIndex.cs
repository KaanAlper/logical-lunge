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
    const int ICON_SIZE = 48;
    static readonly Regex skip = new Regex(@"(uninstall|kaldır|readme|beni oku|help|yardım|documentation|belgeler|release notes|license|lisans|website|web sitesi|manual|kılavuz|changelog|what's new)", RegexOptions.IgnoreCase);
    static readonly Regex document = new Regex(@"\.(txt|pdf|html?|chm|url|md|rtf)$", RegexOptions.IgnoreCase);
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
                    if (id == "Microsoft.Windows.Shell.RunDialog") { name = name + " (Run)"; alias = "run"; }
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
}
