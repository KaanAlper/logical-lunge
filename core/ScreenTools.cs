// illogical-impulse'un ekran araçları, kendi süreçlerinde (çekirdeğin arayüz thread'i hiç beklemez):
//  - lunge.exe --ocr [ms]        (Super+Shift+T) ekrandan metin: alan seçilir (Bölge seçici), Windows'un cihazdaki OCR'ı
//                                okur (tools\lunge-ocr.exe, ağ yok), metin panoya, önizlemesi bizim bildirim kartımızda
//  - lunge.exe --pick-color [ms] (Super+Shift+C) renk seçici: tüm monitörlerin donmuş görüntüsünde imleci izleyen bir
//                                büyüteç; tık / Enter #rrggbb'yi panoya koyar, oklar 1 piksel kaydırır, Esc iptal
// [ms]: önce bu kadar beklenir (Super menüsü kapanırken görüntüye girmesin)
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;

static class ScreenTools
{
    // Aynı anda tek araç: ikinci basış (ya da OCR'dan sonra renk seçici) öncekinin üstüne ikinci bir donmuş ekran açmasın
    static Mutex single;
    public static bool Claim()
    {
        bool created;
        try { single = new Mutex(true, "LogicalLunge.ScreenTool", out created); }
        catch { return true; }
        return created;
    }

    // Çalışan çekirdeğin bildirim kartı (/notify): başlık ve gövde Türkçe kaynak metin, kabuk kendi diline çevirir
    public static void Notify(string kind, string title, string body, string icon, string image = null, bool mono = false)
    {
        var sb = new StringBuilder("/notify?kind=").Append(kind)
            .Append("&title=").Append(Uri.EscapeDataString(title))
            .Append("&body=").Append(Uri.EscapeDataString(body ?? ""));
        if (icon != null) sb.Append("&icon=").Append(icon);
        if (image != null) sb.Append("&image=").Append(Uri.EscapeDataString(image));
        if (mono) sb.Append("&mono=1");
        if (Supervisor.PostToCore(sb.ToString(), 3000) != 204) Slider.Log("ekran aracı: bildirim gönderilemedi (" + title + ")");
    }

    public static bool Copy(string text)
    {
        // true: bu süreç kapanınca da panoda kalsın
        try { Clipboard.SetDataObject(text, true, 10, 100); return true; }
        catch (Exception ex) { Slider.Log("ekran aracı: panoya kopyalanamadı: " + ex.Message); return false; }
    }

    public static void Wait(string[] args)
    {
        int ms;
        if (args.Length >= 2 && int.TryParse(args[1], out ms) && ms > 0 && ms <= 2000) Thread.Sleep(ms);
    }
}

// ---------------- Ekrandan metin (OCR) ----------------
static class TextFromScreen
{
    const int ToolTimeoutMs = 30000;

    public static void Run()
    {
        Rectangle vs;
        byte[] png;
        using (var shot = RegionSearch.CaptureAll(out vs))
        {
            var pk = new RegionSearch.Picker(shot, vs, I18n.T("Metni kopyalanacak alanı seç  •  Esc: iptal"));
            Application.Run(pk);
            var r = pk.Result;
            if (r.IsEmpty) return;
            png = Prepare(shot, r);
        }
        string output, why;
        if (!RunTool(png, Languages(), out output, out why))
        {
            Slider.Log("OCR: " + why);
            ScreenTools.Notify("error", "Metin tanınamadı", why, "error");
            return;
        }
        string lang, error;
        List<List<string>> lines;
        if (!Parse(output, out lang, out lines, out error))
        {
            Slider.Log("OCR: " + error);
            if (error == "nolang")
                ScreenTools.Notify("warning", "Metin tanıma dili yüklü değil",
                    "Windows Ayarlar > Zaman ve dil > Dil ve bölge: dilinizin seçeneklerinden \"Optik karakter tanıma\" özelliğini yükleyin (ya da bir dil ekleyin).", "translate");
            else ScreenTools.Notify("error", "Metin tanınamadı", error, "error");
            return;
        }
        string text = OcrText.Join(lines);
        Slider.Log("OCR (" + lang + "): " + lines.Count + " satır, " + text.Length + " karakter");
        if (text.Length == 0) { ScreenTools.Notify("info", "Metin bulunamadı", "Seçilen alanda okunabilir yazı yok.", "document_scanner"); return; }
        if (!ScreenTools.Copy(text)) { ScreenTools.Notify("error", "Kopyalanamadı", "Pano meşgul", "error"); return; }
        ScreenTools.Notify("ok", "Metin panoya kopyalandı", OcrText.Preview(text, 4, 240), "document_scanner");
    }

    // Arayüzün dili önce, sonra Windows'un dilleri (lunge-ocr.exe kurulu OCR dilleri arasından seçer)
    static List<string> Languages()
    {
        var list = new List<string>();
        foreach (var l in new[] { I18n.Code(), System.Globalization.CultureInfo.CurrentUICulture.Name, System.Globalization.CultureInfo.CurrentCulture.Name })
            if (!string.IsNullOrEmpty(l) && System.Text.RegularExpressions.Regex.IsMatch(l, "^[A-Za-z]{2,3}(-[A-Za-z0-9]{1,8})*$") && !list.Contains(l)) list.Add(l);
        return list;
    }

    // Seçilen alan: küçük yazı için büyütülür, çevresine kenar rengiyle boşluk konur (OcrText.ScaleFor / Canvas)
    static byte[] Prepare(Bitmap shot, Rectangle r)
    {
        double s = OcrText.ScaleFor(r.Width, r.Height);
        int w = Math.Max(1, (int)Math.Round(r.Width * s)), h = Math.Max(1, (int)Math.Round(r.Height * s));
        int cw, ch, ox, oy;
        OcrText.Canvas(w, h, out cw, out ch, out ox, out oy);
        using (var canvas = new Bitmap(cw, ch, System.Drawing.Imaging.PixelFormat.Format32bppArgb))
        {
            using (var g = Graphics.FromImage(canvas))
            {
                g.Clear(shot.GetPixel(r.Left, r.Top));
                g.InterpolationMode = InterpolationMode.HighQualityBicubic;
                g.PixelOffsetMode = PixelOffsetMode.HighQuality;
                g.DrawImage(shot, new Rectangle(ox, oy, w, h), r, GraphicsUnit.Pixel);
            }
            using (var ms = new System.IO.MemoryStream())
            {
                canvas.Save(ms, System.Drawing.Imaging.ImageFormat.Png);
                return ms.ToArray();
            }
        }
    }

    // Görüntü stdin'den gider (geçici dosya yok), cevap stdout'tan gelir
    static bool RunTool(byte[] png, List<string> langs, out string output, out string why)
    {
        output = null; why = null;
        string exe = Paths.Tool("lunge-ocr.exe");
        if (!System.IO.File.Exists(exe)) { why = "Metin tanıma aracı (lunge-ocr.exe) bulunamadı."; return false; }
        try
        {
            var psi = new ProcessStartInfo(exe, string.Join(",", langs))
            {
                UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true, RedirectStandardOutput = true,
                StandardOutputEncoding = new UTF8Encoding(false),
            };
            using (var p = Process.Start(psi))
            {
                var read = p.StandardOutput.ReadToEndAsync();
                try
                {
                    p.StandardInput.BaseStream.Write(png, 0, png.Length);
                    p.StandardInput.Close();
                }
                catch (System.IO.IOException) { } // araç erken çıktı: sebebi çıktısında
                if (!read.Wait(ToolTimeoutMs))
                {
                    try { p.Kill(); } catch { }
                    why = "Metin tanıma çok uzun sürdü.";
                    return false;
                }
                output = read.Result;
                return true;
            }
        }
        catch (Exception ex) { why = ex.GetBaseException().Message; return false; }
    }

    // lunge-ocr.exe'nin cevabı (OcrText.ToolJson / ToolError). false: error = "nolang", "failed: ..." ya da okunamadı
    internal static bool Parse(string output, out string lang, out List<List<string>> lines, out string error)
    {
        lang = ""; lines = new List<List<string>>(); error = null;
        Dictionary<string, object> d;
        try { d = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(output ?? ""); }
        catch { d = null; }
        if (d == null) { error = "OCR aracının cevabı okunamadı"; return false; }
        object v;
        if (d.TryGetValue("error", out v))
        {
            error = v as string ?? "failed";
            object detail;
            if (error != "nolang" && d.TryGetValue("detail", out detail) && detail is string) error += ": " + detail;
            return false;
        }
        if (d.TryGetValue("lang", out v) && v is string) lang = (string)v;
        if (!d.TryGetValue("lines", out v) || !(v is System.Collections.ArrayList)) { error = "OCR aracının cevabı okunamadı"; return false; }
        foreach (var line in (System.Collections.ArrayList)v)
        {
            var words = new List<string>();
            var arr = line as System.Collections.ArrayList;
            if (arr != null) foreach (var w in arr) if (w is string) words.Add((string)w);
            lines.Add(words);
        }
        return true;
    }
}

// ---------------- Renk seçici (büyüteç) ----------------
static class ColorPick
{
    // #rrggbb (küçük harf)
    public static string Hex(Color c) { return "#" + c.R.ToString("x2") + c.G.ToString("x2") + c.B.ToString("x2"); }

    // Büyütecin yeri: imlecin sağ altında; monitörün kenarına taşarsa imlecin öbür yanına geçer, yine de monitörün içinde kalır
    internal static Rectangle Place(Point cursor, Size size, Rectangle monitor, int gap)
    {
        int x = cursor.X + gap, y = cursor.Y + gap;
        if (x + size.Width > monitor.Right) x = cursor.X - gap - size.Width;
        if (y + size.Height > monitor.Bottom) y = cursor.Y - gap - size.Height;
        x = Math.Max(monitor.Left, Math.Min(x, monitor.Right - size.Width));
        y = Math.Max(monitor.Top, Math.Min(y, monitor.Bottom - size.Height));
        return new Rectangle(x, y, size.Width, size.Height);
    }

    // Kartın renk örneği: küçük bir PNG (data URL)
    static string Swatch(Color c)
    {
        using (var b = new Bitmap(48, 48))
        {
            using (var g = Graphics.FromImage(b)) g.Clear(Color.FromArgb(255, c.R, c.G, c.B));
            using (var ms = new System.IO.MemoryStream())
            {
                b.Save(ms, System.Drawing.Imaging.ImageFormat.Png);
                return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray());
            }
        }
    }

    public static void Run()
    {
        Rectangle vs;
        Color? picked;
        using (var shot = RegionSearch.CaptureAll(out vs))
        {
            var f = new Loupe(shot, vs);
            Application.Run(f);
            picked = f.Picked;
        }
        if (!picked.HasValue) return;
        string hex = Hex(picked.Value);
        Slider.Log("renk seçici: " + hex);
        if (!ScreenTools.Copy(hex)) { ScreenTools.Notify("error", "Kopyalanamadı", "Pano meşgul", "error"); return; }
        ScreenTools.Notify("ok", "Renk panoya kopyalandı", hex, "colorize", Swatch(picked.Value), true);
    }

    sealed class Loupe : Form
    {
        [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point pt, uint flags);
        [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr hmon, int type, out uint dx, out uint dy);

        const int Cells = 11;     // büyüteçteki piksel sayısı (tek: ortadaki seçilen)
        const int Cell = 12;      // bir pikselin 96 DPI'daki boyu
        const int Pad = 8, Label = 34, Radius = 16, Gap = 22;
        static readonly Color Accent = Color.FromArgb(208, 188, 255), Surface = Color.FromArgb(242, 29, 27, 32), OnSurface = Color.FromArgb(230, 224, 233);

        readonly Bitmap shot;
        readonly Rectangle bounds;
        Point at;                 // seçilen piksel (görüntüde)
        Rectangle box = Rectangle.Empty, hintBox = Rectangle.Empty;
        public Color? Picked;

        public Loupe(Bitmap shot, Rectangle bounds)
        {
            this.shot = shot; this.bounds = bounds;
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true; KeyPreview = true;
            StartPosition = FormStartPosition.Manual; Bounds = bounds;
            DoubleBuffered = true; Cursor = Cursors.Cross;
            var c = Cursor.Position;
            at = new Point(c.X - bounds.Left, c.Y - bounds.Top);
        }
        protected override CreateParams CreateParams { get { var p = base.CreateParams; p.ExStyle |= 0x80; return p; } }

        // imlecin monitörünün ölçeği (HiDPI'da büyüteç de büyür)
        float DpiScale(Point screen)
        {
            uint dx, dy;
            try { if (GetDpiForMonitor(MonitorFromPoint(screen, 2), 0, out dx, out dy) == 0 && dx > 0) return dx / 96f; } catch { }
            return 1f;
        }

        Rectangle BoxFor(Point p)
        {
            var screen = new Point(p.X + bounds.Left, p.Y + bounds.Top);
            float s = DpiScale(screen);
            int grid = (int)Math.Round(Cells * Cell * s);
            var size = new Size(grid + 2 * (int)(Pad * s), grid + (int)((2 * Pad + Label) * s));
            var mon = Screen.FromPoint(screen).Bounds;
            mon.Offset(-bounds.Left, -bounds.Top);
            return ColorPick.Place(p, size, mon, (int)(Gap * s));
        }

        void MoveTo(Point p)
        {
            p.X = Math.Max(0, Math.Min(p.X, shot.Width - 1));
            p.Y = Math.Max(0, Math.Min(p.Y, shot.Height - 1));
            var old = box;
            at = p;
            box = BoxFor(p);
            if (!old.IsEmpty) Invalidate(Grow(old));
            Invalidate(Grow(box));
        }

        static Rectangle Grow(Rectangle r) { r.Inflate(4, 4); return r; }

        static GraphicsPath Round(Rectangle r, int radius)
        {
            var path = new GraphicsPath();
            int d = Math.Max(2, radius * 2);
            path.AddArc(r.Left, r.Top, d, d, 180, 90);
            path.AddArc(r.Right - d, r.Top, d, d, 270, 90);
            path.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
            path.AddArc(r.Left, r.Bottom - d, d, d, 90, 90);
            path.CloseFigure();
            return path;
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            try { PaintBody(e); } catch (Exception ex) { PaintErrors.Report("renk seçici", ex); }
        }

        void PaintBody(PaintEventArgs e)
        {
            var g = e.Graphics;
            var clip = Rectangle.Intersect(e.ClipRectangle, new Rectangle(0, 0, shot.Width, shot.Height));
            if (clip.Width > 0 && clip.Height > 0) g.DrawImage(shot, clip, clip, GraphicsUnit.Pixel);
            PaintHint(g);
            if (box.IsEmpty) box = BoxFor(at);
            var screen = new Point(at.X + bounds.Left, at.Y + bounds.Top);
            float s = DpiScale(screen);
            int pad = (int)(Pad * s), grid = box.Width - 2 * pad;
            float cell = grid / (float)Cells;
            g.SmoothingMode = SmoothingMode.AntiAlias;
            using (var path = Round(box, (int)(Radius * s)))
            {
                using (var bg = new SolidBrush(Surface)) g.FillPath(bg, path);
                using (var pen = new Pen(Color.FromArgb(150, Accent), Math.Max(1f, s))) g.DrawPath(pen, path);
            }
            // büyütülmüş pikseller (görüntünün dışı boş kalır)
            var gridRect = new Rectangle(box.Left + pad, box.Top + pad, grid, grid);
            using (var gp = Round(gridRect, (int)(10 * s)))
            {
                var state = g.Save();
                g.SetClip(gp, CombineMode.Intersect);
                g.SmoothingMode = SmoothingMode.None;
                g.InterpolationMode = InterpolationMode.NearestNeighbor;
                g.PixelOffsetMode = PixelOffsetMode.Half;
                using (var empty = new SolidBrush(Color.FromArgb(20, 18, 24))) g.FillRectangle(empty, gridRect);
                int half = Cells / 2;
                var src = new Rectangle(at.X - half, at.Y - half, Cells, Cells);
                var inside = Rectangle.Intersect(src, new Rectangle(0, 0, shot.Width, shot.Height));
                if (inside.Width > 0 && inside.Height > 0)
                {
                    var dst = new RectangleF(gridRect.Left + (inside.Left - src.Left) * cell, gridRect.Top + (inside.Top - src.Top) * cell, inside.Width * cell, inside.Height * cell);
                    g.DrawImage(shot, dst, inside, GraphicsUnit.Pixel);
                }
                g.Restore(state);
            }
            // ortadaki piksel: koyu ve açık çerçeve (her renkte görünür)
            g.SmoothingMode = SmoothingMode.None;
            var mid = new RectangleF(gridRect.Left + (Cells / 2) * cell, gridRect.Top + (Cells / 2) * cell, cell, cell);
            using (var dark = new Pen(Color.FromArgb(200, 0, 0, 0), Math.Max(1f, s))) g.DrawRectangle(dark, mid.X - s, mid.Y - s, mid.Width + 2 * s, mid.Height + 2 * s);
            using (var light = new Pen(Color.White, Math.Max(1f, s))) g.DrawRectangle(light, mid.X, mid.Y, mid.Width, mid.Height);
            // örnek + #rrggbb
            var c = shot.GetPixel(at.X, at.Y);
            g.SmoothingMode = SmoothingMode.AntiAlias;
            int sw = (int)(20 * s), ly = gridRect.Bottom + (int)(((2 * Pad + Label) * s - 2 * pad - sw) / 2f) + pad / 2;
            var swatch = new Rectangle(gridRect.Left + (int)(2 * s), ly, sw, sw);
            using (var p = Round(swatch, (int)(6 * s)))
            {
                using (var b = new SolidBrush(Color.FromArgb(255, c.R, c.G, c.B))) g.FillPath(b, p);
                using (var pen = new Pen(Color.FromArgb(120, OnSurface), 1f)) g.DrawPath(pen, p);
            }
            using (var f = new Font("Consolas", 15f * s, FontStyle.Bold, GraphicsUnit.Pixel))
            using (var fg = new SolidBrush(OnSurface))
            {
                string hex = ColorPick.Hex(c).ToUpperInvariant();
                var sz = g.MeasureString(hex, f);
                g.DrawString(hex, f, fg, swatch.Right + 8 * s, swatch.Top + (sw - sz.Height) / 2f);
            }
        }

        // ekranın üstünde (imlecin monitöründe) ne yapılacağı
        void PaintHint(Graphics g)
        {
            string hint = I18n.T("Renk seç: tıkla ya da Enter  •  Oklar: 1 piksel  •  Esc: iptal");
            var screen = new Point(at.X + bounds.Left, at.Y + bounds.Top);
            float s = DpiScale(screen);
            using (var f = new Font("Segoe UI", 16f * s, GraphicsUnit.Pixel))
            {
                var sz = g.MeasureString(hint, f);
                var mon = Screen.FromPoint(screen).Bounds; mon.Offset(-bounds.Left, -bounds.Top);
                float x = mon.Left + (mon.Width - sz.Width) / 2, y = mon.Top + 70 * s;
                var r = new Rectangle((int)(x - 16 * s), (int)(y - 8 * s), (int)(sz.Width + 32 * s), (int)(sz.Height + 16 * s));
                if (r != hintBox)
                {
                    if (!hintBox.IsEmpty) Invalidate(Grow(hintBox));
                    hintBox = r;
                    Invalidate(Grow(r));
                }
                g.SmoothingMode = SmoothingMode.AntiAlias;
                using (var p = Round(r, (int)(r.Height / 2)))
                using (var bg = new SolidBrush(Color.FromArgb(230, 20, 18, 24))) g.FillPath(bg, p);
                using (var fg = new SolidBrush(OnSurface)) g.DrawString(hint, f, fg, x, y);
            }
        }

        protected override void OnMouseMove(MouseEventArgs e) { if (e.Location != at) MoveTo(e.Location); }
        // bırakınca: basış da bırakış da bizde kalır (alttaki pencereye yarım bir tık, sağ tık menüsü gitmez)
        protected override void OnMouseUp(MouseEventArgs e)
        {
            if (e.Button == MouseButtons.Right) Close();
            else if (e.Button == MouseButtons.Left) { MoveTo(e.Location); Pick(); }
        }

        void Pick()
        {
            Picked = shot.GetPixel(at.X, at.Y);
            Close();
        }

        // Oklar imleci 1 piksel (Shift ile 10) kaydırır; Enter / Boşluk seçer; Esc iptal
        protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
        {
            Keys key = keyData & Keys.KeyCode;
            int step = (keyData & Keys.Shift) != 0 ? 10 : 1;
            int dx = key == Keys.Left ? -step : key == Keys.Right ? step : 0, dy = key == Keys.Up ? -step : key == Keys.Down ? step : 0;
            if (dx != 0 || dy != 0)
            {
                var p = new Point(Math.Max(0, Math.Min(at.X + dx, shot.Width - 1)), Math.Max(0, Math.Min(at.Y + dy, shot.Height - 1)));
                Cursor.Position = new Point(p.X + bounds.Left, p.Y + bounds.Top);
                MoveTo(p);
                return true;
            }
            if (key == Keys.Escape) { Close(); return true; }
            if (key == Keys.Enter || key == Keys.Space) { Pick(); return true; }
            return base.ProcessCmdKey(ref msg, keyData);
        }

        System.Windows.Forms.Timer keepTop;
        protected override void OnShown(EventArgs e)
        {
            base.OnShown(e); Activate();
            // Bölge seçicideki gibi: başka bir pencere kendini en üste alıp donmuş görüntünün üstüne çıkmasın
            keepTop = new System.Windows.Forms.Timer { Interval = 30 };
            keepTop.Tick += (o, ev) => Native.SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
            keepTop.Start();
            MoveTo(at);
        }
        protected override void OnFormClosed(FormClosedEventArgs e) { if (keepTop != null) keepTop.Stop(); base.OnFormClosed(e); }
    }
}
