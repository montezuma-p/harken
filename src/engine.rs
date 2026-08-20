//! Core transcription types and the Transcriber trait.

use std::path::{Path, PathBuf};

pub type EngineError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionResult {
    pub source: PathBuf,
    pub text: String,
    pub segments: Vec<Segment>,
    pub language: String,
    pub duration: f64,
}

/// A transcription engine. The batch pipeline is generic over this so tests
/// run fully offline with a fake engine.
pub trait Transcriber {
    fn transcribe(&mut self, path: &Path) -> Result<TranscriptionResult, EngineError>;
}

/// Join per-segment texts (already trimmed) into the full-transcript text.
pub fn join_segments(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Assemble a TranscriptionResult from raw engine segments.
///
/// Whisper segment text carries a leading space by convention (from the
/// tokenizer); trim it so joined text and per-segment cues (SRT/JSON) don't
/// end up with stray or doubled whitespace.
pub fn assemble_result(
    source: &Path,
    segments: Vec<Segment>,
    language: String,
    duration: f64,
) -> TranscriptionResult {
    let segments: Vec<Segment> = segments
        .into_iter()
        .map(|s| Segment {
            start: s.start,
            end: s.end,
            text: s.text.trim().to_string(),
        })
        .collect();
    let text = join_segments(&segments);
    TranscriptionResult {
        source: source.to_path_buf(),
        text,
        segments,
        language,
        duration,
    }
}

/// Real engine backed by whisper.cpp via whisper-rs. The model is loaded
/// lazily on the first transcribe() call and reused for the whole batch.
pub struct WhisperCppEngine {
    pub model: String,
    pub device: String,
    pub language: Option<String>,
}

impl WhisperCppEngine {
    pub fn new(model: String, device: String, language: Option<String>) -> Self {
        Self {
            model,
            device,
            language,
        }
    }
}

impl Transcriber for WhisperCppEngine {
    fn transcribe(&mut self, path: &Path) -> Result<TranscriptionResult, EngineError> {
        if !path.exists() {
            return Err(format!("Audio file not found: {}", path.display()).into());
        }
        Err("whisper.cpp engine not yet wired (M3)".into())
    }
}
