## What

<!-- What does this PR change, and why? -->

## Checklist

- [ ] `make check` passes locally (fmt + clippy + tests + audit + machete)
- [ ] If this changes behavior, the corresponding test in `tests/` was
      consciously updated (the tests are the spec)
- [ ] If `src/ffi.rs`, `build.rs`, or the whisper.cpp submodule changed:
      `cargo test --test ffi_smoke_test -- --ignored` was run
- [ ] If the CLI changed (flags, defaults, output layout):
      `.claude/skills/transcribe-audio/SKILL.md` and `README.md` are updated
