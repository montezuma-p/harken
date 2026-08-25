# Changelog

## v0.7.0 — 2026-08-25

The first-run and cold-start paths, which had the least test coverage and the
worst failure modes: every bug below ended in a wrong answer delivered
confidently — an empty transcript, a silent CPU run, a poisoned cache — rather
than in an error.

- **New subcommand: `harken warm [--model X]`** — downloads the model into the
  cache and exits. Retires the "pre-warm with any CLI run" workaround.
- **MCP mode no longer stalls on the first call.** The server downloads an
  uncached model on a warm-up thread while it already answers
  `initialize`/`server/discover`/`tools/list`; a tool call arriving mid-download
  waits on that download instead of starting a second one. A failed warm-up is
  logged and the call is let through to retry inline, so a transient network
  failure at startup never wedges or kills the server. Measured on a fresh
  cache: `initialize` at 0 ms and `tools/list` at 17 ms while 77 MB streamed.
- **New MCP tool: `transcribe_status`** — reports model, cache state (path and
  size), language, device and whether the whisper context is loaded, without
  transcribing. It reads the cache with a pure stat, so it can never itself
  trigger a download.
- **Fixed:** a file whose every packet failed to decode (valid headers, garbage
  payload) produced an empty transcript with exit 0 — over MCP, `isError: false`
  with empty text, so an agent reasoned over a transcript that did not exist.
  Undecodable packets are still skipped, but now counted: all of them failing is
  an error, a partial skip warns about gaps, and `ResetRequired` truncation
  warns instead of passing silently. "Silence" and "nothing decoded" are no
  longer the same outcome.
- **Fixed:** a malformed glob (`harken '[bad'`) reported `0 done` and exit 0,
  indistinguishable from a valid pattern matching nothing. A pattern that cannot
  compile is now an input error naming the pattern, with exit 2. A **valid**
  glob matching zero files stays exit 0 — that distinction is pinned by a test.
- **Fixed:** a truncated download was renamed into the cache and read as a cache
  hit forever, failing context init on every later run with no hint. The written
  byte count is now checked against `Content-Length`, the `.partial` file
  carries a PID+counter nonce so concurrent cold starts cannot interleave into
  one file, and a guard removes it on any error path. The context-init error
  also names the model path and says to delete it if corrupt.
- **Downloads are verified against HuggingFace's SHA-256** (`X-Linked-Etag`,
  read off the redirect response — the CDN's plain `ETag` is a different hash).
  A mismatch refuses to cache. This is a corruption and mirror guard, **not**
  provenance: the server states its own file's hash.
- **Fixed:** `--device cuda` ran at CPU speed reporting nothing. No shipped
  binary compiles a GPU backend, so the flag found none to bind and ggml quietly
  proceeded on CPU. It now warns on stderr and runs on CPU — a warning rather
  than an error, so scripts passing `--device` optimistically keep working.
  README and `docs/ARCHITECTURE.md` no longer imply GPU offload is available.
- Dependencies: **`ureq` 2 → 3** (the migration Dependabot could not do), which
  drops the `url`/`idna`/`icu_*` chain — `Cargo.lock` went 172 → 150 crates
  before `sha2` was added for the hash check. That chain was half the reason for
  the 1.88 MSRV; the floor is now let-chains alone, so `rust-version` is
  unchanged.
- `src/model.rs` split into `cached_path` (pure stat), `ensure_downloaded`
  (filesystem + ureq, hence `Send`) and `resolve_model` (the CLI wrapper), with
  progress behind a new `ProgressSink` trait — `BarSink` keeps indicatif for the
  CLI, `StderrSink` prints one plain line per 10% in MCP mode, where a drawn bar
  in a piped stderr is escape-code noise.
- `Transcriber` gained `is_loaded()`, defaulted to `false` so existing
  implementors need no change. `mcp::serve()` keeps its signature; `main()` uses
  the new `serve_with_info()`.
- Tests: 137 → 158, including the first coverage of `src/audio.rs` and
  `src/model.rs`. All still offline.

## v0.6.0 — 2026-08-24

- **Commits now follow Conventional Commits** (`<type>(<scope>): <subject>`,
  small and atomic), enforced by a new `commits` CI job. The convention is
  documented in `.github/CONTRIBUTING.md` and `CLAUDE.md`.
- **The MCP server now speaks revision 2026-07-28** alongside the `initialize`
  handshake of 2025-06-18 and older. The current revision is stateless: it
  dropped the handshake, so each request carries its protocol version and client
  capabilities in `_meta`. Both eras are served from one dispatcher,
  discriminated per request with no state kept. New: `server/discover` (which is
  also the stdio backward-compatibility probe), `-32022` for an unsupported
  version, `resultType` and `_meta.serverInfo` on modern results, and
  `ttlMs`/`cacheScope` on `tools/list`. Replies to legacy clients are
  byte-identical to before, which a test enforces.
- **Fixed:** a single non-UTF-8 byte on stdin ended the whole MCP session. It
  now gets `-32700` like any other unparseable frame and the server stays up.
- **Fixed:** a `tools/call` with no `name` reported `Unknown tool: ` instead of
  naming the actual fault.
- `src/mcp.rs` became the `src/mcp/` module directory (`jsonrpc`, `era`, `tools`,
  `whatsapp_tool`). No behavior change from the split itself.
- Dependencies: `zip` 2 → 8 and `indicatif` 0.17 → 0.18. The indicatif bump
  drops `number_prefix`, clearing RUSTSEC-2025-0119 — `cargo audit` now reports
  one allowed advisory instead of two. `ureq` stays on 2 pending the
  `Response::header` / `into_reader` migration (#31).
- **Breaking (internal API):** `harken::mcp::handle_line` is no longer public.
  It was an implementation detail of `serve()`, never used outside the crate,
  and its signature has to change for the protocol work. `serve()` is
  unaffected and remains the entry point.

## v0.5.0 — 2026-08-22

- The README and crate metadata now lead with the MCP server: harken's primary
  surface is "give your agent ears", with the batch CLI and the Claude Code
  skill as the other two faces of the same binary.
- **Relicensed from MIT to MIT OR Apache-2.0** — the Rust ecosystem's dual
  license, adding Apache-2.0's explicit patent grant. `LICENSE` became
  `LICENSE-MIT`, with `LICENSE-APACHE` alongside it.
- **New subcommand: `harken mcp`** — an MCP (Model Context Protocol) server
  over stdio, exposing `transcribe_file` and `transcribe_whatsapp_export` to
  any MCP client (Claude Code, Claude Desktop, Cursor, Windsurf). Protocol
  version 2025-06-18, hand-rolled JSON-RPC 2.0 on serde_json — zero new
  dependencies, no async runtime. Model/language/device are per-server flags
  with the CLI's defaults; one whisper context serves the whole session. The
  WhatsApp tool returns transcripts directly as content and writes no files.
  This is the one mode where stdout is written to (protocol frames only; logs
  stay on stderr).
- The build is now reproducible across machines: `rust-toolchain.toml` pins the
  compiler to 1.97.1 (excluded from the published crate, so `cargo install` is
  unaffected), and cargo-audit/cargo-machete are version-pinned. `clippy -D
  warnings` on a floating stable meant a new Rust release could redden the tree
  with no change of ours.
- CI gained two jobs: `msrv`, which checks the declared 1.88 floor still builds,
  and `deps`, which runs `cargo audit` and `cargo machete` — both were
  local-only gates before.
- The release path is now rehearsable: a pull request builds all five target
  platforms without publishing (`pr-run-mode = "upload"`). Until now the only
  way to exercise the cross-platform build was to push a `v*` tag, which is
  exactly how v0.4.0 died on the Windows build.

## v0.4.2 — 2026-08-22

- The crate is now **edition 2024** and declares an explicit **MSRV of 1.88**
  (`rust-version`). No behavior change: the code already used the 2024 form of
  `unsafe extern` and has no `unsafe fn` bodies, so the only practical effect is
  that let-chains became available — three nested `if`s in `src/whatsapp.rs`
  collapsed into them. 1.88 is the floor twice over: let-chains are not available
  before it, and the dependency graph (`icu_*`, via `url`) already required it.
  Older toolchains now get a clean MSRV message instead of a failure deep in a
  transitive dependency.
- **Fixed: a WhatsApp export with a bad date in a message header crashed
  instead of exiting cleanly.** The header patterns validate shape, not the
  calendar, so `31/02/2026`, `99/99/2026` or a non-ASCII digit panicked while
  parsing untrusted zip content. Such a line is now treated as what it is — not
  a header — and folded into the previous message as a continuation. It is also
  excluded from the day-first/month-first inference, where a single bad line
  used to be able to flip the date order of every valid message in the chat.
- **Fixed: an unreadable chat entry inside an otherwise valid zip panicked**
  instead of exiting `2` like every other input error. An attachment that
  cannot be read is now a warning and the rest of the batch continues.
- **New `--format md`**: a transcript meant to be read, with the source name as
  the title and one `[hh:mm:ss] text` line per segment.

## v0.4.1 — 2026-08-20

- Transcription now goes through FFI bindings kept in this repo (`src/ffi.rs`)
  against a vendored whisper.cpp pinned to **v1.7.6** (`vendor/whisper.cpp`
  submodule). `whisper-rs` is gone. Output is byte-identical to v0.3.1 on the
  same audio.
- Source builds no longer need **libclang**: `bindgen` left the build graph, and
  whisper.cpp no longer goes through CMake. The `CMAKE_POLICY_VERSION_MINIMUM`
  note now applies only to opus, on hosts without a system libopus.
- The build targets an explicit **AVX2/FMA/F16C floor** (Haswell 2013 and newer)
  instead of inheriting the build machine's instruction set, so released
  binaries stop depending on whatever CPU the CI runner had. `HARKEN_NATIVE=1`
  opts a source build into the host's full ISA. Measured within 3% of the v0.3.1
  engine on the same machine (five interleaved A/B pairs).
- New opt-in smoke test that loads a real whisper context through the FFI:
  `cargo test --test ffi_smoke_test -- --ignored`. The 78-test suite runs
  against a fake engine and cannot catch an FFI mistake.
- Building from a git checkout now needs `--recurse-submodules` (or
  `git submodule update --init --recursive`).
- The published package ships only the vendored sources the build compiles —
  upstream's examples, bindings and tests are excluded (1.3 MiB crate).
- (`v0.4.0` was tagged but never released: MSVC spells the C++ standard option
  `/std:c++17` and silently ignored the unix spelling, so the Windows build
  compiled as C++14 and failed on `std::filesystem`.)

## v0.3.1 — 2026-08-20

- First release published on crates.io: `cargo install harken` and
  `cargo binstall harken` now work.
- Explicit `exclude` list in `Cargo.toml` so local-only dirs never ship in
  the package.
- README: documented the `CMAKE_POLICY_VERSION_MINIMUM=3.5` workaround for
  source builds on CMake >= 4 hosts without system libopus.
- Project docs: `CLAUDE.md`, `docs/ARCHITECTURE.md`, CI skips
  documentation-only changes.

## v0.3.0 — 2026-08-20

- Full rewrite in Rust on whisper.cpp (whisper-rs). The Python tree is gone.
- Single static binary: in-process audio decode (opus, mp3, m4a, wav,
  flac, …) via symphonia/ogg/opus/rubato — no Python, no ffmpeg.
- Batch pipeline generic over the `Transcriber` trait; 78 offline
  integration tests ported from the Python suite.
- cargo-dist release pipeline: 5 targets (linux x64/arm64, macOS x64/arm64,
  windows x64) plus shell/powershell installers.
- Fix macOS release builds: CMake 4 rejects the vendored opus tree's
  declared minimum (`CMAKE_POLICY_VERSION_MINIMUM=3.5`).

## v0.2.0 — 2026-08-20 (Python)

- Renamed `hark` to `harken`.
- Android chat-export format support (auto-detected per chat, day-first vs
  month-first inferred from the chat itself).

## v0.1.0 — 2026-08-18 (Python)

- Initial release: local offline audio transcription CLI (faster-whisper),
  batch mode and WhatsApp chat-export mode.
