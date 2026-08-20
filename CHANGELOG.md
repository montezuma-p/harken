# Changelog

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
