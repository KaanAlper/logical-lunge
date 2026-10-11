using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

// Pencere yöneticisinin IPC portunu (6123) kim dinliyor: GetExtendedTcpTable'ın dinleyen soketleri ve sahip süreçleri.
// Eskiden "port boş mu" sorusu portu SO_EXCLUSIVEADDRUSE ile bağlamayı deneyerek soruluyordu; bu, kapanan pencere
// yöneticisinin TIME_WAIT'teki eski bağlantıları bitene kadar (dakikalarca) "dolu" diyordu, kimin tuttuğunu da
// söylemiyordu. Artık yalnızca dinleyen bir soket portu tutar: bizim kurulum klasörümüzdeki eski bir lunge-tiling ise
// kapatılır, başka bir uygulamaysa (portu devralmış) adı söylenir; yabancı bir sürece hiç dokunulmaz.
static class PortOwner
{
    [DllImport("iphlpapi.dll")]
    static extern uint GetExtendedTcpTable(IntPtr table, ref int size, bool order, int af, int tableClass, uint reserved);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool QueryFullProcessImageName(IntPtr process, int flags, StringBuilder name, ref int size);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);

    const int AF_INET = 2, TCP_TABLE_OWNER_PID_LISTENER = 3, ROW_BYTES = 24;
    const uint ERROR_INSUFFICIENT_BUFFER = 122, PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;

    public enum Holder { Free, Starting, OwnStale, Foreign }

    // Portu dinleyen sürecin kimliği; 0: kimse dinlemiyor, -1: tablo okunamadı
    public static int ListenerPid(int port)
    {
        int size = 0;
        GetExtendedTcpTable(IntPtr.Zero, ref size, false, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
        for (int attempt = 0; attempt < 3 && size > 0; attempt++)
        {
            IntPtr buf = Marshal.AllocHGlobal(size);
            try
            {
                uint r = GetExtendedTcpTable(buf, ref size, false, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
                if (r == ERROR_INSUFFICIENT_BUFFER) continue; // tablo bu arada büyüdü; size yeni boyut
                if (r != 0) return -1;
                var bytes = new byte[size];
                Marshal.Copy(buf, bytes, 0, size);
                return FindListener(bytes, port);
            }
            finally { Marshal.FreeHGlobal(buf); }
        }
        return -1;
    }

    // MIB_TCPTABLE_OWNER_PID: satır sayısı, ardından 24 baytlık satırlar (durum, yerel adres, yerel port, uzak adres,
    // uzak port, pid). Port ağ bayt sırasında, DWORD'ün alt iki baytında.
    internal static int FindListener(byte[] table, int port)
    {
        if (table == null || table.Length < 4) return 0;
        int count = BitConverter.ToInt32(table, 0);
        for (int i = 0; i < count; i++)
        {
            int at = 4 + i * ROW_BYTES;
            if (at + ROW_BYTES > table.Length) break;
            int rowPort = (table[at + 8] << 8) | table[at + 9];
            if (rowPort == port) return BitConverter.ToInt32(table, at + 20);
        }
        return 0;
    }

    // Tutan kim: bizim kurulumumuzun lunge-tiling'i mi (yeni açılıyorsa dokunulmaz), başka bir uygulama mı
    internal static Holder Classify(int pid, string imagePath, string ownTiling, double ageSeconds)
    {
        if (pid <= 0) return Holder.Free;
        bool own = !string.IsNullOrEmpty(imagePath) && !string.IsNullOrEmpty(ownTiling)
            && string.Equals(System.IO.Path.GetFullPath(imagePath), System.IO.Path.GetFullPath(ownTiling), StringComparison.OrdinalIgnoreCase);
        if (!own) return Holder.Foreign;
        // yeni başlatılan pencere yöneticisi portu az önce aldı: nöbetçi onu birazdan bulur
        return ageSeconds < 15 ? Holder.Starting : Holder.OwnStale;
    }

    public static string ImagePath(int pid)
    {
        IntPtr h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if (h == IntPtr.Zero) return null;
        try
        {
            var sb = new StringBuilder(1024);
            int len = sb.Capacity;
            return QueryFullProcessImageName(h, 0, sb, ref len) ? sb.ToString(0, len) : null;
        }
        finally { CloseHandle(h); }
    }

    public static double AgeSeconds(int pid)
    {
        try { using (var p = Process.GetProcessById(pid)) return (DateTime.Now - p.StartTime).TotalSeconds; }
        catch { return double.MaxValue; }
    }

    public static string Name(int pid, string imagePath)
    {
        if (!string.IsNullOrEmpty(imagePath)) return System.IO.Path.GetFileName(imagePath);
        try { using (var p = Process.GetProcessById(pid)) return p.ProcessName + ".exe"; }
        catch { return "pid " + pid; }
    }
}
