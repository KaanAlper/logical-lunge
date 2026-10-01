using System;
using System.Text;
using System.Threading;

// Okunup tek değeri değiştirilerek geri yazılan ayar dosyaları (keybinds.json, gece ışığı durumu) için okuma kuralı.
static class SettingsFile
{
    // Dosyayı güncellemek için okur. Dosya yoksa boş başlanır. Kilitli ya da o an yazılıyorsa (okuma hatası, boş metin)
    // kısa süre yeniden denenir; hâlâ okunamıyorsa false döner ve çağıran YAZMAMALIDIR: okunamayan dosyanın üstüne tek
    // değerle yazmak diğer bütün değerleri siler. Çözümlenemeyen (bozuk) dosyanın yedeği ".bad" olarak alınır, boş
    // başlanır. Hep boş okunan dosyada kaybedilecek bir şey yoktur, o da boş başlar.
    public static bool TryReadForUpdate<T>(string path, Func<string, T> parse, Func<T> empty, out T value)
    {
        bool lastReadEmpty = false;
        for (int attempt = 0; attempt < 6; attempt++)
        {
            if (attempt > 0) Thread.Sleep(50);
            string text;
            try
            {
                if (!System.IO.File.Exists(path)) { value = empty(); return true; }
                // Silme / yazma paylaşımıyla: okurken yazanın atomik değiştirmesini engellemez
                using (var fs = new System.IO.FileStream(path, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete))
                using (var sr = new System.IO.StreamReader(fs, Encoding.UTF8)) text = sr.ReadToEnd();
            }
            catch { lastReadEmpty = false; continue; }
            if (text.Trim().Length == 0) { lastReadEmpty = true; continue; }
            try { value = parse(text); return true; }
            catch
            {
                try { System.IO.File.Copy(path, path + ".bad", true); } catch { }
                value = empty();
                return true;
            }
        }
        if (lastReadEmpty) { value = empty(); return true; }
        value = default(T);
        return false;
    }
}
