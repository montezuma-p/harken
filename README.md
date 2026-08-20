# hark

**Local audio transcription for Claude Code and any agent — batch, fully offline, no API key.**

Powered by [faster-whisper](https://github.com/SYSTRAN/faster-whisper).

## Why

Voice notes, meeting recordings, and WhatsApp PTT audio often contain
sensitive content. `hark` transcribes everything on your own machine:
no audio, and no transcript, ever leaves the device. There is no API key,
no upload step, no cloud dependency.

- Runs on CPU with `int8` quantization by default — no GPU required.
- The model loads once per run and is reused for every file in a batch.
- Two modes: transcribe loose audio files (or whole folders), or point it
  straight at a WhatsApp chat-export `.zip` and let it pull out only the
  voice notes.

## Quickstart

Requires Python >= 3.11, managed with [uv](https://docs.astral.sh/uv/).

```bash
git clone https://github.com/montezuma-p/hark && cd hark
uv sync
uv run hark voice-note.opus
```

Or install it as a standalone tool: `uv tool install .` then `hark --help`.

## Use with Claude Code

`hark` ships an [Agent Skill](.claude/skills/transcribe-audio/SKILL.md) that
teaches Claude Code when and how to transcribe audio locally instead of
reaching for a cloud API. Three ways to get it:

1. **Clone the repo** — the skill is project-scoped in `.claude/skills/`,
   so any Claude Code session started inside the repo picks it up
   automatically.
2. **Install as a plugin**:

   ```
   /plugin marketplace add montezuma-p/hark
   /plugin install hark@hark
   ```

3. **Copy or symlink** `.claude/skills/transcribe-audio/` into
   `~/.claude/skills/` to make it available in every project.

> **Note:** the plugin installs the *skill*, not the CLI. You still need
> the `hark` command available — either `uv tool install .` from a clone,
> or run it with `uv run hark ...` from the repo root.

## Usage

### Batch mode — files, folders, or globs

```bash
# One file
hark voice-note.opus

# A whole folder, recursively, written to ./transcripts by default
hark ~/Downloads/meeting-recordings/

# Glob, custom output dir, JSON output, force re-transcription
hark "recordings/*.m4a" --out ./out --format json --force

# Larger model, auto-detect language instead of the pt default
hark recording.wav --model medium --lang auto
```

Flags: `--out DIR` (default `./transcripts`), `--model` (default `small`),
`--lang` (default `pt`; `--lang auto` to auto-detect), `--format`
(`txt`/`json`/`srt`, default `txt`), `--device` (default `cpu`), `--force`
(re-transcribe even if the output file already exists).

Every file gets `<out>/<stem>.<format>`; a running `manifest.jsonl` records
one line per transcription. Progress and a final summary print to stderr;
exit code is `1` if any file failed, `0` otherwise (skips don't count as
failures).

> **Privacy note:** output files — transcripts and `manifest.jsonl` —
> contain the full transcribed text. Default output dirs (`transcripts/`,
> `*-transcripts/`) are gitignored in this repo; keep yours out of version
> control too.

### WhatsApp export mode — transcribe voice notes straight from a chat export

Export a chat from WhatsApp ("Export chat" → without media, or with media
— either works, `hark` only needs the audio attachments) and point
`hark` at the resulting `.zip`:

```bash
# All voice notes in the export
hark whatsapp "WhatsApp Chat with Maria.zip"

# Only a date range, with the model biased to Portuguese
hark whatsapp export.zip --from 2026-07-01 --to 2026-07-15 --lang pt

# Also write a merged chat transcript with each transcript inlined
hark whatsapp export.zip --from 2026-07-01 --to 2026-07-15 --merge --out ./maria-july
```

Flags: `--out DIR` (default `./<zip-stem>-transcripts`), `--from` / `--to`
(`YYYY-MM-DD`, inclusive on both ends), `--merge`, plus `--model`, `--lang`,
`--device`, `--force` as in batch mode.

`hark whatsapp` locates the chat log inside the zip, selects only the
messages carrying an audio attachment within the date range, extracts
those files to `<out>/audio/`, and transcribes them (same skip/force,
manifest, and progress behavior as batch mode — it reuses the same batch
pipeline). With `--merge`, it also writes `<out>/_chat.transcribed.txt`:
the full original chat, with each transcribed attachment line immediately
followed by a `    >> [transcript] <text>` line. Everything outside the
date range, and every non-audio attachment, is left untouched.

Both iOS and Android export formats are auto-detected (per chat, from the
first message header). Android date order (day-first vs month-first) is
inferred from the chat itself; when every date is ambiguous (all components
<= 12), day-first is assumed.

## Models & hardware

- `small` (default) — good accuracy/speed tradeoff on CPU; fine for most
  voice notes and casual recordings.
- `medium` — noticeably better accuracy (accents, background noise,
  technical vocabulary), at a real CPU time cost. Use it when the content
  matters enough to wait.

The chosen model is downloaded once, on first use, to
`~/.cache/huggingface` (~460 MB for `small`; larger models are bigger).
Subsequent runs reuse the cached model with no network access.

CPU with `int8` quantization is the safe default everywhere. If your GPU
is supported by [CTranslate2](https://github.com/OpenNMT/CTranslate2),
pass `--device cuda`.

## Development

```bash
uv run pytest
```

Tests never load a real Whisper model — `faster_whisper.WhisperModel` is
stubbed out, so the suite runs instantly and offline.

## License

[MIT](LICENSE)
