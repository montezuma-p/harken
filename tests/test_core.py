"""Tests for hark.core.Transcriber and its dataclasses."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest


def test_importing_core_does_not_touch_faster_whisper(monkeypatch):
    """hark.core must not import faster_whisper at module load time."""
    monkeypatch.delitem(sys.modules, "faster_whisper", raising=False)
    monkeypatch.delitem(sys.modules, "hark.core", raising=False)

    import hark.core  # noqa: F401

    assert "faster_whisper" not in sys.modules


def test_transcribe_raises_file_not_found_before_loading_model(fake_whisper, tmp_path):
    from hark.core import Transcriber

    missing = tmp_path / "does-not-exist.opus"
    transcriber = Transcriber()

    with pytest.raises(FileNotFoundError):
        transcriber.transcribe(missing)

    assert fake_whisper.instances == []


def test_model_loads_lazily_and_is_cached_across_calls(fake_whisper, tmp_path):
    from hark.core import Transcriber

    audio = tmp_path / "note.opus"
    audio.write_bytes(b"fake-audio")

    transcriber = Transcriber(model_size="small", device="cpu", compute_type="int8")
    assert fake_whisper.instances == []  # not loaded yet

    transcriber.transcribe(audio)
    assert len(fake_whisper.instances) == 1

    transcriber.transcribe(audio)
    assert len(fake_whisper.instances) == 1  # still just one model built

    model = fake_whisper.instances[0]
    assert model.model_size == "small"
    assert model.device == "cpu"
    assert model.compute_type == "int8"


def test_transcriber_defaults_propagate_to_whisper_model(fake_whisper, tmp_path):
    """A bare Transcriber() with no args must reach WhisperModel with the
    documented CPU/int8 defaults."""
    from hark.core import Transcriber

    audio = tmp_path / "note.opus"
    audio.write_bytes(b"fake-audio")

    transcriber = Transcriber()
    transcriber.transcribe(audio)

    model = fake_whisper.instances[0]
    assert model.model_size == "small"
    assert model.device == "cpu"
    assert model.compute_type == "int8"


def test_transcribe_result_fields_with_detected_language(fake_whisper, tmp_path):
    from hark.core import Transcriber

    audio = tmp_path / "note.opus"
    audio.write_bytes(b"fake-audio")

    transcriber = Transcriber(language=None)
    result = transcriber.transcribe(audio)

    assert result.source == audio
    assert result.text == "Hello world."
    assert [s.text for s in result.segments] == ["Hello", "world."]
    assert result.segments[0].start == 0.0
    assert result.segments[0].end == 1.5
    assert result.language == "en"
    assert result.duration == 3.0


def test_transcribe_forces_language_when_set(fake_whisper, tmp_path):
    from hark.core import Transcriber

    audio = tmp_path / "note.opus"
    audio.write_bytes(b"fake-audio")

    transcriber = Transcriber(language="pt")
    result = transcriber.transcribe(audio)

    model = fake_whisper.instances[0]
    assert model.transcribe_calls[0]["language"] == "pt"
    assert result.language == "pt"


def test_transcribe_strips_leading_space_from_segment_text(
    fake_whisper, tmp_path, monkeypatch
):
    """Real faster-whisper segments carry a leading space (tokenizer artifact);
    joined text and per-segment cues must not inherit it."""
    from hark.core import Transcriber
    from conftest import FakeInfo, FakeSegment

    audio = tmp_path / "note.opus"
    audio.write_bytes(b"fake-audio")

    model = fake_whisper("small")

    def transcribe_with_leading_spaces(path, language=None, vad_filter=True):
        segments = (FakeSegment(0.0, 1.0, " Hello"), FakeSegment(1.0, 2.0, " world."))
        return iter(segments), FakeInfo(language=language or "en", duration=2.0)

    monkeypatch.setattr(model, "transcribe", transcribe_with_leading_spaces)

    transcriber = Transcriber()
    transcriber._model = model  # pre-seed the cache; skips real _load_model

    result = transcriber.transcribe(audio)

    assert result.text == "Hello world."
    assert result.segments[0].text == "Hello"
    assert result.segments[1].text == "world."


def test_transcribe_calls_model_with_expected_args(fake_whisper, tmp_path):
    from hark.core import Transcriber

    audio = tmp_path / "note.opus"
    audio.write_bytes(b"fake-audio")

    transcriber = Transcriber(language="pt")
    transcriber.transcribe(audio)

    call = fake_whisper.instances[0].transcribe_calls[0]
    assert call["path"] == str(audio)
    assert call["vad_filter"] is True
