//! JSON-RPC 2.0 plumbing: the request envelope, the response builders, and the
//! error codes this server is allowed to emit.
//!
//! The codes are named rather than written inline because revision 2026-07-28
//! partitions the JSON-RPC server-error range: `-32000..-32019` is legacy and
//! new implementations should not use it at all, while `-32020..-32099` is
//! reserved for the specification, which forbids emitting any code from it that
//! the spec has not defined. A named set is what makes that rule checkable.

use serde::Deserialize;
use serde_json::{Value, json};

/// Malformed JSON — not parseable at all.
pub(crate) const PARSE_ERROR: i64 = -32700;
/// Well-formed JSON that is not a JSON-RPC 2.0 request.
pub(crate) const INVALID_REQUEST: i64 = -32600;
/// Known-good request naming a method this server does not implement.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
/// Bad, missing or unknown parameters — including an unknown tool name.
pub(crate) const INVALID_PARAMS: i64 = -32602;

#[derive(Deserialize)]
pub(crate) struct Request {
    #[serde(default)]
    pub(crate) jsonrpc: Option<String>,
    // Absent id => notification; explicit null id => request answered with
    // null id. serde's Option covers only the first, so the raw Value is kept.
    #[serde(default)]
    pub(crate) id: Option<Value>,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Option<Value>,
}

/// A JSON-RPC protocol error (unknown tool, invalid params). Tool *execution*
/// failures never take this path — they are results with `isError: true`.
pub(crate) struct McpError {
    pub(crate) code: i64,
    pub(crate) message: String,
    /// Spec-defined codes may carry structured detail — -32022 lists the
    /// versions the server does speak. Plain JSON-RPC faults leave it None.
    pub(crate) data: Option<Value>,
}

pub(crate) fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn err(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub(crate) fn err_from(id: Value, e: McpError) -> Value {
    let mut error = json!({ "code": e.code, "message": e.message });
    if let Some(data) = e.data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}
