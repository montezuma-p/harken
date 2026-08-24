//! The `transcribe_whatsapp_export` tool: pull every voice note out of a
//! chat-export zip and transcribe it.
//!
//! Shares the zip and chat-parsing helpers with the CLI's whatsapp mode, but
//! deliberately not `whatsapp::run` itself — that returns an exit code, drops
//! errors on stderr and panics on IO, none of which a long-lived server can
//! afford. Attachments land in a per-call scratch dir and are removed before
//! returning: this tool returns content, never files.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};
use zip::ZipArchive;

use super::jsonrpc::McpError;
use super::tools::{parse_args, success, tool_error};
use crate::engine::{Transcriber, TranscriptionResult};
use crate::whatsapp::{
    Message, extract_attachment, find_attachment_member, find_chat_entry, parse_chat,
    parse_date_arg, select_audio_messages,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WhatsappExportArgs {
    zip_path: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

pub(crate) fn tool_transcribe_whatsapp_export(
    arguments: Value,
    transcriber: &mut dyn Transcriber,
) -> Result<Value, McpError> {
    let args: WhatsappExportArgs = parse_args(arguments)?;
    Ok(match run_whatsapp_export(&args, transcriber) {
        Ok(result) => result,
        Err(message) => tool_error(message),
    })
}

fn message_record(message: &Message, filename: &str) -> serde_json::Map<String, Value> {
    let mut record = serde_json::Map::new();
    record.insert("date".into(), json!(message.date.to_string()));
    record.insert("time".into(), json!(message.time));
    record.insert("sender".into(), json!(message.sender));
    record.insert("filename".into(), json!(filename));
    record
}

fn run_whatsapp_export(
    args: &WhatsappExportArgs,
    transcriber: &mut dyn Transcriber,
) -> Result<Value, String> {
    let date_from = args.from.as_deref().map(parse_date_arg).transpose()?;
    let date_to = args.to.as_deref().map(parse_date_arg).transpose()?;

    let zip_path = Path::new(&args.zip_path);
    if !zip_path.exists() {
        return Err(format!("file not found: {}", zip_path.display()));
    }
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let names: Vec<String> = (0..archive.len())
        .map(|i| {
            archive
                .by_index(i)
                .map(|f| f.name().to_string())
                .unwrap_or_default()
        })
        .collect();

    let chat_name = find_chat_entry(&names)?;
    let raw_bytes = {
        let mut member = archive.by_name(&chat_name).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        member.read_to_end(&mut buf).map_err(|e| e.to_string())?;
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

    // Attachments are extracted to a scratch dir only long enough to be
    // transcribed — MCP returns content, not files. Unique per call, not per
    // process: concurrent callers in one process (the test suite) must not
    // remove each other's extractions.
    static SCRATCH_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let scratch = std::env::temp_dir().join(format!(
        "harken-mcp-{}-{}",
        std::process::id(),
        SCRATCH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;

    let mut lines: Vec<String> = Vec::new();
    let mut records: Vec<Value> = Vec::new();
    let (mut transcribed, mut failed, mut missing) = (0usize, 0usize, 0usize);
    for message in &selected {
        let filename = extract_attachment(&message.body).expect("selected implies attachment");
        let prefix = format!("[{} {}] {}", message.date, message.time, message.sender);
        let mut record = message_record(message, filename);

        let Some(member_name) = find_attachment_member(&names, filename) else {
            missing += 1;
            lines.push(format!("{prefix}: MISSING FROM ZIP: {filename}"));
            record.insert("error".into(), json!("attachment not found in zip"));
            records.push(Value::Object(record));
            continue;
        };

        let outcome = extract_and_transcribe(&mut archive, &member_name, &scratch, transcriber);
        match outcome {
            Ok(result) => {
                transcribed += 1;
                lines.push(format!("{prefix}: {}", result.text));
                record.insert("text".into(), json!(result.text));
                record.insert("duration".into(), json!(result.duration));
                record.insert("language".into(), json!(result.language));
            }
            Err(e) => {
                failed += 1;
                lines.push(format!("{prefix}: FAILED: {e}"));
                record.insert("error".into(), json!(e));
            }
        }
        records.push(Value::Object(record));
    }
    let _ = std::fs::remove_dir_all(&scratch);

    let summary = format!(
        "{} voice notes: {transcribed} transcribed, {failed} failed, {missing} missing",
        selected.len()
    );
    let text = if lines.is_empty() {
        summary.clone()
    } else {
        format!("{}\n---\n{summary}", lines.join("\n"))
    };
    // Partial results are the value: only an all-failed selection is an error
    // (missing-from-zip stays a warning, mirroring the CLI's exit-0 there).
    let all_failed = !selected.is_empty() && failed == selected.len();
    let structured = json!({
        "total": selected.len(),
        "transcribed": transcribed,
        "failed": failed,
        "missing": missing,
        "messages": records,
    });
    Ok(if all_failed {
        tool_error(text)
    } else {
        success(text, structured)
    })
}

fn extract_and_transcribe(
    archive: &mut ZipArchive<File>,
    member_name: &str,
    scratch: &Path,
    transcriber: &mut dyn Transcriber,
) -> Result<TranscriptionResult, String> {
    let mut buf = Vec::new();
    archive
        .by_name(member_name)
        .and_then(|mut member| member.read_to_end(&mut buf).map_err(Into::into))
        .map_err(|e| e.to_string())?;
    // Path components in the chat reference are flattened to the bare name.
    let name = Path::new(member_name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest = scratch.join(name);
    std::fs::write(&dest, buf).map_err(|e| e.to_string())?;
    let result = transcriber.transcribe(&dest).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&dest);
    result
}
