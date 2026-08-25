# harken — architecture

Rust crate, single binary. Transcription engine is whisper.cpp (via direct FFI
bindings in this repo); all audio decoding happens in-process. Ported from a Python
implementation (faster-whisper/CTranslate2) in v0.3.0; the Python test suite
was carried over as the behavior spec, now 137 tests in `tests/`, all offline.

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
                 ├─ writers::write_output               (txt | json | srt | md)
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

MCP mode (`harken mcp`):

```
mcp::serve (JSON-RPC 2.0 over stdio, newline-delimited JSON, one engine per session)
  └─ mcp::handle_line → era::detect(_meta)      no state kept between requests
       ├─ Legacy  (initialize handshake, 2025-06-18 and older)
       │    initialize | ping | tools/list | tools/call
       └─ Modern  (per-request _meta, 2026-07-28)
            server/discover | ping | tools/list | tools/call
            + preflight: -32022 unknown version, -32602 missing _meta field
            + every result gains resultType and _meta.serverInfo
            + tools/list gains ttlMs and cacheScope

  tools/call, both eras:
       ├─ transcribe_file               → engine.transcribe, transcript as content
       ├─ transcribe_whatsapp_export    → same zip/parse/select helpers as
       │    whatsapp mode, attachments extracted to a per-call temp dir,
       │    transcripts returned as content — no files written
       └─ transcribe_status             → model cache state via model::cached_path
            (pure stat, never the network), language, device, context_loaded

  startup: main() spawns a warm-up thread running model::ensure_downloaded
  (StderrSink: a line per 10%) so the read loop answers initialize/discover/
  tools/list in milliseconds while an uncached model streams in; WarmGate
  holds tool calls until the warm settles. A failed warm-up is logged, and
  the gate lets calls through anyway — the engine retries the download
  inline, so the error surfaces per call as isError: true and the server
  never dies.
```

Exit codes everywhere: `0` ok, `1` some transcription failed, `2` input error.
All progress/log output goes to stderr; stdout is never written to — except in
MCP mode, where stdout is the protocol channel and carries exactly the JSON-RPC
frames, nothing else.

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
contract (txt/srt/json/md output depends on it). `WhisperCppEngine` loads the
whisper context lazily on the first `transcribe()` and reuses it for the whole
batch (one model load per run, the crate's main perf property). It talks to
whisper.cpp through the raw bindings in **`src/ffi.rs`** (manually mirrored from
`vendor/whisper.cpp/include/whisper.h`), not through `whisper-rs`.
`install_logging_hooks()` silences whisper.cpp/ggml's chatty stderr. Whisper
timestamps arrive in centiseconds and are converted to seconds here.
`--device` other than `cpu` just flips `use_gpu` — but `build.rs` only ever
compiles ggml's CPU backend, so on every shipped binary the flag finds no
backend to bind and ggml proceeds on CPU; `gpu_fallback_warning` says so on
stderr instead of letting the run masquerade as GPU-accelerated.

**`src/ffi.rs`** — minimal unsafe FFI surface for the subset of whisper.cpp's C
API that `WhisperCppEngine` actually uses: context/state lifecycle, `whisper_full`,
segment iteration, language lookup, and logging hooks. The bindings are kept
manual on purpose so the repo controls the ABI it consumes.

**`vendor/whisper.cpp/`** — git submodule pinned to whisper.cpp **v1.7.6**
(`a8d002cfd879315632a579e73f0148d06959de36`). `build.rs` compiles the required
ggml + CPU backend sources directly with `cc`, removing the `whisper-rs`
maintenance layer while keeping version control fully inside this repo.

**`build.rs`** — two `cc` builds (C and C++) over the vendored sources. Two
non-obvious flags carry almost all of the performance:

- **ISA floor.** ggml picks its CPU kernels at *compile* time (`arch/x86/quants.c`
  and `simd-mappings.h` gate on `__AVX2__`/`__F16C__`), so a build with no `-m`
  flags silently produces scalar code. whisper.cpp's CMake dodges that with
  `-march=native`, which is right for a machine-local build and wrong for a
  distributed one. Here the default is a fixed floor — AVX2/FMA/F16C, Haswell
  (2013) and newer, *narrower* than what `-march=native` on a CI runner emits —
  and `HARKEN_NATIVE=1` opts a source build into the host's full ISA.
- **`NDEBUG`.** `CMAKE_BUILD_TYPE=Release` implies it; `cc` does not add it. Without
  it, every `assert()` in ggml's operator loops stays compiled in.

Measured on an i5-7400, `small`, 60 s of audio, 5 interleaved pairs against a
0.3.1 (whisper-rs/CMake) binary: **+1.7% at the minimum, +3.4% at the median**.
The residual is the baseline's OpenMP (ggml's CMake defaults it on) plus the
extras `-march=native` adds beyond the floor above. Transcription output is
byte-identical.

**`src/audio.rs`** — decodes anything to whisper's input: 16 kHz mono f32.
Non-obvious decision: `.opus` (WhatsApp voice notes, the hot path) is decoded
with libopus **natively at 16 kHz** — Opus supports 8/12/16/24/48 kHz decode
rates, so no resampling pass is ever needed; the OpusHead pre-skip (expressed
in 48 kHz samples) is rescaled to 16 kHz before being dropped. Everything else
goes through symphonia (wav/flac/mp3/m4a/…) with a rubato FFT resample only
when the source rate differs from 16 kHz. `.ogg`/`.oga` first tries symphonia,
then falls back to the libopus path — symphonia demuxes Ogg but cannot decode
an Opus stream. Multi-channel audio is downmixed to mono by averaging.

**`src/model.rs`** — maps `--model` to a local ggml file, split three ways:
`cached_path` (pure stat — what the MCP status tool calls, so it can never
trigger a download), `ensure_downloaded` (filesystem + ureq only, hence
trivially `Send` for the MCP warm-up thread), and `resolve_model` (the CLI
wrapper). An existing file path is used verbatim; otherwise the name must be
one of the known ggml names (`tiny` … `large-v3-turbo`, plus `.en` variants),
optionally with a `-q5_0`/`-q5_1`/`-q8_0` quantization suffix. Cache:
`~/.cache/harken/models/ggml-<name>.bin` (respects `XDG_CACHE_HOME` if
absolute). Download is from the `ggerganov/whisper.cpp` HF repo via ureq;
progress goes through the `ProgressSink` trait (`BarSink` = indicatif for the
CLI, `StderrSink` = one plain line per 10% for MCP mode, where a drawn bar in
a piped stderr would be noise). Integrity: bytes stream into a `.partial`
file with a PID+counter nonce (concurrent cold starts cannot interleave),
guarded so error paths leave no debris; the byte count is checked against
Content-Length and the SHA-256 against HuggingFace's `X-Linked-Etag` — read
off the *redirect* response, which is why the first hop is taken unfollowed —
before the rename. The hash check is a corruption/mirror guard, not
provenance. `harken warm [--model X]` pre-downloads and exits.

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
regexes are exact ports from Python and are locked by 53 tests — don't touch
them casually. They validate *shape*, not the calendar: a header whose date is
not a real date is not a header, and falls through to the continuation branch. Format detection runs once per chat (first line matching either
pattern wins) and applies to the whole corpus. Android day-first vs
month-first is inferred from the entire corpus of header dates: any first
component > 12 proves day-first, any second component > 12 proves month-first,
fully ambiguous chats default to day-first (pt-centric). Non-header lines are
continuations folded into the previous message's body; U+200E/U+200F are
stripped before matching but the merged output preserves the raw lines. The
merge step filters manifest entries down to this run's selection so a reused
`--out` with a narrower date range can't inline stale transcripts. Output dir
creation is deferred until the chat log has been read, so a bad zip exits 2
without leaving an empty `<out>/audio/` behind — locating the entry is not
enough, since reading it can still fail on a corrupt or unsupported member.

**`src/mcp/`** — MCP server mode (`harken mcp`): JSON-RPC 2.0 over stdio,
hand-rolled on serde_json — no SDK, no async runtime, zero new dependencies.
`mod.rs` is framing and dispatch and the only I/O; `jsonrpc.rs` the envelope,
response builders and error codes; `era.rs` the protocol eras; `tools.rs` the
catalog; `whatsapp_tool.rs` the zip work. It was one 434-line file until the
2026-07-28 work roughly doubled the protocol code. The split is also the escape
hatch: if hand-tracking the spec ever stops paying, the protocol layer is what
an SDK (`rmcp`) would replace, and only a layer that is not interleaved with the
tool handlers can be swapped cleanly.

`serve` is generic over `BufRead`/`Write` and takes `&mut dyn Transcriber` — the
same testability seam as the batch pipeline, driven in tests by in-memory
buffers and `FakeEngine`. It reads bytes rather than using `BufRead::lines()`,
which yields `Err` on non-UTF-8 and would end the session; a bad frame gets
-32700 and the loop continues. One engine lives for the whole session, so the
lazy context load is reused across tool calls.

**Two eras, discriminated per request.** 2026-07-28 removed the
`initialize`/`notifications/initialized` handshake: a request now carries its
protocol version and client capabilities in `_meta`, and the server infers
nothing from earlier traffic. Deployed clients still speak the handshake, so
both are served, and the discriminator is the presence of
`_meta.protocolVersion` — which the revision makes required precisely so a
stateless server can branch on it. Nothing is remembered between requests.

| method | Legacy | Modern |
|---|---|---|
| `initialize` | `InitializeResult` | `-32601` — removed in the revision |
| `notifications/initialized` | dropped | dropped |
| `ping` | `{}` | answered too — see below |
| `server/discover` | `-32602` | `DiscoverResult` |
| `tools/list` | `{tools}` | `+ resultType, ttlMs, cacheScope, _meta.serverInfo` |
| `tools/call` | as before | `+ resultType, _meta.serverInfo` |
| `notifications/cancelled` | dropped | dropped |

Three decisions worth not relitigating:

- **New fields are era-gated, not emitted unconditionally.** Sending
  `resultType` to a 2025-06-18 client is probably harmless, but gating keeps the
  25 ported tests in `tests/mcp_test.rs` passing byte-for-byte with no conscious
  change, which `CLAUDE.md`'s inviolable rules make the deciding factor. It
  costs one `bool`. `tests/mcp_protocol_test.rs` locks the separation with a
  recursive absence check; verified by mutation.
- **`server/discover` advertises only the modern revisions.** Listing
  2025-06-18 there would invite a client to select it and send it back as
  `_meta.protocolVersion`, which is a contradiction. The handshake stays an
  undeclared backward-compatibility affordance, as the stdio transport's own
  backward-compatibility section frames it. On stdio `server/discover` is also
  the probe a dual-era client sends first: answering it is what identifies this
  server as modern, and a probe without `_meta` gets -32602 naming the missing
  field (a dual-era client must not key its fallback to a specific code, so
  either error works, and naming the field helps the case that is a real bug).
- **`ping` is answered in both eras** although 2026-07-28 removed it. A -32601
  on a keepalive can make a client tear the connection down, and answering costs
  nothing.

Tool handlers call `transcriber.transcribe()` and the pub `whatsapp` helpers
directly instead of reusing `run_batch_mode`/`whatsapp::run`: those return exit
codes, drop errors on stderr, and panic on IO — fatal in a long-lived server.
Two error channels per the MCP spec: protocol errors are JSON-RPC `error`
objects; everything after a well-formed request (missing file, bad zip, failed
transcription) is a result with `isError: true`. The WhatsApp tool extracts
attachments to a per-call temp dir and returns transcripts as content — it
writes no output files.

Error codes are named in `jsonrpc.rs` because the revision partitions the
server-error range: `-32000..-32019` is legacy and off-limits to new code, and
`-32020..-32099` is reserved for the spec and must not carry an undefined code.
`every_emitted_error_code_is_spec_defined` walks every fault the server can
produce and holds it to that.

**Not implemented, deliberately:** progress notifications, cancellation, the
`io.modelcontextprotocol/tasks` extension, `subscriptions/listen`, resources,
prompts, sampling, roots, elicitation, and MRTR. Transcription still runs inline
on the read loop, so a long call blocks the server and cannot be cancelled —
that needs a worker thread and the two null callbacks in `ffi.rs`, and is
tracked as an issue rather than guessed at here.

**`src/writers.rs`** — output serialization. txt is `text + "\n"`, srt is the
standard numbered cue blocks with `HH:MM:SS,mmm` timestamps (millisecond
rounding via `round()`), json is pretty-printed with a trailing newline, md is
`# <source stem>` then one `[HH:MM:SS] text` line per segment. srt and md split
the timestamp through the same `hms_millis`, so they never disagree on a
boundary; md drops the milliseconds, which truncates to the second. These are
byte-exact contracts (see `tests/writers_test.rs`). The manifest is one compact
JSON object per line: source, output, language, duration, text.

## Contracts inherited from the Python port

Behaviors the tests pin down and that are easy to break by accident:

- Segment text is trimmed and joined with single spaces; no leading/doubled
  whitespace ever reaches an output file.
- txt output ends with exactly one `\n`; srt cue format and timestamp rounding
  are byte-exact; json is pretty-printed and ends with `\n`; md is byte-exact
  too, and a segment-less result is still title + blank line.
- A chat header whose date is shape-valid but not a real date (`31/02`, a
  non-ASCII digit) is not a header: it falls through to the continuation
  branch. Such dates are also excluded from the day-first inference corpus,
  since they read as impossible either way and carry no ordering evidence.
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
- **Direct whisper.cpp vendoring** was chosen for total version control, to
  remove the `whisper-rs` intermediary, and to keep the door open for local
  patches such as revisiting Silero VAD integration later.
- **GPU backends** (Metal/CUDA/Vulkan) are not wired into the release build
  today. Metal on macOS is the first candidate; document as build-from-source
  until then.
- **Published on crates.io** since v0.3.1 — `cargo install harken` /
  `cargo binstall harken` work. Note: the crate builds whisper.cpp and
  (without system libopus) the vendored opus tree, so source installs need
  cmake + a C++ toolchain; CMake >= 4 hosts may need
  `CMAKE_POLICY_VERSION_MINIMUM=3.5` (the repo's `.cargo/config.toml` does
  not travel inside the published package).
