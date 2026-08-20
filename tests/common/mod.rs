//! Shared offline fake engine for integration tests — the Rust equivalent of
//! the Python suite's FakeWhisperModel (tests/conftest.py in the pre-port
//! history): always yields the segments ("Hello", "world.") and duration 3.0,
//! with the language forced or detected as "en".

use std::path::{Path, PathBuf};

use harken::engine::{assemble_result, EngineError, Segment, Transcriber, TranscriptionResult};

pub struct FakeEngine {
    pub language: Option<String>,
    pub fail_names: Vec<String>,
    pub calls: Vec<PathBuf>,
}

#[allow(dead_code)]
impl FakeEngine {
    pub fn new(language: Option<String>) -> Self {
        Self {
            language,
            fail_names: Vec::new(),
            calls: Vec::new(),
        }
    }

    /// Make transcribe() fail for any path whose file name equals `name`.
    pub fn failing_on(mut self, name: &str) -> Self {
        self.fail_names.push(name.to_string());
        self
    }
}

impl Transcriber for FakeEngine {
    fn transcribe(&mut self, path: &Path) -> Result<TranscriptionResult, EngineError> {
        if !path.exists() {
            return Err(format!("Audio file not found: {}", path.display()).into());
        }
        self.calls.push(path.to_path_buf());
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.fail_names.contains(&name) {
            return Err("synthetic failure".into());
        }
        Ok(assemble_result(
            path,
            vec![
                Segment {
                    start: 0.0,
                    end: 1.5,
                    text: " Hello".to_string(),
                },
                Segment {
                    start: 1.5,
                    end: 3.0,
                    text: " world.".to_string(),
                },
            ],
            self.language.clone().unwrap_or_else(|| "en".to_string()),
            3.0,
        ))
    }
}
