#!/usr/bin/env bash
# Shared helpers for the README demo recordings (sourced, not executed).

# asciinema 2.x has no --cols/--rows: COLUMNS/LINES size the child pty, but the
# cast header still records the (absent) controlling tty's 80x24 — patch it.
patch_cast_size() { # patch_cast_size <cast> <cols> <rows>
    python3 - "$@" <<'EOF'
import json, sys
path, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
with open(path) as f:
    header, rest = f.readline(), f.read()
h = json.loads(header)
h["width"], h["height"] = cols, rows
with open(path, "w") as f:
    f.write(json.dumps(h) + "\n" + rest)
EOF
}

cast_to_gif() { # cast_to_gif <cast> <gif> [agg args...]
    local cast="$1" gif="$2"
    shift 2
    mkdir -p "$(dirname "$gif")"
    # agg may not read asciicast v3 (asciinema >= 3.0); fall back to a v2 convert.
    if ! agg --theme monokai --idle-time-limit 2 "$@" "$cast" "$gif"; then
        local v2="${cast%.cast}-v2.cast"
        asciinema convert -f asciicast-v2 "$cast" "$v2"
        agg --theme monokai --idle-time-limit 2 "$@" "$v2" "$gif"
    fi
    ls -lh "$gif" >&2
}

# Whisper downloads its model on first use; a progress bar in the recording
# looks like a dependency the reader does not have. Warm the cache first.
prewarm_model() { # prewarm_model <harken> <scratch-dir>
    local harken="$1" scratch="$2"
    ffmpeg -loglevel error -y -f lavfi -i anullsrc=r=16000:cl=mono -t 1 "$scratch/warm.wav"
    "$harken" "$scratch/warm.wav" --model small --out "$scratch/warm-out" >/dev/null
    rm -rf "$scratch/warm-out" "$scratch/warm.wav"
}
