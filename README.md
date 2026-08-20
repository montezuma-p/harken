# harken

**Local audio transcription for Claude Code and any agent — batch, fully offline, no API key.**

[![PyPI](https://img.shields.io/pypi/v/harken)](https://pypi.org/project/harken/)

Powered by [faster-whisper](https://github.com/SYSTRAN/faster-whisper).

## Why

Voice notes, meeting recordings, and WhatsApp PTT audio often contain
sensitive content. `harken` transcribes everything on your own machine:
no audio, and no transcript, ever leaves the device. There is no API key,
no upload step, no cloud dependency.

- Runs on CPU with `int8` quantization by default — no GPU required.
- The model loads once per run and is reused for every file in a batch.
- Two modes: transcribe loose audio files (or whole folders), or point it
  straight at a WhatsApp chat-export `.zip` and let it pull out only the
  voice notes.

## Quickstart

Requires Python >= 3.11. With [uv](https://docs.astral.sh/uv/), no install
step at all:

```bash
uvx harken voice-note.opus
```

To keep the command on PATH: `uv tool install harken` (or
`pip install harken`), then `harken --help`.

For development, clone and run from source:

```bash
git clone https://github.com/montezuma-p/harken && cd harken
uv sync
uv run harken voice-note.opus
```

## Use with Claude Code

`harken` ships an [Agent Skill](.claude/skills/transcribe-audio/SKILL.md) that
teaches Claude Code when and how to transcribe audio locally instead of
reaching for a cloud API. Install it as a plugin:

```
/plugin marketplace add montezuma-p/harken
/plugin install harken@harken
```

That's all — the skill runs the CLI with `uvx harken`, so the plugin works
on its own with no separate install. (Alternatively: clone the repo and the
project-scoped skill in `.claude/skills/` is picked up automatically, or
copy/symlink `.claude/skills/transcribe-audio/` into `~/.claude/skills/`.)

## Usage

### Batch mode — files, folders, or globs

```bash
# One file
harken voice-note.opus

# A whole folder, recursively, written to ./transcripts by default
harken ~/Downloads/meeting-recordings/

# Glob, custom output dir, JSON output, force re-transcription
harken "recordings/*.m4a" --out ./out --format json --force

# Larger model, auto-detect language instead of the pt default
harken recording.wav --model medium --lang auto
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
— either works, `harken` only needs the audio attachments) and point
`harken` at the resulting `.zip`:

```bash
# All voice notes in the export
harken whatsapp "WhatsApp Chat with Maria.zip"

# Only a date range, with the model biased to Portuguese
harken whatsapp export.zip --from 2026-07-01 --to 2026-07-15 --lang pt

# Also write a merged chat transcript with each transcript inlined
harken whatsapp export.zip --from 2026-07-01 --to 2026-07-15 --merge --out ./maria-july
```

Flags: `--out DIR` (default `./<zip-stem>-transcripts`), `--from` / `--to`
(`YYYY-MM-DD`, inclusive on both ends), `--merge`, plus `--model`, `--lang`,
`--device`, `--force` as in batch mode.

`harken whatsapp` locates the chat log inside the zip, selects only the
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
