//! Offline tests for src/model.rs download hygiene (issue #13). The
//! download itself is untestable here by the no-network rule; these pin the
//! pieces that decide whether bytes ever get committed to the cache.

use std::fs;

use harken::model::{PartialGuard, partial_path};

#[test]
fn partial_paths_are_unique_per_call() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("ggml-small.bin");

    let a = partial_path(&dest);
    let b = partial_path(&dest);

    // Two concurrent cold starts must not interleave writes into one file.
    assert_ne!(a, b);
    assert_eq!(a.parent(), dest.parent());
    assert!(
        a.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ggml-small.bin.partial-"),
        "partial must be recognizable as belonging to its model: {a:?}"
    );
}

#[test]
fn guard_removes_partial_when_dropped_uncommitted() {
    let tmp = tempfile::tempdir().unwrap();
    let partial = tmp.path().join("ggml-x.bin.partial-1-1");
    fs::write(&partial, b"half a model").unwrap();

    drop(PartialGuard::new(partial.clone()));

    assert!(
        !partial.exists(),
        "a failed download must not leave debris in the cache dir"
    );
}

#[test]
fn guard_commit_moves_partial_into_place() {
    let tmp = tempfile::tempdir().unwrap();
    let partial = tmp.path().join("ggml-x.bin.partial-1-2");
    let dest = tmp.path().join("ggml-x.bin");
    fs::write(&partial, b"whole model").unwrap();

    PartialGuard::new(partial.clone()).commit(&dest).unwrap();

    assert!(!partial.exists());
    assert_eq!(fs::read(&dest).unwrap(), b"whole model");
}

// --- SHA-256 verification (issue #25) ------------------------------------

#[test]
fn linked_etag_with_quotes_parses_to_hex_digest() {
    // HF serves: x-linked-etag: "be07e048...c6e1b21" (quoted, 64 hex chars)
    let digest = harken::model::expected_sha256(
        "\"be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21\"",
    );

    assert_eq!(
        digest.as_deref(),
        Some("be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21")
    );
}

#[test]
fn non_sha256_etag_is_ignored() {
    // A weak etag, a short hash, or junk must disable verification rather
    // than fail the download -- the header is best-effort, not a contract.
    assert_eq!(harken::model::expected_sha256("W/\"abc123\""), None);
    assert_eq!(harken::model::expected_sha256("\"deadbeef\""), None);
    assert_eq!(
        harken::model::expected_sha256(
            "\"ZZ07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1bZZ\""
        ),
        None
    );
}
