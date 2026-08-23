//! MCP (Model Context Protocol) server mode: JSON-RPC 2.0 over stdio.
//!
//! The only mode that writes to stdout, and it writes only protocol frames
//! (newline-delimited compact JSON, flushed per message); logs stay on stderr,
//! which the MCP spec explicitly permits. Hand-rolled on serde_json — the
//! subset a tools-only server needs (initialize, tools/list, tools/call, ping)
//! does not justify an SDK plus an async runtime in a synchronous crate.
//!
//! Model, language and device are fixed per server instance (`harken mcp
//! --model ... --lang ...`): the `Transcriber` trait has no per-call language
//! channel, and a per-call model would rebuild the whisper context on every
//! request instead of reusing the one lazy load per session.

use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};
use zip::ZipArchive;

mod jsonrpc;

use crate::engine::{Transcriber, TranscriptionResult};
use crate::whatsapp::{
    Message, extract_attachment, find_attachment_member, find_chat_entry, parse_chat,
    parse_date_arg, select_audio_messages,
};
use jsonrpc::{
    INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, McpError, PARSE_ERROR, Request, err, ok,
};

/// Version answered to clients requesting a revision we don't know.
const LATEST_VERSION: &str = "2025-06-18";

/// Handshake revisions this server speaks. The tools-only surface is
/// identical across all three; newer, handshake-less revisions (2026-07-28+)
/// are deliberately not implemented until deployed clients require them.
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscribeFileArgs {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WhatsappExportArgs {
    zip_path: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

/// Serve MCP until EOF on `reader`. Never touches process stdin/stdout
/// itself: main() passes the real locked handles, tests pass in-memory
/// buffers and a `FakeEngine`.
pub fn serve<R: BufRead, W: Write>(
    reader: R,
    writer: &mut W,
    transcriber: &mut dyn Transcriber,
) -> std::io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(&line, transcriber) {
            writeln!(
                writer,
                "{}",
                serde_json::to_string(&reply).expect("serializable")
            )?;
            // The client blocks on the reply; an unflushed frame deadlocks it.
            writer.flush()?;
        }
    }
    Ok(())
}

/// Dispatch one input line to at most one reply. `None` means the line was a
/// notification (or blank) — JSON-RPC forbids answering anything without an id.
pub fn handle_line(line: &str, transcriber: &mut dyn Transcriber) -> Option<Value> {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Some(err(Value::Null, PARSE_ERROR, "Parse error".to_string())),
    };
    let id_hint = value.get("id").cloned().unwrap_or(Value::Null);
    let request: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(_) => return Some(err(id_hint, INVALID_REQUEST, "Invalid Request".to_string())),
    };
    if request.jsonrpc.as_deref() != Some("2.0") {
        return Some(err(id_hint, INVALID_REQUEST, "Invalid Request".to_string()));
    }
    let id = request.id?;

    Some(match request.method.as_str() {
        "initialize" => ok(id, initialize_result(request.params.as_ref())),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tool_list() })),
        "tools/call" => tools_call(id, request.params, transcriber),
        method => err(id, METHOD_NOT_FOUND, format!("Method not found: {method}")),
    })
}

fn initialize_result(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or(LATEST_VERSION);
    // Spec: echo a supported requested version, otherwise answer with ours —
    // disconnecting on mismatch is the client's decision, never an error.
    let version = if SUPPORTED_VERSIONS.contains(&requested) {
        requested
    } else {
        LATEST_VERSION
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "harken",
            "title": "harken (offline transcription)",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Fully offline whisper.cpp transcription. Model, language and \
                         device are fixed by the server's --model/--lang/--device flags.",
    })
}

fn tool_list() -> Value {
    json!([
        {
            "name": "transcribe_file",
            "title": "Transcribe audio file",
            "description": "Transcribe a local audio file (opus, ogg, mp3, m4a, wav, flac, \
                            mp4, webm, ...) fully offline via whisper.cpp. Model and language \
                            are fixed by the server's --model/--lang flags. Returns the \
                            transcript text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the audio file" }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        },
        {
            "name": "transcribe_whatsapp_export",
            "title": "Transcribe WhatsApp export voice notes",
            "description": "Extract and transcribe every voice-note attachment in a WhatsApp \
                            chat-export .zip (iOS or Android), optionally restricted to an \
                            inclusive date range. Fully offline. Returns one line per voice \
                            note, prefixed with date, time and sender.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "zip_path": {
                        "type": "string",
                        "description": "Absolute path to the WhatsApp chat-export .zip"
                    },
                    "from": {
                        "type": "string",
                        "description": "Only messages on/after this date, YYYY-MM-DD (inclusive)",
                        "pattern": "^\\d{4}-\\d{2}-\\d{2}$"
                    },
                    "to": {
                        "type": "string",
                        "description": "Only messages on/before this date, YYYY-MM-DD (inclusive)",
                        "pattern": "^\\d{4}-\\d{2}-\\d{2}$"
                    }
                },
                "required": ["zip_path"],
                "additionalProperties": false
            }
        }
    ])
}

fn tools_call(id: Value, params: Option<Value>, transcriber: &mut dyn Transcriber) -> Value {
    let params = params.unwrap_or(Value::Null);
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let outcome = match name {
        "transcribe_file" => tool_transcribe_file(arguments, transcriber),
        "transcribe_whatsapp_export" => tool_transcribe_whatsapp_export(arguments, transcriber),
        _ => Err(McpError {
            code: INVALID_PARAMS,
            message: format!("Unknown tool: {name}"),
        }),
    };
    match outcome {
        Ok(result) => ok(id, result),
        Err(e) => err(id, e.code, e.message),
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, McpError> {
    serde_json::from_value(arguments).map_err(|e| McpError {
        code: INVALID_PARAMS,
        message: format!("Invalid params: {e}"),
    })
}

/// Success result: the transcript as the text block for the model, the full
/// record as structuredContent. The spec's SHOULD of mirroring
/// structuredContent into the text block is deliberately not followed — the
/// text block's consumer is the LLM, which wants the transcript, not JSON.
fn success(text: String, structured: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
        "structuredContent": structured,
    })
}

/// Tool execution failure — a result, not a JSON-RPC error, per spec.
fn tool_error(message: String) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

/// Same shape as the json writer's output (writers::JsonOutput).
fn structured_result(result: &TranscriptionResult) -> Value {
    json!({
        "source": result.source.display().to_string(),
        "language": result.language,
        "duration": result.duration,
        "text": result.text,
        "segments": result
            .segments
            .iter()
            .map(|s| json!({ "start": s.start, "end": s.end, "text": s.text }))
            .collect::<Vec<_>>(),
    })
}

fn tool_transcribe_file(
    arguments: Value,
    transcriber: &mut dyn Transcriber,
) -> Result<Value, McpError> {
    let args: TranscribeFileArgs = parse_args(arguments)?;
    Ok(match transcriber.transcribe(Path::new(&args.path)) {
        Ok(result) => success(result.text.clone(), structured_result(&result)),
        Err(e) => tool_error(e.to_string()),
    })
}

fn tool_transcribe_whatsapp_export(
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
