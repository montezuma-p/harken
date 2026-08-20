# harken — architecture

Rust crate, single binary. Transcription engine is whisper.cpp (via
whisper-rs); all audio decoding happens in-process. Ported from a Python
implementation (faster-whisper/CTranslate2) in v0.3.0; the Python test suite
was carried over as the behavior spec (78 tests in `tests/`, all offline).

## Flow

Batch mode (default, no subcommand):

```
CLI (clap, src/cli.rs)
  └─ main.rs builds a WhisperCppEngine, dispatches
       └─ batch::run_batch_mode
            ├─ collect_audio_files   (files verbatim / dirs recursed / globs expanded)
            └─ run_batch             (generic over the Transcriber trait)
                 ├─ engine.transcribe(path)
                 │     ├─ audio::decode_audio_16k_mono  (libopus | symphonia+rubato)
                 │     └─ whisper.cpp full()            (context loaded lazily, once)
                 ├─ writers::write_output               (txt | json | srt)
                 └─ writers::append_manifest            (manifest.jsonl)
```

WhatsApp mode (`harken whatsapp export.zip`):

```
whatsapp::run
  ├─ open zip, list member names
  ├─ find_chat_entry            (*_chat.txt, or a single root .txt)
  ├─ parse_chat                 (iOS/Android format detected once per corpus)
  ├─ select_audio_messages      (audio attachment + inclusive --from/--to range)
  ├─ extract attachments        → <out>/audio/<bare filename>
  ├─ batch::run_batch           (same pipeline, same skip/force/manifest)
  └─ --merge: build_merged_chat → <out>/_chat.transcribed.txt
       (transcripts read back from manifest.jsonl, filtered to THIS run's selection)
```

Exit codes everywhere: `0` ok, `1` some transcription failed, `2` input error.
All progress/log output goes to stderr; stdout is never written to.

## Modules

**`src/cli.rs`** — clap surface. Batch args are flattened at the top level with
`args_conflicts_with_subcommands` + `subcommand_negates_reqs`, so `harken
file.opus` and `harken whatsapp export.zip` coexist without a `transcribe`
subcommand. `language_option` maps `--lang auto` to `None` (engine
auto-detect). Defaults: `small`, `pt`, `txt`, `cpu`, `./transcripts`.

**`src/main.rs`** — thin dispatcher: parse, construct the one real
`WhisperCppEngine`, call the mode entry, `exit(code)`. All logic lives in the
library so tests can drive it with a fake engine.

**`src/engine.rs`** — core types (`Segment`, `TranscriptionResult`) and the
`Transcriber` trait that keeps the whole pipeline testable offline.
`assemble_result` trims the leading space whisper's tokenizer puts on each
segment, then joins segments with single spaces — this trimming is a spec'd
contract (txt/srt/json output depends on it). `WhisperCppEngine` loads the
whisper context lazily on the first `transcribe()` and reuses it for the whole
batch (one model load per run, the crate's main perf property).
`install_logging_hooks()` silences whisper.cpp/ggml's chatty stderr. Whisper
timestamps arrive in centiseconds and are converted to seconds here.
`--device` other than `cpu` just flips `use_gpu` — actual GPU support depends
on whisper-rs build features (none enabled in v0.3.0).

**`src/audio.rs`** — decodes anything to whisper's input: 16 kHz mono f32.
Non-obvious decision: `.opus` (WhatsApp voice notes, the hot path) is decoded
with libopus **natively at 16 kHz** — Opus supports 8/12/16/24/48 kHz decode
rates, so no resampling pass is ever needed; the OpusHead pre-skip (expressed
in 48 kHz samples) is rescaled to 16 kHz before being dropped. Everything else
goes through symphonia (wav/flac/mp3/m4a/…) with a rubato FFT resample only
when the source rate differs from 16 kHz. `.ogg`/`.oga` first tries symphonia,
then falls back to the libopus path — symphonia demuxes Ogg but cannot decode
an Opus stream. Multi-channel audio is downmixed to mono by averaging.

**`src/model.rs`** — maps `--model` to a local ggml file. An existing file
path is used verbatim; otherwise the name must be one of the known ggml names
(`tiny` … `large-v3-turbo`, plus `.en` variants), optionally with a
`-q5_0`/`-q5_1`/`-q8_0` quantization suffix. Cache:
`~/.cache/harken/models/ggml-<name>.bin` (respects `XDG_CACHE_HOME` if
absolute). Download is from the `ggerganov/whisper.cpp` HF repo via ureq with
an indicatif progress bar, written to a `.partial` file and renamed into place
(no half-downloaded models in the cache).

**`src/batch.rs`** — input collection and the batch loop. `collect_audio_files`
distinguishes explicit paths (included verbatim, any extension; missing → hard
error, exit 2) from dirs/globs (filtered to `AUDIO_EXTENSIONS`); results are a
`BTreeSet`, so ordering is deterministic. Stem-collision handling
(`a/x.opus` + `b/x.opus` → `x.txt`, `x-2.txt`) is driven purely by a per-run
counter — **never** by filesystem existence; the skip/force decision is made
independently afterward. `manifest.jsonl` is append-only; on re-runs it
accumulates duplicate sources, and readers take the last entry (last-wins —
`whatsapp::load_manifest_texts` relies on this).

**`src/whatsapp.rs`** — chat-export mode. The iOS and Android message-header
regexes are exact ports from Python and are locked by 47 tests — don't touch
them casually. Format detection runs once per chat (first line matching either
pattern wins) and applies to the whole corpus. Android day-first vs
month-first is inferred from the entire corpus of header dates: any first
component > 12 proves day-first, any second component > 12 proves month-first,
fully ambiguous chats default to day-first (pt-centric). Non-header lines are
continuations folded into the previous message's body; U+200E/U+200F are
stripped before matching but the merged output preserves the raw lines. The
merge step filters manifest entries down to this run's selection so a reused
`--out` with a narrower date range can't inline stale transcripts. Output dir
creation is deferred until the chat log is located, so a bad zip exits 2
without leaving an empty `<out>/audio/` behind.

**`src/writers.rs`** — output serialization. txt is `text + "\n"`, srt is the
standard numbered cue blocks with `HH:MM:SS,mmm` timestamps (millisecond
rounding via `round()`), json is pretty-printed with a trailing newline. These
are byte-exact contracts (see `tests/writers_test.rs`). The manifest is one
compact JSON object per line: source, output, language, duration, text.

## Contracts inherited from the Python port

Behaviors the tests pin down and that are easy to break by accident:

- Segment text is trimmed and joined with single spaces; no leading/doubled
  whitespace ever reaches an output file.
- txt output ends with exactly one `\n`; srt cue format and timestamp rounding
  are byte-exact; json is pretty-printed and ends with `\n`.
- Skips (`output exists && !force`) are not failures and don't affect the exit
  code; a failed file doesn't stop the batch.
- Stem-collision numbering counts per run, independent of what's on disk.
- Explicit file paths bypass the audio-extension filter; dirs and globs don't.
  A missing explicit path or dir is exit 2 before any work happens.
- `transcribe()` on a missing file errors *before* any decode/model work.
- WhatsApp: format detected once per chat; dates inclusive on both ends;
  attachment paths in chat text are flattened to the bare filename on
  extraction; attachments referenced but missing from the zip are a warning,
  not an error; chat text is decoded as utf-8-sig (leading BOM stripped);
  merged chat leaves everything outside the selection byte-identical and
  appends a final `\n`.
- Merge transcripts come from `manifest.jsonl` (last-wins per source),
  filtered by the current run's selection.

## Discarded / future

- **No VAD in v0.3.0.** The Python version (faster-whisper) ran Silero VAD
  before transcription; whisper.cpp does not, so long silences may transcribe
  slightly differently. Accepted trade-off for the single-binary pitch.
- **GPU backends** (Metal/CUDA/Vulkan) are whisper-rs build features, not
  enabled in released binaries — cargo-dist has no clean per-target feature
  story. Metal on macOS is the first candidate; document as build-from-source
  until then.
- **crates.io publish pending** — the README's `cargo install harken` /
  `cargo binstall harken` lines are aspirational until the crate is published.
