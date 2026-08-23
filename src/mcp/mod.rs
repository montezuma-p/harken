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

use std::io::{BufRead, Write};

use serde_json::{Value, json};

mod jsonrpc;
mod tools;
mod whatsapp_tool;

use crate::engine::Transcriber;
use jsonrpc::{INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, Request, err, ok};
use tools::{tool_list, tools_call};

/// Version answered to clients requesting a revision we don't know.
const LATEST_VERSION: &str = "2025-06-18";

/// Handshake revisions this server speaks. The tools-only surface is
/// identical across all three; newer, handshake-less revisions (2026-07-28+)
/// are deliberately not implemented until deployed clients require them.
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

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
pub(crate) fn handle_line(line: &str, transcriber: &mut dyn Transcriber) -> Option<Value> {
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
