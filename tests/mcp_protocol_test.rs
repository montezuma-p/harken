//! Protocol-level tests for harken::mcp: framing, era detection, and the
//! error codes the server is allowed to emit.
//!
//! Separate from mcp_test.rs, which is the ported behavior spec for the tools
//! themselves and is deliberately left diff-free. Fully offline: FakeEngine
//! over in-memory buffers.

mod common;

use serde_json::{Value, json};

use common::FakeEngine;
use harken::engine::Transcriber;

/// Like mcp_test.rs's run_session, but takes raw bytes so a frame can be
/// invalid UTF-8. Parses every output line, which doubles as the assertion
/// that stdout carries only JSON frames.
fn run_session_bytes(input: &[u8], engine: &mut dyn Transcriber) -> Vec<Value> {
    let mut out = Vec::new();
    harken::mcp::serve(std::io::Cursor::new(input.to_vec()), &mut out, engine)
        .expect("serve returns Ok on EOF");
    String::from_utf8(out)
        .expect("stdout is UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("every stdout line is JSON"))
        .collect()
}

fn request(id: u64, method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

// --- framing -----------------------------------------------------------------

#[test]
fn invalid_utf8_line_is_32700_and_loop_survives() {
    // 0xff is never valid UTF-8. Before this was fixed, BufRead::lines()
    // yielded Err here and serve() returned it, killing the whole session.
    let mut input: Vec<u8> = Vec::new();
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"");
    input.push(0xff);
    input.extend_from_slice(b"\"}\n");
    input.extend_from_slice(request(2, "ping", json!({})).as_bytes());
    input.push(b'\n');

    let replies = run_session_bytes(&input, &mut FakeEngine::new(None));

    assert_eq!(replies.len(), 2, "the bad frame is answered, not fatal");
    assert_eq!(replies[0]["error"]["code"], -32700);
    assert_eq!(replies[0]["id"], Value::Null);
    // The session survived: the next frame is still served.
    assert_eq!(replies[1]["id"], 2);
    assert_eq!(replies[1]["result"], json!({}));
}

#[test]
fn crlf_framing_is_accepted() {
    let mut input = request(1, "ping", json!({})).into_bytes();
    input.extend_from_slice(b"\r\n");

    let replies = run_session_bytes(&input, &mut FakeEngine::new(None));

    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["result"], json!({}));
}

#[test]
fn a_final_frame_without_a_trailing_newline_is_served() {
    let input = request(1, "ping", json!({})).into_bytes();

    let replies = run_session_bytes(&input, &mut FakeEngine::new(None));

    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["id"], 1);
}

// --- tools/call params -------------------------------------------------------

#[test]
fn tools_call_without_a_name_is_invalid_params() {
    // Previously the missing name defaulted to "" and produced the nonsense
    // message `Unknown tool: `, which tells a client nothing about the fault.
    let mut input = request(1, "tools/call", json!({ "arguments": {} })).into_bytes();
    input.push(b'\n');

    let replies = run_session_bytes(&input, &mut FakeEngine::new(None));

    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["error"]["code"], -32602);
    let message = replies[0]["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("missing tool name"),
        "message should name the fault, got: {message}"
    );
}

#[test]
fn tools_call_with_an_unknown_name_still_names_it() {
    let mut input = request(1, "tools/call", json!({ "name": "make_coffee" })).into_bytes();
    input.push(b'\n');

    let replies = run_session_bytes(&input, &mut FakeEngine::new(None));

    assert_eq!(replies[0]["error"]["code"], -32602);
    assert!(
        replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("make_coffee")
    );
}

// --- era detection -----------------------------------------------------------

/// A 2026-07-28 request: the era is carried per-request in `_meta`, with no
/// handshake before it.
fn modern_request(id: u64, method: &str, meta_extra: Value) -> String {
    let mut meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    for (k, v) in meta_extra.as_object().cloned().unwrap_or_default() {
        meta[k] = v;
    }
    request(id, method, json!({ "_meta": meta }))
}

fn drive(frames: &[String]) -> Vec<Value> {
    let mut input = Vec::new();
    for f in frames {
        input.extend_from_slice(f.as_bytes());
        input.push(b'\n');
    }
    run_session_bytes(&input, &mut FakeEngine::new(None))
}

#[test]
fn modern_request_with_unsupported_version_is_32022() {
    let frame = request(
        1,
        "tools/list",
        json!({ "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2025-11-25",
            "io.modelcontextprotocol/clientCapabilities": {},
        }}),
    );

    let replies = drive(&[frame]);

    assert_eq!(replies[0]["error"]["code"], -32022);
    assert_eq!(
        replies[0]["error"]["data"]["supported"],
        json!(["2026-07-28"]),
        "the error must tell the client what we do speak"
    );
    assert!(replies[0].get("result").is_none());
}

#[test]
fn modern_request_missing_client_capabilities_is_32602() {
    // protocolVersion present makes this a modern request; capabilities are
    // required on every one of them, so its absence is malformed params.
    let frame = request(
        1,
        "tools/list",
        json!({ "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        }}),
    );

    let replies = drive(&[frame]);

    assert_eq!(replies[0]["error"]["code"], -32602);
}

#[test]
fn a_modern_tools_list_is_served() {
    let replies = drive(&[modern_request(1, "tools/list", json!({}))]);

    let tools = replies[0]["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 2);
}

#[test]
fn the_era_is_per_request_and_not_remembered() {
    // A modern frame, then a legacy one on the same session. The legacy frame
    // must not inherit anything from the modern one — that is the whole point
    // of the stateless model.
    let replies = drive(&[
        modern_request(1, "tools/list", json!({})),
        request(2, "tools/list", json!({})),
    ]);

    assert_eq!(replies.len(), 2);
    assert!(replies[0]["result"]["tools"].is_array());
    assert!(replies[1]["result"]["tools"].is_array());
}

// --- server/discover ---------------------------------------------------------

#[test]
fn discover_returns_versions_capabilities_and_cache_hints() {
    let replies = drive(&[modern_request(1, "server/discover", json!({}))]);

    let r = &replies[0]["result"];
    assert_eq!(r["resultType"], "complete");
    assert_eq!(r["supportedVersions"], json!(["2026-07-28"]));
    assert!(r["capabilities"]["tools"].is_object());
    assert!(r["instructions"].as_str().unwrap().contains("offline"));
    assert!(
        r["ttlMs"].as_i64().is_some_and(|t| t >= 0),
        "ttlMs must be a non-negative integer"
    );
    assert_eq!(r["cacheScope"], "public");
    assert_eq!(
        r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "harken"
    );
    assert!(
        r["_meta"]["io.modelcontextprotocol/serverInfo"]["version"]
            .as_str()
            .is_some()
    );
}

#[test]
fn discover_does_not_advertise_the_handshake_revisions() {
    // A client that picked 2025-06-18 here and then sent it as
    // _meta.protocolVersion would be contradicting itself. The handshake stays
    // an undeclared affordance.
    let replies = drive(&[modern_request(1, "server/discover", json!({}))]);

    let versions = replies[0]["result"]["supportedVersions"].to_string();
    assert!(!versions.contains("2025-06-18"));
    assert!(!versions.contains("2024-11-05"));
}

#[test]
fn discover_without_per_request_meta_is_an_error_not_a_result() {
    // The stdio backward-compatibility probe: a dual-era client sends
    // server/discover first and falls back to initialize on *any* error, so
    // what matters is that this is not answered with a DiscoverResult.
    let replies = drive(&[request(1, "server/discover", json!({}))]);

    assert!(replies[0].get("result").is_none());
    assert_eq!(replies[0]["error"]["code"], -32602);
}

#[test]
fn the_legacy_handshake_still_works_after_a_failed_probe() {
    // The full dual-era client flow: probe, fall back, then use the server.
    let replies = drive(&[
        request(1, "server/discover", json!({})),
        request(
            2,
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
        ),
        request(3, "tools/list", json!({})),
    ]);

    assert_eq!(replies[0]["error"]["code"], -32602);
    assert_eq!(replies[1]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[2]["result"]["tools"].as_array().unwrap().len(), 2);
}
