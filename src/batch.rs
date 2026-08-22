//! Audio file collection and batch transcription.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::engine::Transcriber;
use crate::writers::{OutputFormat, append_manifest, write_output};

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "opus", "ogg", "oga", "mp3", "m4a", "wav", "flac", "mp4", "webm", "aac", "wma", "amr",
];

const GLOB_CHARS: &[char] = &['*', '?', '['];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn walk_audio_files(dir: &Path, collected: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_audio_files(&path, collected);
        } else if path.is_file() && is_audio(&path) {
            collected.insert(path);
        }
    }
}

/// Resolve CLI input arguments to a sorted, de-duplicated list of files.
///
/// Each input may be:
/// - an explicit file path: included verbatim, regardless of extension;
/// - a directory: recursed, filtered to AUDIO_EXTENSIONS;
/// - a glob pattern: expanded, filtered to AUDIO_EXTENSIONS (files matched
///   by a glob are not "explicit" the way a bare path is).
///
/// A missing explicit path or directory is a hard error: Err carries the raw
/// input, and the caller reports it and exits 2.
pub fn collect_audio_files(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut collected: BTreeSet<PathBuf> = BTreeSet::new();

    for raw in inputs {
        if raw.contains(GLOB_CHARS) {
            if let Ok(matches) = glob::glob(raw) {
                for m in matches.flatten() {
                    if m.is_dir() {
                        walk_audio_files(&m, &mut collected);
                    } else if is_audio(&m) {
                        collected.insert(m);
                    }
                }
            }
            continue;
        }

        let path = PathBuf::from(raw);
        if !path.exists() {
            return Err(raw.clone());
        }

        if path.is_dir() {
            walk_audio_files(&path, &mut collected);
        } else {
            collected.insert(path);
        }
    }

    Ok(collected.into_iter().collect())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BatchStats {
    pub total: usize,
    pub done: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Assign a unique-in-run output path for `source`'s stem.
///
/// Numbering is driven purely by how many times this stem has already been
/// assigned in this run, never by filesystem existence -- that is the
/// skip/force decision's job, made independently below.
fn next_output_path(
    source: &Path,
    out_dir: &Path,
    fmt: OutputFormat,
    seen: &mut HashMap<String, usize>,
) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let count = seen.get(&stem).copied().unwrap_or(0) + 1;
    seen.insert(stem.clone(), count);
    let name = if count == 1 {
        stem
    } else {
        format!("{stem}-{count}")
    };
    out_dir.join(format!("{name}.{}", fmt.extension()))
}

/// Transcribe every file with one shared Transcriber, writing outputs.
///
/// Returns BatchStats; the caller decides the process exit code from it
/// (0 if stats.failed == 0, else 1).
pub fn run_batch(
    files: &[PathBuf],
    out_dir: &Path,
    transcriber: &mut dyn Transcriber,
    fmt: OutputFormat,
    force: bool,
) -> BatchStats {
    std::fs::create_dir_all(out_dir).expect("failed to create output directory");
    let manifest = out_dir.join("manifest.jsonl");
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut stats = BatchStats {
        total: files.len(),
        ..Default::default()
    };

    for (idx, source) in files.iter().enumerate() {
        let i = idx + 1;
        let output_path = next_output_path(source, out_dir, fmt, &mut seen);
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        if output_path.exists() && !force {
            stats.skipped += 1;
            eprintln!("[{i}/{}] {name} ... skipped (exists)", stats.total);
            continue;
        }

        let result = match transcriber.transcribe(source) {
            Ok(r) => r,
            Err(exc) => {
                stats.failed += 1;
                eprintln!("[{i}/{}] {name} ... FAILED: {exc}", stats.total);
                continue;
            }
        };

        write_output(fmt, &result, &output_path).expect("failed to write output");
        append_manifest(&manifest, &result, &output_path).expect("failed to write manifest");
        stats.done += 1;
        eprintln!(
            "[{i}/{}] {name} ... done ({:.1}s audio)",
            stats.total, result.duration
        );
    }

    eprintln!(
        "batch complete: {} done, {} skipped, {} failed (of {})",
        stats.done, stats.skipped, stats.failed, stats.total
    );
    stats
}

/// Batch-mode entry: collect inputs, run the batch, map to an exit code.
pub fn run_batch_mode(
    inputs: &[String],
    out: &str,
    fmt: OutputFormat,
    force: bool,
    transcriber: &mut dyn Transcriber,
) -> i32 {
    let files = match collect_audio_files(inputs) {
        Ok(f) => f,
        Err(raw) => {
            eprintln!("error: path not found: {raw}");
            return 2;
        }
    };
    let stats = run_batch(&files, Path::new(out), transcriber, fmt, force);
    if stats.failed > 0 { 1 } else { 0 }
}
