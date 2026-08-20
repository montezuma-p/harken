//! Tests for harken::whatsapp: chat-export parsing and transcription flow.
//!
//! Port of tests/test_whatsapp.py (the Python suite is the behavior spec).

mod common;

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use clap::Parser as _;

use common::FakeEngine;
use harken::cli::{Cli, Commands, WhatsappArgs};
use harken::engine::{EngineError, Segment, Transcriber, TranscriptionResult};
use harken::whatsapp::{
    build_merged_chat, default_out_dir, extract_attachment, find_attachment_member,
    find_chat_entry, is_audio_attachment, parse_chat, run, select_audio_messages, Message,
};
use harken::writers::OutputFormat;

const U200E: &str = "\u{200E}";

fn d(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// --- find_chat_entry -------------------------------------------------------

#[test]
fn test_find_chat_entry_prefers_name_ending_in_chat_txt() {
    let names = names(&[
        "WhatsApp Chat with Bob/_chat.txt",
        "WhatsApp Chat with Bob/00001.opus",
    ]);

    assert_eq!(
        find_chat_entry(&names).unwrap(),
        "WhatsApp Chat with Bob/_chat.txt"
    );
}

#[test]
fn test_find_chat_entry_falls_back_to_single_root_txt() {
    let names = names(&["notes.txt", "00001.opus"]);

    assert_eq!(find_chat_entry(&names).unwrap(), "notes.txt");
}

#[test]
fn test_find_chat_entry_errors_when_ambiguous() {
    let names = names(&["a.txt", "b.txt"]);

    assert!(find_chat_entry(&names).is_err());
}

#[test]
fn test_find_chat_entry_errors_when_none_found() {
    assert!(find_chat_entry(&names(&["00001.opus"])).is_err());
}

// --- find_attachment_member --------------------------------------------------

#[test]
fn test_find_attachment_member_exact_match() {
    let names = names(&["_chat.txt", "00001.opus"]);

    assert_eq!(
        find_attachment_member(&names, "00001.opus"),
        Some("00001.opus".to_string())
    );
}

#[test]
fn test_find_attachment_member_nested_path() {
    let names = names(&["export/_chat.txt", "export/00001.opus"]);

    assert_eq!(
        find_attachment_member(&names, "00001.opus"),
        Some("export/00001.opus".to_string())
    );
}

#[test]
fn test_find_attachment_member_missing_returns_none() {
    let names = names(&["_chat.txt"]);

    assert_eq!(find_attachment_member(&names, "00001.opus"), None);
}

// --- extract_attachment ------------------------------------------------------

#[test]
fn test_extract_attachment_anexado() {
    assert_eq!(
        extract_attachment("\u{200E}<anexado: 00001-AUDIO.opus>"),
        Some("00001-AUDIO.opus")
    );
}

#[test]
fn test_extract_attachment_attached() {
    assert_eq!(
        extract_attachment("<attached: 00001-AUDIO.opus>"),
        Some("00001-AUDIO.opus")
    );
}

#[test]
fn test_extract_attachment_none_when_absent() {
    assert_eq!(extract_attachment("fica no aguardo ai"), None);
}

// --- is_audio_attachment ------------------------------------------------------

#[test]
fn test_is_audio_attachment_true_for_opus() {
    assert!(is_audio_attachment("00001-AUDIO.opus"));
}

#[test]
fn test_is_audio_attachment_false_for_image() {
    assert!(!is_audio_attachment("00004-IMG.jpg"));
}

// --- parse_chat ---------------------------------------------------------------

#[test]
fn test_parse_chat_single_message() {
    let text = format!("{U200E}[10/07/2026, 09:00:00] Alice: fica no aguardo ai");

    let messages = parse_chat(&text);

    assert_eq!(
        messages,
        vec![Message {
            date: d(2026, 7, 10),
            time: "09:00:00".to_string(),
            sender: "Alice".to_string(),
            body: "fica no aguardo ai".to_string(),
            line_index: 0,
        }]
    );
}

#[test]
fn test_parse_chat_strips_invisible_marks_around_attachment() {
    let text = format!("{U200E}[13/07/2026, 21:04:39] Bob: {U200E}<anexado: 00001935-AUDIO.opus>");

    let messages = parse_chat(&text);

    // matching (and the captured groups) happen on the invisible-mark-stripped
    // line, so the parsed body is clean even though the raw line wasn't.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "<anexado: 00001935-AUDIO.opus>");
    assert_eq!(
        extract_attachment(&messages[0].body),
        Some("00001935-AUDIO.opus")
    );
}

#[test]
fn test_parse_chat_joins_continuation_lines() {
    let text = "[11/07/2026, 10:00:00] Bob: primeira parte\nsegunda parte (continuacao)";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].body,
        "primeira parte\nsegunda parte (continuacao)"
    );
}

#[test]
fn test_parse_chat_leading_system_line_is_ignored() {
    let text = "Messages and calls are end-to-end encrypted.\n[10/07/2026, 09:00:00] Alice: oi";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].line_index, 1);
}

// --- parse_chat: Android format -------------------------------------------------

#[test]
fn test_parse_chat_android_single_message() {
    let text = "10/07/2026, 21:04 - Bob: fica no aguardo ai";

    let messages = parse_chat(text);

    assert_eq!(
        messages,
        vec![Message {
            date: d(2026, 7, 10),
            time: "21:04".to_string(),
            sender: "Bob".to_string(),
            body: "fica no aguardo ai".to_string(),
            line_index: 0,
        }]
    );
}

#[test]
fn test_parse_chat_android_without_comma_after_date() {
    let text = "10/07/2026 21:04 - Bob: oi";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].date, d(2026, 7, 10));
    assert_eq!(messages[0].sender, "Bob");
}

#[test]
fn test_parse_chat_android_two_digit_year() {
    let text = "10/07/26, 21:04 - Bob: oi";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].date, d(2026, 7, 10));
}

#[test]
fn test_parse_chat_android_12h_time_with_narrow_nbsp() {
    let narrow_nbsp = "\u{202F}";
    let text = format!("13/07/26, 9:05{narrow_nbsp}PM - Bob: hi");

    let messages = parse_chat(&text);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].date, d(2026, 7, 13));
    assert_eq!(messages[0].time, format!("9:05{narrow_nbsp}PM"));
}

#[test]
fn test_parse_chat_android_day_first_when_first_component_exceeds_12() {
    let text = "10/07/2026, 21:04 - Bob: oi\n13/07/2026, 21:05 - Bob: tudo certo";

    let messages = parse_chat(text);

    let dates: Vec<NaiveDate> = messages.iter().map(|m| m.date).collect();
    assert_eq!(dates, vec![d(2026, 7, 10), d(2026, 7, 13)]);
}

#[test]
fn test_parse_chat_android_month_first_when_second_component_exceeds_12() {
    let text = "7/10/26, 9:04 PM - Bob: hi\n7/13/26, 9:05 PM - Bob: all good";

    let messages = parse_chat(text);

    let dates: Vec<NaiveDate> = messages.iter().map(|m| m.date).collect();
    assert_eq!(dates, vec![d(2026, 7, 10), d(2026, 7, 13)]);
}

#[test]
fn test_parse_chat_android_ambiguous_dates_default_day_first() {
    let text = "05/07/2026, 21:04 - Bob: oi";

    let messages = parse_chat(text);

    assert_eq!(messages[0].date, d(2026, 7, 5));
}

#[test]
fn test_parse_chat_android_joins_continuation_lines() {
    let text = "11/07/2026, 10:00 - Bob: primeira parte\nsegunda parte (continuacao)";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].body,
        "primeira parte\nsegunda parte (continuacao)"
    );
}

#[test]
fn test_parse_chat_android_leading_system_line_is_ignored() {
    let text = "10/07/2026, 21:00 - Messages and calls are end-to-end encrypted. \
                Tap to learn more.\n\
                10/07/2026, 21:04 - Bob: oi";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, "Bob");
    assert_eq!(messages[0].line_index, 1);
}

#[test]
fn test_parse_chat_does_not_mix_formats() {
    // Format is detected once per chat: after an android header, an
    // iOS-style line in a message body is a continuation, not a header.
    let text = "10/07/2026, 21:04 - Bob: olha esse formato:\n\
                [10/07/2026, 09:00:00] Alice: nao sou uma mensagem";

    let messages = parse_chat(text);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, "Bob");
    assert_eq!(messages[0].line_index, 0);
    assert!(messages[0].body.contains("nao sou uma mensagem"));
}

// --- extract_attachment: Android format ------------------------------------------

#[test]
fn test_extract_attachment_android_file_attached() {
    assert_eq!(
        extract_attachment("PTT-20260710-WA0001.opus (file attached)"),
        Some("PTT-20260710-WA0001.opus")
    );
}

#[test]
fn test_extract_attachment_android_arquivo_anexado() {
    assert_eq!(
        extract_attachment("PTT-20260710-WA0001.opus (arquivo anexado)"),
        Some("PTT-20260710-WA0001.opus")
    );
}

#[test]
fn test_extract_attachment_android_media_omitted_returns_none() {
    assert_eq!(extract_attachment("<Media omitted>"), None);
}

// --- select_audio_messages -----------------------------------------------------

fn msg(day: u32, filename: Option<&str>) -> Message {
    let body = match filename {
        Some(f) => format!("<anexado: {f}>"),
        None => "texto qualquer".to_string(),
    };
    Message {
        date: d(2026, 7, day),
        time: "09:00:00".to_string(),
        sender: "Bob".to_string(),
        body,
        line_index: day as usize,
    }
}

#[test]
fn test_select_audio_messages_filters_non_attachment() {
    let messages = vec![msg(10, None)];

    assert!(select_audio_messages(&messages, None, None).is_empty());
}

#[test]
fn test_select_audio_messages_filters_non_audio_attachment() {
    let messages = vec![msg(10, Some("photo.jpg"))];

    assert!(select_audio_messages(&messages, None, None).is_empty());
}

#[test]
fn test_select_audio_messages_inclusive_date_bounds() {
    let messages = vec![
        msg(9, Some("a.opus")),
        msg(10, Some("b.opus")),
        msg(12, Some("c.opus")),
        msg(13, Some("d.opus")),
    ];

    let selected = select_audio_messages(&messages, Some(d(2026, 7, 10)), Some(d(2026, 7, 12)));

    let bodies: Vec<&str> = selected.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, vec!["<anexado: b.opus>", "<anexado: c.opus>"]);
}

#[test]
fn test_select_audio_messages_no_bounds_selects_all_audio() {
    let messages = vec![msg(9, Some("a.opus")), msg(20, Some("b.opus"))];

    let selected = select_audio_messages(&messages, None, None);

    assert_eq!(selected.len(), 2);
}

// --- default_out_dir -----------------------------------------------------------

#[test]
fn test_default_out_dir_from_zip_stem() {
    assert_eq!(
        default_out_dir(Path::new("chat_export.zip")),
        PathBuf::from("./chat_export-transcripts")
    );
}

// --- build_merged_chat ----------------------------------------------------------

#[test]
fn test_build_merged_chat_inserts_transcript_after_attachment_line_only() {
    let raw_text = "[10/07/2026, 09:00:00] Alice: oi\n\
                    [10/07/2026, 09:05:00] Bob: <anexado: a.opus>\n\
                    [20/07/2026, 09:00:00] Bob: <anexado: b.opus>\n";
    let messages = parse_chat(raw_text);
    let audio_dir = Path::new("/tmp/out/audio");
    let mut transcripts = HashMap::new();
    transcripts.insert(
        audio_dir.join("a.opus").display().to_string(),
        "oi tudo bem".to_string(),
    );

    let merged = build_merged_chat(raw_text, &messages, &transcripts, audio_dir);

    let lines: Vec<&str> = merged.lines().collect();
    assert_eq!(lines[0], "[10/07/2026, 09:00:00] Alice: oi");
    assert_eq!(lines[1], "[10/07/2026, 09:05:00] Bob: <anexado: a.opus>");
    assert_eq!(lines[2], "    >> [transcript] oi tudo bem");
    assert_eq!(lines[3], "[20/07/2026, 09:00:00] Bob: <anexado: b.opus>");
    assert_eq!(lines.len(), 4);
}

#[test]
fn test_build_merged_chat_normalizes_attachment_path_component() {
    // The marker filename can carry a path component (e.g. `media/foo.opus`)
    // while transcripts are keyed by `audio_dir / <basename>` (extraction
    // strips the path). Lookup must normalize the same way so the transcript
    // still gets inlined.
    let raw_text = "[10/07/2026, 09:05:00] Bob: <anexado: media/a.opus>\n";
    let messages = parse_chat(raw_text);
    let audio_dir = Path::new("/tmp/out/audio");
    let mut transcripts = HashMap::new();
    transcripts.insert(
        audio_dir.join("a.opus").display().to_string(),
        "oi tudo bem".to_string(),
    );

    let merged = build_merged_chat(raw_text, &messages, &transcripts, audio_dir);

    let lines: Vec<&str> = merged.lines().collect();
    assert_eq!(
        lines[0],
        "[10/07/2026, 09:05:00] Bob: <anexado: media/a.opus>"
    );
    assert_eq!(lines[1], "    >> [transcript] oi tudo bem");
}

#[test]
fn test_build_merged_chat_preserves_continuation_lines_untouched() {
    let raw_text = "[11/07/2026, 10:00:00] Bob: primeira parte\n\
                    segunda parte (continuacao)\n";
    let messages = parse_chat(raw_text);

    let merged = build_merged_chat(
        raw_text,
        &messages,
        &HashMap::new(),
        Path::new("/tmp/out/audio"),
    );

    assert_eq!(merged, raw_text);
}

// --- CLI parser (build_parser equivalent) -----------------------------------------

fn parse_whatsapp_args(argv: &[&str]) -> WhatsappArgs {
    let mut full = vec!["harken", "whatsapp"];
    full.extend_from_slice(argv);
    let cli = Cli::try_parse_from(full).expect("argv parses");
    match cli.command {
        Some(Commands::Whatsapp(args)) => args,
        other => panic!("expected whatsapp subcommand, got {other:?}"),
    }
}

#[test]
fn test_parser_defaults() {
    let args = parse_whatsapp_args(&["export.zip"]);

    assert_eq!(args.export_zip, "export.zip");
    assert_eq!(args.out, None);
    assert_eq!(args.date_from, None);
    assert_eq!(args.date_to, None);
    assert!(!args.merge);
    assert_eq!(args.model, "small");
    assert_eq!(args.lang, "pt");
    assert_eq!(args.device, "cpu");
    assert!(!args.force);
}

#[test]
fn test_parser_accepts_all_flags() {
    let args = parse_whatsapp_args(&[
        "export.zip",
        "--out",
        "/tmp/out",
        "--from",
        "2026-07-01",
        "--to",
        "2026-07-31",
        "--merge",
        "--model",
        "medium",
        "--lang",
        "auto",
        "--device",
        "cuda",
        "--force",
    ]);

    assert_eq!(args.out.as_deref(), Some("/tmp/out"));
    assert_eq!(args.date_from.as_deref(), Some("2026-07-01"));
    assert_eq!(args.date_to.as_deref(), Some("2026-07-31"));
    assert!(args.merge);
    assert_eq!(args.model, "medium");
    assert_eq!(args.lang, "auto");
    assert_eq!(args.device, "cuda");
    assert!(args.force);
}

// --- run(): end-to-end with a synthetic zip ---------------------------------------

fn write_zip(zip_path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(zip_path).expect("create zip");
    let mut zw = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        zw.start_file(*name, options).expect("start zip entry");
        zw.write_all(data).expect("write zip entry");
    }
    zw.finish().expect("finish zip");
}

fn build_export_zip(zip_path: &Path) {
    let chat_text = format!(
        "Messages and calls are end-to-end encrypted.\n\
         {U200E}[10/07/2026, 09:00:00] Alice: fica no aguardo ai\n\
         {U200E}[10/07/2026, 09:05:00] Bob: {U200E}<anexado: 00001-AUDIO-2026-07-10.opus>\n\
         [11/07/2026, 10:00:00] Bob: primeira parte\n\
         segunda parte (continuacao)\n\
         {U200E}[12/07/2026, 08:00:00] Alice: {U200E}<anexado: 00002-AUDIO-2026-07-12.opus>\n\
         {U200E}[20/07/2026, 08:00:00] Bob: {U200E}<anexado: 00003-AUDIO-2026-07-20.opus>\n\
         {U200E}[10/07/2026, 09:10:00] Bob: {U200E}<anexado: 00004-IMG-2026-07-10.jpg>\n"
    );
    write_zip(
        zip_path,
        &[
            ("_chat.txt", chat_text.as_bytes()),
            ("00001-AUDIO-2026-07-10.opus", b"fake-audio-1"),
            ("00002-AUDIO-2026-07-12.opus", b"fake-audio-2"),
            ("00003-AUDIO-2026-07-20.opus", b"fake-audio-3"),
            ("00004-IMG-2026-07-10.jpg", b"fake-image"),
        ],
    );
}

fn build_export_zip_android(zip_path: &Path) {
    let chat_text = "10/07/2026, 08:55 - Messages and calls are end-to-end encrypted. \
                     Tap to learn more.\n\
                     10/07/2026, 09:00 - Alice: fica no aguardo ai\n\
                     10/07/2026, 09:05 - Bob: PTT-20260710-WA0001.opus (file attached)\n\
                     11/07/2026, 10:00 - Bob: primeira parte\n\
                     segunda parte (continuacao)\n\
                     12/07/2026, 08:00 - Alice: PTT-20260712-WA0002.opus (arquivo anexado)\n\
                     20/07/2026, 08:00 - Bob: PTT-20260720-WA0003.opus (file attached)\n\
                     10/07/2026, 09:10 - Bob: IMG-20260710-WA0004.jpg (file attached)\n\
                     10/07/2026, 09:11 - Bob: <Media omitted>\n";
    write_zip(
        zip_path,
        &[
            ("WhatsApp Chat with Bob.txt", chat_text.as_bytes()),
            ("PTT-20260710-WA0001.opus", b"fake-audio-1"),
            ("PTT-20260712-WA0002.opus", b"fake-audio-2"),
            ("PTT-20260720-WA0003.opus", b"fake-audio-3"),
            ("IMG-20260710-WA0004.jpg", b"fake-image"),
        ],
    );
}

/// Rust equivalent of the Python suite's FakeTranscriber: returns
/// "transcript of {filename}" so merge assertions can match per-file text.
struct NameTranscriber;

impl Transcriber for NameTranscriber {
    fn transcribe(&mut self, path: &Path) -> Result<TranscriptionResult, EngineError> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = format!("transcript of {name}");
        Ok(TranscriptionResult {
            source: path.to_path_buf(),
            text: text.clone(),
            segments: vec![Segment {
                start: 0.0,
                end: 1.0,
                text,
            }],
            language: "pt".to_string(),
            duration: 1.0,
        })
    }
}

fn wa_args(
    zip_path: &Path,
    out: Option<&Path>,
    from: Option<&str>,
    to: Option<&str>,
    merge: bool,
) -> WhatsappArgs {
    WhatsappArgs {
        export_zip: zip_path.display().to_string(),
        out: out.map(|p| p.display().to_string()),
        date_from: from.map(str::to_string),
        date_to: to.map(str::to_string),
        merge,
        format: OutputFormat::Txt,
        model: "small".to_string(),
        lang: "pt".to_string(),
        device: "cpu".to_string(),
        force: false,
    }
}

fn index_of(lines: &[&str], target: &str) -> usize {
    lines
        .iter()
        .position(|l| *l == target)
        .unwrap_or_else(|| panic!("line not found: {target:?}"))
}

#[test]
fn test_run_end_to_end_selects_extracts_transcribes_and_merges() {
    // Drives the real run() -> real zip parsing -> real run_batch chain.
    // Only the Transcriber is faked; collection, extraction, batch
    // orchestration, writers, and merge all run for real.
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("chat_export.zip");
    build_export_zip(&zip_path);
    let out_dir = tmp.path().join("out");

    let args = wa_args(
        &zip_path,
        Some(&out_dir),
        Some("2026-07-10"),
        Some("2026-07-12"),
        true,
    );
    let exit_code = run(&args, &mut NameTranscriber);

    assert_eq!(exit_code, 0);

    let audio_dir = out_dir.join("audio");
    assert_eq!(
        std::fs::read(audio_dir.join("00001-AUDIO-2026-07-10.opus")).unwrap(),
        b"fake-audio-1"
    );
    assert_eq!(
        std::fs::read(audio_dir.join("00002-AUDIO-2026-07-12.opus")).unwrap(),
        b"fake-audio-2"
    );
    assert!(!audio_dir.join("00003-AUDIO-2026-07-20.opus").exists());
    assert!(!audio_dir.join("00004-IMG-2026-07-10.jpg").exists());

    assert_eq!(
        std::fs::read_to_string(out_dir.join("00001-AUDIO-2026-07-10.txt")).unwrap(),
        "transcript of 00001-AUDIO-2026-07-10.opus\n"
    );
    assert_eq!(
        std::fs::read_to_string(out_dir.join("00002-AUDIO-2026-07-12.txt")).unwrap(),
        "transcript of 00002-AUDIO-2026-07-12.opus\n"
    );

    let manifest = std::fs::read_to_string(out_dir.join("manifest.jsonl")).unwrap();
    let manifest_lines: Vec<&str> = manifest.lines().collect();
    assert_eq!(manifest_lines.len(), 2);
    let sources: std::collections::HashSet<String> = manifest_lines
        .iter()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["source"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    let expected: std::collections::HashSet<String> = [
        audio_dir.join("00001-AUDIO-2026-07-10.opus"),
        audio_dir.join("00002-AUDIO-2026-07-12.opus"),
    ]
    .iter()
    .map(|p| p.display().to_string())
    .collect();
    assert_eq!(sources, expected);

    let merged = std::fs::read_to_string(out_dir.join("_chat.transcribed.txt")).unwrap();
    let merged_lines: Vec<&str> = merged.lines().collect();
    let idx_a = index_of(
        &merged_lines,
        &format!(
            "{U200E}[10/07/2026, 09:05:00] Bob: {U200E}<anexado: 00001-AUDIO-2026-07-10.opus>"
        ),
    );
    assert_eq!(
        merged_lines[idx_a + 1],
        "    >> [transcript] transcript of 00001-AUDIO-2026-07-10.opus"
    );
    let idx_b = index_of(
        &merged_lines,
        &format!(
            "{U200E}[12/07/2026, 08:00:00] Alice: {U200E}<anexado: 00002-AUDIO-2026-07-12.opus>"
        ),
    );
    assert_eq!(
        merged_lines[idx_b + 1],
        "    >> [transcript] transcript of 00002-AUDIO-2026-07-12.opus"
    );
    let idx_c = index_of(
        &merged_lines,
        &format!(
            "{U200E}[20/07/2026, 08:00:00] Bob: {U200E}<anexado: 00003-AUDIO-2026-07-20.opus>"
        ),
    );
    // out-of-range attachment stays untouched: no transcript line follows it
    assert_eq!(
        merged_lines[idx_c + 1],
        format!("{U200E}[10/07/2026, 09:10:00] Bob: {U200E}<anexado: 00004-IMG-2026-07-10.jpg>")
    );
    // continuation line preserved verbatim
    assert!(merged_lines.contains(&"segunda parte (continuacao)"));
}

#[test]
fn test_run_end_to_end_android_export() {
    // Android-format zip through the real run(): chat located via the
    // single-root-txt fallback, android headers parsed, `(file attached)`
    // markers extracted, date filter applied, merge inlined.
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("chat_export.zip");
    build_export_zip_android(&zip_path);
    let out_dir = tmp.path().join("out");

    let args = wa_args(
        &zip_path,
        Some(&out_dir),
        Some("2026-07-10"),
        Some("2026-07-12"),
        true,
    );
    let exit_code = run(&args, &mut NameTranscriber);

    assert_eq!(exit_code, 0);

    let audio_dir = out_dir.join("audio");
    assert_eq!(
        std::fs::read(audio_dir.join("PTT-20260710-WA0001.opus")).unwrap(),
        b"fake-audio-1"
    );
    assert_eq!(
        std::fs::read(audio_dir.join("PTT-20260712-WA0002.opus")).unwrap(),
        b"fake-audio-2"
    );
    assert!(!audio_dir.join("PTT-20260720-WA0003.opus").exists());
    assert!(!audio_dir.join("IMG-20260710-WA0004.jpg").exists());

    let manifest = std::fs::read_to_string(out_dir.join("manifest.jsonl")).unwrap();
    assert_eq!(manifest.lines().count(), 2);

    let merged = std::fs::read_to_string(out_dir.join("_chat.transcribed.txt")).unwrap();
    let merged_lines: Vec<&str> = merged.lines().collect();
    let idx_a = index_of(
        &merged_lines,
        "10/07/2026, 09:05 - Bob: PTT-20260710-WA0001.opus (file attached)",
    );
    assert_eq!(
        merged_lines[idx_a + 1],
        "    >> [transcript] transcript of PTT-20260710-WA0001.opus"
    );
    let idx_c = index_of(
        &merged_lines,
        "20/07/2026, 08:00 - Bob: PTT-20260720-WA0003.opus (file attached)",
    );
    // out-of-range attachment stays untouched: no transcript line follows it
    assert_eq!(
        merged_lines[idx_c + 1],
        "10/07/2026, 09:10 - Bob: IMG-20260710-WA0004.jpg (file attached)"
    );
    assert!(merged_lines.contains(&"segunda parte (continuacao)"));
}

#[test]
fn test_run_returns_1_when_a_transcription_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("chat_export.zip");
    build_export_zip(&zip_path);
    let out_dir = tmp.path().join("out");

    let mut engine = FakeEngine::new(None).failing_on("00001-AUDIO-2026-07-10.opus");
    let args = wa_args(&zip_path, Some(&out_dir), None, None, false);
    let exit_code = run(&args, &mut engine);

    assert_eq!(exit_code, 1);
}

#[test]
fn test_run_missing_zip_returns_2() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope.zip");

    let args = wa_args(&missing, Some(&tmp.path().join("out")), None, None, false);
    let exit_code = run(&args, &mut NameTranscriber);

    assert_eq!(exit_code, 2);
}

#[test]
fn test_run_corrupt_zip_returns_2() {
    let tmp = tempfile::tempdir().unwrap();
    let bad_zip = tmp.path().join("chat_export.zip");
    std::fs::write(&bad_zip, b"not actually a zip file").unwrap();

    let args = wa_args(&bad_zip, Some(&tmp.path().join("out")), None, None, false);
    let exit_code = run(&args, &mut NameTranscriber);

    assert_eq!(exit_code, 2);
}

#[test]
fn test_run_no_chat_entry_leaves_no_output_dir() {
    // A zip without a locatable chat file (*_chat.txt or single root .txt)
    // exits 2 before any output directory is created -- no empty <out>/audio/
    // left behind.
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("chat_export.zip");
    write_zip(
        &zip_path,
        &[("a.txt", b"irrelevant"), ("b.txt", b"irrelevant")],
    );
    let out_dir = tmp.path().join("out");

    let args = wa_args(&zip_path, Some(&out_dir), None, None, false);
    let exit_code = run(&args, &mut NameTranscriber);

    assert_eq!(exit_code, 2);
    assert!(!out_dir.exists());
}

#[test]
fn test_run_malformed_date_returns_2() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("chat_export.zip");
    build_export_zip(&zip_path);

    let args = wa_args(
        &zip_path,
        Some(&tmp.path().join("out")),
        Some("not-a-date"),
        None,
        false,
    );
    let exit_code = run(&args, &mut NameTranscriber);

    assert_eq!(exit_code, 2);
}

#[test]
fn test_run_merge_does_not_leak_stale_manifest_entries_across_runs() {
    // A second run with a narrower date range, reusing the same --out,
    // must not inline a transcript left over in manifest.jsonl from a wider
    // first run onto a line that's outside *this* run's filter.
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("chat_export.zip");
    build_export_zip(&zip_path);
    let out_dir = tmp.path().join("out");

    // Run 1: wide range, no merge -- populates manifest.jsonl with both
    // 00001 (07-10) and 00003 (07-20).
    let args = wa_args(&zip_path, Some(&out_dir), None, None, false);
    let exit_code = run(&args, &mut NameTranscriber);
    assert_eq!(exit_code, 0);
    let manifest = std::fs::read_to_string(out_dir.join("manifest.jsonl")).unwrap();
    assert_eq!(manifest.lines().count(), 3); // 00001, 00002, 00003

    // Run 2: narrow range covering only 00001, with --merge.
    let args = wa_args(
        &zip_path,
        Some(&out_dir),
        Some("2026-07-10"),
        Some("2026-07-10"),
        true,
    );
    let exit_code = run(&args, &mut NameTranscriber);
    assert_eq!(exit_code, 0);

    let merged = std::fs::read_to_string(out_dir.join("_chat.transcribed.txt")).unwrap();
    let merged_lines: Vec<&str> = merged.lines().collect();
    let idx_a = index_of(
        &merged_lines,
        &format!(
            "{U200E}[10/07/2026, 09:05:00] Bob: {U200E}<anexado: 00001-AUDIO-2026-07-10.opus>"
        ),
    );
    assert_eq!(
        merged_lines[idx_a + 1],
        "    >> [transcript] transcript of 00001-AUDIO-2026-07-10.opus"
    );
    let idx_c = index_of(
        &merged_lines,
        &format!(
            "{U200E}[20/07/2026, 08:00:00] Bob: {U200E}<anexado: 00003-AUDIO-2026-07-20.opus>"
        ),
    );
    // 00003 was transcribed in run 1 (it's in the manifest) but is outside
    // run 2's filter -- it must stay untouched, not get a stale transcript
    // line inlined from the old manifest entry.
    assert_ne!(
        merged_lines[idx_c + 1],
        "    >> [transcript] transcript of 00003-AUDIO-2026-07-20.opus"
    );
    assert!(!merged_lines[idx_c + 1].starts_with("    >> [transcript]"));
}
