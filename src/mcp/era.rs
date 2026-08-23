//! Per-request protocol era.
//!
//! Revision 2026-07-28 removed the `initialize` / `notifications/initialized`
//! handshake: every request now carries its own protocol version and client
//! capabilities in `_meta`, and a server infers nothing from earlier traffic on
//! the same connection. Deployed clients (Claude Code, Claude Desktop, Cursor,
//! Windsurf) still speak the handshake, so both eras are served — discriminated
//! per request, with no state kept between them.
//!
//! The discriminator is the presence of `_meta.protocolVersion`, which the
//! revision makes a required field precisely so a stateless server can branch
//! on it.

use serde_json::{Value, json};

use super::jsonrpc::{INVALID_PARAMS, McpError};

pub(crate) const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// A modern request naming a revision this server does not speak. Spec-defined
/// in the reserved -32020..-32099 range.
pub(crate) const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// Revisions that carry per-request `_meta`.
pub(crate) const MODERN_VERSIONS: &[&str] = &["2026-07-28"];

/// Answered to a legacy client that asked for a revision we don't know.
pub(crate) const LEGACY_LATEST: &str = "2025-06-18";

/// Handshake revisions echoed back on `initialize`. Deliberately *not*
/// advertised by `server/discover`: a client that picked one of these and then
/// sent it as `_meta.protocolVersion` would be contradicting itself, since
/// per-request metadata is a modern-era construct. The handshake stays an
/// undeclared backward-compatibility affordance, which is how the stdio
/// transport's own backward-compatibility section frames it.
pub(crate) const LEGACY_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Natural-language guidance for the model, returned by both handshakes.
const INSTRUCTIONS: &str = "Fully offline whisper.cpp transcription. Model, language and \
                            device are fixed by the server's --model/--lang/--device flags.";

const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// Freshness hint for the tool catalog and for discover. Both describe a
/// surface that is a static array in this binary, so an hour is honest rather
/// than optimistic; a client that caches for it saves a round-trip per session
/// and keeps its upstream prompt cache stable.
const CATALOG_TTL_MS: u64 = 3_600_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Era {
    /// `initialize` handshake, 2025-06-18 and older.
    Legacy,
    /// Stateless per-request `_meta`, 2026-07-28 and newer.
    Modern,
}

fn meta(params: Option<&Value>) -> Option<&Value> {
    params?.get("_meta")
}

pub(crate) fn detect(params: Option<&Value>) -> Era {
    match meta(params).and_then(|m| m.get(META_PROTOCOL_VERSION)) {
        Some(_) => Era::Modern,
        None => Era::Legacy,
    }
}

/// Validate a modern request's envelope before dispatching it.
///
/// Both faults are spec-mandated: an unknown version is -32022 carrying the
/// versions we do speak, and a missing required `_meta` field makes the request
/// malformed, which is -32602.
pub(crate) fn modern_preflight(params: Option<&Value>) -> Result<(), McpError> {
    let meta = meta(params);

    let version = meta
        .and_then(|m| m.get(META_PROTOCOL_VERSION))
        .and_then(|v| v.as_str());
    match version {
        Some(v) if MODERN_VERSIONS.contains(&v) => {}
        Some(v) => {
            return Err(McpError {
                code: UNSUPPORTED_PROTOCOL_VERSION,
                message: format!("Unsupported protocol version: {v}"),
                data: Some(json!({ "supported": MODERN_VERSIONS })),
            });
        }
        // detect() only returns Modern when the key is present, so this is a
        // present-but-not-a-string value.
        None => {
            return Err(McpError {
                code: INVALID_PARAMS,
                message: format!("Invalid params: {META_PROTOCOL_VERSION} must be a string"),
                data: None,
            });
        }
    }

    // A server must not rely on a capability the client did not declare, so the
    // field is required even when — as here — it is empty.
    if meta.and_then(|m| m.get(META_CLIENT_CAPABILITIES)).is_none() {
        return Err(McpError {
            code: INVALID_PARAMS,
            message: format!("Invalid params: {META_CLIENT_CAPABILITIES} is required"),
            data: None,
        });
    }

    Ok(())
}

pub(crate) fn initialize_result(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or(LEGACY_LATEST);
    // Spec: echo a supported requested version, otherwise answer with ours —
    // disconnecting on mismatch is the client's decision, never an error.
    let version = if LEGACY_VERSIONS.contains(&requested) {
        requested
    } else {
        LEGACY_LATEST
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "harken",
            "title": "harken (offline transcription)",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

fn server_info() -> Value {
    json!({ "name": "harken", "version": env!("CARGO_PKG_VERSION") })
}

/// Applied to every reply on its way out.
///
/// A modern result gains `resultType` — required on every result in 2026-07-28
/// — and the server identity, which servers SHOULD report on each result since
/// there is no handshake to carry it. Errors are not results and pass through;
/// so does everything legacy, which is what keeps the ported behavior spec in
/// tests/mcp_test.rs byte-identical. `or_insert` rather than `insert`, so a
/// result that already set either field (discover does) keeps its own.
pub(crate) fn decorate_reply(era: Era, mut reply: Value) -> Value {
    if era != Era::Modern {
        return reply;
    }
    if let Some(result) = reply.get_mut("result").and_then(|r| r.as_object_mut()) {
        result
            .entry("resultType")
            .or_insert_with(|| json!("complete"));
        match result.get_mut("_meta").and_then(|m| m.as_object_mut()) {
            Some(meta) => {
                meta.entry(META_SERVER_INFO).or_insert_with(server_info);
            }
            None => {
                result.insert(
                    "_meta".to_string(),
                    json!({ (META_SERVER_INFO): server_info() }),
                );
            }
        }
    }
    reply
}

/// Marks a list result cacheable. 2026-07-28 requires both fields on every
/// result from tools/list, prompts/list, resources/list and resources/read.
///
/// `cacheScope` is public because harken's catalog does not vary by caller:
/// there is no auth, and model and language are fixed per server instance, so
/// two callers of one process always see the same two tools.
pub(crate) fn cacheable(mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("ttlMs".to_string(), json!(CATALOG_TTL_MS));
        obj.insert("cacheScope".to_string(), json!("public"));
    }
    result
}

/// The 2026-07-28 replacement for the handshake. Servers MUST implement it, and
/// on stdio it doubles as the backward-compatibility probe: a dual-era client
/// sends it first, and answering it is what identifies this server as modern.
///
/// supportedVersions lists only the modern revisions — see LEGACY_VERSIONS for
/// why the handshake ones are deliberately absent.
pub(crate) fn discover_result() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": MODERN_VERSIONS,
        "capabilities": { "tools": {} },
        "instructions": INSTRUCTIONS,
        // The catalog is a static array, so a long freshness hint is honest,
        // and it does not vary by caller, so a shared cache may hold it.
        "ttlMs": CATALOG_TTL_MS,
        "cacheScope": "public",
        "_meta": { (META_SERVER_INFO): server_info() },
    })
}
