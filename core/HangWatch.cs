using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;

// ---------------- Kilitlenme bekçisi ----------------
// Çekirdeğin klavye ve fare kancaları kendi iş parçacıklarında, arayüzü ana iş parçacığında çalışır; Windows'un
// söktüğü kancayı yeniden kuran bekçiler de aynı iş parçacıklarındadır. Bir iş parçacığı takılınca kendi bekçisi de
// takılır: Super, kısayollar, sağ tık ve masaüstü geçişleri ölür (2026-10-05: iki kez, biri arka arkaya pencere
// taşırken, biri bir widget kaldırılırken; ikisinde de kullanıcı masaüstünü elle yeniledi). Bu bekçi hiçbir kilidi
// paylaşmayan ayrı bir iş parçacığında: izlenen bir iş parçacığı 8 sn nabız vermezse takılanların o anki çağrı
// yığınını logs\hang-*.txt'ye yazar (kök neden bir dahakine orada) ve çekirdeği yeniden başlatır (arayüz bekçisi
// SelfHeal.WatchUi gibi; o yalnızca arayüzü ve 30 sn'de yakalıyordu). Bar ve pencere yöneticisi yerinde kalır.
static class HangWatch
{
    sealed class Beat { public string Name; public Thread Thread; public int Tick; }
    static readonly List<Beat> beats = new List<Beat>();
    internal static int StuckMs = 8000; // testler kısaltır
    // Yenileme sonrası yine takılırsa döngüye girmesin: on dakikada bir
    const int AGAIN_MS = 600000;
    static int lastRecovery = Environment.TickCount - AGAIN_MS;
    // Raporların klasörü (testler geçici bir klasör verir)
    internal static Func<string> ReportDir = () => Paths.LogsDir;
    // Çekirdeği yeniden başlatır ve bu süreci kapatır. Yeniden başlatma log kilidinde takılırsa (takılmanın parçası
    // olabilir) 3 sn sonra masaüstünü yenileme yoluna geçer. Testler değiştirir.
    internal static Action<string> Recover = why =>
    {
        var respawn = new Thread(() => SelfHeal.Respawn(why)) { IsBackground = true };
        respawn.Start();
        if (!respawn.Join(3000)) Supervisor.RestartDesktopDetached();
        Thread.Sleep(1000);
        Process.GetCurrentProcess().Kill();
    };

    // İzlenen iş parçacığı kendi döngüsünde (1 sn'lik zamanlayıcı) dönen nabzı çağırır
    public static Action Register(string name)
    {
        var b = new Beat { Name = name, Thread = Thread.CurrentThread, Tick = Environment.TickCount };
        lock (beats) beats.Add(b);
        return () => Volatile.Write(ref b.Tick, Environment.TickCount);
    }

    public static void Start()
    {
        new Thread(Loop) { IsBackground = true, Name = "kilitlenme bekçisi", Priority = ThreadPriority.AboveNormal }.Start();
    }

    // Takılanlar: nabzı sınırdan eski olanlar
    internal static List<string> Stuck(IEnumerable<KeyValuePair<string, int>> ticks, int now, int limitMs)
    {
        var stuck = new List<string>();
        foreach (var t in ticks) if (unchecked(now - t.Value) > limitMs) stuck.Add(t.Key);
        return stuck;
    }

    // Bekçinin kendi turu bu kadar geç geldiyse bilgisayar uyudu ya da durdu: nabızlar o tur sayılmaz (uyanınca
    // hepsi eski görünür, masaüstü boşuna yenilenirdi)
    internal static bool Slept(int loopGapMs) { return loopGapMs > 3000; }

    static void Loop()
    {
        int last = Environment.TickCount;
        while (true)
        {
            Thread.Sleep(1000);
            int now = Environment.TickCount;
            bool slept = Slept(unchecked(now - last));
            last = now;
            Beat[] all;
            lock (beats) all = beats.ToArray();
            if (slept)
            {
                foreach (var b in all) Volatile.Write(ref b.Tick, now);
                continue;
            }
            var ticks = new List<KeyValuePair<string, int>>();
            foreach (var b in all) ticks.Add(new KeyValuePair<string, int>(b.Name, Volatile.Read(ref b.Tick)));
            var stuck = Stuck(ticks, now, StuckMs);
            if (stuck.Count == 0 || unchecked(now - lastRecovery) < AGAIN_MS) continue;
            lastRecovery = now;
            try { Handle(all, stuck, now); } catch { }
        }
    }

    static void Handle(Beat[] all, List<string> stuck, int now)
    {
        var sb = new StringBuilder();
        string why = string.Join(", ", stuck.ToArray()) + " " + StuckMs / 1000 + " sn'dir nabız vermiyor";
        sb.Append(DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff")).Append(" çekirdek takıldı: ").Append(why).AppendLine();
        foreach (var b in all)
        {
            if (!stuck.Contains(b.Name)) continue;
            sb.Append("--- ").Append(b.Name).Append(" (").Append(unchecked(now - Volatile.Read(ref b.Tick)) / 1000.0).Append(" sn)").AppendLine();
            sb.Append(CaptureStack(b.Thread, 2000)).AppendLine();
        }
        string report = sb.ToString();
        // Doğrudan dosyaya: log yazıcısının kilidi takılmanın parçası olabilir
        string path = null;
        try
        {
            path = Path.Combine(ReportDir(), "hang-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".txt");
            File.WriteAllText(path, report, new UTF8Encoding(false));
        }
        catch { }
        ThreadPool.QueueUserWorkItem(_ => { try { Slider.Log("KİLİTLENME: " + why + "; yığınlar " + (path ?? "?")); } catch { } });
        Recover(why + " (yığınlar: " + (path ?? "?") + ")");
    }

    // Takılmış iş parçacığının yönetilen çağrı yığını. Yardımcı bir iş parçacığında, süre sınırıyla: askıya alma ya da
    // yığın yürüyüşü takılırsa bekçi beklemez, yenileme yine yapılır.
    internal static string CaptureStack(Thread t, int timeoutMs)
    {
        string result = "(yığın alınamadı: süre doldu)";
        var helper = new Thread(() =>
        {
            bool suspended = false;
            try
            {
#pragma warning disable 618 // Suspend/StackTrace(Thread): yalnızca takılmış bir iş parçacığının yığınını okumak için
                t.Suspend(); suspended = true;
                result = new StackTrace(t, false).ToString();
            }
            catch (Exception ex) { result = "(yığın alınamadı: " + ex.GetBaseException().Message + ")"; }
            finally { if (suspended) try { t.Resume(); } catch { } }
#pragma warning restore 618
        }) { IsBackground = true };
        helper.Start();
        helper.Join(timeoutMs);
        return result;
    }
}
