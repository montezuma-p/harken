//! Raw FFI bindings to whisper.cpp v1.7.6.
//! Keep these definitions in sync with `vendor/whisper.cpp/include/whisper.h`.

use std::ffi::{c_char, c_float, c_int, c_void};

pub type GgmlLogLevel = c_int;
pub type WhisperAlignmentHeadsPreset = c_int;
pub type WhisperSamplingStrategy = c_int;
pub type WhisperToken = c_int;

pub const WHISPER_SAMPLING_GREEDY: WhisperSamplingStrategy = 0;

#[repr(C)]
pub struct WhisperContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct WhisperState {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperAhead {
    pub n_text_layer: c_int,
    pub n_head: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperAheads {
    pub n_heads: usize,
    pub heads: *const WhisperAhead,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperContextParams {
    pub use_gpu: bool,
    pub flash_attn: bool,
    pub gpu_device: c_int,
    pub dtw_token_timestamps: bool,
    pub dtw_aheads_preset: WhisperAlignmentHeadsPreset,
    pub dtw_n_top: c_int,
    pub dtw_aheads: WhisperAheads,
    pub dtw_mem_size: usize,
}

impl Default for WhisperContextParams {
    fn default() -> Self {
        unsafe { whisper_context_default_params() }
    }
}

pub type WhisperNewSegmentCallback =
    Option<unsafe extern "C" fn(*mut WhisperContext, *mut WhisperState, c_int, *mut c_void)>;
pub type WhisperProgressCallback =
    Option<unsafe extern "C" fn(*mut WhisperContext, *mut WhisperState, c_int, *mut c_void)>;
pub type WhisperEncoderBeginCallback =
    Option<unsafe extern "C" fn(*mut WhisperContext, *mut WhisperState, *mut c_void) -> bool>;
pub type WhisperLogitsFilterCallback = Option<
    unsafe extern "C" fn(
        *mut WhisperContext,
        *mut WhisperState,
        *const c_void,
        c_int,
        *mut c_float,
        *mut c_void,
    ),
>;
pub type GgmlAbortCallback = Option<unsafe extern "C" fn(*mut c_void) -> bool>;
pub type GgmlLogCallback = Option<unsafe extern "C" fn(GgmlLogLevel, *const c_char, *mut c_void)>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperGreedyParams {
    pub best_of: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperBeamSearchParams {
    pub beam_size: c_int,
    pub patience: c_float,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperVadParams {
    pub threshold: c_float,
    pub min_speech_duration_ms: c_int,
    pub min_silence_duration_ms: c_int,
    pub max_speech_duration_s: c_float,
    pub speech_pad_ms: c_int,
    pub samples_overlap: c_float,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WhisperFullParams {
    pub strategy: WhisperSamplingStrategy,
    pub n_threads: c_int,
    pub n_max_text_ctx: c_int,
    pub offset_ms: c_int,
    pub duration_ms: c_int,
    pub translate: bool,
    pub no_context: bool,
    pub no_timestamps: bool,
    pub single_segment: bool,
    pub print_special: bool,
    pub print_progress: bool,
    pub print_realtime: bool,
    pub print_timestamps: bool,
    pub token_timestamps: bool,
    pub thold_pt: c_float,
    pub thold_ptsum: c_float,
    pub max_len: c_int,
    pub split_on_word: bool,
    pub max_tokens: c_int,
    pub debug_mode: bool,
    pub audio_ctx: c_int,
    pub tdrz_enable: bool,
    pub suppress_regex: *const c_char,
    pub initial_prompt: *const c_char,
    pub prompt_tokens: *const WhisperToken,
    pub prompt_n_tokens: c_int,
    pub language: *const c_char,
    pub detect_language: bool,
    pub suppress_blank: bool,
    pub suppress_nst: bool,
    pub temperature: c_float,
    pub max_initial_ts: c_float,
    pub length_penalty: c_float,
    pub temperature_inc: c_float,
    pub entropy_thold: c_float,
    pub logprob_thold: c_float,
    pub no_speech_thold: c_float,
    pub greedy: WhisperGreedyParams,
    pub beam_search: WhisperBeamSearchParams,
    pub new_segment_callback: WhisperNewSegmentCallback,
    pub new_segment_callback_user_data: *mut c_void,
    pub progress_callback: WhisperProgressCallback,
    pub progress_callback_user_data: *mut c_void,
    pub encoder_begin_callback: WhisperEncoderBeginCallback,
    pub encoder_begin_callback_user_data: *mut c_void,
    pub abort_callback: GgmlAbortCallback,
    pub abort_callback_user_data: *mut c_void,
    pub logits_filter_callback: WhisperLogitsFilterCallback,
    pub logits_filter_callback_user_data: *mut c_void,
    pub grammar_rules: *const *const c_void,
    pub n_grammar_rules: usize,
    pub i_start_rule: usize,
    pub grammar_penalty: c_float,
    pub vad: bool,
    pub vad_model_path: *const c_char,
    pub vad_params: WhisperVadParams,
}

impl WhisperFullParams {
    pub fn greedy() -> Self {
        unsafe { whisper_full_default_params(WHISPER_SAMPLING_GREEDY) }
    }
}

unsafe extern "C" {
    pub fn whisper_context_default_params() -> WhisperContextParams;
    pub fn whisper_full_default_params(strategy: WhisperSamplingStrategy) -> WhisperFullParams;
    pub fn whisper_init_from_file_with_params(
        path_model: *const c_char,
        params: WhisperContextParams,
    ) -> *mut WhisperContext;
    pub fn whisper_init_state(ctx: *mut WhisperContext) -> *mut WhisperState;
    pub fn whisper_full_with_state(
        ctx: *mut WhisperContext,
        state: *mut WhisperState,
        params: WhisperFullParams,
        samples: *const c_float,
        n_samples: c_int,
    ) -> c_int;
    pub fn whisper_full_n_segments_from_state(state: *mut WhisperState) -> c_int;
    pub fn whisper_full_get_segment_t0_from_state(
        state: *mut WhisperState,
        i_segment: c_int,
    ) -> i64;
    pub fn whisper_full_get_segment_t1_from_state(
        state: *mut WhisperState,
        i_segment: c_int,
    ) -> i64;
    pub fn whisper_full_get_segment_text_from_state(
        state: *mut WhisperState,
        i_segment: c_int,
    ) -> *const c_char;
    pub fn whisper_full_lang_id_from_state(state: *mut WhisperState) -> c_int;
    pub fn whisper_lang_str(id: c_int) -> *const c_char;
    pub fn whisper_free_state(state: *mut WhisperState);
    pub fn whisper_free(ctx: *mut WhisperContext);
    pub fn whisper_log_set(log_callback: GgmlLogCallback, user_data: *mut c_void);
}
