using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

// ---------------- Dikkat isteyen pencereler ----------------
// Bir uygulama dikkat isteyince (FlashWindowEx: yeni mesaj, biten indirme) Windows görev çubuğu düğmesini yakıp söndürürdü;
// görev çubuğu gizli olduğundan bu istek hiçbir yerde görünmüyordu (Hyprland "urgent"). Kabuk bildirimleri (shell hook)
// dinlenir: yanıp sönen pencere listeye girer, öne gelince ya da kapanınca çıkar. Bar listeyi /urgent.json'dan okur ve o
// pencerenin workspace'ini işaretler; "focus-urgent-or-last" kısayolu en son dikkat isteyen pencereye gider.
static class Urgent
{
    const int HSHELL_WINDOWDESTROYED = 2, HSHELL_WINDOWACTIVATED = 4, HSHELL_RUDEAPPACTIVATED = 0x8004, HSHELL_FLASH = 0x8006;

    [DllImport("user32.dll")] static extern bool RegisterShellHookWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool DeregisterShellHookWindow(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern uint RegisterWindowMessage(string name);
    [DllImport("user32.dll")] static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);

    static readonly object gate = new object();
    // en eskiden en yeniye
    static readonly List<long> order = new List<long>();
    static Sink sink;

    // Kabuk bildirimleri gizli bir üst düzey pencereye gelir (yalnızca mesaj penceresine gelmez)
    sealed class Sink : NativeWindow
    {
        readonly uint shellHook;
        public Sink()
        {
            shellHook = RegisterWindowMessage("SHELLHOOK");
            CreateHandle(new CreateParams { Caption = "lunge-urgent", ExStyle = 0x80 /*TOOLWINDOW*/, Style = unchecked((int)0x80000000) /*POPUP*/ });
            if (!RegisterShellHookWindow(Handle)) Slider.Log("urgent: kabuk bildirimleri alınamadı (" + Marshal.GetLastWin32Error() + ")");
        }
        protected override void WndProc(ref Message m)
        {
            if (shellHook != 0 && m.Msg == (int)shellHook) OnShell(m.WParam.ToInt64() & 0xFFFF, m.LParam);
            base.WndProc(ref m);
        }
    }

    public static void Start()
    {
        var t = new Thread(() =>
        {
            try { sink = new Sink(); Application.Run(); }
            catch (Exception ex) { Slider.Log("urgent: " + ex.Message); }
        }) { IsBackground = true, Name = "urgent" };
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
    }

    static void OnShell(long code, IntPtr hwnd)
    {
        long h = hwnd.ToInt64();
        bool changed;
        lock (gate)
        {
            if (code == HSHELL_FLASH)
            {
                // zaten öndeki pencere dikkat isteyemez: kullanıcı ona bakıyor
                if (GetAncestor(GetForegroundWindow(), 2) == hwnd) return;
                order.Remove(h);
                order.Add(h);
                changed = true;
            }
            else if (code == HSHELL_WINDOWACTIVATED || code == HSHELL_RUDEAPPACTIVATED || code == HSHELL_WINDOWDESTROYED)
                changed = order.Remove(h);
            else return;
        }
        if (changed) Toasts.Emit("ll:urgent");
    }

    // Bar için: dikkat isteyen pencerelerin tutamaçları (kapanmış olanlar atılır)
    public static List<long> Snapshot()
    {
        lock (gate)
        {
            order.RemoveAll(h => !IsWindow(new IntPtr(h)));
            return new List<long>(order);
        }
    }

    // En son dikkat isteyen pencere (yoksa 0)
    public static long Latest()
    {
        var list = Snapshot();
        return list.Count > 0 ? list[list.Count - 1] : 0;
    }

    public static string Json()
    {
        var parts = new List<string>();
        foreach (var h in Snapshot()) parts.Add(h.ToString());
        return "[" + string.Join(",", parts) + "]";
    }
}
