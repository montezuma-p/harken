//! Drift guard for the `outputSchema` each tool declares (issue #21).
//!
//! Declaring a schema upgrades drift from "nobody notices" to "the client
//! rejects the result", so the schemas cannot be allowed to describe a payload
//! the code stopped producing. These tests run each tool for real through
//! `serve`, then walk the declared `properties`/`required` key sets against the
//! keys the tool actually emitted — recursively into `segments` and
//! `messages`.
//!
//! Deliberately a key-set comparison and not real JSON Schema validation: a
//! validator crate would buy type checking at the cost of a dependency in a
//! crate whose posture is an auditable dependency list, and the realistic
//! failure is a field added on one side and not the other, which this catches.
//! Offline like everything else — FakeEngine produces the payloads.

mod common;

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use common::FakeEngine;
use harken::mcp::serve;

const U200E: char = '\u{200e}';

fn run_session(input: &str, engine: &mut FakeEngine) -> Vec<Value> {
    let mut out = Vec::new();
    serve(std::io::Cursor::new(input.as_bytes()), &mut out, engine)
        .expect("serve returns Ok on EOF");
    String::from_utf8(out)
        .expect("stdout is UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("every stdout line is JSON"))
        .collect()
}

fn call_line(name: &str, arguments: Value) -> String {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
    .to_string()
        + "\n"
}

fn tool_named(name: &str) -> Value {
    let mut engine = FakeEngine::new(None);
    let input = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string() + "\n";
    run_session(&input, &mut engine)[0]["result"]["tools"]
        .as_array()
        .expect("tools is an array")
        .iter()
        .find(|t| t["name"] == json!(name))
        .unwrap_or_else(|| panic!("{name} is in the catalog"))
        .clone()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect()
}

fn declared_keys(schema: &Value) -> BTreeSet<String> {
    keys(&schema["properties"])
}

fn required_keys(schema: &Value) -> BTreeSet<String> {
    schema["required"]
        .as_array()
        .expect("required is an array")
        .iter()
        .map(|v| v.as_str().expect("required names are strings").to_string())
        .collect()
}

/// Every key the payload emits is declared, and every declared key is present.
/// Exact equality is the point: a field added to the code and not the schema
/// breaks a validating client, and a field declared but never produced is a
/// promise the server does not keep.
fn assert_no_drift(schema: &Value, produced: &Value, what: &str) {
    assert_eq!(
        declared_keys(schema),
        keys(produced),
        "{what}: declared properties and produced keys have drifted"
    );
    assert!(
        required_keys(schema).is_subset(&declared_keys(schema)),
        "{what}: required names a key that is not declared"
    );
}

/// Required keys must be present in a payload; optional ones may be absent, but
/// nothing outside the declared set may appear.
fn assert_no_drift_partial(schema: &Value, produced: &Value, what: &str) {
    let produced = keys(produced);
    assert!(
        produced.is_subset(&declared_keys(schema)),
        "{what}: produced a key the schema does not declare ({:?})",
        produced
            .difference(&declared_keys(schema))
            .collect::<Vec<_>>()
    );
    assert!(
        required_keys(schema).is_subset(&produced),
        "{what}: a required key is missing ({:?})",
        required_keys(schema)
            .difference(&produced)
            .collect::<Vec<_>>()
    );
}

#[test]
fn transcribe_file_output_matches_its_declared_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let audio = tmp.path().join("note.opus");
    std::fs::write(&audio, b"fake").unwrap();
    let mut engine = FakeEngine::new(None);

    let replies = run_session(
        &call_line(
            "transcribe_file",
            json!({ "path": audio.to_str().unwrap() }),
        ),
        &mut engine,
    );
    let produced = &replies[0]["result"]["structuredContent"];
    let schema = tool_named("transcribe_file")["outputSchema"].clone();

    assert_no_drift(&schema, produced, "transcribe_file");

    let segment_schema = &schema["properties"]["segments"]["items"];
    let segments = produced["segments"].as_array().expect("segments array");
    assert!(!segments.is_empty(), "the fixture must produce segments");
    for (i, segment) in segments.iter().enumerate() {
        assert_no_drift(segment_schema, segment, &format!("segments[{i}]"));
    }
}

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

/// Three voice notes covering all three shapes a `messages[]` entry can take:
/// one transcribed, one whose transcription fails, and one whose attachment is
/// missing from the zip. The heterogeneity is exactly what the schema's short
/// `required` list exists for, so the guard has to see all of it.
fn build_mixed_export(zip_path: &Path) {
    let chat_text = format!(
        "Messages and calls are end-to-end encrypted.\n\
         {U200E}[05/01/2026, 10:32:11] Maria: {U200E}<anexado: 00001-AUDIO.opus>\n\
         {U200E}[06/01/2026, 11:00:00] Pedro: {U200E}<anexado: 00002-AUDIO.opus>\n\
         {U200E}[07/01/2026, 12:00:00] Maria: {U200E}<anexado: 00003-AUDIO.opus>\n"
    );
    write_zip(
        zip_path,
        &[
            ("_chat.txt", chat_text.as_bytes()),
            ("00001-AUDIO.opus", b"fake-audio-1"),
            ("00002-AUDIO.opus", b"fake-audio-2"),
            // 00003 is referenced by the chat but absent from the zip.
        ],
    );
}

#[test]
fn whatsapp_export_output_matches_its_declared_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("export.zip");
    build_mixed_export(&zip_path);
    let mut engine = FakeEngine::new(None).failing_on("00002-AUDIO.opus");

    let replies = run_session(
        &call_line(
            "transcribe_whatsapp_export",
            json!({ "zip_path": zip_path.to_str().unwrap() }),
        ),
        &mut engine,
    );
    let produced = &replies[0]["result"]["structuredContent"];
    let schema = tool_named("transcribe_whatsapp_export")["outputSchema"].clone();

    assert_no_drift(&schema, produced, "transcribe_whatsapp_export");
    assert_eq!(produced["transcribed"], json!(1));
    assert_eq!(produced["failed"], json!(1));
    assert_eq!(produced["missing"], json!(1));

    let message_schema = &schema["properties"]["messages"]["items"];
    let messages = produced["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 3, "all three shapes must be exercised");
    for (i, message) in messages.iter().enumerate() {
        assert_no_drift_partial(message_schema, message, &format!("messages[{i}]"));
    }
}

#[test]
fn status_output_matches_its_declared_schema() {
    let mut engine = FakeEngine::new(None);

    let replies = run_session(&call_line("transcribe_status", json!({})), &mut engine);
    let produced = &replies[0]["result"]["structuredContent"];
    let schema = tool_named("transcribe_status")["outputSchema"].clone();

    assert_no_drift(&schema, produced, "transcribe_status");
    let warm = produced["warm"].as_str().expect("warm is a string");
    let allowed = schema["properties"]["warm"]["enum"]
        .as_array()
        .expect("warm declares an enum");
    assert!(
        allowed.iter().any(|v| v == warm),
        "warm produced {warm:?}, which the declared enum does not allow"
    );
}

#[test]
fn every_tool_declares_an_output_schema_and_read_only_annotations() {
    for name in [
        "transcribe_file",
        "transcribe_whatsapp_export",
        "transcribe_status",
    ] {
        let tool = tool_named(name);
        assert_eq!(
            tool["outputSchema"]["type"],
            json!("object"),
            "{name} must declare an object outputSchema"
        );
        assert_eq!(
            tool["annotations"],
            json!({ "readOnlyHint": true, "openWorldHint": false }),
            "{name}: all three tools are read-only and closed-world"
        );
        // destructiveHint/idempotentHint are meaningful only when readOnlyHint
        // is false, so emitting them would be noise a client has to ignore.
        assert!(tool["annotations"]["destructiveHint"].is_null());
        assert!(tool["annotations"]["idempotentHint"].is_null());
    }
}
