#!/usr/bin/env bash
# Record the README's main demo GIF: Claude Code reading a coworker's WhatsApp
# voice notes through harken, and answering with the task.
#
# The claude TUI is a real interactive session, driven by tmux send-keys: stage
# the export, type the prompt, wait for the answer, exit, then trim the cast
# (the TUI repaints the whole screen every tick) and render the GIF.
#
# The procedure is scripted, the take is not deterministic — the agent picks its
# own commands and wording every run, so ALWAYS watch the resulting GIF before
# committing it. Re-run until a take shows the harken invocation and a tight
# answer; discard the ones that leak a session scratchpad path.
#
# Needs: asciinema, agg, ffmpeg, tmux, `claude` on PATH, a release build
# (`cargo build --release`), and espeak-ng (or piper + PIPER_VOICE) when no
# pre-validated zip is passed.
#
# Usage: scripts/demo/record-claude-demo.sh [demo.zip]
#        (writes docs/assets/demo-claude.gif)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=scripts/demo/lib.sh
source "$REPO_ROOT/scripts/demo/lib.sh"

HARKEN="$REPO_ROOT/target/release/harken"
[[ -x "$HARKEN" ]] || { echo "error: build first: cargo build --release" >&2; exit 1; }
for tool in claude tmux asciinema agg; do
    command -v "$tool" >/dev/null || { echo "error: $tool is not on PATH" >&2; exit 1; }
done

# Short, boring paths: whatever is here shows up inside the GIF.
STAGE="${HARKEN_DEMO_DIR:-/tmp/wa-export}"
ZIP_NAME="WhatsApp Chat - Ana.zip"
# Warm-up turns first: their output scrolls the welcome banner (which carries
# the operator's name and plan) out of the viewport, so the trimmed GIF never
# shows it. The trim starts at PROMPT, so the warm-ups survive only as
# scrollback — two of them, because one is not enough to clear 12 banner rows.
WARMUPS=(
    'list the files in this folder, one line, no commentary'
    'print the chat log inside that zip, raw, no commentary'
)
PROMPT='what task did my coworker send me in this chat export? transcribe into ./transcripts and answer in english, 5 short bullets max'
SOCK=harkendemo
COLS=100
ROWS=30
GIF="$REPO_ROOT/docs/assets/demo-claude.gif"

WORK="$(mktemp -d)"
trap 'tmux -L "$SOCK" kill-server 2>/dev/null; rm -rf "$WORK"' EXIT

rm -rf "$STAGE"
mkdir -p "$STAGE/.claude/skills"
# A copy of the skill is all the context the session gets, so the repo's
# CLAUDE.md and auto-memory never show up in the recording.
cp -r "$REPO_ROOT/.claude/skills/transcribe-audio" "$STAGE/.claude/skills/"
if [[ -n "${1:-}" ]]; then
    cp "$1" "$STAGE/$ZIP_NAME"
else
    bash "$REPO_ROOT/scripts/demo/make-demo-zip.sh" "$STAGE/$ZIP_NAME"
fi

prewarm_model "$HARKEN" "$WORK"

{
    echo 'set -g status off'                      # no tmux status bar in frame
    echo 'set -g focus-events on'                 # silences the TUI's focus-events hint
    echo 'set -g default-terminal "xterm-256color"'
} > "$WORK/tmux.conf"

# Runs inside the asciinema pty: asciinema sizes the pty at 80x24 regardless of
# COLUMNS/LINES, and tmux reads the real ioctl size — so fix it here.
cat > "$WORK/inner.sh" <<EOF
#!/usr/bin/env bash
stty rows $ROWS cols $COLS
export PATH="$REPO_ROOT/target/release:\$PATH"
cd "$STAGE" || exit 1
exec tmux -L $SOCK -f "$WORK/tmux.conf" new-session -s d \\
    "claude --allowedTools Skill Bash Read Glob \\
        --append-system-prompt 'Never call the advisor tool. Answer directly and concisely.'"
EOF

# Outside WORK on purpose: if the trim below fails, the take is still on disk
CAST="${TMPDIR:-/tmp}/harken-demo-claude.cast"
tmux -L "$SOCK" kill-server 2>/dev/null || true
(cd "$STAGE" && asciinema rec "$CAST" --overwrite --idle-time-limit 2 \
    -c "bash '$WORK/inner.sh'") &
REC=$!

pane() { tmux -L "$SOCK" capture-pane -p -t d 2>/dev/null; }
wait_for() { # wait_for <regex> <tries>
    local i
    for ((i = 0; i < $2; i++)); do
        sleep 1
        grep -qE "$1" <<<"$(pane)" && return 0
    done
    return 1
}

wait_for 'auto mode on|Enter to confirm' 30 || { echo "error: TUI never came up" >&2; exit 1; }
if grep -q "Enter to confirm" <<<"$(pane)"; then # first run in this dir: trust prompt
    tmux -L "$SOCK" send-keys -t d Enter
    wait_for 'auto mode on' 30
fi

ask() { # ask <prompt> — type it, submit it, wait until the answer settles
    tmux -L "$SOCK" send-keys -t d -l "$1"
    sleep 1
    tmux -L "$SOCK" send-keys -t d Enter
    # Done when the footer stops offering to interrupt and the screen stops moving.
    local prev="" cur
    for _ in $(seq 1 60); do
        sleep 6
        cur="$(pane)"
        if ! grep -q "esc to interrupt" <<<"$cur" && [[ "$cur" == "$prev" ]]; then
            return 0
        fi
        prev="$cur"
    done
}

for warmup in "${WARMUPS[@]}"; do
    ask "$warmup"
    tmux -L "$SOCK" send-keys -t d Escape # drop the suggested follow-up from the input box
    sleep 1
done
ask "$PROMPT"

# No /exit: typing it would land in the last frames. Killing the server ends
# the pty, which ends the recording.
tmux -L "$SOCK" send-keys -t d Escape # drop the suggested follow-up from the input box
sleep 2
tmux -L "$SOCK" kill-server 2>/dev/null || true
wait "$REC" || true

python3 "$REPO_ROOT/scripts/demo/trim-cast.py" "$CAST" "$WORK/trim.cast" \
    "${PROMPT:0:25}" "$COLS" "$ROWS"
cast_to_gif "$WORK/trim.cast" "$GIF" --font-size 14 --fps-cap 10
