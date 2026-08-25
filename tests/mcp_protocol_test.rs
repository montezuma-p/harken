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
    // 3 since transcribe_status (issue #19) joined the catalog.
    assert_eq!(tools.len(), 3);
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
    assert_eq!(replies[2]["result"]["tools"].as_array().unwrap().len(), 3);
}

// --- the handshake is legacy-only --------------------------------------------

#[test]
fn initialize_under_a_modern_meta_is_32601() {
    let frame = request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
            }
        }),
    );

    let replies = drive(&[frame]);

    assert_eq!(replies[0]["error"]["code"], -32601);
    assert!(
        replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("server/discover"),
        "the error should point at the replacement"
    );
}

#[test]
fn initialize_without_meta_is_still_served() {
    let replies = drive(&[request(
        1,
        "initialize",
        json!({ "protocolVersion": "2024-11-05", "capabilities": {} }),
    )]);

    assert_eq!(replies[0]["result"]["protocolVersion"], "2024-11-05");
}

// --- modern result decoration ------------------------------------------------

#[test]
fn modern_results_carry_result_type_and_server_info() {
    let replies = drive(&[
        modern_request(1, "tools/list", json!({})),
        modern_request(2, "ping", json!({})),
    ]);

    for reply in &replies {
        assert_eq!(reply["result"]["resultType"], "complete");
        assert_eq!(
            reply["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "harken"
        );
    }
}

#[test]
fn a_modern_tool_result_carries_result_type() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("note.opus");
    std::fs::write(&audio, b"not really audio").unwrap();

    let frame = request(
        1,
        "tools/call",
        json!({
            "name": "transcribe_file",
            "arguments": { "path": audio.to_str().unwrap() },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
            }
        }),
    );

    let replies = drive(&[frame]);

    assert_eq!(replies[0]["result"]["resultType"], "complete");
    assert_eq!(replies[0]["result"]["isError"], false);
    assert_eq!(
        replies[0]["result"]["structuredContent"]["text"],
        "Hello world."
    );
}

#[test]
fn modern_errors_are_not_decorated() {
    // resultType belongs to results. An error object must stay an error object.
    let replies = drive(&[modern_request(1, "resources/list", json!({}))]);

    assert_eq!(replies[0]["error"]["code"], -32601);
    assert!(replies[0].get("result").is_none());
    assert!(replies[0]["error"].get("resultType").is_none());
}

#[test]
fn discover_keeps_its_own_result_type_and_server_info() {
    // decorate_reply must not clobber a result that already set them.
    let replies = drive(&[modern_request(1, "server/discover", json!({}))]);

    assert_eq!(replies[0]["result"]["resultType"], "complete");
    let meta = &replies[0]["result"]["_meta"];
    assert_eq!(
        meta.as_object().unwrap().len(),
        1,
        "serverInfo should not be duplicated under a second key"
    );
}

// --- cache hints -------------------------------------------------------------

#[test]
fn a_modern_tools_list_carries_ttl_and_cache_scope() {
    let replies = drive(&[modern_request(1, "tools/list", json!({}))]);

    let r = &replies[0]["result"];
    assert!(
        r["ttlMs"].as_u64().is_some(),
        "ttlMs must be a non-negative integer, got {:?}",
        r["ttlMs"]
    );
    assert_eq!(r["cacheScope"], "public");
}

// --- era separation ----------------------------------------------------------

/// Collect every object key appearing anywhere in a value.
fn all_keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.push(k.clone());
                all_keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| all_keys(i, out)),
        _ => {}
    }
}

#[test]
fn legacy_replies_never_carry_a_modern_only_field() {
    // The correctness crux of serving two eras from one dispatcher: a field
    // introduced in 2026-07-28 leaking into a 2025-06-18 reply gives a deployed
    // client a result shape it has never seen. Checked across every legacy
    // method rather than one, and recursively, so a nested leak also fails.
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("note.opus");
    std::fs::write(&audio, b"not really audio").unwrap();

    let replies = drive(&[
        request(
            1,
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {} }),
        ),
        request(2, "ping", json!({})),
        request(3, "tools/list", json!({})),
        request(
            4,
            "tools/call",
            json!({
                "name": "transcribe_file",
                "arguments": { "path": audio.to_str().unwrap() }
            }),
        ),
        request(5, "does/not/exist", json!({})),
    ]);

    assert_eq!(replies.len(), 5);
    const MODERN_ONLY: &[&str] = &["resultType", "ttlMs", "cacheScope", "_meta"];
    for reply in &replies {
        let mut keys = Vec::new();
        all_keys(reply, &mut keys);
        for field in MODERN_ONLY {
            assert!(
                !keys.iter().any(|k| k == field),
                "legacy reply leaked the modern-only field {field}: {reply}"
            );
        }
    }
}

// --- error codes -------------------------------------------------------------

#[test]
fn every_emitted_error_code_is_spec_defined() {
    // 2026-07-28 partitions the JSON-RPC server-error range: -32000..-32019 is
    // legacy and new implementations should not use it at all, and
    // -32020..-32099 is reserved for the spec, which forbids emitting a code
    // from it that the spec has not defined. This walks every fault the server
    // can produce and holds it to that.
    const ALLOWED: &[i64] = &[
        -32700, // parse error
        -32600, // invalid request
        -32601, // method not found
        -32602, // invalid params
        -32022, // unsupported protocol version
    ];

    let mut frames: Vec<String> = vec![
        // Malformed JSON.
        "{not json".to_string(),
        // Valid JSON, not a request.
        json!({ "id": 1, "no_method": true }).to_string(),
        // Wrong protocol.
        json!({ "jsonrpc": "1.0", "id": 2, "method": "ping" }).to_string(),
        // Unknown method.
        request(3, "resources/list", json!({})),
        // tools/call with no name.
        request(4, "tools/call", json!({ "arguments": {} })),
        // tools/call naming a tool that does not exist.
        request(5, "tools/call", json!({ "name": "make_coffee" })),
        // tools/call with an argument the schema forbids.
        request(
            6,
            "tools/call",
            json!({ "name": "transcribe_file", "arguments": { "nope": 1 } }),
        ),
        // Modern probe without the required per-request metadata.
        request(7, "server/discover", json!({})),
        // Handshake under a modern envelope.
        modern_request(8, "initialize", json!({})),
        // Unknown modern method.
        modern_request(9, "prompts/list", json!({})),
    ];
    // Modern envelope naming a revision we do not speak.
    frames.push(request(
        10,
        "tools/list",
        json!({ "_meta": {
            "io.modelcontextprotocol/protocolVersion": "1999-01-01",
            "io.modelcontextprotocol/clientCapabilities": {},
        }}),
    ));
    // Modern envelope missing the required capabilities field.
    frames.push(request(
        11,
        "tools/list",
        json!({ "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        }}),
    ));

    let mut input = Vec::new();
    for f in &frames {
        input.extend_from_slice(f.as_bytes());
        input.push(b'\n');
    }
    // One frame that is not UTF-8 at all.
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"");
    input.push(0xff);
    input.extend_from_slice(b"\"}\n");

    let replies = run_session_bytes(&input, &mut FakeEngine::new(None));
    assert_eq!(replies.len(), frames.len() + 1, "every frame is answered");

    let mut seen = Vec::new();
    for reply in &replies {
        let code = reply["error"]["code"]
            .as_i64()
            .unwrap_or_else(|| panic!("expected an error, got {reply}"));
        assert!(
            ALLOWED.contains(&code),
            "emitted an error code the spec does not define: {code} in {reply}"
        );
        // Belt and braces on the reserved range, so a future code added without
        // updating ALLOWED still trips something.
        if (-32099..=-32020).contains(&code) {
            assert_eq!(
                code, -32022,
                "reserved-range code {code} is not spec-defined"
            );
        }
        assert!(
            !(-32019..=-32000).contains(&code),
            "code {code} is in the legacy sub-range new implementations must avoid"
        );
        seen.push(code);
    }

    // The table is only meaningful if it actually exercises the range.
    for expected in ALLOWED {
        assert!(
            seen.contains(expected),
            "no frame in this table provokes {expected}"
        );
    }
}
