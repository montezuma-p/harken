#!/usr/bin/env bash
# Benchmark harken: model × wall time × peak RAM for one audio file.
# Prints a ready-to-paste markdown table (stdout); progress goes to stderr.
#
# Usage:
#   scripts/bench/bench.sh <audio-file>     # your own ~10-min recording
#   scripts/bench/bench.sh --synth          # generate a 10-min PT-BR TTS wav
#
# Env: MODELS="tiny small ..." (default below), REPEATS=3
#
# Methodology: model downloads are excluded (cache pre-warmed with a 1 s run),
# median of $REPEATS runs, measured with GNU time -v (wall clock + max RSS).
set -euo pipefail

# GNU time / printf parse floats with a dot regardless of the user's locale
export LC_NUMERIC=C

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HARKEN="$REPO_ROOT/target/release/harken"
[[ -x "$HARKEN" ]] || { echo "error: build first: cargo build --release" >&2; exit 1; }

MODELS="${MODELS:-tiny small small-q5_1 medium}"
REPEATS="${REPEATS:-3}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [[ "${1:-}" == "--synth" ]]; then
    AUDIO="$TMP/synth-10min.wav"
    echo "generating ~10 min of PT-BR TTS audio ..." >&2
    TEXT="O harken transcreve áudio localmente, sem nuvem e sem chave de API. \
Esta gravação sintética existe só para medir o tempo de transcrição por modelo. \
O conteúdo não importa: o custo de computação depende da duração do áudio."
    espeak-ng -v pt-br -s 150 -w "$TMP/base.wav" "$TEXT"
    BASE_DUR=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$TMP/base.wav")
    LOOPS=$(python3 -c "import math; print(math.ceil(600 / $BASE_DUR) - 1)")
    ffmpeg -loglevel error -y -stream_loop "$LOOPS" -i "$TMP/base.wav" -t 600 "$AUDIO"
else
    AUDIO="${1:?usage: bench.sh <audio-file> | --synth}"
    [[ -f "$AUDIO" ]] || { echo "error: no such file: $AUDIO" >&2; exit 2; }
fi

AUDIO_DUR=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$AUDIO")

# GNU time prints elapsed as [h:]m:ss(.ss) — normalize to seconds.
to_secs() {
    python3 -c "
parts = '$1'.split(':')
print(sum(float(p) * 60 ** i for i, p in enumerate(reversed(parts))))"
}

echo "## Environment"
echo
echo "- CPU: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | xargs) ($(nproc) threads)"
echo "- harken: $("$HARKEN" --version)"
echo "- audio: $(basename "$AUDIO") ($(printf '%.0f' "$AUDIO_DUR") s), lang pt, ${REPEATS} runs (median), GNU time -v"
echo
echo "| Model | Wall time | Speed | Peak RAM |"
echo "|---|---|---|---|"

for m in $MODELS; do
    # Pre-warm: trigger the model download outside the timed runs.
    if ! ls "${XDG_CACHE_HOME:-$HOME/.cache}/harken/models/ggml-$m.bin" >/dev/null 2>&1; then
        echo "[$m] downloading model ..." >&2
        ffmpeg -loglevel error -y -f lavfi -i anullsrc=r=16000:cl=mono -t 1 "$TMP/warm.wav"
        "$HARKEN" "$TMP/warm.wav" --model "$m" --out "$TMP/warm-$m" >/dev/null
    fi

    walls=() rams=()
    for r in $(seq "$REPEATS"); do
        echo "[$m] run $r/$REPEATS ..." >&2
        /usr/bin/time -v "$HARKEN" "$AUDIO" --model "$m" --lang pt \
            --out "$TMP/out-$m" --force 2> "$TMP/$m-$r.time"
        wall=$(grep 'Elapsed (wall clock)' "$TMP/$m-$r.time" | awk '{print $NF}')
        walls+=("$(to_secs "$wall")")
        rams+=("$(grep 'Maximum resident set size' "$TMP/$m-$r.time" | awk '{print $NF}')")
        cpu_pct=$(grep 'Percent of CPU' "$TMP/$m-$r.time" | grep -o '[0-9]*%')
        echo "[$m] run $r: ${wall} wall, ${cpu_pct} CPU" >&2
    done

    read -r wall_med ram_med < <(python3 -c "
import statistics
walls = sorted(map(float, '${walls[*]}'.split()))
rams = sorted(map(int, '${rams[*]}'.split()))
print(statistics.median(walls), statistics.median(rams))")

    speed=$(python3 -c "print(f'{$AUDIO_DUR / $wall_med:.1f}x realtime')")
    wall_fmt=$(python3 -c "s=$wall_med; print(f'{int(s)//60} min {int(s)%60:02d} s' if s >= 60 else f'{s:.0f} s')")
    ram_fmt=$(python3 -c "print(f'{$ram_med / 1048576:.1f} GB' if $ram_med >= 1048576 else f'{$ram_med / 1024:.0f} MB')")
    echo "| \`$m\` | $wall_fmt | $speed | $ram_fmt |"
done
