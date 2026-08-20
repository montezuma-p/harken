# harken

**Local audio transcription for Claude Code and any agent — batch, fully offline, no API key.**

[![CI](https://github.com/montezuma-p/harken/actions/workflows/ci.yml/badge.svg)](https://github.com/montezuma-p/harken/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A single static binary powered by [whisper.cpp](https://github.com/ggml-org/whisper.cpp).
No Python, no ffmpeg, no runtime dependencies — audio decoding (opus, mp3,
m4a, wav, flac, …) happens in-process.

## Why

Voice notes, meeting recordings, and WhatsApp PTT audio often contain
sensitive content. `harken` transcribes everything on your own machine:
no audio, and no transcript, ever leaves the device. There is no API key,
no upload step, no cloud dependency.

- Runs on CPU by default — no GPU required.
- The model loads once per run and is reused for every file in a batch.
- Two modes: transcribe loose audio files (or whole folders), or point it
  straight at a WhatsApp chat-export `.zip` and let it pull out only the
  voice notes.

## Install

One-liner (Linux/macOS):

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/montezuma-p/harken/releases/latest/download/harken-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/montezuma-p/harken/releases/latest/download/harken-installer.ps1 | iex"
```

Other options:

```bash
cargo binstall harken        # prebuilt binary via cargo-binstall
cargo install harken --locked  # build from source (needs cmake + a C++ toolchain)
```

> Building from source with CMake >= 4 and no system libopus? The vendored
> opus tree declares an old CMake minimum; prepend
> `CMAKE_POLICY_VERSION_MINIMUM=3.5` to the `cargo install` line.

## Use with Claude Code

`harken` ships an [Agent Skill](.claude/skills/transcribe-audio/SKILL.md) that
teaches Claude Code when and how to transcribe audio locally instead of
reaching for a cloud API. Install it as a plugin:

```
/plugin marketplace add montezuma-p/harken
/plugin install harken@harken
```

The skill invokes the `harken` binary and knows how to install it with the
one-liner above if it is missing. (Alternatively: clone the repo and the
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

Export a chat from WhatsApp ("Export chat" → with media) and point
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
`--format`, `--device`, `--force` as in batch mode.

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

`--model` accepts a [whisper.cpp ggml model](https://huggingface.co/ggerganov/whisper.cpp)
name or a path to a local `.bin` file:

- `small` (default, ~466 MB) — good accuracy/speed tradeoff on CPU; fine
  for most voice notes and casual recordings.
- `medium` (~1.5 GB) — noticeably better accuracy (accents, background
  noise, technical vocabulary), at a real CPU time cost.
- Quantized variants — append `-q5_0`, `-q5_1`, or `-q8_0` to any name
  (e.g. `small-q5_1`, ~182 MB): ~60% smaller download, marginal quality
  loss.
- Also: `tiny`, `base`, `large-v3`, `large-v3-turbo`, and their `.en`
  English-only variants.

The chosen model is downloaded once, on first use, to
`~/.cache/harken/models`. Subsequent runs reuse the cached model with no
network access.

CPU is the safe default everywhere. Passing `--device` with anything other
than `cpu` enables GPU offload when the binary was built with a GPU backend
(Metal/CUDA/Vulkan — see the whisper-rs build features).

## Development

```bash
make check   # fmt + clippy + tests + cargo-audit + cargo-machete
```

Tests never load a real Whisper model — the transcription engine is a
trait, and the suite runs against a fake, so it is instant and offline.

## License

[MIT](LICENSE)
