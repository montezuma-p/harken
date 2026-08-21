//! Integration tests for harken::writers — ported from the Python suite's
//! tests/test_writers.py. Byte-exact contracts for txt and srt; parsed-value
//! contracts for json and the manifest, mirroring the Python assertions.

use std::path::Path;

use harken::engine::{Segment, TranscriptionResult};
use harken::writers::{append_manifest, write_json, write_md, write_srt, write_txt};
use serde_json::{json, Value};

/// Python fixture `sample_result`.
fn sample_result(dir: &Path) -> TranscriptionResult {
    TranscriptionResult {
        source: dir.join("note.opus"),
        text: "Hello world.".to_string(),
        segments: vec![
            Segment {
                start: 0.0,
                end: 1.5,
                text: "Hello".to_string(),
            },
            Segment {
                start: 1.5,
                end: 3.75,
                text: "world.".to_string(),
            },
        ],
        language: "en".to_string(),
        duration: 3.75,
    }
}

/// Python: test_write_txt_is_plain_text_with_trailing_newline.
#[test]
fn write_txt_is_plain_text_with_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let result = sample_result(dir.path());
    let dest = dir.path().join("note.txt");

    write_txt(&result, &dest).unwrap();

    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "Hello world.\n");
}

/// Python: test_write_json_matches_expected_shape.
#[test]
fn write_json_matches_expected_shape() {
    let dir = tempfile::tempdir().unwrap();
    let result = sample_result(dir.path());
    let dest = dir.path().join("note.json");

    write_json(&result, &dest).unwrap();

    let raw = std::fs::read_to_string(&dest).unwrap();
    assert!(raw.ends_with('\n'), "JSON output must end with a newline");
    let data: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        data,
        json!({
            "source": result.source.display().to_string(),
            "language": "en",
            "duration": 3.75,
            "text": "Hello world.",
            "segments": [
                {"start": 0.0, "end": 1.5, "text": "Hello"},
                {"start": 1.5, "end": 3.75, "text": "world."},
            ],
        })
    );
}

/// Python: test_write_srt_uses_standard_numbering_and_timestamps.
#[test]
fn write_srt_uses_standard_numbering_and_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let result = sample_result(dir.path());
    let dest = dir.path().join("note.srt");

    write_srt(&result, &dest).unwrap();

    let expected = "1\n\
                    00:00:00,000 --> 00:00:01,500\n\
                    Hello\n\
                    \n\
                    2\n\
                    00:00:01,500 --> 00:00:03,750\n\
                    world.\n\
                    \n";
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), expected);
}

/// Python: test_write_srt_formats_hour_and_millisecond_boundaries.
#[test]
fn write_srt_formats_hour_and_millisecond_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let result = TranscriptionResult {
        source: dir.path().join("long.opus"),
        text: "late segment".to_string(),
        segments: vec![Segment {
            start: 3661.234,
            end: 3662.0,
            text: "late segment".to_string(),
        }],
        language: "en".to_string(),
        duration: 3662.0,
    };
    let dest = dir.path().join("long.srt");

    write_srt(&result, &dest).unwrap();

    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "1\n01:01:01,234 --> 01:01:02,000\nlate segment\n\n"
    );
}

/// New in the Rust port: `--format md`, a reading transcript with per-segment
/// timestamps. Byte-exact, like txt and srt.
#[test]
fn write_md_titles_by_stem_and_prefixes_each_segment_with_a_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let result = sample_result(dir.path());
    let dest = dir.path().join("note.md");

    write_md(&result, &dest).unwrap();

    let expected = "# note\n\
                    \n\
                    [00:00:00] Hello\n\
                    [00:00:01] world.\n";
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), expected);
}

#[test]
fn write_md_formats_hour_boundaries_and_truncates_to_the_second() {
    let dir = tempfile::tempdir().unwrap();
    let result = TranscriptionResult {
        source: dir.path().join("long.opus"),
        text: "late segment".to_string(),
        segments: vec![Segment {
            start: 3661.987,
            end: 3662.0,
            text: "late segment".to_string(),
        }],
        language: "en".to_string(),
        duration: 3662.0,
    };
    let dest = dir.path().join("long.md");

    write_md(&result, &dest).unwrap();

    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "# long\n\n[01:01:01] late segment\n"
    );
}

#[test]
fn write_md_with_no_segments_is_just_the_title() {
    let dir = tempfile::tempdir().unwrap();
    let result = TranscriptionResult {
        source: dir.path().join("silent.opus"),
        text: String::new(),
        segments: Vec::new(),
        language: "en".to_string(),
        duration: 0.0,
    };
    let dest = dir.path().join("silent.md");

    write_md(&result, &dest).unwrap();

    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "# silent\n\n");
}

/// Python: test_append_manifest_writes_one_json_line.
#[test]
fn append_manifest_writes_one_json_line() {
    let dir = tempfile::tempdir().unwrap();
    let result = sample_result(dir.path());
    let manifest = dir.path().join("manifest.jsonl");
    let output_file = dir.path().join("note.txt");

    append_manifest(&manifest, &result, &output_file).unwrap();

    let content = std::fs::read_to_string(&manifest).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);
    let entry: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(
        entry,
        json!({
            "source": result.source.display().to_string(),
            "output": output_file.display().to_string(),
            "language": "en",
            "duration": 3.75,
            "text": "Hello world.",
        })
    );
}

/// Python: test_append_manifest_appends_across_calls.
#[test]
fn append_manifest_appends_across_calls() {
    let dir = tempfile::tempdir().unwrap();
    let result = sample_result(dir.path());
    let manifest = dir.path().join("manifest.jsonl");

    append_manifest(&manifest, &result, &dir.path().join("a.txt")).unwrap();
    append_manifest(&manifest, &result, &dir.path().join("b.txt")).unwrap();

    let content = std::fs::read_to_string(&manifest).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(
        first["output"],
        dir.path().join("a.txt").display().to_string().as_str()
    );
    assert_eq!(
        second["output"],
        dir.path().join("b.txt").display().to_string().as_str()
    );
}
