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

fn download_model(filename: &str, dest: &Path) -> Result<(), String> {
    let url = format!("{HF_BASE_URL}/{filename}");
    eprintln!("downloading model {filename} ...");

    let response = ureq::get(&url)
        .call()
        .map_err(|e| format!("failed to download {url}: {e}"))?;

    let total: u64 = response
        .header("Content-Length")
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
    let tmp = dest.with_extension("bin.partial");
    let mut out = std::fs::File::create(&tmp)
        .map_err(|e| format!("failed to create {}: {e}", tmp.display()))?;

    let mut reader = response.into_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("write failed: {e}"))?;
        bar.inc(n as u64);
    }
    bar.finish_and_clear();
    drop(out);

    std::fs::rename(&tmp, dest).map_err(|e| format!("failed to move model into place: {e}"))?;
    eprintln!("model saved to {}", dest.display());
    Ok(())
}
