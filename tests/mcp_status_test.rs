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

/// Serializes this file's tests and points the model cache at an empty dir.
///
/// The isolation is what makes "is `small` cached?" a property of the test
/// rather than of the machine: a developer box usually *does* hold
/// ggml-small.bin, so a not-cached expectation would pass in CI and fail
/// locally. Only an env var can move the cache (`model::cache_dir` reads
/// XDG_CACHE_HOME), and writing the environment is only sound while nothing
/// else reads it — which in a test binary includes `tempfile::tempdir()`
/// reading TMPDIR on a parallel thread. So every test here holds this lock for
/// its whole body: they are all instant, and serializing nine of them costs
/// nothing next to a race that would only ever fail in someone else's CI.
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static CACHE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    // A poisoned lock carries no broken state — the guarded value is ().
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = CACHE.get_or_init(|| tempfile::tempdir().expect("temp cache dir"));
    // SAFETY: the lock above is held by every test in this binary, so no other
    // thread is reading or writing the environment while this runs.
    unsafe { std::env::set_var("XDG_CACHE_HOME", dir.path()) };
    guard
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
    let _env = env_guard();
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
        ..ServerInfo::default()
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
    let _env = env_guard();
    // An operator typo in --model should be visible in status, not a tool
    // failure: the agent asking "can this server transcribe?" deserves the
    // diagnosis.
    let info = ServerInfo {
        model: "not-a-model".to_string(),
        lang: "auto".to_string(),
        device: "cpu".to_string(),
        ..ServerInfo::default()
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
fn status_reports_a_warm_up_in_flight_instead_of_a_future_download() {
    let _env = env_guard();
    // The window this exists for: the model is not on disk yet *because* the
    // warm-up thread is downloading it right now. A stat-only answer would say
    // the first call "will download it", and an agent reading that reasonably
    // decides to avoid calling or to warn about a cold start — when the right
    // advice is that a call now just waits a little.
    let info = ServerInfo {
        model: "small".to_string(),
        lang: "pt".to_string(),
        device: "cpu".to_string(),
        // Pending is the state a live warm-up thread leaves it in; nothing here
        // downloads anything.
        warm: Some(Warmth::new()),
    };
    let mut engine = FakeEngine::new(None);

    let replies = run_session_with_info(&status_call_line(), &mut engine, &info);

    let result = &replies[0]["result"];
    let s = &result["structuredContent"];
    assert_eq!(s["cached"], json!(false));
    assert_eq!(s["warm"], json!("downloading"));
    assert_eq!(s["warm_error"], json!(null));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("downloading now") && text.contains("will wait"),
        "the text must say a call waits on the running download: {text}"
    );
    assert!(
        !text.contains("will download it"),
        "the stale 'first call will download it' claim must be gone: {text}"
    );
}

#[test]
fn status_surfaces_a_failed_warm_up() {
    let _env = env_guard();
    // Until now a failed warm-up existed only as a stderr line: an agent asking
    // status right after got the plain "not cached" message with no hint that
    // the download had already failed once.
    let warmth = Warmth::new();
    warmth.set_failed("network down".to_string());
    let info = ServerInfo {
        model: "small".to_string(),
        lang: "pt".to_string(),
        device: "cpu".to_string(),
        warm: Some(warmth),
    };
    let mut engine = FakeEngine::new(None);

    let replies = run_session_with_info(&status_call_line(), &mut engine, &info);

    let result = &replies[0]["result"];
    assert_eq!(
        result["isError"],
        json!(false),
        "a failed warm-up is a diagnosis, not a tool failure"
    );
    let s = &result["structuredContent"];
    assert_eq!(s["warm"], json!("failed"));
    assert_eq!(s["warm_error"], json!("network down"));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("network down") && text.contains("retry"),
        "the recorded failure and the retry behaviour must both surface: {text}"
    );
}

#[test]
fn status_reports_no_warm_up_when_none_is_running() {
    let _env = env_guard();
    // serve() (and any ServerInfo::default()) has no warm thread behind it. A
    // fresh Warmth would read Pending forever and report a download that does
    // not exist, so the absence is modelled as absence.
    let info = ServerInfo::default();
    let mut engine = FakeEngine::new(None);

    let replies = run_session_with_info(&status_call_line(), &mut engine, &info);

    let s = &replies[0]["result"]["structuredContent"];
    assert_eq!(s["warm"], json!("idle"));
    assert_eq!(s["warm_error"], json!(null));
}

#[test]
fn status_tool_is_listed_with_no_required_arguments() {
    let _env = env_guard();
    let info = ServerInfo::default();
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
    let _env = env_guard();
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
fn warm_gate_survives_a_panicking_warm_thread() {
    let _env = env_guard();
    use harken::engine::Transcriber;
    use harken::mcp::WarmSettle;

    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("a.wav");
    std::fs::write(&audio, b"x").unwrap();

    let warmth = Warmth::new();
    let mut engine = FakeEngine::new(None);
    let mut gate = WarmGate::new(&mut engine, warmth.clone());

    // A warm thread that panics never reaches set_ready/set_failed. Without the
    // drop guard the state stays Pending and this transcribe() blocks forever —
    // the join below is the assertion that it did not.
    let result = std::thread::scope(|s| {
        let warm = warmth.clone();
        let panicker = s.spawn(move || {
            let _settle = WarmSettle::new(warm);
            panic!("warm-up exploded");
        });
        let caller = s.spawn(|| gate.transcribe(&audio));
        assert!(panicker.join().is_err(), "the warm thread did panic");
        caller.join().expect("the gated call did not panic")
    });

    assert!(result.is_ok());
    assert_eq!(engine.calls, vec![audio]);
    assert_eq!(
        warmth.failure().as_deref(),
        Some("model warm-up panicked"),
        "the panic must be recorded as a warm-up failure, not left Pending"
    );
}

#[test]
fn warm_gate_after_failed_warm_still_delegates() {
    let _env = env_guard();
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
