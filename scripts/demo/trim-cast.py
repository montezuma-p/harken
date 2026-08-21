#!/usr/bin/env python3
"""Trim a recorded claude-TUI cast down to a README-sized loop.

The TUI repaints the whole screen on every tick, so a two-minute session is
~700 full-screen frames of mostly spinner. Keep the prompt and the final answer
intact, thin the churn in between, cap idle gaps, and hold the last frame.

Usage: trim-cast.py <in.cast> <out.cast> <start-marker> <cols> <rows> [stop-marker]
"""
import json, re, sys

src, dst, start_mark, cols, rows = sys.argv[1:6]
stop_mark = sys.argv[6] if len(sys.argv) > 6 else "Exit the CLI"
cols, rows = int(cols), int(rows)
END_RE = re.compile(r"for (?:\d+m )?\d+s")  # the TUI's "Sautéed for 1m 17s" footer
MAX_GAP = 0.1
KEEP_EVERY = 3
HEAD, TAIL = 5, 15
TAIL_HOLD = 2.5  # hold the answer before the loop restarts

ANSI = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][B0]")


def text(ev):
    """Event payload with escape sequences stripped: markers span color runs."""
    return ANSI.sub("", ev[2])


with open(src) as f:
    header = json.loads(f.readline())
    events = [json.loads(line) for line in f if line.strip()]

start = max(0, next(i for i, e in enumerate(events) if start_mark in text(e)) - 2)
# stop before the session teardown (the /exit autocomplete menu), then search
# backwards: the footer shape ticks during the run, only the last is the answer
limit = next((i for i, e in enumerate(events) if i > start and stop_mark in text(e)), len(events))
end = next(i for i in range(limit - 1, start, -1) if END_RE.search(text(events[i])))
sel = events[start:end + 1]
if len(sel) < 50:
    sys.exit(f"trim: only {len(sel)} events between markers — check the markers")

kept = [e for i, e in enumerate(sel)
        if i < HEAD or i >= len(sel) - TAIL or i % KEEP_EVERY == 0]

out, t, prev = [], 0.0, kept[0][0]
for ev in kept:
    t += min(max(ev[0] - prev, 0.0), MAX_GAP)
    prev = ev[0]
    out.append([round(t, 3), ev[1], ev[2]])

# full repaint at the trim point, so nothing from the dropped prefix lingers
out.insert(0, [0.0, "o", "\x1b[2J\x1b[H"])
# hold the answer on screen before the loop restarts
out.append([round(t + TAIL_HOLD, 3), "o", out[-1][2]])
header.update(width=cols, height=rows, idle_time_limit=1.0)

with open(dst, "w") as f:
    f.write(json.dumps(header) + "\n")
    for ev in out:
        f.write(json.dumps(ev) + "\n")

print(f"trimmed {len(events)} -> {len(out)} events, {t:.1f}s", file=sys.stderr)
