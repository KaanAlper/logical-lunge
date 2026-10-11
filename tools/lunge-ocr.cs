// Ekrandan metin (Super+Shift+T): stdin'den gelen PNG'yi Windows'un kendi OCR'ıyla (Windows.Media.Ocr, cihazda,
// ağ yok) okur ve satır / kelimeleri stdout'a JSON yazar (OcrText.ToolJson). Çekirdek (lunge.exe --ocr) alanı seçer,
// bu aracı başlatır, metni panoya koyar ve bildirimi gösterir: WinRT yalnızca bu ayrı süreçte.
// lunge-ocr.exe <dil[,dil...]>   (ör. "tr,tr-TR": kullanıcının arayüz dili önce)
// Derleme: build.ps1 (Windows 10 SDK gerekir; core\OcrText.cs ile birlikte)
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using Windows.Globalization;
using Windows.Graphics.Imaging;
using Windows.Media.Ocr;

static class Program
{
    static int Main(string[] args)
    {
        var output = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
        int code = 0;
        try
        {
            var available = new List<string>();
            foreach (var l in OcrEngine.AvailableRecognizerLanguages) available.Add(l.LanguageTag);
            if (available.Count == 0)
            {
                output.Write(OcrText.ToolError("nolang", null));
                code = 2;
            }
            else
            {
                var wanted = new List<string>((args.Length > 0 ? args[0] : "").Split(new[] { ',' }, StringSplitOptions.RemoveEmptyEntries));
                string tag = OcrText.PickLanguage(wanted, available);
                OcrEngine engine = tag != null ? OcrEngine.TryCreateFromLanguage(new Language(tag)) : null;
                if (engine == null) engine = OcrEngine.TryCreateFromUserProfileLanguages();
                if (engine == null) engine = OcrEngine.TryCreateFromLanguage(new Language(available[0]));
                if (engine == null)
                {
                    output.Write(OcrText.ToolError("nolang", null));
                    code = 2;
                }
                else
                {
                    var png = new MemoryStream();
                    Console.OpenStandardInput().CopyTo(png);
                    png.Position = 0;
                    SoftwareBitmap bmp;
                    using (var stream = png.AsRandomAccessStream())
                    {
                        var decoder = BitmapDecoder.CreateAsync(stream).AsTask().Result;
                        bmp = decoder.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied).AsTask().Result;
                    }
                    if (bmp.PixelWidth > OcrEngine.MaxImageDimension || bmp.PixelHeight > OcrEngine.MaxImageDimension)
                    {
                        output.Write(OcrText.ToolError("toolarge", bmp.PixelWidth + "x" + bmp.PixelHeight));
                        code = 1;
                    }
                    else
                    {
                        var result = engine.RecognizeAsync(bmp).AsTask().Result;
                        var lines = new List<List<string>>();
                        foreach (var line in result.Lines)
                        {
                            var words = new List<string>();
                            foreach (var w in line.Words) words.Add(w.Text);
                            lines.Add(words);
                        }
                        output.Write(OcrText.ToolJson(engine.RecognizerLanguage.LanguageTag, lines));
                    }
                }
            }
        }
        catch (Exception ex)
        {
            output.Write(OcrText.ToolError("failed", ex.GetBaseException().Message));
            code = 1;
        }
        output.Flush();
        return code;
    }
}
