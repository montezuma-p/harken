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
        }
    ])
}

pub(crate) fn tools_call(
    id: Value,
    params: Option<Value>,
    transcriber: &mut dyn Transcriber,
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

pub(crate) fn parse_args<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, McpError> {
    serde_json::from_value(arguments).map_err(|e| McpError {
        code: INVALID_PARAMS,
        message: format!("Invalid params: {e}"),
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
