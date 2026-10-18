#!/usr/bin/env python3
"""Renders the README's benchmark charts (SVG) from a comparison's comparison.json.

    python3 bench/aws/charts.py <result-dir>/compare/comparison.json docs/images

Writes bench-overhead.svg (H1: overhead p50/p99 at 5,000 RPS, log scale), bench-resources.svg
(H1: CPU per request and peak memory) and bench-throughput.svg (C32.c64: requests per second at 64
connections). Text is mid-grey so the charts read on light and dark pages.
"""
import json
import math
import sys
from pathlib import Path

ORDER = ["antirion", "bifrost", "litellm"]
LABEL = {"antirion": "Antirion", "bifrost": "Bifrost", "litellm": "LiteLLM"}
COLOR = {"antirion": "#22c3d6", "bifrost": "#8b7cf6", "litellm": "#e3a33b"}
TEXT = "#8b949e"
GRID = "#8b949e33"
FONT = "font-family=\"-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif\""


def cell(data, name):
    for c in data["cells"]:
        if c["cell"] == name:
            return c["targets"]
    sys.exit(f"no cell {name}")


def version(data, target):
    for t in data["targets"]:
        if t["name"] == target:
            return t["version"]
    return ""


def svg(width, height, body, title):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
            f'viewBox="0 0 {width} {height}" role="img" aria-label="{title}" {FONT}>\n'
            f'<title>{title}</title>\n{body}</svg>\n')


def text(x, y, s, size=13, anchor="start", weight="400", fill=TEXT):
    return (f'<text x="{x}" y="{y}" font-size="{size}" text-anchor="{anchor}" '
            f'font-weight="{weight}" fill="{fill}">{s}</text>\n')


def ms(us):
    v = us / 1000
    return f"{v:.2f} ms" if v < 10 else f"{v:.0f} ms" if v >= 100 else f"{v:.1f} ms"


def overhead_chart(data):
    h1 = cell(data, "H1")
    w, left, right, top = 760, 150, 40, 70
    lo, hi = 0.1, 300.0  # ms, log scale
    span = w - left - right
    xs = lambda v: left + span * (math.log10(max(v, lo)) - math.log10(lo)) / (math.log10(hi) - math.log10(lo))
    body = text(20, 30, "Gateway overhead at 5,000 requests/s (lower is better)", 16, weight="600")
    body += text(20, 50, "Simple chat, open loop, one c7i.xlarge gateway. Overhead over a direct call to the same mock.", 12)
    row, bar = 46, 14
    rows = ORDER
    height = top + row * len(rows) + 50
    for tick in (0.1, 1, 10, 100):
        x = xs(tick)
        body += f'<line x1="{x:.1f}" y1="{top - 8}" x2="{x:.1f}" y2="{top + row * len(rows)}" stroke="{GRID}"/>\n'
        body += text(x, top + row * len(rows) + 18, f"{tick:g} ms", 11, "middle")
    for i, t in enumerate(rows):
        y = top + i * row
        r = h1[t]
        body += text(left - 12, y + 20, LABEL[t], 13, "end", "600")
        body += text(left - 12, y + 35, version(data, t).replace("transports/", ""), 10, "end")
        if r["p50_us"] is None:
            body += text(left + 6, y + 24, f"Failed: {r['error_rate'] * 100:.0f} % errors at this load", 12, fill=COLOR[t])
            continue
        for j, (key, op) in enumerate((("p50_us", "1"), ("p99_us", "0.45"))):
            v = r[key] / 1000
            x2 = xs(v)
            yy = y + 6 + j * (bar + 3)
            body += (f'<rect x="{left}" y="{yy}" width="{max(x2 - left, 2):.1f}" height="{bar}" rx="3" '
                     f'fill="{COLOR[t]}" fill-opacity="{op}"/>\n')
            body += text(x2 + 6, yy + 11, f"{'p50' if j == 0 else 'p99'} {ms(r[key])}", 11)
    body += text(20, height - 8, "Log scale. Medians of 5 repetitions; full results and method in docs/benchmarks.md.", 11)
    return svg(w, height, body, "Gateway overhead at 5,000 requests per second")


def resources_chart(data):
    h1 = cell(data, "H1")
    w, top = 760, 92
    panel_w, gap, left = 330, 40, 20
    body = text(20, 30, "Resources at 5,000 requests/s (lower is better)", 16, weight="600")
    body += text(20, 50, "Same run as above: CPU time per completed request and peak resident memory of the gateway.", 12)
    panels = [("CPU per request", "cpu_us_per_request", lambda v: f"{v:.0f} µs"),
              ("Peak memory", "rss_peak_bytes", lambda v: f"{v / 2**20:,.0f} MiB")]
    row, bar = 34, 18
    height = top + row * len(ORDER) + 40
    for p, (title, key, fmt) in enumerate(panels):
        x0 = left + p * (panel_w + gap)
        vals = [h1[t][key] for t in ORDER if h1[t][key] is not None]
        peak = max(vals) if vals else 1
        body += text(x0, top - 10, title, 13, weight="600")
        for i, t in enumerate(ORDER):
            y = top + i * row
            v = h1[t][key]
            body += text(x0, y + 14, LABEL[t], 12)
            bx = x0 + 70
            if v is None:
                body += text(bx, y + 14, "n/a (no request completed)", 11)
                continue
            bw = (panel_w - 150) * v / peak
            body += f'<rect x="{bx}" y="{y + 1}" width="{max(bw, 2):.1f}" height="{bar}" rx="3" fill="{COLOR[t]}"/>\n'
            body += text(bx + bw + 6, y + 14, fmt(v), 11)
    body += text(20, height - 8, "Idle memory: " + ", ".join(
        f"{LABEL[t]} {h1[t]['rss_idle_bytes'] / 2**20:,.0f} MiB" for t in ORDER), 11)
    return svg(w, height, body, "CPU per request and peak memory at 5,000 requests per second")


def throughput_chart(data):
    c = cell(data, "cmp-C32.c64")
    w, left, top = 760, 150, 70
    span = w - left - 150
    body = text(20, 30, "Throughput at 64 concurrent connections (higher is better)", 16, weight="600")
    body += text(20, 50, "Simple chat, closed loop: each connection sends its next request when the last one answers.", 12)
    row, bar = 34, 18
    peak = max(c[t]["achieved_rps"] or 0 for t in ORDER) or 1
    height = top + row * len(ORDER) + 30
    for i, t in enumerate(ORDER):
        y = top + i * row
        v = c[t]["achieved_rps"] or 0
        body += text(left - 12, y + 14, LABEL[t], 13, "end", "600")
        bw = span * v / peak
        body += f'<rect x="{left}" y="{y + 1}" width="{max(bw, 2):.1f}" height="{bar}" rx="3" fill="{COLOR[t]}"/>\n'
        body += text(left + bw + 6, y + 14, f"{v:,.0f} requests/s", 11)
    body += text(20, height - 8, "One c7i.xlarge gateway (4 vCPUs). Medians of 3 repetitions.", 11)
    return svg(w, height, body, "Throughput at 64 concurrent connections")


def main():
    data = json.loads(Path(sys.argv[1]).read_text())
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    (out / "bench-overhead.svg").write_text(overhead_chart(data))
    (out / "bench-resources.svg").write_text(resources_chart(data))
    (out / "bench-throughput.svg").write_text(throughput_chart(data))
    print("wrote", ", ".join(str(out / n) for n in ("bench-overhead.svg", "bench-resources.svg", "bench-throughput.svg")))


if __name__ == "__main__":
    main()
