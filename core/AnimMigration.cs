using System;
using System.Text.RegularExpressions;

// Hareket süreleri (animations:) için RuleMigration'ın karşılığı: config.yaml kullanıcıda kaldığından varsayılan
// sürelerin değişmesi mevcut kurulumlara ulaşmazdı. Önceki sürümlerin gönderdiği satır kullanıcıda AYNEN duruyorsa
// (boşluk ve tırnak farkı sayılmaz) yeni varsayılana çevrilir; kullanıcının değiştirdiği bir satıra dokunulmaz.
static class AnimMigration
{
    // Anahtar, eski gönderilen gövde, yeni gövde. Kayma: 520 ms (kuyruğu kısaltılmış) -> ii'nin 700 ms'si.
    static readonly string[][] Upgrades = {
        new[] { "workspaces", "{ duration: 520, curve: menu_decel }", "{ duration: 700, curve: menu_decel }" },
    };

    static string Norm(string body) { return Regex.Replace(body, @"[\s'""]+", ""); }

    // Yalnızca üst düzey animations: bölümü (kenarlıkların borders: altında kendi animations: anahtarı var)
    internal static string Upgrade(string yaml, out int upgraded)
    {
        upgraded = 0;
        if (string.IsNullOrEmpty(yaml)) return yaml;
        var blk = Regex.Match(yaml, @"(?m)^animations:[ \t]*(?:#.*)?\r?\n((?:(?:[ \t]+[^\r\n]*|[ \t]*)(?:\r?\n|$))*)");
        string body = blk.Success ? blk.Groups[1].Value : "";
        int count = 0;
        string next = Regex.Replace(body, @"(?m)^([ \t]+)([A-Za-z_][\w-]*)([ \t]*:[ \t]*)(\{[^}\r\n]*\})", m =>
        {
            string key = m.Groups[2].Value.Replace("_", "").Replace("-", "").ToLowerInvariant();
            foreach (var u in Upgrades)
                if (key == u[0] && Norm(m.Groups[4].Value) == Norm(u[1]))
                {
                    count++;
                    return m.Groups[1].Value + m.Groups[2].Value + m.Groups[3].Value + u[2];
                }
            return m.Value;
        });
        string result = count == 0 ? yaml : yaml.Substring(0, blk.Groups[1].Index) + next + yaml.Substring(blk.Groups[1].Index + body.Length);
        // Kenarlığın odak renk geçişi: gönderilen 180 ms EaseInOutQuad -> ii'nin border'ı (1000 ms emphasizedDecel)
        int borders = 0;
        result = Regex.Replace(result, @"(?m)^([ \t]+-[ \t]+type:[ \t]*Fade[ \t]*\r?\n[ \t]+duration:[ \t]*)180([ \t]*\r?\n[ \t]+easing:[ \t]*)EaseInOutQuad(?=[ \t]*(?:\r?\n|\z))", m =>
        {
            borders++;
            return m.Groups[1].Value + "1000" + m.Groups[2].Value + "[0.05, 0.7, 0.1, 1.0]";
        });
        upgraded = count + borders;
        return upgraded == 0 ? yaml : result;
    }

    // RuleMigration'dan sonra aynı iş parçacığında (hepsi config.yaml'ı yazar)
    public static void Run()
    {
        try
        {
            if (!System.IO.File.Exists(Paths.ConfigFile)) return;
            string user = System.IO.File.ReadAllText(Paths.ConfigFile);
            int upgraded;
            string next = Upgrade(user, out upgraded);
            if (upgraded == 0) return;
            if (!Files.WriteAtomic(Paths.ConfigFile, next)) { Slider.Log("hareket göçü: config.yaml yazılamadı"); return; }
            try
            {
                string shaFile = Paths.State("config.sha256");
                if (System.IO.File.Exists(shaFile) && System.IO.File.ReadAllText(shaFile).Trim().Equals(BindMigration.Sha(user), StringComparison.OrdinalIgnoreCase))
                    Files.WriteAtomic(shaFile, BindMigration.Sha(next) + Environment.NewLine);
            }
            catch (Exception ex) { Slider.Log("hareket göçü: config.sha256 güncellenemedi: " + ex.Message); }
            Slider.Log("hareket göçü: " + upgraded + " eski varsayılan süre yenisine çevrildi");
            Anims.Load();
        }
        catch (Exception ex) { Slider.Log("hareket göçü: " + ex.Message); }
    }
}
