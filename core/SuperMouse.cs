using System;
using System.Collections.Concurrent;
using System.Threading;

// Hyprland'in fare kısayolları (ii): Super + sol/orta tuş pencereyi taşır, Super + sağ tuş boyutlandırır. Yerel kanca
// yalnızca basışı ve bırakışı bildirir; pencereyi imlecin peşinden pencere yöneticisi götürür (wm-mouse-drag). Komutlar
// sırayla ve kendi bağlantılarıyla gider: bırakış hiçbir zaman basıştan önce varmaz, workspace kaydırmalarının
// kuyruğunu da beklemez.
static class SuperMouse
{
    static readonly BlockingCollection<string> queue = new BlockingCollection<string>(64);
    static readonly object gate = new object();
    static bool started;

    public static void Drag(bool resize) { Send("wm-mouse-drag " + (resize ? "resize" : "move")); }
    public static void End() { Send("wm-mouse-drag end"); }

    static void Send(string command)
    {
        lock (gate)
        {
            if (!started)
            {
                started = true;
                new Thread(Run) { IsBackground = true, Name = "super-mouse" }.Start();
            }
        }
        if (!queue.TryAdd(command)) Slider.Log("fare kısayolu: kuyruk dolu, atıldı: " + command);
    }

    static void Run()
    {
        var client = new TilingClient();
        foreach (var command in queue.GetConsumingEnumerable())
        {
            try
            {
                string error;
                if (!client.TryCommand(command, out error)) Slider.Log("fare kısayolu (" + command + "): " + error);
            }
            catch (Exception ex) { Slider.Log("fare kısayolu (" + command + "): " + ex.GetBaseException().Message); }
        }
    }
}
