---
name: transcribe-audio
description: Use when a task requires reading, transcribing, or extracting content from audio or voice — WhatsApp PTT/voice notes (.opus/.ogg), meeting recordings, .mp3/.m4a/.wav/.mp4 files, or audio attachments inside a WhatsApp chat-export zip.
---

# Transcribe Audio (harken, local-only)

Transcribe audio with **harken**, a local whisper.cpp CLI (single static
binary). Audio never leaves the machine — do not send audio to cloud
transcription APIs, and do not write inline whisper one-liners (they reload
the model per file; harken loads it once per batch).

Invocation: use `harken ...` if the command is on PATH. If it is missing,
install it with the one-liner (no Python, no runtime deps):

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/montezuma-p/harken/releases/latest/download/harken-installer.sh | sh
```

```bash
# Batch: files, dirs, or globs (model loads once per run)
harken ~/Downloads/audios/*.opus --out /path/to/transcripts

# WhatsApp export zip: no manual unzip needed; dates inclusive
# (quote paths with spaces using $HOME — a quoted ~ does not expand)
harken whatsapp "$HOME/Downloads/WhatsApp Chat - X.zip" \
  --from 2026-07-13 --to 2026-07-14 --out /path/to/out --merge
```

- Outputs: per-file `.txt` + `manifest.jsonl` (source, text, duration) in
  `--out`. Existing outputs are skipped; `--force` redoes them.
- `--merge` writes `_chat.transcribed.txt` — the chat with each voice
  note's transcript inlined (`>> [transcript] ...`).
- Defaults: `--lang pt`, `--model small`, CPU (safe everywhere, no GPU
  required).
- `--lang auto` for non-Portuguese audio; `--model medium` when accuracy
  matters more than speed; `--model small-q5_1` for a 60% smaller download;
  `--format json|srt|md` for timestamps (`md` is the readable one: one
  `[hh:mm:ss] line` per segment).
- First ever run downloads the ggml model (~466 MB for `small`) to
  `~/.cache/harken/models`; after that it works fully offline.
- Full docs: [README](https://github.com/montezuma-p/harken#readme).
