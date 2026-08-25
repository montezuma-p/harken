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
use jsonrpc::{
    INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, Request, err, err_from, ok,
};
use tools::{tool_list, tools_call};

/// The per-instance configuration the `transcribe_status` tool reports.
/// Data only — the serve loop itself never reads it for dispatch.
pub struct ServerInfo {
    pub model: String,
    pub lang: String,
    pub device: String,
    /// The startup warm-up's state, when one is running. `transcribe_status`
    /// reads it (never blocking on it) so it can tell "no model and nothing
    /// happening" apart from "the download is already in flight".
    ///
    /// `None` rather than a fresh `Warmth` is the honest default: a `Warmth`
    /// nobody settles reads as Pending forever, so a `ServerInfo` built without
    /// a warm thread would report a download that does not exist. Only main()
    /// spawns that thread, so only main() fills this in.
    pub warm: Option<Warmth>,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            model: "small".to_string(),
            lang: "pt".to_string(),
            device: "cpu".to_string(),
            warm: None,
        }
    }
}

/// Shared warm-up state between main's model-download thread and the
/// WarmGate holding tool calls. Pending until the thread settles it.
#[derive(Clone, Default)]
pub struct Warmth(std::sync::Arc<(std::sync::Mutex<WarmState>, std::sync::Condvar)>);

#[derive(Default)]
enum WarmState {
    #[default]
    Pending,
    Ready,
    Failed(String),
}

impl Warmth {
    pub fn new() -> Self {
        Self::default()
    }

    // Every lock here recovers from poisoning instead of unwrapping. No current
    // caller can poison it — each method below takes the guard, does one move or
    // clone, and drops it, so there is no window where a panic runs under the
    // lock — but the cost of being wrong is severe and asymmetric: set_failed()
    // runs from a Drop impl (WarmSettle), where a panic aborts the process, and
    // wait_until_settled() would wedge every tool call on the mutex the drop
    // guard exists to release. WarmState is a plain enum with no invariant an
    // unwind can leave broken, so the poisoned value is always safe to keep.
    // Unreachable today, uninsurable if it ever becomes reachable.
    fn state(&self) -> std::sync::MutexGuard<'_, WarmState> {
        self.0.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_ready(&self) {
        *self.state() = WarmState::Ready;
        self.0.1.notify_all();
    }

    pub fn set_failed(&self, message: String) {
        *self.state() = WarmState::Failed(message);
        self.0.1.notify_all();
    }

    /// The recorded warm-up failure, if the warm thread reported one.
    pub fn failure(&self) -> Option<String> {
        match &*self.state() {
            WarmState::Failed(msg) => Some(msg.clone()),
            _ => None,
        }
    }

    /// Whether the warm-up is still running. Never blocks — `transcribe_status`
    /// reads this and must answer instantly even mid-download.
    pub fn is_pending(&self) -> bool {
        matches!(*self.state(), WarmState::Pending)
    }

    fn wait_until_settled(&self) {
        let mut state = self.state();
        while matches!(*state, WarmState::Pending) {
            state = self.0.1.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// Settles a `Warmth` on unwind. `set_ready`/`set_failed` both run after
/// `ensure_downloaded` returns, so a warm thread that *panics* instead of
/// returning would leave the state `Pending` forever — and `WarmGate` would
/// block every tool call for the life of the process, a failure worse than
/// dying: the client sees a `tools/call` that never returns and never errors.
/// Holding this for the length of the warm closure makes an unwind settle the
/// state too, so the gate's failure path (delegate and let the engine retry
/// inline) covers a panic just like it covers a download error.
pub struct WarmSettle {
    warmth: Warmth,
    settled: bool,
}

impl WarmSettle {
    pub fn new(warmth: Warmth) -> Self {
        Self {
            warmth,
            settled: false,
        }
    }

    /// Record that the happy path settled the state itself.
    pub fn done(mut self) {
        self.settled = true;
    }
}

impl Drop for WarmSettle {
    fn drop(&mut self) {
        if !self.settled {
            self.warmth.set_failed("model warm-up panicked".to_string());
        }
    }
}

/// Transcriber wrapper for MCP mode: holds each call until the warm-up
/// thread settles, so a call arriving mid-download waits for that download
/// instead of starting a duplicate one. After a *failed* warm-up it
/// delegates anyway — the engine's own model resolution retries the
/// download inline and surfaces a live error as `isError: true`, so a
/// transient network failure at startup never wedges or kills the server.
pub struct WarmGate<'a, T: Transcriber + ?Sized> {
    inner: &'a mut T,
    warmth: Warmth,
}

impl<'a, T: Transcriber + ?Sized> WarmGate<'a, T> {
    pub fn new(inner: &'a mut T, warmth: Warmth) -> Self {
        Self { inner, warmth }
    }
}

impl<T: Transcriber + ?Sized> Transcriber for WarmGate<'_, T> {
    fn transcribe(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::engine::TranscriptionResult, crate::engine::EngineError> {
        self.warmth.wait_until_settled();
        self.inner.transcribe(path)
    }

    fn is_loaded(&self) -> bool {
        self.inner.is_loaded()
    }
}

/// Serve MCP until EOF on `reader`. Never touches process stdin/stdout
/// itself: main() passes the real locked handles, tests pass in-memory
/// buffers and a `FakeEngine`. This wrapper keeps the original spec'd
/// signature; `main()` uses `serve_with_info` to hand the status tool the
/// real per-instance configuration.
pub fn serve<R: BufRead, W: Write>(
    reader: R,
    writer: &mut W,
    transcriber: &mut dyn Transcriber,
) -> std::io::Result<()> {
    serve_with_info(reader, writer, transcriber, &ServerInfo::default())
}

/// serve(), plus the per-instance configuration `transcribe_status` reports.
pub fn serve_with_info<R: BufRead, W: Write>(
    mut reader: R,
    writer: &mut W,
    transcriber: &mut dyn Transcriber,
    info: &ServerInfo,
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
            Ok(line) => handle_line(line, transcriber, info),
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
pub(crate) fn handle_line(
    line: &str,
    transcriber: &mut dyn Transcriber,
    info: &ServerInfo,
) -> Option<Value> {
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

    let reply = match (era, request.method.as_str()) {
        (Era::Legacy, "initialize") => ok(id, era::initialize_result(request.params.as_ref())),
        // A request that declares a modern protocol version and then asks for
        // the handshake that revision removed is contradicting itself. Serving
        // it would leave the client believing in a negotiated state this server
        // does not keep.
        (Era::Modern, "initialize") => err(
            id,
            METHOD_NOT_FOUND,
            "Method not found: initialize was removed in 2026-07-28; use server/discover"
                .to_string(),
        ),
        // Removed in 2026-07-28, but answering a keepalive costs nothing and a
        // -32601 on one can make a client tear the connection down.
        (_, "ping") => ok(id, json!({})),
        (Era::Modern, "server/discover") => ok(id, era::discover_result()),
        // server/discover exists only in the modern era, so a probe without the
        // per-request metadata is malformed rather than unimplemented. Either
        // answer works for a dual-era client, which must not key its fallback
        // to a specific code, but naming the missing field helps a modern
        // client that simply forgot it.
        (Era::Legacy, "server/discover") => err(
            id,
            INVALID_PARAMS,
            format!("Invalid params: {} is required", era::META_PROTOCOL_VERSION),
        ),
        (Era::Modern, "tools/list") => ok(id, era::cacheable(json!({ "tools": tool_list() }))),
        (Era::Legacy, "tools/list") => ok(id, json!({ "tools": tool_list() })),
        (_, "tools/call") => tools_call(id, request.params, transcriber, info),
        (_, method) => err(id, METHOD_NOT_FOUND, format!("Method not found: {method}")),
    };
    // One place, so no dispatch arm can be forgotten.
    Some(era::decorate_reply(era, reply))
}
