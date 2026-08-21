//! Opt-in smoke test for the real whisper.cpp FFI layer (`src/ffi.rs`).
//!
//! Every other test in this suite drives the pipeline through `FakeEngine`, so
//! nothing here exercises `WhisperCppEngine` — the struct layouts mirrored from
//! `whisper.h`, `whisper_full`, segment extraction, or the language lookup. A
//! mistake there compiles fine and only shows up at runtime, which is exactly
//! how a vendored-whisper.cpp bump can break the binary with green CI.
//!
//! This test is `#[ignore]`d: it never runs in CI and never downloads a model.
//! Run it by hand after touching `src/ffi.rs`, `build.rs`, or the submodule:
//!
//! ```bash
//! cargo test --test ffi_smoke_test -- --ignored --nocapture
//! ```
//!
//! It needs `ggml-tiny.bin` already in the model cache; without it, it skips.

use std::fs;
use std::path::PathBuf;

use harken::engine::{Transcriber, WhisperCppEngine};

/// Same cache location `harken::model` resolves to, without triggering a
/// download: an existing file path is used verbatim by `resolve_model`.
fn cached_tiny_model() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) if PathBuf::from(&dir).is_absolute() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    let model = base.join("harken/models/ggml-tiny.bin");
    model.exists().then_some(model)
}

/// Minimal 16 kHz mono 16-bit PCM WAV: 2 s of a quiet 220 Hz tone. The point is
/// not what whisper hears, it is that the FFI round-trip stays structurally
/// sane — no crash, plausible timestamps, a language string, one model load.
fn write_test_wav(dest: &std::path::Path) {
    const RATE: u32 = 16_000;
    const SECONDS: u32 = 2;
    let n_samples = RATE * SECONDS;
    let data_bytes = n_samples * 2;

    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());

    for i in 0..n_samples {
        let t = i as f32 / RATE as f32;
        let sample =
            (0.2 * (2.0 * std::f32::consts::PI * 220.0 * t).sin() * i16::MAX as f32) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }

    fs::write(dest, wav).expect("write test wav");
}

#[test]
#[ignore = "needs ggml-tiny.bin in the model cache; loads a real whisper context"]
fn real_ffi_transcribe_round_trip() {
    let Some(model) = cached_tiny_model() else {
        eprintln!("skipping: no ggml-tiny.bin in the model cache");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("tone.wav");
    write_test_wav(&audio);

    let mut engine = WhisperCppEngine::new(
        model.to_string_lossy().into_owned(),
        "cpu".to_string(),
        Some("pt".to_string()),
    );

    let result = engine.transcribe(&audio).expect("real transcription");

    assert_eq!(result.language, "pt", "requested language must be reported");
    assert!(
        (result.duration - 2.0).abs() < 0.1,
        "duration should reflect the 2 s input, got {}",
        result.duration
    );
    // Segments may legitimately be empty for a tone, but any segment that does
    // come back has to have sane, monotonic timestamps — that is the FFI
    // contract (centiseconds converted to seconds) most likely to break.
    let mut previous_end = 0.0;
    for segment in &result.segments {
        assert!(
            segment.start >= 0.0 && segment.end >= segment.start,
            "bad segment window: {:?}",
            segment
        );
        assert!(
            segment.start + 0.001 >= previous_end,
            "segments must not overlap or run backwards: {:?}",
            segment
        );
        previous_end = segment.end;
    }

    // A second call must reuse the already-loaded context, not reload it.
    engine
        .transcribe(&audio)
        .expect("second transcription on the reused context");
}
