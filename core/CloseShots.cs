using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;

// Kapanan pencerenin son görüntüsü (Hyprland windowsOut, ii: 200 ms emphasizedDecel, popin %90 + fadeOut): pencere
// kapanınca son karesi küçülerek solar. DWM önizlemesi pencereyle birlikte ölür; bu yüzden kapatma isteğinden hemen önce
// pencerenin görüntüsü alınır (PrintWindow, DWM'in kendi karesi), köşeleri pencereninki gibi yuvarlanıp ekran dışındaki
// saydam bir pencereye çizilir. Pencere gerçekten kapanınca (Dwindle.OnWindowGone) animasyon bu görüntünün DWM
// önizlemesiyle oynar: ölçek ve saydamlık GPU'da, her karede çizim yok. Kapanmazsa (kaydet sorusu) görüntü atılır.
static class CloseShots
{
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    const uint PW_RENDERFULLCONTENT = 2;
    const int MaxPixels = 3840 * 2400; // daha büyüğü (tam ekran oyun) kopyalanmaz
    const int KeepMs = 2500;           // pencere bu sürede kapanmazsa görüntü atılır

    internal sealed class Shot
    {
        public ShotWindow Win; public Native.RECT Frame; public int At;
        public void Dispose() { try { if (Win != null) Win.Dispose(); } catch { } Win = null; }
    }

    static readonly Dictionary<long, Shot> shots = new Dictionary<long, Shot>();
    public static Control Ui;

    // Kanca thread'inden: görüntü alınır (en çok 100 ms beklenir), sonra kapatma isteği gönderilir. Animasyon kapalıysa,
    // oyun modundaysa ya da pencere askıdaysa doğrudan kapatılır.
    public static void CloseWithShot(IntPtr h, Action close)
    {
        var ui = Ui;
        if (ui == null || !Prefs.Animations || Anims.Budget(Anims.WindowsOut.Ms) <= 0 || Native.IsHungAppWindow(h)) { close(); return; }
        ThreadPool.QueueUserWorkItem(_ =>
        {
            Native.RECT frame = new Native.RECT();
            var task = Task.Factory.StartNew(() => Capture(h, out frame));
            bool done = false;
            try { done = task.Wait(100); } catch { }
            Bitmap shot = done ? task.Result : null;
            if (shot != null)
            {
                var fr = frame;
                try { ui.BeginInvoke((Action)(() => Keep(h, shot, fr))); }
                catch { shot.Dispose(); }
            }
            else if (!done) task.ContinueWith(t => { try { if (t.Result != null) t.Result.Dispose(); } catch { } });
            close();
        });
    }

    // Pencerenin görünen çerçevesi (gölge payları hariç), köşeleri pencereninki gibi yuvarlanmış, önceden çarpılmış ARGB
    static Bitmap Capture(IntPtr h, out Native.RECT frame)
    {
        frame = new Native.RECT();
        try
        {
            if (!Native.IsWindow(h) || !Native.IsWindowVisible(h) || Native.IsIconic(h)) return null;
            int cloaked;
            if (Native.DwmGetWindowAttribute(h, Native.DWMWA_CLOAKED, out cloaked, 4) == 0 && cloaked != 0) return null;
            var wr = Slider.WinRect(h); var fr = Slider.FrameRect(h);
            int ww = wr.Right - wr.Left, wh = wr.Bottom - wr.Top, fw = fr.Right - fr.Left, fh = fr.Bottom - fr.Top;
            if (ww <= 0 || wh <= 0 || fw < 8 || fh < 8 || (long)ww * wh > MaxPixels) return null;
            var screen = Screen.FromHandle(h).Bounds;
            if (Slider.CoversMonitor(fr, screen.X, screen.Y, screen.Width, screen.Height)) return null; // tam ekran: animasyon yok
            using (var full = new Bitmap(ww, wh, PixelFormat.Format32bppRgb))
            {
                using (var g = Graphics.FromImage(full))
                {
                    IntPtr dc = g.GetHdc();
                    bool ok;
                    try { ok = PrintWindow(h, dc, PW_RENDERFULLCONTENT); }
                    finally { g.ReleaseHdc(dc); }
                    if (!ok) return null;
                }
                var shot = new Bitmap(fw, fh, PixelFormat.Format32bppPArgb);
                using (var g = Graphics.FromImage(shot))
                {
                    g.Clear(Color.Transparent);
                    int radius = Rounder.RadiusFor(h);
                    if (radius > 0)
                        using (var path = new GraphicsPathHelper(new RectangleF(0, 0, fw, fh), radius)) g.SetClip(path.Path);
                    g.DrawImage(full, new Rectangle(0, 0, fw, fh), new Rectangle(fr.Left - wr.Left, fr.Top - wr.Top, fw, fh), GraphicsUnit.Pixel);
                }
                frame = fr;
                return shot;
            }
        }
        catch (Exception ex) { Slider.Log("kapanış görüntüsü: " + ex.Message); return null; }
    }

    // UI thread'inde
    static void Keep(IntPtr h, Bitmap bmp, Native.RECT frame)
    {
        Shot old, s;
        try { s = new Shot { Win = new ShotWindow(bmp), Frame = frame, At = Environment.TickCount }; }
        catch (Exception ex) { Slider.Log("kapanış görüntüsü: " + ex.Message); return; }
        finally { bmp.Dispose(); }
        lock (shots) { shots.TryGetValue(h.ToInt64(), out old); shots[h.ToInt64()] = s; }
        if (old != null) old.Dispose();
        var timer = new System.Windows.Forms.Timer { Interval = KeepMs };
        timer.Tick += (o, e) =>
        {
            timer.Stop(); timer.Dispose();
            bool mine;
            lock (shots) { Shot cur; mine = shots.TryGetValue(h.ToInt64(), out cur) && cur == s; if (mine) shots.Remove(h.ToInt64()); }
            if (mine) s.Dispose(); // pencere kapanmadı (kaydet sorusu ...)
        };
        timer.Start();
    }

    // Pencere kapandı (UI thread'i): görüntüsü varsa animasyon onundur
    public static Shot Take(long h)
    {
        Shot s;
        lock (shots) { if (!shots.TryGetValue(h, out s)) return null; shots.Remove(h); }
        if (unchecked(Environment.TickCount - s.At) > KeepMs) { s.Dispose(); return null; }
        return s;
    }

    // Hyprland popin: pencere merkezine doğru `popin` yüzdesine küçülürken saydamlaşır (e: 0..1 eğri değeri)
    internal static Native.RECT Shrink(Native.RECT r, int popin, double e)
    {
        double k = 1 - (1 - Math.Max(10, Math.Min(100, popin)) / 100.0) * e;
        double cx = (r.Left + r.Right) / 2.0, cy = (r.Top + r.Bottom) / 2.0;
        double hw = (r.Right - r.Left) * k / 2, hh = (r.Bottom - r.Top) * k / 2;
        return new Native.RECT { Left = (int)Math.Round(cx - hw), Top = (int)Math.Round(cy - hh), Right = (int)Math.Round(cx + hw), Bottom = (int)Math.Round(cy + hh) };
    }
}

// Görüntüyü tutan ekran dışı saydam pencere (RingTemplate gibi: DWM önizlemesi ekran dışındayken de çizer)
sealed class ShotWindow : Form
{
    [StructLayout(LayoutKind.Sequential)] struct PT { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct SZ { public int cx, cy; }
    [StructLayout(LayoutKind.Sequential)] struct BLEND { public byte Op, Flags, Alpha, Format; }
    [DllImport("user32.dll")] static extern bool UpdateLayeredWindow(IntPtr h, IntPtr hdcDst, ref PT pptDst, ref SZ psize, IntPtr hdcSrc, ref PT pptSrc, uint crKey, ref BLEND pblend, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    const int X = -32000, Y = -32000;
    public readonly IntPtr Hwnd;

    public ShotWindow(Bitmap bmp)
    {
        FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; StartPosition = FormStartPosition.Manual;
        Text = "lunge-close-shot";
        Bounds = new Rectangle(X, Y, bmp.Width, bmp.Height);
        CreateControl(); Hwnd = Handle;
        Show();
        IntPtr screenDc = GetDC(IntPtr.Zero), memDc = CreateCompatibleDC(screenDc), hbm = bmp.GetHbitmap(Color.FromArgb(0)), old = SelectObject(memDc, hbm);
        try
        {
            var dst = new PT { x = X, y = Y }; var sz = new SZ { cx = bmp.Width, cy = bmp.Height }; var src = new PT();
            var bl = new BLEND { Op = 0, Flags = 0, Alpha = 255, Format = 1 }; // AC_SRC_OVER, AC_SRC_ALPHA
            UpdateLayeredWindow(Hwnd, screenDc, ref dst, ref sz, memDc, ref src, 0, ref bl, 2); // ULW_ALPHA
        }
        finally { SelectObject(memDc, old); Native.DeleteObject(hbm); DeleteDC(memDc); ReleaseDC(IntPtr.Zero, screenDc); }
    }
    protected override bool ShowWithoutActivation { get { return true; } }
    protected override CreateParams CreateParams
    {
        get { var cp = base.CreateParams; cp.ExStyle |= 0x80 | 0x08000000 | 0x00080000 | 0x20; return cp; } // TOOLWINDOW | NOACTIVATE | LAYERED | TRANSPARENT
    }
}
