//! Tests for harken::mcp: the stdio MCP server (JSON-RPC 2.0 over
//! newline-delimited JSON). Fully offline: the engine is the shared
//! FakeEngine and the streams are in-memory buffers.

mod common;

use std::fs::File;
use std::io::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use common::FakeEngine;
use harken::engine::Transcriber;

const U200E: &str = "\u{200E}";

/// Drive a whole session through serve() and parse every output line —
/// which doubles as the assertion that stdout carries only JSON frames.
fn run_session(input: &str, engine: &mut dyn Transcriber) -> Vec<Value> {
    let mut out = Vec::new();
    harken::mcp::serve(std::io::Cursor::new(input.as_bytes()), &mut out, engine)
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

fn initialize_line(version: &str) -> String {
    request(
        1,
        "initialize",
        json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0.0.0" }
        }),
    )
}

fn call(id: u64, tool: &str, arguments: Value) -> String {
    request(
        id,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
}

// --- lifecycle ---------------------------------------------------------------

#[test]
fn initialize_handshake() {
    let mut engine = FakeEngine::new(None);
    let input = format!(
        "{}\n{}\n",
        initialize_line("2025-06-18"),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
    );

    let replies = run_session(&input, &mut engine);

    assert_eq!(replies.len(), 1);
    let result = &replies[0]["result"];
    assert_eq!(replies[0]["id"], json!(1));
    assert_eq!(result["protocolVersion"], json!("2025-06-18"));
    assert_eq!(result["serverInfo"]["name"], json!("harken"));
    assert_eq!(
        result["serverInfo"]["version"],
        json!(env!("CARGO_PKG_VERSION"))
    );
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["capabilities"].get("resources").is_none());
    assert!(result["capabilities"].get("prompts").is_none());
}

#[test]
fn initialize_echoes_supported_older_version() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(&format!("{}\n", initialize_line("2024-11-05")), &mut engine);

    assert_eq!(replies[0]["result"]["protocolVersion"], json!("2024-11-05"));
}

#[test]
fn initialize_answers_latest_for_unknown_version() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(&format!("{}\n", initialize_line("1.0.0")), &mut engine);

    // A version mismatch is a result, never an error: disconnecting is the
    // client's decision.
    assert!(replies[0].get("error").is_none());
    assert_eq!(replies[0]["result"]["protocolVersion"], json!("2025-06-18"));
}

#[test]
fn ping_returns_empty_object() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(&format!("{}\n", request(7, "ping", json!({}))), &mut engine);

    assert_eq!(replies[0]["id"], json!(7));
    assert_eq!(replies[0]["result"], json!({}));
}

#[test]
fn notifications_are_never_answered() {
    let mut engine = FakeEngine::new(None);
    let input = format!(
        "{}\n{}\n{}\n{}\n",
        request(1, "ping", json!({})),
        json!({ "jsonrpc": "2.0", "method": "notifications/cancelled" }),
        json!({ "jsonrpc": "2.0", "method": "notifications/does-not-exist" }),
        request(2, "ping", json!({}))
    );

    let replies = run_session(&input, &mut engine);

    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["id"], json!(1));
    assert_eq!(replies[1]["id"], json!(2));
}

#[test]
fn unknown_method_is_32601() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &format!("{}\n", request(3, "resources/list", json!({}))),
        &mut engine,
    );

    assert_eq!(replies[0]["error"]["code"], json!(-32601));
}

#[test]
fn parse_error_is_32700_and_loop_survives() {
    let mut engine = FakeEngine::new(None);
    let input = format!("{{not json\n{}\n", request(9, "ping", json!({})));

    let replies = run_session(&input, &mut engine);

    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["error"]["code"], json!(-32700));
    assert_eq!(replies[0]["id"], json!(null));
    assert_eq!(replies[1]["id"], json!(9));
}

#[test]
fn non_request_json_is_32600() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session("{\"id\": 4, \"no_method\": true}\n", &mut engine);

    assert_eq!(replies[0]["error"]["code"], json!(-32600));
    assert_eq!(replies[0]["id"], json!(4));
}

#[test]
fn empty_input_shuts_down_cleanly() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session("", &mut engine);

    assert!(replies.is_empty());
}

// --- tools/list ---------------------------------------------------------------

#[test]
fn tools_list_shape() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &format!("{}\n", request(1, "tools/list", json!({}))),
        &mut engine,
    );

    let result = &replies[0]["result"];
    assert!(result.get("nextCursor").is_none());
    let tools = result["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["name"], json!("transcribe_file"));
    assert_eq!(tools[0]["inputSchema"]["required"], json!(["path"]));
    assert_eq!(
        tools[0]["inputSchema"]["additionalProperties"],
        json!(false)
    );
    assert_eq!(tools[1]["name"], json!("transcribe_whatsapp_export"));
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["zip_path"]));
    assert_eq!(
        tools[1]["inputSchema"]["additionalProperties"],
        json!(false)
    );
}

// --- tools/call: transcribe_file ------------------------------------------------

#[test]
fn transcribe_file_happy_path() {
    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("a.opus");
    std::fs::write(&audio, b"x").unwrap();
    let mut engine = FakeEngine::new(None);
    let input = format!(
        "{}\n",
        call(
            1,
            "transcribe_file",
            json!({ "path": audio.to_str().unwrap() })
        )
    );

    let replies = run_session(&input, &mut engine);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    assert_eq!(result["content"][0]["type"], json!("text"));
    assert_eq!(result["content"][0]["text"], json!("Hello world."));
    let structured = &result["structuredContent"];
    assert_eq!(structured["text"], json!("Hello world."));
    assert_eq!(structured["language"], json!("en"));
    assert_eq!(structured["duration"], json!(3.0));
    assert_eq!(structured["segments"].as_array().unwrap().len(), 2);
    assert_eq!(structured["segments"][0]["text"], json!("Hello"));
}

#[test]
fn transcribe_file_missing_file_is_tool_error_not_protocol_error() {
    let mut engine = FakeEngine::new(None);
    let input = format!(
        "{}\n",
        call(
            1,
            "transcribe_file",
            json!({ "path": "/nope/missing.opus" })
        )
    );

    let replies = run_session(&input, &mut engine);

    assert!(replies[0].get("error").is_none());
    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(true));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("/nope/missing.opus"), "got: {text}");
}

#[test]
fn transcribe_file_missing_path_arg_is_32602() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &format!("{}\n", call(1, "transcribe_file", json!({}))),
        &mut engine,
    );

    assert_eq!(replies[0]["error"]["code"], json!(-32602));
}

#[test]
fn transcribe_file_unknown_extra_arg_is_32602() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &format!(
            "{}\n",
            call(
                1,
                "transcribe_file",
                json!({ "path": "a.opus", "lang": "en" })
            )
        ),
        &mut engine,
    );

    assert_eq!(replies[0]["error"]["code"], json!(-32602));
}

#[test]
fn unknown_tool_is_32602() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &format!("{}\n", call(1, "make_coffee", json!({}))),
        &mut engine,
    );

    assert_eq!(replies[0]["error"]["code"], json!(-32602));
    let message = replies[0]["error"]["message"].as_str().unwrap();
    assert!(message.contains("make_coffee"), "got: {message}");
}

#[test]
fn one_engine_serves_the_whole_session() {
    let tmp = tempfile::tempdir().unwrap();
    for name in ["a.opus", "b.opus"] {
        std::fs::write(tmp.path().join(name), b"x").unwrap();
    }
    let mut engine = FakeEngine::new(None);
    let input = format!(
        "{}\n{}\n",
        call(
            1,
            "transcribe_file",
            json!({ "path": tmp.path().join("a.opus").to_str().unwrap() })
        ),
        call(
            2,
            "transcribe_file",
            json!({ "path": tmp.path().join("b.opus").to_str().unwrap() })
        )
    );

    let replies = run_session(&input, &mut engine);

    assert_eq!(replies.len(), 2);
    assert_eq!(engine.calls.len(), 2);
}

// --- tools/call: transcribe_whatsapp_export ---------------------------------------

fn write_zip(zip_path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(zip_path).expect("create zip");
    let mut zw = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        zw.start_file(*name, options).expect("start zip entry");
        zw.write_all(data).expect("write zip entry");
    }
    zw.finish().expect("finish zip");
}

/// Two voice notes (Maria on 2026-01-05, Pedro on 2026-01-20), one text-only
/// message and one image attachment that must both be ignored.
fn build_export_zip(zip_path: &Path) {
    let chat_text = format!(
        "Messages and calls are end-to-end encrypted.\n\
         {U200E}[05/01/2026, 10:32:11] Maria: {U200E}<anexado: 00001-AUDIO.opus>\n\
         [05/01/2026, 11:00:00] Pedro: texto puro\n\
         {U200E}[20/01/2026, 08:00:00] Pedro: {U200E}<anexado: 00002-AUDIO.opus>\n\
         {U200E}[21/01/2026, 09:00:00] Maria: {U200E}<anexado: 00003-IMG.jpg>\n"
    );
    write_zip(
        zip_path,
        &[
            ("_chat.txt", chat_text.as_bytes()),
            ("00001-AUDIO.opus", b"fake-audio-1"),
            ("00002-AUDIO.opus", b"fake-audio-2"),
            ("00003-IMG.jpg", b"fake-image"),
        ],
    );
}

fn whatsapp_call(zip_path: &Path, extra: Value) -> String {
    let mut arguments = json!({ "zip_path": zip_path.to_str().unwrap() });
    for (k, v) in extra.as_object().unwrap() {
        arguments[k] = v.clone();
    }
    format!("{}\n", call(1, "transcribe_whatsapp_export", arguments))
}

#[test]
fn whatsapp_export_happy_path() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_export_zip(&zip_path);
    let mut engine = FakeEngine::new(Some("pt".to_string()));

    let replies = run_session(&whatsapp_call(&zip_path, json!({})), &mut engine);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("[2026-01-05 10:32:11] Maria: Hello world."),
        "got: {text}"
    );
    assert!(
        text.contains("[2026-01-20 08:00:00] Pedro: Hello world."),
        "got: {text}"
    );
    assert!(
        text.contains("2 voice notes: 2 transcribed, 0 failed, 0 missing"),
        "got: {text}"
    );
    let structured = &result["structuredContent"];
    assert_eq!(structured["total"], json!(2));
    assert_eq!(structured["transcribed"], json!(2));
    assert_eq!(structured["messages"][0]["sender"], json!("Maria"));
    assert_eq!(structured["messages"][0]["text"], json!("Hello world."));
    assert_eq!(structured["messages"][0]["language"], json!("pt"));
}

#[test]
fn whatsapp_export_date_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_export_zip(&zip_path);
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &whatsapp_call(
            &zip_path,
            json!({ "from": "2026-01-01", "to": "2026-01-10" }),
        ),
        &mut engine,
    );

    let structured = &replies[0]["result"]["structuredContent"];
    assert_eq!(structured["total"], json!(1));
    assert_eq!(structured["messages"][0]["sender"], json!("Maria"));
}

#[test]
fn whatsapp_export_empty_selection_is_success() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_export_zip(&zip_path);
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &whatsapp_call(&zip_path, json!({ "from": "2030-01-01" })),
        &mut engine,
    );

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    assert_eq!(
        result["content"][0]["text"],
        json!("0 voice notes: 0 transcribed, 0 failed, 0 missing")
    );
}

#[test]
fn whatsapp_export_invalid_date_is_tool_error() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_export_zip(&zip_path);
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &whatsapp_call(&zip_path, json!({ "from": "05/01/2026" })),
        &mut engine,
    );

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(true));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("YYYY-MM-DD"), "got: {text}");
}

#[test]
fn whatsapp_export_missing_zip_is_tool_error() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &whatsapp_call(Path::new("/nope/export.zip"), json!({})),
        &mut engine,
    );

    assert_eq!(replies[0]["result"]["isError"], json!(true));
}

#[test]
fn whatsapp_export_zip_without_chat_log_is_tool_error() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    write_zip(&zip_path, &[("00001-AUDIO.opus", b"fake-audio")]);
    let mut engine = FakeEngine::new(None);

    let replies = run_session(&whatsapp_call(&zip_path, json!({})), &mut engine);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(true));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("could not locate a chat log"), "got: {text}");
}

#[test]
fn whatsapp_export_partial_failure_is_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_export_zip(&zip_path);
    let mut engine = FakeEngine::new(None).failing_on("00002-AUDIO.opus");

    let replies = run_session(&whatsapp_call(&zip_path, json!({})), &mut engine);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("FAILED: synthetic failure"), "got: {text}");
    let structured = &result["structuredContent"];
    assert_eq!(structured["transcribed"], json!(1));
    assert_eq!(structured["failed"], json!(1));
    assert_eq!(
        structured["messages"][1]["error"],
        json!("synthetic failure")
    );
}

#[test]
fn whatsapp_export_all_failed_is_tool_error() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_export_zip(&zip_path);
    let mut engine = FakeEngine::new(None)
        .failing_on("00001-AUDIO.opus")
        .failing_on("00002-AUDIO.opus");

    let replies = run_session(&whatsapp_call(&zip_path, json!({})), &mut engine);

    assert_eq!(replies[0]["result"]["isError"], json!(true));
}

#[test]
fn whatsapp_export_attachment_missing_from_zip_is_counted_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    let chat_text = format!(
        "{U200E}[05/01/2026, 10:32:11] Maria: {U200E}<anexado: 00001-AUDIO.opus>\n\
         {U200E}[06/01/2026, 10:00:00] Pedro: {U200E}<anexado: 00099-GONE.opus>\n"
    );
    write_zip(
        &zip_path,
        &[
            ("_chat.txt", chat_text.as_bytes()),
            ("00001-AUDIO.opus", b"fake-audio-1"),
        ],
    );
    let mut engine = FakeEngine::new(None);

    let replies = run_session(&whatsapp_call(&zip_path, json!({})), &mut engine);

    let result = &replies[0]["result"];
    assert_eq!(result["isError"], json!(false));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("MISSING FROM ZIP: 00099-GONE.opus"),
        "got: {text}"
    );
    let structured = &result["structuredContent"];
    assert_eq!(structured["transcribed"], json!(1));
    assert_eq!(structured["missing"], json!(1));
}
