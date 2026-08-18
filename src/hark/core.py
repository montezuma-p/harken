"""Core transcription types and the Transcriber wrapper around faster-whisper."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass
class Segment:
    start: float
    end: float
    text: str


@dataclass
class TranscriptionResult:
    source: Path
    text: str
    segments: list[Segment]
    language: str
    duration: float


class Transcriber:
    """Thin wrapper around faster_whisper.WhisperModel.

    The model is loaded lazily on the first call to transcribe() and
    reused for every subsequent call on the same instance.
    """

    def __init__(
        self,
        model_size: str = "small",
        device: str = "cpu",
        compute_type: str = "int8",
        language: str | None = None,
    ):
        self.model_size = model_size
        self.device = device
        self.compute_type = compute_type
        self.language = language
        self._model = None

    def _load_model(self):
        if self._model is None:
            from faster_whisper import WhisperModel

            self._model = WhisperModel(
                self.model_size, device=self.device, compute_type=self.compute_type
            )
        return self._model

    def transcribe(self, path: Path) -> TranscriptionResult:
        if not path.exists():
            raise FileNotFoundError(f"Audio file not found: {path}")

        model = self._load_model()
        segments_gen, info = model.transcribe(
            str(path), language=self.language, vad_filter=True
        )
        # faster-whisper segment text carries a leading space by convention
        # (from the tokenizer); strip it so joined text and per-segment
        # cues (SRT/JSON) don't end up with stray or doubled whitespace.
        segments = [
            Segment(start=s.start, end=s.end, text=s.text.strip())
            for s in segments_gen
        ]
        text = " ".join(s.text for s in segments)

        return TranscriptionResult(
            source=path,
            text=text,
            segments=segments,
            language=info.language,
            duration=info.duration,
        )
