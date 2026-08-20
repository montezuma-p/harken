//! Core transcription types and the Transcriber trait.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Once;

use crate::ffi;

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

/// Real engine backed by whisper.cpp via direct FFI. The model is loaded
/// lazily on the first transcribe() call and reused for the whole batch.
pub struct WhisperCppEngine {
    model: String,
    device: String,
    language: Option<String>,
    ctx: Option<*mut ffi::WhisperContext>,
}

struct WhisperStateGuard(*mut ffi::WhisperState);

impl Drop for WhisperStateGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ffi::whisper_free_state(self.0) };
        }
    }
}

static INSTALL_LOGGING_HOOKS: Once = Once::new();

fn install_logging_hooks() {
    INSTALL_LOGGING_HOOKS.call_once(|| unsafe {
        ffi::whisper_log_set(Some(silent_whisper_log), ptr::null_mut());
    });
}

unsafe extern "C" fn silent_whisper_log(
    _level: ffi::GgmlLogLevel,
    _text: *const c_char,
    _user_data: *mut c_void,
) {
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

    fn load_context(&mut self) -> Result<*mut ffi::WhisperContext, EngineError> {
        if self.ctx.is_none() {
            // Route whisper.cpp/ggml logs into the (absent) log backend —
            // i.e. silence their chatty stderr output.
            install_logging_hooks();
            let model_path = crate::model::resolve_model(&self.model)?;
            let params = ffi::WhisperContextParams {
                use_gpu: self.device != "cpu",
                ..ffi::WhisperContextParams::default()
            };

            let model_path =
                CString::new(model_path.to_str().ok_or("model path is not valid UTF-8")?)?;
            let ctx =
                unsafe { ffi::whisper_init_from_file_with_params(model_path.as_ptr(), params) };
            if ctx.is_null() {
                return Err("failed to initialize whisper.cpp context".into());
            }
            self.ctx = Some(ctx);
        }
        Ok(self.ctx.expect("just loaded"))
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
        let state = unsafe { ffi::whisper_init_state(ctx) };
        if state.is_null() {
            return Err("failed to initialize whisper.cpp state".into());
        }
        let state = WhisperStateGuard(state);

        let mut params = ffi::WhisperFullParams::greedy();
        params.greedy.best_of = 1;
        let language = CString::new(language.as_deref().unwrap_or("auto"))?;
        params.language = language.as_ptr();
        params.print_special = false;
        params.print_progress = false;
        params.print_realtime = false;
        params.print_timestamps = false;

        let n_samples = i32::try_from(samples.len()).map_err(|_| "audio file is too large")?;
        let status = unsafe {
            ffi::whisper_full_with_state(ctx, state.0, params, samples.as_ptr(), n_samples)
        };
        if status != 0 {
            return Err(format!("whisper.cpp transcription failed with code {status}").into());
        }

        let mut segments = Vec::new();
        let n_segments = unsafe { ffi::whisper_full_n_segments_from_state(state.0) };
        for i in 0..n_segments {
            // Whisper timestamps are in centiseconds.
            let text = unsafe {
                let ptr = ffi::whisper_full_get_segment_text_from_state(state.0, i);
                if ptr.is_null() {
                    return Err("whisper.cpp returned a null segment text".into());
                }
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            };
            segments.push(Segment {
                start: unsafe { ffi::whisper_full_get_segment_t0_from_state(state.0, i) as f64 }
                    / 100.0,
                end: unsafe { ffi::whisper_full_get_segment_t1_from_state(state.0, i) as f64 }
                    / 100.0,
                text,
            });
        }

        let detected = match &self.language {
            Some(lang) => lang.clone(),
            None => unsafe {
                let lang = ffi::whisper_lang_str(ffi::whisper_full_lang_id_from_state(state.0));
                if lang.is_null() {
                    "unknown".to_string()
                } else {
                    CStr::from_ptr(lang).to_string_lossy().into_owned()
                }
            },
        };

        Ok(assemble_result(path, segments, detected, duration))
    }
}

impl Drop for WhisperCppEngine {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.take() {
            unsafe { ffi::whisper_free(ctx) };
        }
    }
}
