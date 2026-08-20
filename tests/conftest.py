"""Shared pytest fixtures for harken tests."""

from __future__ import annotations

import sys
import types
from dataclasses import dataclass, field

import pytest


@dataclass
class FakeSegment:
    start: float
    end: float
    text: str


@dataclass
class FakeInfo:
    language: str
    duration: float


class FakeWhisperModel:
    """Stand-in for faster_whisper.WhisperModel.

    Records every construction so tests can assert the model was built
    lazily and only once per Transcriber instance.
    """

    instances: list["FakeWhisperModel"] = []

    def __init__(self, model_size, device="cpu", compute_type="int8"):
        self.model_size = model_size
        self.device = device
        self.compute_type = compute_type
        self.transcribe_calls: list[dict] = []
        FakeWhisperModel.instances.append(self)

    def transcribe(self, path, language=None, vad_filter=True):
        self.transcribe_calls.append(
            {"path": path, "language": language, "vad_filter": vad_filter}
        )
        segments = (
            FakeSegment(0.0, 1.5, "Hello"),
            FakeSegment(1.5, 3.0, "world."),
        )
        info = FakeInfo(language=language or "en", duration=3.0)
        return iter(segments), info


@pytest.fixture
def fake_whisper(monkeypatch):
    """Install a stub `faster_whisper` module into sys.modules.

    Ensures harken.core never has to import the real (heavy) library
    during tests. Resets the recorded instances before each test.
    """
    FakeWhisperModel.instances = []
    module = types.ModuleType("faster_whisper")
    module.WhisperModel = FakeWhisperModel
    monkeypatch.setitem(sys.modules, "faster_whisper", module)
    yield FakeWhisperModel
