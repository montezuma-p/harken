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

mod era;
mod jsonrpc;
mod tools;
mod whatsapp_tool;

use crate::engine::Transcriber;
use era::Era;
use jsonrpc::{INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, Request, err, err_from, ok};
use tools::{tool_list, tools_call};

/// Serve MCP until EOF on `reader`. Never touches process stdin/stdout
/// itself: main() passes the real locked handles, tests pass in-memory
/// buffers and a `FakeEngine`.
pub fn serve<R: BufRead, W: Write>(
    mut reader: R,
    writer: &mut W,
    transcriber: &mut dyn Transcriber,
) -> std::io::Result<()> {
    // Bytes rather than BufRead::lines(): that iterator yields Err on a line
    // that is not UTF-8, and propagating it would end the session. A client
    // that emits one bad byte gets a parse error and stays connected.
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        // Same trimming BufRead::lines() does: the delimiter, then one CR.
        if buf.last() == Some(&b'\n') {
            buf.pop();
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
        }

        let reply = match std::str::from_utf8(&buf) {
            Ok(line) if line.trim().is_empty() => continue,
            Ok(line) => handle_line(line, transcriber),
            Err(_) => Some(err(Value::Null, PARSE_ERROR, "Parse error".to_string())),
        };

        if let Some(reply) = reply {
            writeln!(
                writer,
                "{}",
                serde_json::to_string(&reply).expect("serializable")
            )?;
            // The client blocks on the reply; an unflushed frame deadlocks it.
            writer.flush()?;
        }
    }
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

    // Stateless: the era is re-derived per request and nothing is remembered
    // between them.
    let era = era::detect(request.params.as_ref());
    if era == Era::Modern
        && let Err(e) = era::modern_preflight(request.params.as_ref())
    {
        return Some(err_from(id, e));
    }

    Some(match request.method.as_str() {
        "initialize" => ok(id, era::initialize_result(request.params.as_ref())),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tool_list() })),
        "tools/call" => tools_call(id, request.params, transcriber),
        method => err(id, METHOD_NOT_FOUND, format!("Method not found: {method}")),
    })
}
