// Ekrandan metin (OCR, Super+Shift+T) için saf yardımcılar: dil seçimi, görüntünün ölçeği, satır ve kelimelerin
// birleşmesi, bildirim önizlemesi ve tools\lunge-ocr.exe'nin çekirdeğe yazdığı JSON. lunge-ocr.exe de bu dosyayla
// derlenir (build.ps1): burada yalnızca BCL kullanılır.
using System;
using System.Collections.Generic;
using System.Text;

static class OcrText
{
    // OcrEngine.MaxImageDimension (Windows 10/11: 2600 piksel)
    public const int MaxSide = 2600;
    // Yazı kenara değince tanınmıyordu: görüntünün çevresine bu kadar boşluk konur (ölçekten sonra)
    public const int Margin = 16;
    // Çok küçük alanlar (tek kelime) bu boyuta kadar boşlukla büyütülür
    public const int MinSide = 64;

    // Kullanıcının dilleri (sırayla) ve kurulu OCR dilleri: önce birebir etiket (tr-TR), sonra aynı ana dil (tr ~ tr-TR,
    // zh ~ zh-Hans-CN). Hiçbiri yoksa null: araç Windows'un kullanıcı dillerine, o da yoksa kurulu ilk dile düşer.
    public static string PickLanguage(IList<string> wanted, IList<string> available)
    {
        if (wanted == null || available == null || available.Count == 0) return null;
        foreach (var w in wanted)
        {
            if (string.IsNullOrEmpty(w)) continue;
            foreach (var a in available) if (string.Equals(a, w, StringComparison.OrdinalIgnoreCase)) return a;
        }
        foreach (var w in wanted)
        {
            if (string.IsNullOrEmpty(w)) continue;
            string p = Primary(w);
            foreach (var a in available) if (!string.IsNullOrEmpty(a) && Primary(a) == p) return a;
        }
        return null;
    }

    static string Primary(string tag)
    {
        int i = tag.IndexOf('-');
        return (i < 0 ? tag : tag.Substring(0, i)).ToLowerInvariant();
    }

    // Küçük ekran yazısı OCR'da kayboluyordu: alan 2 kata kadar büyütülür; en uzun kenar (boşluklarla) MaxSide'ı geçmez
    public static double ScaleFor(int w, int h)
    {
        int side = Math.Max(w, h);
        if (side <= 0) return 1.0;
        return Math.Min(2.0, (double)(MaxSide - 2 * Margin) / side);
    }

    // Ölçeklenmiş alanın çevresine boşluk: tuvalin boyutu (en az MinSide) ve alanın tuvaldeki yeri (ortada)
    public static void Canvas(int w, int h, out int cw, out int ch, out int ox, out int oy)
    {
        cw = Math.Max(MinSide, w + 2 * Margin);
        ch = Math.Max(MinSide, h + 2 * Margin);
        ox = (cw - w) / 2;
        oy = (ch - h) / 2;
    }

    // Satırlar alt alta (\r\n), satırdaki kelimeler boşlukla; Çince / Japonca karakterler arasına boşluk konmaz (Windows OCR
    // bu dillerde her karakteri ayrı bir kelime olarak verir). Boş kelimeler ve boş satırlar atlanır.
    public static string Join(IEnumerable<IEnumerable<string>> lines)
    {
        var sb = new StringBuilder();
        if (lines == null) return "";
        foreach (var line in lines)
        {
            if (line == null) continue;
            var l = new StringBuilder();
            string prev = null;
            foreach (var raw in line)
            {
                string w = raw == null ? "" : raw.Trim();
                if (w.Length == 0) continue;
                if (prev != null && !(Dense(prev[prev.Length - 1]) && Dense(w[0]))) l.Append(' ');
                l.Append(w);
                prev = w;
            }
            if (l.Length == 0) continue;
            if (sb.Length > 0) sb.Append("\r\n");
            sb.Append(l);
        }
        return sb.ToString();
    }

    // Boşluksuz yazılan yazılar: Han (Çince, Japonca kanji), hiragana, katakana, CJK noktalama ve tam genişlikli biçimler.
    // Korece (hangul) kelimeleri boşlukla ayırır: buraya girmez.
    internal static bool Dense(char c)
    {
        return (c >= 0x3000 && c <= 0x30FF) || (c >= 0x31F0 && c <= 0x31FF) || (c >= 0x3400 && c <= 0x4DBF)
            || (c >= 0x4E00 && c <= 0x9FFF) || (c >= 0xF900 && c <= 0xFAFF) || (c >= 0xFF00 && c <= 0xFFEF);
    }

    // Bildirim kartının önizlemesi: en çok maxLines satır ve maxChars karakter; kısalınca sonunda "…"
    public static string Preview(string text, int maxLines, int maxChars)
    {
        if (string.IsNullOrEmpty(text)) return "";
        var lines = text.Replace("\r\n", "\n").Split('\n');
        var sb = new StringBuilder();
        bool cut = lines.Length > maxLines;
        for (int i = 0; i < lines.Length && i < maxLines; i++)
        {
            if (i > 0) sb.Append('\n');
            sb.Append(lines[i]);
        }
        if (sb.Length > maxChars)
        {
            int n = maxChars;
            if (n > 0 && char.IsHighSurrogate(sb[n - 1])) n--;
            sb.Length = n;
            cut = true;
        }
        string s = sb.ToString().TrimEnd();
        return cut ? s + "…" : s;
    }

    // lunge-ocr.exe -> çekirdek: {"lang":"tr","lines":[["Merhaba","dünya"],["..."]]}
    public static string ToolJson(string lang, IEnumerable<IEnumerable<string>> lines)
    {
        var sb = new StringBuilder("{\"lang\":");
        Str(sb, lang ?? "");
        sb.Append(",\"lines\":[");
        bool firstLine = true;
        if (lines != null)
            foreach (var line in lines)
            {
                if (!firstLine) sb.Append(',');
                firstLine = false;
                sb.Append('[');
                bool firstWord = true;
                if (line != null)
                    foreach (var w in line)
                    {
                        if (!firstWord) sb.Append(',');
                        firstWord = false;
                        Str(sb, w ?? "");
                    }
                sb.Append(']');
            }
        return sb.Append("]}").ToString();
    }

    // {"error":"nolang"} (kurulu OCR dili yok), {"error":"failed","detail":"..."}
    public static string ToolError(string code, string detail)
    {
        var sb = new StringBuilder("{\"error\":");
        Str(sb, code ?? "failed");
        if (!string.IsNullOrEmpty(detail)) { sb.Append(",\"detail\":"); Str(sb, detail); }
        return sb.Append('}').ToString();
    }

    static void Str(StringBuilder sb, string s)
    {
        sb.Append('"');
        foreach (char c in s)
        {
            if (c == '"') sb.Append("\\\"");
            else if (c == '\\') sb.Append("\\\\");
            else if (c < 0x20 || c == 0x2028 || c == 0x2029) sb.Append("\\u").Append(((int)c).ToString("x4"));
            else sb.Append(c);
        }
        sb.Append('"');
    }
}
