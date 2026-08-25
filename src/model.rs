//! Model name resolution: map `--model <name>` to a local ggml file,
//! downloading it from Hugging Face (ggerganov/whisper.cpp) on first use.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use indicatif::{ProgressBar, ProgressStyle};

/// Base model names published in the ggerganov/whisper.cpp HF repo.
pub const KNOWN_MODELS: &[&str] = &[
    "tiny",
    "tiny.en",
    "base",
    "base.en",
    "small",
    "small.en",
    "medium",
    "medium.en",
    "large-v1",
    "large-v2",
    "large-v3",
    "large-v3-turbo",
];

/// Quantization suffixes accepted on any base name (e.g. `small-q5_1`).
const QUANT_SUFFIXES: &[&str] = &["-q5_0", "-q5_1", "-q8_0"];

const HF_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

fn is_known_name(name: &str) -> bool {
    if KNOWN_MODELS.contains(&name) {
        return true;
    }
    QUANT_SUFFIXES.iter().any(|suffix| {
        name.strip_suffix(suffix)
            .map(|base| KNOWN_MODELS.contains(&base))
            .unwrap_or(false)
    })
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".cache")
        });
    base.join("harken").join("models")
}

/// Where `--model` already resolves locally, if anywhere. Pure stat — this
/// never touches the network, which is what lets the MCP `transcribe_status`
/// tool answer "would a call download?" without ever risking one.
///
/// Ok(Some) — an existing file path, or a known name present in the cache.
/// Ok(None) — a known name that would need a download.
/// Err      — a name that is neither a file nor a known model.
pub fn cached_path(model: &str) -> Result<Option<PathBuf>, String> {
    let as_path = Path::new(model);
    if as_path.is_file() {
        return Ok(Some(as_path.to_path_buf()));
    }

    if !is_known_name(model) {
        return Err(format!(
            "invalid model '{model}': expected a path to a ggml .bin file or one of {} \
             (optionally with a -q5_0/-q5_1/-q8_0 suffix)",
            KNOWN_MODELS.join(", ")
        ));
    }

    let dest = cache_dir().join(format!("ggml-{model}.bin"));
    Ok(dest.is_file().then_some(dest))
}

/// Make `--model` locally available, downloading on a cache miss. Touches
/// only the filesystem and ureq, so it is trivially Send — the MCP mode runs
/// it on a warm-up thread before the first tool call needs it.
pub fn ensure_downloaded(model: &str, sink: &mut dyn ProgressSink) -> Result<PathBuf, String> {
    if let Some(path) = cached_path(model)? {
        return Ok(path);
    }
    let filename = format!("ggml-{model}.bin");
    let dest = cache_dir().join(&filename);
    download_model(&filename, &dest, sink)?;
    Ok(dest)
}

/// Resolve `--model` to a local ggml file path (CLI entry: progress bar).
///
/// A value that is an existing file path is used directly. A known model
/// name resolves to the cache (downloading on first use). Anything else is
/// an error listing the valid names.
pub fn resolve_model(model: &str) -> Result<PathBuf, String> {
    ensure_downloaded(model, &mut BarSink::default())
}

/// Download progress reporting, decoupled from indicatif so MCP mode can
/// report on a piped stderr where a drawn bar would be noise (or worse,
/// escape codes in a client's log). A future MCP progress-notification
/// stream is a third implementor away.
pub trait ProgressSink {
    fn start(&mut self, filename: &str, total_bytes: u64);
    fn advance(&mut self, bytes: u64);
    fn finish(&mut self);
}

/// CLI sink: the indicatif bar (spinner when the length is unknown).
#[derive(Default)]
pub struct BarSink {
    bar: Option<ProgressBar>,
}

impl ProgressSink for BarSink {
    fn start(&mut self, _filename: &str, total_bytes: u64) {
        let bar = if total_bytes > 0 {
            let bar = ProgressBar::new(total_bytes);
            bar.set_style(
                ProgressStyle::with_template(
                    "{bar:40} {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
                )
                .expect("valid template"),
            );
            bar
        } else {
            ProgressBar::new_spinner()
        };
        self.bar = Some(bar);
    }

    fn advance(&mut self, bytes: u64) {
        if let Some(bar) = &self.bar {
            bar.inc(bytes);
        }
    }

    fn finish(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
    }
}

/// MCP sink: one plain stderr line per 10% so a piped log stays readable.
#[derive(Default)]
pub struct StderrSink {
    filename: String,
    total: u64,
    written: u64,
    last_decile: u64,
}

impl ProgressSink for StderrSink {
    fn start(&mut self, filename: &str, total_bytes: u64) {
        self.filename = filename.to_string();
        self.total = total_bytes;
    }

    fn advance(&mut self, bytes: u64) {
        self.written += bytes;
        if self.total == 0 {
            return;
        }
        let decile = self.written * 10 / self.total;
        if decile > self.last_decile {
            self.last_decile = decile;
            eprintln!("download {}: {}%", self.filename, (decile * 10).min(100));
        }
    }

    fn finish(&mut self) {}
}

/// A unique-per-call temp path next to `dest`.
///
/// The nonce (PID + a process-local counter) means two concurrent cold
/// starts — two MCP clients launching at once, or a batch run alongside an
/// agent — each write their own file instead of interleaving into one and
/// committing a corrupt model that both processes think succeeded. Same
/// pattern as the MCP WhatsApp tool's scratch dir. Deliberately not a
/// lockfile: a stale lock after a SIGKILL is a worse failure mode than a
/// benign duplicate download.
pub fn partial_path(dest: &Path) -> PathBuf {
    static PARTIAL_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".partial-{}-{}",
        std::process::id(),
        PARTIAL_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    dest.with_file_name(name)
}

/// Removes the partial file on drop unless the download was committed, so
/// no error path can leave debris — and nothing half-written can ever be
/// renamed into the cache.
pub struct PartialGuard {
    path: PathBuf,
    committed: bool,
}

impl PartialGuard {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Move the completed download into place.
    pub fn commit(mut self, dest: &Path) -> Result<(), String> {
        std::fs::rename(&self.path, dest)
            .map_err(|e| format!("failed to move model into place: {e}"))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for PartialGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Extract a SHA-256 hex digest from an ETag-shaped header value.
///
/// HuggingFace states an LFS object's SHA-256 in `X-Linked-Etag` as a quoted
/// 64-char hex string. Anything else — a weak etag, a chunk hash, junk —
/// returns None and disables verification rather than failing the download:
/// the header is best-effort, not a contract.
pub fn expected_sha256(etag: &str) -> Option<String> {
    let hex = etag.trim().trim_matches('"');
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

fn download_model(filename: &str, dest: &Path, sink: &mut dyn ProgressSink) -> Result<(), String> {
    let url = format!("{HF_BASE_URL}/{filename}");
    eprintln!("downloading model {filename} ...");

    // The SHA-256 lives on the *redirect* response (X-Linked-Etag on the
    // HF -> CDN 302); following redirects automatically would surface only
    // the CDN response, whose plain ETag is a different hash. So take the
    // first hop unfollowed, read the digest, then fetch the Location.
    let first = ureq::get(&url)
        .config()
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .call()
        .map_err(|e| format!("failed to download {url}: {e}"))?;

    let expected = first
        .headers()
        .get("X-Linked-Etag")
        .and_then(|v| v.to_str().ok())
        .and_then(expected_sha256);

    let response = if first.status().is_redirection() {
        let location = first
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| format!("redirect from {url} carried no Location header"))?
            .to_string();
        ureq::get(&location)
            .call()
            .map_err(|e| format!("failed to download {url}: {e}"))?
    } else {
        first
    };

    let total: u64 = response
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    sink.start(filename, total);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let guard = PartialGuard::new(partial_path(dest));
    let mut out = std::fs::File::create(guard.path())
        .map_err(|e| format!("failed to create {}: {e}", guard.path().display()))?;

    let mut reader = response.into_body().into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    let mut hasher = expected
        .as_ref()
        .map(|_| <sha2::Sha256 as sha2::Digest>::new());
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("write failed: {e}"))?;
        if let Some(h) = hasher.as_mut() {
            sha2::Digest::update(h, &buf[..n]);
        }
        written += n as u64;
        sink.advance(n as u64);
    }
    sink.finish();
    drop(out);

    // A dropped connection can end the stream without an IO error; committing
    // a file we already know is the wrong length would poison the cache
    // permanently (every later run reads it as a cache hit and fails context
    // init with no hint).
    if total > 0 && written != total {
        return Err(format!(
            "download truncated: got {written} bytes, expected {total} — not caching, re-run to retry"
        ));
    }

    // The server states its own file's hash, so this catches corruption in
    // transit and misbehaving mirrors. It is NOT provenance: it cannot
    // protect against HuggingFace itself, and must not be read as a
    // supply-chain guard.
    let verified = match (&expected, hasher) {
        (Some(expected), Some(h)) => {
            let computed = format!("{:x}", sha2::Digest::finalize(h));
            if &computed != expected {
                return Err(format!(
                    "download corrupted: SHA-256 {computed} does not match the server-stated {expected} — not caching, re-run to retry"
                ));
            }
            true
        }
        _ => false,
    };

    // If another process won the race, keep its file and drop our temp
    // rather than clobbering a good model with an identical one.
    if dest.is_file() {
        return Ok(());
    }

    guard.commit(dest)?;
    eprintln!(
        "model saved to {}{}",
        dest.display(),
        if verified { " (SHA-256 verified)" } else { "" }
    );
    Ok(())
}
