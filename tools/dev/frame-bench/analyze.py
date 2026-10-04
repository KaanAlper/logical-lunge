"""Compares the bench's pacing methods by what the screen showed.

For every refresh during a run: smooth (the marker moved one frame's worth), repeat (it did not move: the update missed
the frame), skip (it moved two frames' worth: two updates landed in one frame), gap (no frame presented for a refresh).
Latency: from the update's submission to the present that showed it.
    python analyze.py <bench.csv> <capture.csv>
"""
import csv
import re
import statistics
import sys


def read(path):
    with open(path, encoding="utf-8") as f:
        head = f.readline()
        rows = list(csv.DictReader(f))
    return head, rows


def main(bench_path, capture_path):
    head, updates = read(bench_path)
    freq = int(re.search(r"qpc frequency (\d+)", head).group(1))
    travel, dur = (int(v) for v in re.search(r"travel (\d+) px in (\d+) ms", head).groups())
    _, captured = read(capture_path)
    frames = sorted((int(r["qpc"]), int(r["x"])) for r in captured)
    deltas = [(b[0] - a[0]) / freq * 1000 for a, b in zip(frames, frames[1:])]
    period = statistics.median(d for d in deltas if 0 < d < 30)
    step = travel / dur * period

    runs = {}
    for u in updates:
        runs.setdefault(int(u["run"]), []).append((u["method"], int(u["qpc"]), int(u["x"])))
    totals = {}
    for run, ups in sorted(runs.items()):
        method = ups[0][0]
        start, end = ups[0][1], ups[-1][1] + freq // 20
        seq = []
        for q, x in frames:
            if start <= q <= end and x >= 0 and (x > 0 or seq):
                seq.append((q, x))
                if x >= travel:
                    break
        counts = {"smooth": 0, "repeat": 0, "skip": 0, "gap": 0}
        for (qa, xa), (qb, xb) in zip(seq, seq[1:]):
            refreshes = round((qb - qa) / freq * 1000 / period)
            moved = xb - xa
            kind = "gap" if refreshes >= 2 else "repeat" if moved == 0 else "skip" if moved > 1.5 * step else "smooth"
            counts[kind] += 1
        submitted = {}
        for _, q, x in ups:
            submitted.setdefault(x, q)
        latency = [(q - submitted[x]) / freq * 1000 for q, x in seq if x in submitted]
        t = totals.setdefault(method, {"smooth": 0, "repeat": 0, "skip": 0, "gap": 0, "latency": []})
        for k, v in counts.items():
            t[k] += v
        t["latency"] += latency
        n = max(1, sum(counts.values()))
        print(f"run {run:2} {method:5}: smooth {counts['smooth'] / n * 100:5.1f}%  repeat {counts['repeat']:2}  skip {counts['skip']:2}  "
              f"gap {counts['gap']:2}  latency {statistics.median(latency) if latency else float('nan'):4.1f} ms")
    print(f"refresh {period:.2f} ms, {step:.1f} px per frame")
    for method, t in totals.items():
        n = max(1, t["smooth"] + t["repeat"] + t["skip"] + t["gap"])
        lat = sorted(t["latency"]) or [float("nan")]
        print(f"{method:5}: smooth {t['smooth'] / n * 100:5.1f}%  repeat {t['repeat'] / n * 100:4.1f}%  skip {t['skip'] / n * 100:4.1f}%  "
              f"gap {t['gap'] / n * 100:4.1f}%  latency median {statistics.median(lat):.1f} ms, p95 {lat[int(len(lat) * 0.95)]:.1f} ms")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
