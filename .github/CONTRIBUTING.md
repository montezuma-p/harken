# Contributing to harken

Thanks for taking the time. This is a small, deliberately low-maintenance
project — small focused PRs are the easiest to review and merge.

## Building

```sh
git clone https://github.com/montezuma-p/harken
cd harken
git submodule update --init --recursive   # vendor/whisper.cpp — required
cargo build
```

You need a C++ toolchain: `build.rs` compiles the vendored whisper.cpp sources
directly (no CMake). On Linux, `gcc-c++`/`g++` is enough; libopus links
dynamically if pkg-config finds it, otherwise a vendored copy is built.

Toolchain: `rust-toolchain.toml` pins the compiler used in CI. MSRV is **1.88**
(edition 2024) and is checked by a dedicated CI job — don't use features newer
than that in `src/`.

## Before opening a PR

Run the full local CI:

```sh
make check   # fmt + clippy -D warnings + test + cargo audit + cargo machete
```

(`make install-dev-tools` installs the pinned cargo-audit/cargo-machete.)

If you touched `src/ffi.rs`, `build.rs`, or the whisper.cpp submodule pin, also
run the opt-in FFI smoke test — the regular suite uses a fake engine and cannot
catch FFI mistakes:

```sh
cargo test --test ffi_smoke_test -- --ignored
```

## Ground rules

- **The integration tests in `tests/` are the behavior spec.** A behavior
  change must consciously change the corresponding test — never adjust a test
  just to make code pass.
- **stdout stays clean.** Transcripts only. All progress, logs, and summaries
  go to stderr.
- **Exit codes:** `0` ok (skips are not failures), `1` at least one
  transcription failed, `2` input error.
- **Tests stay offline.** Never add a test that downloads a model, loads a real
  whisper context, or touches the network — use `FakeEngine`
  (`tests/common/mod.rs`).
- **`.github/workflows/release.yml` is generated** by `dist generate`. Never
  edit it by hand; change `dist-workspace.toml` and regenerate.
- Any CLI change (flags, defaults, output layout) must be reflected in
  `.claude/skills/transcribe-audio/SKILL.md` and `README.md` in the same PR.

See `docs/ARCHITECTURE.md` for the architecture map and `CLAUDE.md` for the
full set of project rules.
