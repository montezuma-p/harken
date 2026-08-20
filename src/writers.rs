//! Output writers for transcription results: txt, json, srt, and manifest.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

use crate::engine::TranscriptionResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Txt,
    Json,
    Srt,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Txt => "txt",
            OutputFormat::Json => "json",
            OutputFormat::Srt => "srt",
        }
    }
}

pub fn write_output(
    fmt: OutputFormat,
    result: &TranscriptionResult,
    dest: &Path,
) -> io::Result<()> {
    match fmt {
        OutputFormat::Txt => write_txt(result, dest),
        OutputFormat::Json => write_json(result, dest),
        OutputFormat::Srt => write_srt(result, dest),
    }
}

pub fn write_txt(result: &TranscriptionResult, dest: &Path) -> io::Result<()> {
    std::fs::write(dest, format!("{}\n", result.text))
}

#[derive(Serialize)]
struct JsonSegment<'a> {
    start: f64,
    end: f64,
    text: &'a str,
}

#[derive(Serialize)]
struct JsonOutput<'a> {
    source: String,
    language: &'a str,
    duration: f64,
    text: &'a str,
    segments: Vec<JsonSegment<'a>>,
}

pub fn write_json(result: &TranscriptionResult, dest: &Path) -> io::Result<()> {
    let data = JsonOutput {
        source: result.source.display().to_string(),
        language: &result.language,
        duration: result.duration,
        text: &result.text,
        segments: result
            .segments
            .iter()
            .map(|s| JsonSegment {
                start: s.start,
                end: s.end,
                text: &s.text,
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&data).expect("serializable");
    std::fs::write(dest, format!("{json}\n"))
}

fn srt_timestamp(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as i64;
    let (hours, rem_ms) = (total_ms / 3_600_000, total_ms % 3_600_000);
    let (minutes, rem_ms) = (rem_ms / 60_000, rem_ms % 60_000);
    let (secs, millis) = (rem_ms / 1000, rem_ms % 1000);
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}

pub fn write_srt(result: &TranscriptionResult, dest: &Path) -> io::Result<()> {
    let mut out = String::new();
    for (i, segment) in result.segments.iter().enumerate() {
        let start = srt_timestamp(segment.start);
        let end = srt_timestamp(segment.end);
        out.push_str(&format!(
            "{}\n{start} --> {end}\n{}\n\n",
            i + 1,
            segment.text
        ));
    }
    std::fs::write(dest, out)
}

#[derive(Serialize)]
struct ManifestEntry<'a> {
    source: String,
    output: String,
    language: &'a str,
    duration: f64,
    text: &'a str,
}

pub fn append_manifest(
    manifest: &Path,
    result: &TranscriptionResult,
    output_file: &Path,
) -> io::Result<()> {
    let entry = ManifestEntry {
        source: result.source.display().to_string(),
        output: output_file.display().to_string(),
        language: &result.language,
        duration: result.duration,
        text: &result.text,
    };
    let line = serde_json::to_string(&entry).expect("serializable");
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest)?;
    writeln!(f, "{line}")
}
