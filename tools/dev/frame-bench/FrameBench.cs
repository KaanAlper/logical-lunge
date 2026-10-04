// Frame pacing as the screen shows it. Moves a DWM thumbnail across a strip of the primary monitor the way the core's
// slides do (PresentClock time, one thumbnail update per frame), paced either by DwmFlush after every frame (the core)
// or by a vblank-phased high resolution timer (FramePacer from 2f49b4a, kept here as the comparison). Every update's
// QPC and x go to a CSV; frame-capture records what was presented; analyze.py compares the two.
// Built with the core sources (Native, PresentClock). Run through frame-bench.ps1.
//   FrameBench <out.csv> <runs per method>
using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

static class FrameBench
{
    public const int StripY = 600, StripH = 64, Marker = 32, Travel = 1700, DurMs = 500;

    sealed class ToolForm : Form
    {
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override CreateParams CreateParams
        {
            // a tool window: the window manager does not tile it, the rounder leaves it alone, clicks go through
            get { var cp = base.CreateParams; cp.ExStyle |= 0x80 /*TOOLWINDOW*/ | 0x08000000 /*NOACTIVATE*/ | 0x20 /*TRANSPARENT*/; return cp; }
        }
    }

    // 2f49b4a's FramePacer: wakes at each vblank phase on a high resolution waitable timer instead of waiting for DWM
    sealed class TimerPacer : IDisposable
    {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateWaitableTimerExW(IntPtr security, string name, uint flags, uint access);
        [DllImport("kernel32.dll")] static extern bool SetWaitableTimer(IntPtr timer, ref long due, int period, IntPtr callback, IntPtr argument, bool resume);
        [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
        [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
        IntPtr timer;
        readonly long period;
        long deadline;

        public TimerPacer()
        {
            double ms = FrameStats.RefreshPeriodMs();
            period = Math.Max(1, (long)Math.Round(ms * Stopwatch.Frequency / 1000.0));
            var t = new Native.DWM_TIMING_INFO(); t.cbSize = (uint)Marshal.SizeOf(typeof(Native.DWM_TIMING_INFO));
            if (Native.DwmGetCompositionTimingInfo(IntPtr.Zero, ref t) == 0 && t.qpcVBlank > 0) deadline = Next(Stopwatch.GetTimestamp(), (long)t.qpcVBlank, period);
            timer = CreateWaitableTimerExW(IntPtr.Zero, null, 2 /*high resolution*/, 0x1f0003);
            if (timer == IntPtr.Zero) timer = CreateWaitableTimerExW(IntPtr.Zero, null, 0, 0x1f0003);
        }

        static long Next(long now, long previous, long interval)
        {
            if (previous <= 0) return now + interval;
            if (previous > now) return previous;
            return previous + ((now - previous) / interval + 1) * interval;
        }

        public void Wait()
        {
            long now = Stopwatch.GetTimestamp();
            deadline = Next(now, deadline, period);
            double ms = (deadline - now) * 1000.0 / Stopwatch.Frequency;
            long due = -Math.Max(1, (long)Math.Ceiling(ms * 10000));
            if (timer != IntPtr.Zero && SetWaitableTimer(timer, ref due, 0, IntPtr.Zero, IntPtr.Zero, false)) WaitForSingleObject(timer, (uint)Math.Ceiling(ms + 16));
            else Thread.Sleep(Math.Max(1, (int)Math.Ceiling(ms)));
            deadline += period;
        }

        public void Dispose() { if (timer != IntPtr.Zero) { CloseHandle(timer); timer = IntPtr.Zero; } }
    }

    [STAThread]
    static int Main(string[] args)
    {
        string outPath = args[0];
        int runs = int.Parse(args[1]);
        try { Process.GetCurrentProcess().PriorityClass = ProcessPriorityClass.High; } catch { }   // as the core
        Thread.CurrentThread.Priority = ThreadPriority.Highest;                                     // as its slide thread

        var src = new ToolForm { FormBorderStyle = FormBorderStyle.None, ShowInTaskbar = false, StartPosition = FormStartPosition.Manual,
                                 BackColor = Color.FromArgb(255, 0, 255), Bounds = new Rectangle(1860, StripY + 16, Marker, Marker), Text = "frame-bench marker" };
        var ov = new ToolForm { FormBorderStyle = FormBorderStyle.None, ShowInTaskbar = false, StartPosition = FormStartPosition.Manual, TopMost = true,
                                BackColor = Color.Black, Bounds = new Rectangle(0, StripY, 1920, StripH), Text = "frame-bench strip" };
        src.Show(); ov.Show();
        Application.DoEvents();
        IntPtr thumb;
        if (Native.DwmRegisterThumbnail(ov.Handle, src.Handle, out thumb) != 0) { Console.Error.WriteLine("no thumbnail"); return 1; }
        Action<int> place = x =>
        {
            var p = new Native.DWM_THUMBNAIL_PROPERTIES
            {
                dwFlags = Native.DWM_TNP_RECTDESTINATION | Native.DWM_TNP_VISIBLE | Native.DWM_TNP_OPACITY,
                rcDestination = new Native.RECT { Left = x, Top = 16, Right = x + Marker, Bottom = 16 + Marker },
                fVisible = true, opacity = 255
            };
            Native.DwmUpdateThumbnailProperties(thumb, ref p);
        };

        var log = new StringBuilder("# qpc frequency " + Stopwatch.Frequency + ", travel " + Travel + " px in " + DurMs + " ms\nrun,method,qpc,x\n");
        place(0); Native.DwmFlush(); Thread.Sleep(500);
        for (int run = 0; run < runs * 2; run++)
        {
            bool timed = run % 2 == 1;
            Thread.Sleep(350);
            var pc = new PresentClock();
            using (var pace = new TimerPacer())
                while (true)
                {
                    double p = Math.Min(1.0, pc.Ms() / DurMs);
                    int x = (int)Math.Round(p * Travel);
                    place(x);
                    log.Append(run).Append(timed ? ",timer," : ",flush,").Append(Stopwatch.GetTimestamp()).Append(',').Append(x).Append('\n');
                    if (p >= 1.0) { Native.DwmFlush(); break; }
                    if (timed) pace.Wait(); else Native.DwmFlush();
                }
            Thread.Sleep(250);
            place(0); Native.DwmFlush();
            Application.DoEvents();
        }
        File.WriteAllText(outPath, log.ToString());
        Native.DwmUnregisterThumbnail(thumb);
        ov.Close(); src.Close();
        return 0;
    }
}
