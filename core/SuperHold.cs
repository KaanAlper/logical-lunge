using System.Threading;

// Super basılı tutulunca bar, noktaların yerine workspace numaralarını gösterir (ii): 100 ms sonra, Super bırakılınca
// geri döner. Super ile bir kısayola basmak arada bir anlık titreme olmasın diye: 100 ms içinde bırakılırsa hiç gösterilmez.
static class SuperHold
{
    public const int DelayMs = 100;
    static readonly object gate = new object();
    static Timer timer;
    static bool down, shown;

    public static void Down()
    {
        lock (gate)
        {
            if (down) return;
            down = true; shown = false;
            if (timer == null) timer = new Timer(Fire);
            timer.Change(DelayMs, Timeout.Infinite);
        }
    }

    public static void Up()
    {
        bool was;
        lock (gate)
        {
            if (!down) return;
            down = false;
            was = shown; shown = false;
            if (timer != null) timer.Change(Timeout.Infinite, Timeout.Infinite);
        }
        if (was) ThreadPool.QueueUserWorkItem(_ => Toasts.Emit("ll:ws-numbers-release"));
    }

    static void Fire(object state)
    {
        lock (gate)
        {
            if (!down || shown) return;
            shown = true;
        }
        Toasts.Emit("ll:ws-numbers-hold");
    }
}
