using System;
using System.Collections.Generic;
using System.Text;
using System.Text.RegularExpressions;

// Edits only the top-level workspaces block. Other YAML sections and
// per-workspace options such as display_name and keep_alive are preserved.
internal static class WorkspaceConfigText
{
    internal sealed class Entry
    {
        internal int Number;
        internal int? Monitor;
        internal List<string> Lines = new List<string>();
    }

    static readonly Regex Name = new Regex(@"^  - name: ['""']?([1-9][0-9]*)['""']?\s*(?:#.*)?$");
    static readonly Regex Binding = new Regex(@"^    bind_to_monitor:\s*([0-9]+)\s*(?:#.*)?$");
    static readonly Regex Section = new Regex(@"^[A-Za-z_][A-Za-z0-9_-]*:");

    internal static bool TryRead(string yaml, out List<Entry> entries, out string error)
    {
        int start, end;
        string[] lines;
        return Parse(yaml, out lines, out start, out end, out entries, out error);
    }

    internal static bool TryRewrite(string yaml, int? requestedCount, int? requestedFirst,
        Dictionary<int, int> assignments, int monitorCount, out string result, out string error)
    {
        result = null;
        int start, end;
        string[] lines;
        List<Entry> entries;
        if (!Parse(yaml, out lines, out start, out end, out entries, out error)) return false;
        if (monitorCount < 1) { error = "Pencere yöneticisinden monitör bilgisi alınamadı."; return false; }
        int count = requestedCount ?? entries.Count;
        if (count < monitorCount || count > 100) {
            error = "Çalışma alanı sayısı bağlı monitör sayısından az veya 100'den fazla olamaz.";
            return false;
        }
        int first = requestedFirst ?? entries[0].Number;
        if (first < 1 || first > count) {
            error = "İlk çalışma alanı mevcut aralıkta olmalı.";
            return false;
        }
        foreach (var pair in assignments) {
            if (pair.Key < 1 || pair.Key > count || pair.Value < 0 || pair.Value >= monitorCount) {
                error = "Geçersiz çalışma alanı veya monitör numarası.";
                return false;
            }
        }
        if (assignments.Count == count) {
            var used = new HashSet<int>(assignments.Values);
            if (used.Count != monitorCount) {
                error = "Her bağlı monitöre en az bir çalışma alanı atanmalı.";
                return false;
            }
        }
        var old = new Dictionary<int, Entry>();
        foreach (var entry in entries) old[entry.Number] = entry;
        var output = new List<string>();
        for (int i = 0; i < start; i++) output.Add(lines[i]);
        output.Add("workspaces:");
        // Keep comments and blank lines preceding the first item.
        int firstItem = start + 1;
        while (firstItem < end && !Name.IsMatch(lines[firstItem])) output.Add(lines[firstItem++]);
        for (int offset = 0; offset < count; offset++) {
            int i = (first - 1 + offset) % count + 1;
            Entry entry;
            if (!old.TryGetValue(i, out entry)) {
                entry = new Entry { Number = i };
                entry.Lines.Add("  - name: '" + i + "'");
            }
            int monitor;
            bool assigned = assignments.TryGetValue(i, out monitor);
            bool inserted = false;
            for (int line = 0; line < entry.Lines.Count; line++) {
                string value = entry.Lines[line];
                if (Binding.IsMatch(value) && assigned) continue;
                output.Add(value);
                if (line == 0 && assigned) {
                    output.Add("    bind_to_monitor: " + monitor);
                    inserted = true;
                }
            }
            if (assigned && !inserted) { error = "Çalışma alanı kaydı okunamadı."; return false; }
        }
        for (int i = end; i < lines.Length; i++) output.Add(lines[i]);
        string newline = yaml.Contains("\r\n") ? "\r\n" : "\n";
        result = string.Join(newline, output);
        error = null;
        return true;
    }

    static bool Parse(string yaml, out string[] lines, out int start, out int end,
        out List<Entry> entries, out string error)
    {
        lines = (yaml ?? "").Replace("\r\n", "\n").Split('\n');
        start = -1; end = lines.Length;
        entries = new List<Entry>();
        error = null;
        for (int i = 0; i < lines.Length; i++) {
            if (lines[i].Trim() == "workspaces:" && lines[i] == "workspaces:") {
                if (start >= 0) { error = "Birden çok workspaces bölümü var."; return false; }
                start = i;
            } else if (start >= 0 && i > start && Section.IsMatch(lines[i])) {
                end = i;
                break;
            }
        }
        if (start < 0) { error = "config.yaml içinde workspaces bölümü bulunamadı."; return false; }
        Entry current = null;
        var seen = new HashSet<int>();
        for (int i = start + 1; i < end; i++) {
            var name = Name.Match(lines[i]);
            if (name.Success) {
                int number;
                if (!int.TryParse(name.Groups[1].Value, out number) || !seen.Add(number)) {
                    error = "Çalışma alanı adları sayısal ve benzersiz olmalı."; return false;
                }
                current = new Entry { Number = number };
                entries.Add(current);
            } else if (lines[i].StartsWith("  - ")) {
                error = "Desteklenmeyen çalışma alanı kaydı: " + lines[i].Trim(); return false;
            }
            if (current != null) {
                current.Lines.Add(lines[i]);
                var binding = Binding.Match(lines[i]);
                if (binding.Success) {
                    int index;
                    if (int.TryParse(binding.Groups[1].Value, out index)) current.Monitor = index;
                }
            }
        }
        if (entries.Count == 0) { error = "Tanımlı çalışma alanı bulunamadı."; return false; }
        for (int i = 1; i <= entries.Count; i++) {
            if (!seen.Contains(i)) { error = "Çalışma alanları 1'den başlayarak sıralı olmalı."; return false; }
        }
        return true;
    }
}
