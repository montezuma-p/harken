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

/// Resolve `--model` to a local ggml file path.
///
/// A value that is an existing file path is used directly. A known model
/// name resolves to the cache (downloading on first use). Anything else is
/// an error listing the valid names.
pub fn resolve_model(model: &str) -> Result<PathBuf, String> {
    let as_path = Path::new(model);
    if as_path.is_file() {
        return Ok(as_path.to_path_buf());
    }

    if !is_known_name(model) {
        return Err(format!(
            "invalid model '{model}': expected a path to a ggml .bin file or one of {} \
             (optionally with a -q5_0/-q5_1/-q8_0 suffix)",
            KNOWN_MODELS.join(", ")
        ));
    }

    let filename = format!("ggml-{model}.bin");
    let dest = cache_dir().join(&filename);
    if dest.is_file() {
        return Ok(dest);
    }

    download_model(&filename, &dest)?;
    Ok(dest)
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

fn download_model(filename: &str, dest: &Path) -> Result<(), String> {
    let url = format!("{HF_BASE_URL}/{filename}");
    eprintln!("downloading model {filename} ...");

    let response = ureq::get(&url)
        .call()
        .map_err(|e| format!("failed to download {url}: {e}"))?;

    let total: u64 = response
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let bar = if total > 0 {
        let bar = ProgressBar::new(total);
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
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("write failed: {e}"))?;
        written += n as u64;
        bar.inc(n as u64);
    }
    bar.finish_and_clear();
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

    // If another process won the race, keep its file and drop our temp
    // rather than clobbering a good model with an identical one.
    if dest.is_file() {
        return Ok(());
    }

    guard.commit(dest)?;
    eprintln!("model saved to {}", dest.display());
    Ok(())
}
