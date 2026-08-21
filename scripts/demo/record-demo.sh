#!/usr/bin/env bash
# Record the README's command-line demo GIF, non-interactively.
#
# Needs: asciinema, agg, ffmpeg, espeak-ng (or piper + PIPER_VOICE),
# and a release build (`cargo build --release`).
#
# Usage: scripts/demo/record-demo.sh [demo.zip]  (writes docs/assets/demo-cli.gif)
#
# Pass a pre-validated zip: piper's TTS sampling is stochastic, so regenerate
# with make-demo-zip.sh + spot-check the transcripts, then record with the
# good take. With no argument a fresh (unchecked) zip is generated.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/demo/lib.sh
source "$REPO_ROOT/scripts/demo/lib.sh"

HARKEN="$REPO_ROOT/target/release/harken"
[[ -x "$HARKEN" ]] || { echo "error: build first: cargo build --release" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

if [[ -n "${1:-}" ]]; then
    cp "$1" "$STAGE/demo-export.zip"
else
    bash "$REPO_ROOT/scripts/demo/make-demo-zip.sh" "$STAGE/demo-export.zip"
fi

prewarm_model "$HARKEN" "$STAGE"

CAST="$STAGE/demo.cast"
GIF="$REPO_ROOT/docs/assets/demo-cli.gif"

(
    cd "$STAGE"
    PATH="$REPO_ROOT/target/release:$PATH" COLUMNS=100 LINES=20 \
        asciinema rec "$CAST" --overwrite --idle-time-limit 2 \
        -c "bash '$REPO_ROOT/scripts/demo/demo-session.sh'"
)
patch_cast_size "$CAST" 100 20
cast_to_gif "$CAST" "$GIF" --font-size 16
