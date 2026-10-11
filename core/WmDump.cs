using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;

// Yanıt vermeyen pencere yöneticisi kapatılmadan önce bir döküm (minidump: her thread'in yığını, modülleri) alınır:
// %LOCALAPPDATA%\LogicalLunge\state\wm-hang-<tarih>.dmp, son 3'ü kalır. Bir v1.5.2 raporunda pencere yöneticisi
// dakikalarca tam işlemciyle takıldı ve öldürüldü; nerede döndüğü hiçbir yerde kalmamıştı. Döküm, o sürümün CI'daki
// PDB'siyle açılınca takılan yeri gösterir; hata raporu en yenisinin yolunu ve boyutunu yazar.
static class WmDump
{
    const int Keep = 3;
    const string Prefix = "wm-hang-";

    [DllImport("dbghelp.dll", SetLastError = true)]
    static extern bool MiniDumpWriteDump(IntPtr hProcess, uint processId, Microsoft.Win32.SafeHandles.SafeFileHandle hFile,
        uint dumpType, IntPtr exceptionParam, IntPtr userStreamParam, IntPtr callbackParam);

    // MiniDumpNormal (yığınlar) | WithThreadInfo (thread süreleri) | WithUnloadedModules | WithProcessThreadData
    const uint DumpType = 0x00000000 | 0x00001000 | 0x00000020 | 0x00000100;

    // Dökümü yazar; yolu ya da (alınamadıysa) null döner. Ne olursa olsun kapatmayı geciktirmez (en çok birkaç saniye).
    public static string Write(Process p)
    {
        string dir = Paths.StateDir;
        string path = Path.Combine(dir, Prefix + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".dmp");
        try
        {
            bool ok;
            using (var fs = new FileStream(path, FileMode.Create, FileAccess.ReadWrite, FileShare.None))
                ok = MiniDumpWriteDump(p.Handle, (uint)p.Id, fs.SafeFileHandle, DumpType, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            if (!ok)
            {
                int err = Marshal.GetLastWin32Error();
                try { File.Delete(path); } catch { }
                Slider.Log("tiling nöbetçisi: döküm alınamadı (hata " + err + ")");
                return null;
            }
            Prune(dir);
            return path;
        }
        catch (Exception ex)
        {
            try { File.Delete(path); } catch { }
            Slider.Log("tiling nöbetçisi: döküm alınamadı: " + ex.Message);
            return null;
        }
    }

    // En yeni `Keep` döküm kalır
    static void Prune(string dir)
    {
        foreach (var old in Oldest(Existing(dir), Keep))
            try { File.Delete(old); } catch { }
    }

    static IEnumerable<string> Existing(string dir)
    {
        try { return Directory.GetFiles(dir, Prefix + "*.dmp"); } catch { return new string[0]; }
    }

    // Silinecekler: adı tarih taşıdığı için ada göre sıralı en eskiler (en yeni `keep` tanesi dışındakiler)
    internal static List<string> Oldest(IEnumerable<string> files, int keep)
    {
        var sorted = files.OrderByDescending(f => Path.GetFileName(f), StringComparer.Ordinal).ToList();
        return sorted.Skip(keep).ToList();
    }

    // Hata raporu için: dökümlerin yolu, boyutu ve zamanı (en yenisi önce); yoksa null
    public static string ReportLines()
    {
        var files = Existing(Paths.StateDir).OrderByDescending(f => Path.GetFileName(f), StringComparer.Ordinal).ToList();
        if (files.Count == 0) return null;
        var lines = new List<string>();
        foreach (var f in files)
        {
            try
            {
                var info = new FileInfo(f);
                lines.Add(info.FullName + " (" + (info.Length / 1024) + " KB, " + info.LastWriteTime.ToString("yyyy-MM-dd HH:mm:ss") + ")");
            }
            catch { }
        }
        return lines.Count == 0 ? null : string.Join("\n", lines.ToArray());
    }
}
