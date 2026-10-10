using System;
using System.Collections.Generic;

// Super+ok odağı, Hyprland'in yönlü pencere seçimi (getWindowInDirection) gibi:
//  - döşeli pencereden: yalnızca o yöndeki kenarı bitişik (aradaki boşluk kadar tolerans) ve dik eksende kesişen döşeli
//    pencereler aday; aralarından en son odaklanan seçilir (eşitlikte kesişimi uzun olan);
//  - yüzen pencereden: yalnızca yüzen pencereler, yöne en küçük açıyla bakan (eşitlikte yakın olan);
//  - o tür içinde aday yoksa öbür tür aynı kurallarla denenir; hiç yoksa null (çağıran yandaki monitöre geçer).
// Saf mantık: koordinatlar ekran pikseli, Rank küçük olan daha yakın zamanda odaklanmış.
static class FocusDirection
{
    public struct Win
    {
        public string Id;
        public int X, Y, W, H;
        public bool Floating;
        public int Rank;
    }

    // Döşeli komşuların bitişik sayılacağı en büyük kenar aralığı (pencere boşlukları + kenarlıklar)
    public const int Sticks = 40;

    public static string Pick(IList<Win> wins, Win cur, string dir)
    {
        var first = cur.Floating ? PickFloating(wins, cur, dir) : PickTiled(wins, cur, dir);
        if (first != null) return first;
        return cur.Floating ? PickTiled(wins, cur, dir) : PickFloating(wins, cur, dir);
    }

    static string PickTiled(IList<Win> wins, Win cur, string dir)
    {
        string best = null;
        int bestRank = int.MaxValue, bestOverlap = -1;
        foreach (var w in wins)
        {
            if (w.Id == cur.Id || w.Floating) continue;
            int gap, overlap;
            switch (dir)
            {
                case "left": gap = cur.X - (w.X + w.W); overlap = Overlap(cur.Y, cur.H, w.Y, w.H); break;
                case "right": gap = w.X - (cur.X + cur.W); overlap = Overlap(cur.Y, cur.H, w.Y, w.H); break;
                case "up": gap = cur.Y - (w.Y + w.H); overlap = Overlap(cur.X, cur.W, w.X, w.W); break;
                default: gap = w.Y - (cur.Y + cur.H); overlap = Overlap(cur.X, cur.W, w.X, w.W); break;
            }
            if (gap < -Sticks || gap > Sticks || overlap <= 0) continue;
            if (w.Rank < bestRank || (w.Rank == bestRank && overlap > bestOverlap))
            {
                best = w.Id; bestRank = w.Rank; bestOverlap = overlap;
            }
        }
        return best;
    }

    static string PickFloating(IList<Win> wins, Win cur, string dir)
    {
        double dx0 = dir == "left" ? -1 : dir == "right" ? 1 : 0, dy0 = dir == "up" ? -1 : dir == "down" ? 1 : 0;
        double cx = cur.X + cur.W / 2.0, cy = cur.Y + cur.H / 2.0;
        string best = null;
        double bestAngle = double.MaxValue, bestDist = double.MaxValue;
        foreach (var w in wins)
        {
            if (w.Id == cur.Id || !w.Floating) continue;
            double vx = w.X + w.W / 2.0 - cx, vy = w.Y + w.H / 2.0 - cy;
            double dist = Math.Sqrt(vx * vx + vy * vy);
            if (dist < 1) continue;
            double dot = (vx * dx0 + vy * dy0) / dist;
            if (dot <= 0) continue; // arkada ya da tam yanda
            double angle = Math.Acos(Math.Min(1, dot));
            if (angle < bestAngle - 1e-9 || (Math.Abs(angle - bestAngle) <= 1e-9 && dist < bestDist))
            {
                best = w.Id; bestAngle = angle; bestDist = dist;
            }
        }
        return best;
    }

    static int Overlap(int a, int al, int b, int bl)
    {
        return Math.Min(a + al, b + bl) - Math.Max(a, b);
    }

    // Monitörler arasında o yöndeki komşu: kenarı bitişik ve dik eksende kesişen, en uzun kesişimli monitör
    public static int PickMonitor(IList<Win> monitors, Win cur, string dir)
    {
        int best = -1, bestOverlap = 0;
        for (int i = 0; i < monitors.Count; i++)
        {
            var m = monitors[i];
            if (m.Id == cur.Id) continue;
            int gap, overlap;
            switch (dir)
            {
                case "left": gap = cur.X - (m.X + m.W); overlap = Overlap(cur.Y, cur.H, m.Y, m.H); break;
                case "right": gap = m.X - (cur.X + cur.W); overlap = Overlap(cur.Y, cur.H, m.Y, m.H); break;
                case "up": gap = cur.Y - (m.Y + m.H); overlap = Overlap(cur.X, cur.W, m.X, m.W); break;
                default: gap = m.Y - (cur.Y + cur.H); overlap = Overlap(cur.X, cur.W, m.X, m.W); break;
            }
            if (gap < -2 || gap > 2 || overlap <= 0) continue;
            if (overlap > bestOverlap) { best = i; bestOverlap = overlap; }
        }
        return best;
    }
}
