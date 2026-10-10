using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

// ---------------- Olay kancalarının pompası ----------------
// Bağlam dışı bir olay kancasının (SetWinEventHook) olayları kancayı kuran thread'in mesaj kuyruğuna gelir, geri çağrı
// o thread'de çalışır. Kancalar önceden işi yapan thread'lerdeydi (UI thread'i, köşe thread'i): o thread meşgulken olaylar
// birikti, imlecin ve her pencerenin her konum değişikliği tek tek işlendi; yük altında "olay kancası gecikti: köşe
// 10938 ms". Şimdi bütün kancalar yalnızca mesaj pompalayan kendi thread'inde: geri çağrı olayı süzer, kuyruğa koyar ve
// döner. İş tüketicinin thread'inde (UI thread'i, köşe thread'i) ya da kendi işçi thread'inde yapılır; aynı pencerenin
// bekleyen konum değişiklikleri tek olaya (en sonuncusuna) iner.
static class WinEventPump
{
    public struct Ev { public uint Event; public IntPtr Hwnd; public int Object, Child; public uint Time; }

    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam, lParam; public uint time; public int x, y; }
    [DllImport("user32.dll")] static extern int GetMessage(out MSG m, IntPtr h, uint min, uint max);
    [DllImport("user32.dll")] static extern bool PeekMessage(out MSG m, IntPtr h, uint min, uint max, uint remove);
    [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
    [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG m);
    [DllImport("user32.dll")] static extern bool PostThreadMessage(uint thread, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();

    const uint WM_RUN = 0x8000 + 0x57; // WM_APP: sıradaki kurma / sökme işleri
    const uint WINEVENT_SKIPOWNPROCESS = 0x0002; // bağlam dışı (0) ve kendi pencerelerimiz hariç
    static readonly object gate = new object();
    static volatile uint threadId;
    static readonly Queue<Action> work = new Queue<Action>();
    static readonly Dictionary<IntPtr, EventQueue> owners = new Dictionary<IntPtr, EventQueue>(); // yalnızca pompa thread'i
    static readonly Native.WinEventDelegate cb = OnEvent; // Windows çağırdıkça canlı kalmalı

    static void EnsureThread()
    {
        lock (gate)
        {
            if (threadId != 0) return;
            var ready = new ManualResetEvent(false);
            new Thread(() =>
            {
                MSG m;
                PeekMessage(out m, IntPtr.Zero, 0, 0, 0); // thread'in mesaj kuyruğu
                threadId = GetCurrentThreadId();
                ready.Set();
                while (GetMessage(out m, IntPtr.Zero, 0, 0) > 0)
                {
                    if (m.hwnd == IntPtr.Zero && m.message == WM_RUN) { RunQueued(); continue; }
                    TranslateMessage(ref m); DispatchMessage(ref m);
                }
            }) { IsBackground = true, Name = "olay-kancaları", Priority = ThreadPriority.AboveNormal }.Start();
            if (!ready.WaitOne(3000)) Slider.Log("olay kancaları: pompa thread'i başlamadı");
        }
    }

    static void Run(Action a)
    {
        EnsureThread();
        if (threadId == GetCurrentThreadId()) { a(); return; }
        lock (work) work.Enqueue(a);
        if (!PostThreadMessage(threadId, WM_RUN, IntPtr.Zero, IntPtr.Zero)) Slider.Log("olay kancaları: pompaya iş gönderilemedi");
    }

    static void RunQueued()
    {
        while (true)
        {
            Action a;
            lock (work) { if (work.Count == 0) return; a = work.Dequeue(); }
            try { a(); } catch (Exception ex) { Callback.Failed("olay kancası kurulumu", ex); }
        }
    }

    // Kancayı pompa thread'inde kurar (min..max olayları q'ya gider); tanıtıcısı ya da kurulamadıysa sıfır döner
    public static IntPtr Hook(uint min, uint max, EventQueue q)
    {
        IntPtr h = IntPtr.Zero;
        var done = new ManualResetEvent(false);
        Run(() =>
        {
            h = Native.SetWinEventHook(min, max, IntPtr.Zero, cb, 0, 0, WINEVENT_SKIPOWNPROCESS);
            if (h != IntPtr.Zero) owners[h] = q;
            else Slider.Log("olay kancası kurulamadı: " + q.Name + " 0x" + min.ToString("X"));
            done.Set();
        });
        if (!done.WaitOne(3000)) Slider.Log("olay kancası zamanında kurulamadı: " + q.Name);
        return h;
    }

    public static void Unhook(IntPtr hook)
    {
        if (hook == IntPtr.Zero) return;
        Run(() => { UnhookWinEvent(hook); owners.Remove(hook); });
    }

    // Pompa thread'inde: yalnızca süz ve kuyruğa koy
    static void OnEvent(IntPtr hook, uint ev, IntPtr hwnd, int idObject, int idChild, uint thread, uint time)
    {
        try
        {
            EventQueue q;
            if (!owners.TryGetValue(hook, out q)) return;
            EventLag.Note(q.Name, time);
            q.Add(new Ev { Event = ev, Hwnd = hwnd, Object = idObject, Child = idChild, Time = time });
        }
        catch (Exception ex) { Callback.Failed("olay kancası", ex); }
    }
}

// Bir tüketicinin olayları: sırayla işlenir; birleşen olay (merges) aynı pencerenin bekleyen eşinin yerine en sona geçer.
// İş post'un gönderdiği thread'de, birkaç düzinelik turlarla yapılır (aradaki işi o thread'in kendisi de görsün).
sealed class EventQueue
{
    public readonly string Name;
    readonly Func<WinEventPump.Ev, bool> keep, merges;
    readonly Action<WinEventPump.Ev> handle;
    readonly Action<Action> post;
    readonly LinkedList<WinEventPump.Ev> items = new LinkedList<WinEventPump.Ev>();
    readonly Dictionary<long, LinkedListNode<WinEventPump.Ev>> pending = new Dictionary<long, LinkedListNode<WinEventPump.Ev>>();
    readonly Action drain;
    bool scheduled;
    int dropped;
    public const int Max = 8192, Batch = 64;

    // keep: pompa thread'inde, ucuz (null: hepsi); merges: birleşen olay (null: hiçbiri); post: işin thread'i
    public EventQueue(string name, Func<WinEventPump.Ev, bool> keep, Func<WinEventPump.Ev, bool> merges,
        Action<WinEventPump.Ev> handle, Action<Action> post)
    {
        Name = name; this.keep = keep; this.merges = merges; this.handle = handle; this.post = post;
        drain = Drain;
    }

    // Kendi işçi thread'i: işi hiçbir thread'e bağlı olmayan tüketiciler için
    public static Action<Action> Worker(string name)
    {
        var q = new BlockingCollection<Action>();
        new Thread(() => { foreach (var a in q.GetConsumingEnumerable()) a(); }) { IsBackground = true, Name = name }.Start();
        return a => q.Add(a);
    }

    static long Key(WinEventPump.Ev e) { return ((long)e.Event << 40) ^ e.Hwnd.ToInt64(); }

    public int Count { get { lock (items) return items.Count; } }

    public void Add(WinEventPump.Ev e)
    {
        if (keep != null && !keep(e)) return;
        bool schedule;
        lock (items)
        {
            if (merges != null && merges(e))
            {
                long k = Key(e);
                LinkedListNode<WinEventPump.Ev> old;
                if (pending.TryGetValue(k, out old)) items.Remove(old);
                pending[k] = items.AddLast(e);
            }
            else if (items.Count >= Max) { dropped++; return; }
            else items.AddLast(e);
            schedule = !scheduled;
            scheduled = true;
        }
        if (schedule) Post();
    }

    void Post()
    {
        try { post(drain); }
        catch (Exception ex)
        {
            lock (items) scheduled = false; // hedef thread yok (kapanıyor): sonraki olay yine dener
            Callback.Failed(Name + " olay kuyruğu", ex);
        }
    }

    void Drain()
    {
        for (int n = 0; n < Batch; n++)
        {
            WinEventPump.Ev e;
            int lost;
            lock (items)
            {
                if (items.Count == 0) { scheduled = false; return; }
                var node = items.First;
                e = node.Value;
                items.RemoveFirst();
                LinkedListNode<WinEventPump.Ev> cur;
                if (merges != null && pending.TryGetValue(Key(e), out cur) && cur == node) pending.Remove(Key(e));
                lost = dropped; dropped = 0;
            }
            if (lost > 0) Slider.Log("olay kuyruğu doldu: " + Name + " " + lost + " olay atlandı");
            try { handle(e); } catch (Exception ex) { Callback.Failed(Name + " olayı", ex); }
        }
        Post(); // kalanlar sonraki turda
    }
}
