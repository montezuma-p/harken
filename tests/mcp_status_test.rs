//! Tests for issue #19's MCP-warm surface: the `transcribe_status` tool
//! (never touches the network — it reports cache state via cached_path) and
//! the WarmGate that holds tool calls while the model warm-up thread runs.
//! Fully offline, like every test here.

mod common;

use std::io::Write as _;

use serde_json::{Value, json};

use common::FakeEngine;
use harken::mcp::{ServerInfo, WarmGate, Warmth, serve_with_info};

fn run_session_with_info(input: &str, engine: &mut FakeEngine, info: &ServerInfo) -> Vec<Value> {
    let mut out = Vec::new();
    serve_with_info(
        std::io::Cursor::new(input.as_bytes()),
        &mut out,
        engine,
        info,
    )
    .expect("serve returns Ok on EOF");
    String::from_utf8(out)
        .expect("stdout is UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("every stdout line is JSON"))
        .collect()
}

fn status_call_line() -> String {
    json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": "transcribe_status", "arguments": {} }
    })
    .to_string()
        + "\n"
}

// --- transcribe_status --------------------------------------------------

#[test]
fn status_reports_a_cached_model_with_path_and_size() {
    // A model given as a file path is "cached" by definition; using a temp
    // file keeps the test independent of the machine's real cache dir.
    let tmp = tempfile::tempdir().unwrap();
    let model_file = tmp.path().join("ggml-fake.bin");
    let mut f = std::fs::File::create(&model_file).unwrap();
    f.write_all(&[0u8; 128]).unwrap();
    let info = ServerInfo {
        model: model_file.to_string_lossy().into_owned(),
        lang: "pt".to_string(),
        device: "cpu".to_string(),
    };
    let mut engine = FakeEngine::new(None);

    let replies = run_session_with_info(&status_call_line(), &mut engine, &info);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    let s = &result["structuredContent"];
    assert_eq!(s["model"], json!(info.model));
    assert_eq!(s["language"], json!("pt"));
    assert_eq!(s["device"], json!("cpu"));
    assert_eq!(s["cached"], json!(true));
    assert_eq!(s["path"], json!(model_file.to_string_lossy()));
    assert_eq!(s["size_bytes"], json!(128));
    assert_eq!(s["context_loaded"], json!(false));
    assert!(
        engine.calls.is_empty(),
        "status must not transcribe anything"
    );
}

#[test]
fn status_reports_an_invalid_model_without_erroring() {
    // An operator typo in --model should be visible in status, not a tool
    // failure: the agent asking "can this server transcribe?" deserves the
    // diagnosis.
    let info = ServerInfo {
        model: "not-a-model".to_string(),
        lang: "auto".to_string(),
        device: "cpu".to_string(),
    };
    let mut engine = FakeEngine::new(None);

    let replies = run_session_with_info(&status_call_line(), &mut engine, &info);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    let s = &result["structuredContent"];
    assert_eq!(s["cached"], json!(false));
    assert!(
        s["error"].as_str().unwrap().contains("invalid model"),
        "the typo diagnosis must surface: {s}"
    );
}

#[test]
fn status_tool_is_listed_with_no_required_arguments() {
    let info = ServerInfo {
        model: "small".to_string(),
        lang: "pt".to_string(),
        device: "cpu".to_string(),
    };
    let mut engine = FakeEngine::new(None);
    let input = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string() + "\n";

    let replies = run_session_with_info(&input, &mut engine, &info);

    let tools = replies[0]["result"]["tools"].as_array().unwrap();
    let status = tools
        .iter()
        .find(|t| t["name"] == json!("transcribe_status"))
        .expect("transcribe_status is in the catalog");
    assert_eq!(status["inputSchema"]["required"], json!([]));
    assert_eq!(status["inputSchema"]["additionalProperties"], json!(false));
}

// --- WarmGate ------------------------------------------------------------

#[test]
fn warm_gate_blocks_a_call_until_ready_then_delegates() {
    use harken::engine::Transcriber;

    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("a.wav");
    std::fs::write(&audio, b"x").unwrap();

    let warmth = Warmth::new();
    let mut engine = FakeEngine::new(None);
    let mut gate = WarmGate::new(&mut engine, warmth.clone());

    // transcribe() blocks while Pending; marking ready from another thread
    // releases it. No sleeps: the join IS the assertion that it unblocked.
    let result = std::thread::scope(|s| {
        let handle = s.spawn(|| gate.transcribe(&audio));
        warmth.set_ready();
        handle.join().expect("no panic")
    });

    assert!(result.is_ok());
    assert_eq!(engine.calls, vec![audio]);
}

#[test]
fn warm_gate_after_failed_warm_still_delegates() {
    use harken::engine::Transcriber;

    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("a.wav");
    std::fs::write(&audio, b"x").unwrap();

    let warmth = Warmth::new();
    warmth.set_failed("network down".to_string());
    let mut engine = FakeEngine::new(None);
    let mut gate = WarmGate::new(&mut engine, warmth);

    // A failed warm-up must not kill or wedge the server: the call goes
    // through to the engine, whose own model resolution retries the download
    // inline and surfaces a live error if it still fails.
    let result = gate.transcribe(&audio);

    assert!(result.is_ok());
    assert_eq!(engine.calls, vec![audio]);
}
