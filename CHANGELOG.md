# Changelog

## Unreleased

- The build is now reproducible across machines: `rust-toolchain.toml` pins the
  compiler to 1.97.1 (excluded from the published crate, so `cargo install` is
  unaffected), and cargo-audit/cargo-machete are version-pinned. `clippy -D
  warnings` on a floating stable meant a new Rust release could redden the tree
  with no change of ours.
- CI gained two jobs: `msrv`, which checks the declared 1.88 floor still builds,
  and `deps`, which runs `cargo audit` and `cargo machete` — both were
  local-only gates before.

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
