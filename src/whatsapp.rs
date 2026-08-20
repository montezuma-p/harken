//! WhatsApp chat-export transcription mode.
//!
//! Parses a WhatsApp chat-export zip, selects audio-attachment messages within
//! an optional date range, extracts and transcribes them (reusing the batch
//! pipeline), and optionally writes a merged chat transcript with transcript
//! lines inlined after each attachment line.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::Regex;
use zip::ZipArchive;

use crate::batch::run_batch;
use crate::cli::WhatsappArgs;
use crate::engine::Transcriber;

fn ios_message_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\u{200E}?\[(\d{2}/\d{2}/\d{4}), (\d{2}:\d{2}:\d{2})\] ([^:]+): (.*)$")
            .unwrap()
    })
}

fn android_message_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(\d{1,2}/\d{1,2}/\d{2,4}),? (\d{1,2}:\d{2}(?:[\s\u{202F}]?[APap]\.?[Mm]\.?)?) - ([^:]+): (.*)$",
        )
        .unwrap()
    })
}

fn attachment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<(?:anexado|attached): ([^>]+)>").unwrap())
}

fn android_attachment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\S+\.\w+) \((?:file attached|arquivo anexado)\)").unwrap())
}

const INVISIBLE_CHARS: &[char] = &['\u{200E}', '\u{200F}'];

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub date: NaiveDate,
    pub time: String,
    pub sender: String,
    pub body: String,
    pub line_index: usize,
}

fn strip_invisible(line: &str) -> String {
    line.chars()
        .filter(|c| !INVISIBLE_CHARS.contains(c))
        .collect()
}

fn parse_date(date_str: &str, day_first: bool) -> NaiveDate {
    let parts: Vec<i64> = date_str
        .split('/')
        .map(|p| p.parse().expect("numeric date component"))
        .collect();
    let (a, b, mut year) = (parts[0], parts[1], parts[2]);
    if year < 100 {
        year += 2000;
    }
    let (day, month) = if day_first { (a, b) } else { (b, a) };
    NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
        .expect("invalid date in chat header")
}

/// Decide the date order of an Android chat from its whole corpus of
/// header dates: any first component > 12 proves day-first, any second
/// component > 12 proves month-first. Fully ambiguous chats default to
/// day-first (harken is pt-centric).
fn infer_day_first(date_strs: &[&str]) -> bool {
    let components: Vec<(u32, u32)> = date_strs
        .iter()
        .map(|ds| {
            let mut it = ds.split('/').map(|p| p.parse::<u32>().unwrap());
            (it.next().unwrap(), it.next().unwrap())
        })
        .collect();
    if components.iter().any(|(a, _)| *a > 12) {
        return true;
    }
    if components.iter().any(|(_, b)| *b > 12) {
        return false;
    }
    true
}

/// Parse raw WhatsApp chat-export text into a list of Message.
///
/// The export format (iOS `[date, time] sender: body` or Android
/// `date, time - sender: body`) is detected from the first line matching
/// either header pattern and applied to the whole chat. Lines that don't
/// match the header pattern are continuations of the previous message and
/// get folded into its body. Any lines before the first recognized message
/// header (e.g. the encryption notice) are dropped.
pub fn parse_chat(text: &str) -> Vec<Message> {
    let raw_lines: Vec<&str> = text.lines().collect();
    let lines: Vec<String> = raw_lines.iter().map(|l| strip_invisible(l)).collect();

    let mut pattern: Option<&Regex> = None;
    for line in &lines {
        if ios_message_re().is_match(line) {
            pattern = Some(ios_message_re());
            break;
        }
        if android_message_re().is_match(line) {
            pattern = Some(android_message_re());
            break;
        }
    }
    let Some(pattern) = pattern else {
        return Vec::new();
    };

    let is_android = std::ptr::eq(pattern, android_message_re());
    let day_first = if is_android {
        let date_strs: Vec<&str> = lines
            .iter()
            .filter_map(|l| pattern.captures(l))
            .map(|c| c.get(1).unwrap().as_str())
            .collect::<Vec<_>>();
        infer_day_first(&date_strs)
    } else {
        true
    };

    let mut messages: Vec<Message> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(caps) = pattern.captures(line) {
            messages.push(Message {
                date: parse_date(caps.get(1).unwrap().as_str(), day_first),
                time: caps.get(2).unwrap().as_str().to_string(),
                sender: caps.get(3).unwrap().as_str().to_string(),
                body: caps.get(4).unwrap().as_str().to_string(),
                line_index: i,
            });
        } else if let Some(last) = messages.last_mut() {
            last.body.push('\n');
            last.body.push_str(raw_lines[i]);
        }
    }

    messages
}

/// Return the attachment filename in `body`, if any — iOS
/// `<anexado|attached: file>` or Android `file (file attached|arquivo
/// anexado)` markers.
pub fn extract_attachment(body: &str) -> Option<&str> {
    attachment_re()
        .captures(body)
        .or_else(|| android_attachment_re().captures(body))
        .map(|c| c.get(1).unwrap().as_str())
}

pub fn is_audio_attachment(filename: &str) -> bool {
    crate::batch::is_audio(Path::new(filename))
}

/// Return messages carrying an audio attachment, within [date_from, date_to].
pub fn select_audio_messages(
    messages: &[Message],
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| {
            let Some(filename) = extract_attachment(&m.body) else {
                return false;
            };
            if !is_audio_attachment(filename) {
                return false;
            }
            if let Some(from) = date_from {
                if m.date < from {
                    return false;
                }
            }
            if let Some(to) = date_to {
                if m.date > to {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

/// Locate the chat-log member inside a WhatsApp export zip's namelist.
pub fn find_chat_entry(names: &[String]) -> Result<String, String> {
    if let Some(name) = names.iter().find(|n| n.ends_with("_chat.txt")) {
        return Ok(name.clone());
    }

    let root_txts: Vec<&String> = names
        .iter()
        .filter(|n| n.to_lowercase().ends_with(".txt") && !n.contains('/'))
        .collect();
    if root_txts.len() == 1 {
        return Ok(root_txts[0].clone());
    }

    Err("could not locate a chat log (*_chat.txt or a single root .txt)".to_string())
}

/// Find the zip member matching an attachment filename referenced in the chat.
pub fn find_attachment_member(names: &[String], filename: &str) -> Option<String> {
    names
        .iter()
        .find(|n| *n == filename || n.ends_with(&format!("/{filename}")))
        .cloned()
}

/// Rebuild the chat text, inserting a transcript line after each
/// attachment line whose extracted audio has an entry in `transcripts`
/// (keyed by the extracted file's path). Everything else is untouched.
pub fn build_merged_chat(
    raw_text: &str,
    messages: &[Message],
    transcripts: &HashMap<String, String>,
    audio_dir: &Path,
) -> String {
    let mut inserts: HashMap<usize, &str> = HashMap::new();
    for message in messages {
        let Some(filename) = extract_attachment(&message.body) else {
            continue;
        };
        let key = extracted_path(audio_dir, filename);
        if let Some(text) = transcripts.get(&key) {
            inserts.insert(message.line_index, text);
        }
    }

    let mut out_lines: Vec<String> = Vec::new();
    for (i, line) in raw_text.lines().enumerate() {
        out_lines.push(line.to_string());
        if let Some(text) = inserts.get(&i) {
            out_lines.push(format!("    >> [transcript] {text}"));
        }
    }

    format!("{}\n", out_lines.join("\n"))
}

pub fn default_out_dir(export_zip: &Path) -> PathBuf {
    let stem = export_zip
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    PathBuf::from(format!("./{stem}-transcripts"))
}

fn parse_date_arg(value: &str) -> Result<NaiveDate, ()> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| ())
}

fn load_manifest_texts(manifest: &Path) -> HashMap<String, String> {
    let mut texts = HashMap::new();
    let Ok(content) = std::fs::read_to_string(manifest) else {
        return texts;
    };
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            if let (Some(source), Some(text)) = (
                entry.get("source").and_then(|v| v.as_str()),
                entry.get("text").and_then(|v| v.as_str()),
            ) {
                texts.insert(source.to_string(), text.to_string());
            }
        }
    }
    texts
}

/// The on-disk path an attachment is extracted to (path components in the
/// chat reference are flattened to the bare filename).
fn extracted_path(audio_dir: &Path, filename: &str) -> String {
    let name = Path::new(filename)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    audio_dir.join(name).display().to_string()
}

fn fmt_opt(value: &Option<String>) -> String {
    match value {
        Some(v) => format!("'{v}'"),
        None => "None".to_string(),
    }
}

pub fn run(args: &WhatsappArgs, transcriber: &mut dyn Transcriber) -> i32 {
    let export_zip = PathBuf::from(&args.export_zip);
    if !export_zip.exists() {
        eprintln!("error: file not found: {}", export_zip.display());
        return 2;
    }

    let date_from = match &args.date_from {
        Some(v) => match parse_date_arg(v) {
            Ok(d) => Some(d),
            Err(()) => {
                eprintln!(
                    "error: invalid date (expected YYYY-MM-DD): {} / {}",
                    fmt_opt(&args.date_from),
                    fmt_opt(&args.date_to)
                );
                return 2;
            }
        },
        None => None,
    };
    let date_to = match &args.date_to {
        Some(v) => match parse_date_arg(v) {
            Ok(d) => Some(d),
            Err(()) => {
                eprintln!(
                    "error: invalid date (expected YYYY-MM-DD): {} / {}",
                    fmt_opt(&args.date_from),
                    fmt_opt(&args.date_to)
                );
                return 2;
            }
        },
        None => None,
    };

    let out_dir = args
        .out
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_out_dir(&export_zip));
    let audio_dir = out_dir.join("audio");

    let file = match File::open(&export_zip) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let mut archive = match ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let names: Vec<String> = (0..archive.len())
        .map(|i| {
            archive
                .by_index(i)
                .map(|f| f.name().to_string())
                .unwrap_or_default()
        })
        .collect();

    let chat_name = match find_chat_entry(&names) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // No output directory is created until the chat entry is located --
    // a zip that fails to locate one exits 2 without leaving an empty
    // <out>/audio/ behind.
    std::fs::create_dir_all(&audio_dir).expect("failed to create audio directory");

    let raw_bytes = {
        let mut member = archive.by_name(&chat_name).expect("chat entry readable");
        let mut buf = Vec::new();
        member.read_to_end(&mut buf).expect("chat entry readable");
        buf
    };
    // Decode as utf-8-sig: strip a leading BOM if present.
    let raw_bytes = raw_bytes
        .strip_prefix(b"\xef\xbb\xbf".as_slice())
        .map(|b| b.to_vec())
        .unwrap_or(raw_bytes);
    let raw_text = String::from_utf8_lossy(&raw_bytes).into_owned();

    let messages = parse_chat(&raw_text);
    let selected = select_audio_messages(&messages, date_from, date_to);

    let mut extracted: Vec<PathBuf> = Vec::new();
    for message in &selected {
        let filename = extract_attachment(&message.body).expect("selected implies attachment");
        let Some(member_name) = find_attachment_member(&names, filename) else {
            eprintln!("warning: attachment not found in zip: {filename}");
            continue;
        };
        let name = Path::new(filename)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dest = audio_dir.join(name);
        let mut member = archive.by_name(&member_name).expect("member readable");
        let mut buf = Vec::new();
        member.read_to_end(&mut buf).expect("member readable");
        std::fs::write(&dest, buf).expect("failed to extract attachment");
        extracted.push(dest);
    }
    drop(archive);

    let stats = run_batch(&extracted, &out_dir, transcriber, args.format, args.force);

    if args.merge {
        // Gate by *this run's* selection, not the full accumulated manifest --
        // out_dir/manifest.jsonl persists across runs, so on a reused --out
        // with a narrower date range it could otherwise carry stale entries
        // from a previous, wider-range run and inline a transcript onto a
        // line outside the current filter.
        let selected_paths: HashSet<String> = selected
            .iter()
            .map(|m| extracted_path(&audio_dir, extract_attachment(&m.body).unwrap()))
            .collect();
        let transcripts: HashMap<String, String> =
            load_manifest_texts(&out_dir.join("manifest.jsonl"))
                .into_iter()
                .filter(|(path, _)| selected_paths.contains(path))
                .collect();
        let merged = build_merged_chat(&raw_text, &messages, &transcripts, &audio_dir);
        std::fs::write(out_dir.join("_chat.transcribed.txt"), merged)
            .expect("failed to write merged chat");
    }

    if stats.failed > 0 {
        1
    } else {
        0
    }
}
