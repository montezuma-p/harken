//! Integration tests for harken::engine — ported from the Python suite's
//! tests/test_core.py. Only the portable contracts are here; lazy model
//! loading, WhisperModel constructor args, and compute_type defaults now
//! live in the real whisper.cpp engine (tested in M3).

mod common;

use std::path::Path;

use common::FakeEngine;
use harken::engine::{assemble_result, Segment, Transcriber};

/// Python: test_transcribe_raises_file_not_found_before_loading_model.
/// A missing audio file must fail before any transcription work happens.
#[test]
fn transcribe_errors_on_missing_file_before_any_work() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.opus");

    let mut engine = FakeEngine::new(None);
    let err = engine.transcribe(&missing).unwrap_err();

    assert!(
        err.to_string().contains("Audio file not found"),
        "unexpected error: {err}"
    );
    assert!(
        engine.calls.is_empty(),
        "no work should happen for a missing file"
    );
}

/// Python: test_transcribe_result_fields_with_detected_language.
/// With no forced language, the result carries the detected one ("en") and
/// all fields (source/text/segments/language/duration) are populated.
#[test]
fn transcribe_result_fields_with_detected_language() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("note.opus");
    std::fs::write(&audio, b"fake-audio").unwrap();

    let mut engine = FakeEngine::new(None);
    let result = engine.transcribe(&audio).unwrap();

    assert_eq!(result.source, audio);
    assert_eq!(result.text, "Hello world.");
    let texts: Vec<&str> = result.segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, ["Hello", "world."]);
    assert_eq!(result.segments[0].start, 0.0);
    assert_eq!(result.segments[0].end, 1.5);
    assert_eq!(result.language, "en");
    assert_eq!(result.duration, 3.0);
}

/// Python: test_transcribe_forces_language_when_set.
#[test]
fn transcribe_forces_language_when_set() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("note.opus");
    std::fs::write(&audio, b"fake-audio").unwrap();

    let mut engine = FakeEngine::new(Some("pt".to_string()));
    let result = engine.transcribe(&audio).unwrap();

    assert_eq!(result.language, "pt");
}

/// Python: test_transcribe_strips_leading_space_from_segment_text.
/// Whisper segments carry a leading space (tokenizer artifact); the joined
/// text and the per-segment cues must not inherit it.
#[test]
fn assemble_result_strips_leading_space_from_segment_text() {
    let source = Path::new("note.opus");
    let segments = vec![
        Segment {
            start: 0.0,
            end: 1.0,
            text: " Hello".to_string(),
        },
        Segment {
            start: 1.0,
            end: 2.0,
            text: " world.".to_string(),
        },
    ];

    let result = assemble_result(source, segments, "en".to_string(), 2.0);

    assert_eq!(result.text, "Hello world.");
    assert_eq!(result.segments[0].text, "Hello");
    assert_eq!(result.segments[1].text, "world.");
}
