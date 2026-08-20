//! Port of tests/test_batch.py: file collection and batch transcription.
//!
//! The Python suite asserted stderr messages via capsys; stderr is not
//! capturable in-process here, so those tests assert exit-relevant state
//! (BatchStats and filesystem effects) instead.

mod common;

use std::fs;
use std::path::PathBuf;

use harken::batch::{collect_audio_files, run_batch, BatchStats};
use harken::writers::OutputFormat;

use common::FakeEngine;

fn touch(path: &PathBuf) {
    fs::write(path, b"x").unwrap();
}

// --- collect_audio_files -----------------------------------------------

#[test]
fn collect_recurses_into_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("sub")).unwrap();
    let a = root.join("sub").join("a.mp3");
    touch(&a);
    let b = root.join("b.wav");
    touch(&b);
    fs::write(root.join("note.txt"), "not audio").unwrap();

    let result = collect_audio_files(&[root.to_string_lossy().into_owned()]).unwrap();

    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(result, expected);
}

#[test]
fn collect_expands_glob_and_filters_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let x = root.join("x.opus");
    touch(&x);
    let y = root.join("y.opus");
    touch(&y);
    fs::write(root.join("z.txt"), "not audio").unwrap();

    let pattern = root.join("*").to_string_lossy().into_owned();
    let result = collect_audio_files(&[pattern]).unwrap();

    let mut expected = vec![x, y];
    expected.sort();
    assert_eq!(result, expected);
}

#[test]
fn collect_includes_explicit_non_audio_file() {
    let tmp = tempfile::tempdir().unwrap();
    let weird = tmp.path().join("weird.xyz");
    touch(&weird);

    let result = collect_audio_files(&[weird.to_string_lossy().into_owned()]).unwrap();

    assert_eq!(result, vec![weird]);
}

#[test]
fn collect_dedups_file_reachable_two_ways() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let a = root.join("a.mp3");
    touch(&a);

    let result = collect_audio_files(&[
        root.to_string_lossy().into_owned(),
        a.to_string_lossy().into_owned(),
    ])
    .unwrap();

    assert_eq!(result, vec![a]);
}

#[test]
fn collect_missing_path_is_err_with_raw_path() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope.mp3");
    let raw = missing.to_string_lossy().into_owned();

    let err = collect_audio_files(std::slice::from_ref(&raw)).unwrap_err();

    assert_eq!(err, raw);
    assert!(err.contains("nope.mp3"));
}

// --- run_batch -----------------------------------------------------------

#[test]
fn run_batch_writes_output_and_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    let a = src.join("a.wav");
    touch(&a);
    let out_dir = tmp.path().join("out");
    let mut engine = FakeEngine::new(Some("pt".to_string()));

    let stats = run_batch(
        std::slice::from_ref(&a),
        &out_dir,
        &mut engine,
        OutputFormat::Txt,
        false,
    );

    assert_eq!(
        stats,
        BatchStats {
            total: 1,
            done: 1,
            skipped: 0,
            failed: 0
        }
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("a.txt")).unwrap(),
        "Hello world.\n"
    );
    let manifest = fs::read_to_string(out_dir.join("manifest.jsonl")).unwrap();
    let lines: Vec<&str> = manifest.lines().collect();
    assert_eq!(lines.len(), 1);
    let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["source"], a.display().to_string());
    assert_eq!(entry["output"], out_dir.join("a.txt").display().to_string());
    assert_eq!(entry["text"], "Hello world.");
}

#[test]
fn run_batch_skips_existing_output_without_force() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.wav");
    touch(&a);
    let out_dir = tmp.path().join("out");
    fs::create_dir(&out_dir).unwrap();
    fs::write(out_dir.join("a.txt"), "already here\n").unwrap();
    let mut engine = FakeEngine::new(None);

    let stats = run_batch(&[a], &out_dir, &mut engine, OutputFormat::Txt, false);

    assert_eq!(
        stats,
        BatchStats {
            total: 1,
            done: 0,
            skipped: 1,
            failed: 0
        }
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("a.txt")).unwrap(),
        "already here\n"
    );
    assert!(engine.calls.is_empty());
    assert!(!out_dir.join("manifest.jsonl").exists());
}

#[test]
fn run_batch_force_reprocesses_existing_output() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.wav");
    touch(&a);
    let out_dir = tmp.path().join("out");
    fs::create_dir(&out_dir).unwrap();
    fs::write(out_dir.join("a.txt"), "already here\n").unwrap();
    let mut engine = FakeEngine::new(None);

    let stats = run_batch(std::slice::from_ref(&a), &out_dir, &mut engine, OutputFormat::Txt, true);

    assert_eq!(
        stats,
        BatchStats {
            total: 1,
            done: 1,
            skipped: 0,
            failed: 0
        }
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("a.txt")).unwrap(),
        "Hello world.\n"
    );
    assert_eq!(engine.calls, vec![a]);
}

#[test]
fn run_batch_collision_suffix_independent_of_skip() {
    // Stem-collision numbering must not be perturbed by pre-existing outputs.
    //
    // First source (dir1/note.wav) maps to note.txt, which already exists on
    // disk -> skipped. The second source (dir2/note.wav) must still receive
    // the *next* free suffix (note-2.txt) and, since that path is free, must
    // be processed rather than itself being skipped or renamed further.
    let tmp = tempfile::tempdir().unwrap();
    let dir1 = tmp.path().join("dir1");
    let dir2 = tmp.path().join("dir2");
    fs::create_dir(&dir1).unwrap();
    fs::create_dir(&dir2).unwrap();
    let note1 = dir1.join("note.wav");
    touch(&note1);
    let note2 = dir2.join("note.wav");
    touch(&note2);
    let out_dir = tmp.path().join("out");
    fs::create_dir(&out_dir).unwrap();
    fs::write(out_dir.join("note.txt"), "pre-existing\n").unwrap();
    let mut engine = FakeEngine::new(None);

    let stats = run_batch(
        &[note1, note2.clone()],
        &out_dir,
        &mut engine,
        OutputFormat::Txt,
        false,
    );

    assert_eq!(
        stats,
        BatchStats {
            total: 2,
            done: 1,
            skipped: 1,
            failed: 0
        }
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("note.txt")).unwrap(),
        "pre-existing\n"
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("note-2.txt")).unwrap(),
        "Hello world.\n"
    );
    assert_eq!(engine.calls, vec![note2]);
}

#[test]
fn run_batch_continues_after_failure_and_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let good = tmp.path().join("good.wav");
    touch(&good);
    let bad = tmp.path().join("bad.wav");
    touch(&bad);
    let out_dir = tmp.path().join("out");
    let mut engine = FakeEngine::new(None).failing_on("bad.wav");

    let stats = run_batch(
        &[bad, good],
        &out_dir,
        &mut engine,
        OutputFormat::Txt,
        false,
    );

    assert_eq!(
        stats,
        BatchStats {
            total: 2,
            done: 1,
            skipped: 0,
            failed: 1
        }
    );
    assert!(out_dir.join("good.txt").exists());
    assert!(!out_dir.join("bad.txt").exists());
    let manifest = fs::read_to_string(out_dir.join("manifest.jsonl")).unwrap();
    assert_eq!(manifest.lines().count(), 1);
}

#[test]
fn run_batch_creates_out_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.wav");
    touch(&a);
    let out_dir = tmp.path().join("does").join("not").join("exist");
    let mut engine = FakeEngine::new(None);

    run_batch(&[a], &out_dir, &mut engine, OutputFormat::Txt, false);

    assert!(out_dir.is_dir());
}
