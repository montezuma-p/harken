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
