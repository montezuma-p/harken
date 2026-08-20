#!/usr/bin/env bash
# Record the README demo GIF, non-interactively.
#
# Needs: asciinema, agg, ffmpeg, espeak-ng (or piper + PIPER_VOICE),
# and a release build (`cargo build --release`).
#
# Usage: scripts/demo/record-demo.sh [demo.zip]   (writes docs/assets/demo.gif)
#
# Pass a pre-validated zip: piper's TTS sampling is stochastic, so regenerate
# with make-demo-zip.sh + spot-check the transcripts, then record with the
# good take. With no argument a fresh (unchecked) zip is generated.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HARKEN="$REPO_ROOT/target/release/harken"
[[ -x "$HARKEN" ]] || { echo "error: build first: cargo build --release" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

if [[ -n "${1:-}" ]]; then
    cp "$1" "$STAGE/demo-export.zip"
else
    bash "$REPO_ROOT/scripts/demo/make-demo-zip.sh" "$STAGE/demo-export.zip"
fi

# Pre-warm the small model so no download bar shows up in the recording.
ffmpeg -loglevel error -y -f lavfi -i anullsrc=r=16000:cl=mono -t 1 "$STAGE/warm.wav"
"$HARKEN" "$STAGE/warm.wav" --model small --out "$STAGE/warm-out" >/dev/null
rm -rf "$STAGE/warm-out" "$STAGE/warm.wav"

CAST="$STAGE/demo.cast"
GIF="$REPO_ROOT/docs/assets/demo.gif"
mkdir -p "$(dirname "$GIF")"

# asciinema 2.x has no --cols/--rows: COLUMNS/LINES size the child pty, but the
# cast header still records the (absent) controlling tty's 80x24 — patch it.
(
    cd "$STAGE"
    PATH="$REPO_ROOT/target/release:$PATH" COLUMNS=100 LINES=20 \
        asciinema rec "$CAST" --overwrite --idle-time-limit 2 \
        -c "bash '$REPO_ROOT/scripts/demo/demo-session.sh'"
)
python3 - "$CAST" <<'EOF'
import json, sys
path = sys.argv[1]
with open(path) as f:
    header, rest = f.readline(), f.read()
h = json.loads(header)
h["width"], h["height"] = 100, 20
with open(path, "w") as f:
    f.write(json.dumps(h) + "\n" + rest)
EOF

# agg may not read asciicast v3 (asciinema >= 3.0); fall back to a v2 convert.
if ! agg --theme monokai --font-size 16 --idle-time-limit 2 "$CAST" "$GIF"; then
    asciinema convert -f asciicast-v2 "$CAST" "$STAGE/demo-v2.cast"
    agg --theme monokai --font-size 16 --idle-time-limit 2 "$STAGE/demo-v2.cast" "$GIF"
fi

ls -lh "$GIF" >&2
