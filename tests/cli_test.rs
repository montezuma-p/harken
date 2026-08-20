//! Port of tests/test_cli.py: argument parsing and batch-mode entry.
//!
//! Not ported from the Python suite:
//! - test_main_batch_mode_does_not_require_whatsapp_module: guarded a lazy
//!   Python import; Rust links everything statically, nothing to assert.
//! - stderr assertions (capsys): stderr is not capturable in-process; the
//!   equivalent tests assert exit codes and filesystem effects instead.

mod common;

use std::fs;

use clap::error::ErrorKind;
use clap::Parser;

use harken::batch::run_batch_mode;
use harken::cli::{language_option, Cli, Commands};
use harken::writers::OutputFormat;

use common::FakeEngine;

// --- parser ---------------------------------------------------------------

#[test]
fn parser_defaults() {
    let cli = Cli::try_parse_from(["harken", "a.wav"]).unwrap();

    assert!(cli.command.is_none());
    assert_eq!(cli.batch.inputs, vec!["a.wav"]);
    assert_eq!(cli.batch.out, "./transcripts");
    assert_eq!(cli.batch.model, "small");
    assert_eq!(cli.batch.lang, "pt");
    assert_eq!(cli.batch.format, OutputFormat::Txt);
    assert_eq!(cli.batch.device, "cpu");
    assert!(!cli.batch.force);
}

#[test]
fn parser_accepts_multiple_inputs_and_flags() {
    let cli = Cli::try_parse_from([
        "harken", "a.wav", "b.mp3", "--out", "/tmp/out", "--model", "medium", "--lang", "auto",
        "--format", "srt", "--device", "cuda", "--force",
    ])
    .unwrap();

    assert_eq!(cli.batch.inputs, vec!["a.wav", "b.mp3"]);
    assert_eq!(cli.batch.out, "/tmp/out");
    assert_eq!(cli.batch.model, "medium");
    assert_eq!(cli.batch.lang, "auto");
    assert_eq!(cli.batch.format, OutputFormat::Srt);
    assert_eq!(cli.batch.device, "cuda");
    assert!(cli.batch.force);
}

#[test]
fn parser_rejects_bad_format() {
    let result = Cli::try_parse_from(["harken", "a.wav", "--format", "mp3"]);

    assert!(result.is_err());
}

#[test]
fn parser_version_flag_displays_version() {
    let err = Cli::try_parse_from(["harken", "--version"]).unwrap_err();

    assert_eq!(err.kind(), ErrorKind::DisplayVersion);
}

// --- whatsapp subcommand ----------------------------------------------------

#[test]
fn parser_routes_whatsapp_subcommand_with_args() {
    let cli = Cli::try_parse_from(["harken", "whatsapp", "x.zip", "--from", "2026-01-01"]).unwrap();

    match cli.command {
        Some(Commands::Whatsapp(args)) => {
            assert_eq!(args.export_zip, "x.zip");
            assert_eq!(args.date_from.as_deref(), Some("2026-01-01"));
            assert_eq!(args.date_to, None);
            assert!(!args.merge);
            assert_eq!(args.format, OutputFormat::Txt);
        }
        other => panic!("expected whatsapp subcommand, got {other:?}"),
    }
}

// --- lang auto -> language=None ---------------------------------------------

#[test]
fn lang_auto_maps_to_none() {
    assert_eq!(language_option("auto"), None);
}

#[test]
fn default_lang_pt_passed_through() {
    let cli = Cli::try_parse_from(["harken", "a.wav"]).unwrap();

    assert_eq!(language_option(&cli.batch.lang), Some("pt".to_string()));
}

// --- run_batch_mode (main() equivalent) --------------------------------------

#[test]
fn batch_mode_end_to_end_success_writes_real_output() {
    // Drive real run_batch_mode -> real collect_audio_files -> real run_batch.
    // Only the engine is faked (no whisper.cpp); everything else -- file
    // collection, batch orchestration, writers -- runs for real against a
    // temp directory, so this exercises the actual wiring.
    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("sample.wav");
    fs::write(&audio, b"x").unwrap();
    let out_dir = tmp.path().join("out");
    let mut engine = FakeEngine::new(Some("pt".to_string()));

    let exit_code = run_batch_mode(
        &[audio.to_string_lossy().into_owned()],
        &out_dir.to_string_lossy(),
        OutputFormat::Txt,
        false,
        &mut engine,
    );

    assert_eq!(exit_code, 0);
    assert_eq!(
        fs::read_to_string(out_dir.join("sample.txt")).unwrap(),
        "Hello world.\n"
    );
    assert!(out_dir.join("manifest.jsonl").exists());
}

#[test]
fn batch_mode_returns_1_when_a_file_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("a.wav");
    fs::write(&audio, b"x").unwrap();
    let out_dir = tmp.path().join("out");
    let mut engine = FakeEngine::new(None).failing_on("a.wav");

    let exit_code = run_batch_mode(
        &[audio.to_string_lossy().into_owned()],
        &out_dir.to_string_lossy(),
        OutputFormat::Txt,
        false,
        &mut engine,
    );

    assert_eq!(exit_code, 1);
}

#[test]
fn batch_mode_returns_2_when_input_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope.mp3");
    let out_dir = tmp.path().join("out");
    let mut engine = FakeEngine::new(None);

    let exit_code = run_batch_mode(
        &[missing.to_string_lossy().into_owned()],
        &out_dir.to_string_lossy(),
        OutputFormat::Txt,
        false,
        &mut engine,
    );

    assert_eq!(exit_code, 2);
    assert!(engine.calls.is_empty());
}
