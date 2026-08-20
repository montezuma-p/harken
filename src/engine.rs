//! Core transcription types and the Transcriber trait.

use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

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
    model: String,
    device: String,
    language: Option<String>,
    ctx: Option<WhisperContext>,
}

impl WhisperCppEngine {
    pub fn new(model: String, device: String, language: Option<String>) -> Self {
        Self {
            model,
            device,
            language,
            ctx: None,
        }
    }

    fn load_context(&mut self) -> Result<&WhisperContext, EngineError> {
        if self.ctx.is_none() {
            // Route whisper.cpp/ggml logs into the (absent) log backend —
            // i.e. silence their chatty stderr output.
            whisper_rs::install_logging_hooks();
            let model_path = crate::model::resolve_model(&self.model)?;
            let mut params = WhisperContextParameters::default();
            params.use_gpu(self.device != "cpu");
            let ctx = WhisperContext::new_with_params(
                model_path.to_str().ok_or("model path is not valid UTF-8")?,
                params,
            )?;
            self.ctx = Some(ctx);
        }
        Ok(self.ctx.as_ref().expect("just loaded"))
    }
}

impl Transcriber for WhisperCppEngine {
    fn transcribe(&mut self, path: &Path) -> Result<TranscriptionResult, EngineError> {
        if !path.exists() {
            return Err(format!("Audio file not found: {}", path.display()).into());
        }

        let samples = crate::audio::decode_audio_16k_mono(path)?;
        let duration = samples.len() as f64 / crate::audio::WHISPER_SAMPLE_RATE as f64;
        let language = self.language.clone();

        let ctx = self.load_context()?;
        let mut state = ctx.create_state()?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language.as_deref().unwrap_or("auto")));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state.full(params, &samples)?;

        let mut segments = Vec::new();
        for segment in state.as_iter() {
            // Whisper timestamps are in centiseconds.
            segments.push(Segment {
                start: segment.start_timestamp() as f64 / 100.0,
                end: segment.end_timestamp() as f64 / 100.0,
                text: segment.to_str_lossy()?.into_owned(),
            });
        }

        let detected = match &self.language {
            Some(lang) => lang.clone(),
            None => whisper_rs::get_lang_str(state.full_lang_id_from_state())
                .unwrap_or("unknown")
                .to_string(),
        };

        Ok(assemble_result(path, segments, detected, duration))
    }
}
