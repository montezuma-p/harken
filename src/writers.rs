//! Output writers for transcription results: txt, json, srt, md, and manifest.

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
    Md,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Txt => "txt",
            OutputFormat::Json => "json",
            OutputFormat::Srt => "srt",
            OutputFormat::Md => "md",
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
        OutputFormat::Md => write_md(result, dest),
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

/// Split a timestamp into (hours, minutes, seconds, milliseconds), rounding to
/// the nearest millisecond first so both timestamp formats agree on the split.
fn hms_millis(seconds: f64) -> (i64, i64, i64, i64) {
    let total_ms = (seconds * 1000.0).round() as i64;
    let (hours, rem_ms) = (total_ms / 3_600_000, total_ms % 3_600_000);
    let (minutes, rem_ms) = (rem_ms / 60_000, rem_ms % 60_000);
    let (secs, millis) = (rem_ms / 1000, rem_ms % 1000);
    (hours, minutes, secs, millis)
}

fn srt_timestamp(seconds: f64) -> String {
    let (hours, minutes, secs, millis) = hms_millis(seconds);
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}

fn md_timestamp(seconds: f64) -> String {
    let (hours, minutes, secs, _) = hms_millis(seconds);
    format!("{hours:02}:{minutes:02}:{secs:02}")
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

/// A transcript meant to be read: the source stem as the title, then one line
/// per segment, each prefixed with its start time. Segment text is emitted
/// verbatim -- the `[hh:mm:ss] ` prefix means no segment can open a markdown
/// block, so there is nothing to escape.
pub fn write_md(result: &TranscriptionResult, dest: &Path) -> io::Result<()> {
    let title = result
        .source
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let mut out = format!("# {title}\n\n");
    for segment in &result.segments {
        out.push_str(&format!(
            "[{}] {}\n",
            md_timestamp(segment.start),
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
