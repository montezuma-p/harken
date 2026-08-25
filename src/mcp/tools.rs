//! The tool catalog and the `tools/call` dispatch.
//!
//! Two error channels, per spec: a malformed call (unknown tool, bad arguments)
//! is a JSON-RPC error, while anything that fails *after* a well-formed call —
//! a missing file, a bad zip, a failed transcription — is a result carrying
//! `isError: true`. `success`/`tool_error` build the latter.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use super::jsonrpc::{INVALID_PARAMS, McpError, err, ok};
use crate::engine::{Transcriber, TranscriptionResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscribeFileArgs {
    path: String,
}

pub(crate) fn tool_list() -> Value {
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
        },
        {
            "name": "transcribe_status",
            "title": "Transcription server status",
            "description": "Report this server's fixed configuration and model cache state \
                            without transcribing anything: model name, whether the model file \
                            is already cached locally (path and size in bytes) or the first \
                            transcription call would have to download it first (~466 MB for \
                            the default 'small'), the startup warm-up's state ('downloading' \
                            means a call would wait for a download already in flight rather \
                            than start one; 'failed' reports the error), language, device, \
                            and whether the whisper context is loaded. Never touches the \
                            network.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }
        }
    ])
}

pub(crate) fn tools_call(
    id: Value,
    params: Option<Value>,
    transcriber: &mut dyn Transcriber,
    info: &super::ServerInfo,
) -> Value {
    let params = params.unwrap_or(Value::Null);
    // A call with no name is malformed params, not a call to a tool named "".
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return err(
            id,
            INVALID_PARAMS,
            "Invalid params: missing tool name".to_string(),
        );
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let outcome = match name {
        "transcribe_file" => tool_transcribe_file(arguments, transcriber),
        "transcribe_whatsapp_export" => {
            super::whatsapp_tool::tool_transcribe_whatsapp_export(arguments, transcriber)
        }
        "transcribe_status" => tool_transcribe_status(arguments, transcriber, info),
        _ => Err(McpError {
            code: INVALID_PARAMS,
            message: format!("Unknown tool: {name}"),
            data: None,
        }),
    };
    match outcome {
        Ok(result) => ok(id, result),
        Err(e) => err(id, e.code, e.message),
    }
}

pub(crate) fn parse_args<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, McpError> {
    serde_json::from_value(arguments).map_err(|e| McpError {
        code: INVALID_PARAMS,
        message: format!("Invalid params: {e}"),
        data: None,
    })
}

/// Success result: the transcript as the text block for the model, the full
/// record as structuredContent. The spec's SHOULD of mirroring
/// structuredContent into the text block is deliberately not followed — the
/// text block's consumer is the LLM, which wants the transcript, not JSON.
pub(crate) fn success(text: String, structured: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
        "structuredContent": structured,
    })
}

/// Tool execution failure — a result, not a JSON-RPC error, per spec.
pub(crate) fn tool_error(message: String) -> Value {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusArgs {}

/// The startup warm-up as this tool reports it. Derived from `Warmth` without
/// ever waiting on it — `is_pending`/`failure` both take the lock and return.
fn warm_state(info: &super::ServerInfo) -> (&'static str, Option<String>) {
    let Some(warm) = info.warm.as_ref() else {
        // No warm-up thread in this instance (the plain serve() entry point).
        return ("idle", None);
    };
    if warm.is_pending() {
        return ("downloading", None);
    }
    match warm.failure() {
        Some(message) => ("failed", Some(message)),
        None => ("ready", None),
    }
}

/// Cache state comes from `model::cached_path`, which is a pure stat — this
/// tool can never trigger a download, which is what makes it a trustworthy
/// answer to "will the first call stall?". An invalid --model is reported as
/// a diagnosis in the result, not a tool failure: the agent asking "can this
/// server transcribe?" deserves the answer.
///
/// The stat alone is not the whole story during startup, though: while the
/// warm-up thread downloads, the file is not there yet and a stat-only report
/// would say the first call "will download it" — both halves wrong, since the
/// download is already running and the call will *wait* on it via the WarmGate.
/// So the warm state is read alongside the stat (never waited on), which also
/// gives the one place a failed warm-up becomes visible to a client: until now
/// it existed only as a stderr line nothing read.
fn tool_transcribe_status(
    arguments: Value,
    transcriber: &mut dyn Transcriber,
    info: &super::ServerInfo,
) -> Result<Value, McpError> {
    let StatusArgs {} = parse_args(arguments)?;
    let (cached, path, size_bytes, error) = match crate::model::cached_path(&info.model) {
        Ok(Some(p)) => {
            let size = std::fs::metadata(&p).map(|m| m.len()).ok();
            (true, Some(p.display().to_string()), size, None)
        }
        Ok(None) => (false, None, None, None),
        Err(e) => (false, None, None, Some(e)),
    };
    let context_loaded = transcriber.is_loaded();
    let (warm, warm_error) = warm_state(info);

    let text = match (&error, cached) {
        // An unusable --model outranks everything else: no warm state makes a
        // typo transcribable.
        (Some(e), _) => format!("model '{}' is unusable: {e}", info.model),
        (None, true) => format!(
            "model {} cached at {} ({} bytes); language {}; device {}; context loaded: {}",
            info.model,
            path.as_deref().unwrap_or("?"),
            size_bytes.unwrap_or(0),
            info.lang,
            info.device,
            context_loaded,
        ),
        (None, false) => {
            let cache_state = match (warm, warm_error.as_deref()) {
                ("downloading", _) => "is downloading now: a transcription call \
                                       will wait for it rather than start a second download"
                    .to_string(),
                ("failed", Some(e)) => format!(
                    "is NOT cached and the startup download already failed ({e}): \
                     a transcription call will retry it"
                ),
                _ => "is NOT cached: the first transcription call will download it".to_string(),
            };
            format!(
                "model {} {cache_state}; language {}; device {}",
                info.model, info.lang, info.device,
            )
        }
    };
    Ok(success(
        text,
        json!({
            "model": info.model,
            "language": info.lang,
            "device": info.device,
            "cached": cached,
            "path": path,
            "size_bytes": size_bytes,
            "context_loaded": context_loaded,
            "warm": warm,
            "warm_error": warm_error,
            "error": error,
        }),
    ))
}
