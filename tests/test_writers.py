"""Tests for harken.writers output formats."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from harken.core import Segment, TranscriptionResult
from harken.writers import append_manifest, write_json, write_srt, write_txt


@pytest.fixture
def sample_result(tmp_path):
    return TranscriptionResult(
        source=tmp_path / "note.opus",
        text="Hello world.",
        segments=[
            Segment(start=0.0, end=1.5, text="Hello"),
            Segment(start=1.5, end=3.75, text="world."),
        ],
        language="en",
        duration=3.75,
    )


def test_write_txt_is_plain_text_with_trailing_newline(sample_result, tmp_path):
    dest = tmp_path / "note.txt"

    write_txt(sample_result, dest)

    assert dest.read_text(encoding="utf-8") == "Hello world.\n"


def test_write_json_matches_expected_shape(sample_result, tmp_path):
    dest = tmp_path / "note.json"

    write_json(sample_result, dest)

    raw = dest.read_text(encoding="utf-8")
    assert raw.endswith("\n")
    data = json.loads(raw)
    assert data == {
        "source": str(sample_result.source),
        "language": "en",
        "duration": 3.75,
        "text": "Hello world.",
        "segments": [
            {"start": 0.0, "end": 1.5, "text": "Hello"},
            {"start": 1.5, "end": 3.75, "text": "world."},
        ],
    }


def test_write_srt_uses_standard_numbering_and_timestamps(sample_result, tmp_path):
    dest = tmp_path / "note.srt"

    write_srt(sample_result, dest)

    expected = (
        "1\n"
        "00:00:00,000 --> 00:00:01,500\n"
        "Hello\n"
        "\n"
        "2\n"
        "00:00:01,500 --> 00:00:03,750\n"
        "world.\n"
        "\n"
    )
    assert dest.read_text(encoding="utf-8") == expected


def test_write_srt_formats_hour_and_millisecond_boundaries(tmp_path):
    result = TranscriptionResult(
        source=tmp_path / "long.opus",
        text="late segment",
        segments=[Segment(start=3661.234, end=3662.0, text="late segment")],
        language="en",
        duration=3662.0,
    )
    dest = tmp_path / "long.srt"

    write_srt(result, dest)

    assert dest.read_text(encoding="utf-8") == (
        "1\n01:01:01,234 --> 01:01:02,000\nlate segment\n\n"
    )


def test_append_manifest_writes_one_json_line(sample_result, tmp_path):
    manifest = tmp_path / "manifest.jsonl"
    output_file = tmp_path / "note.txt"

    append_manifest(manifest, sample_result, output_file)

    lines = manifest.read_text(encoding="utf-8").splitlines()
    assert len(lines) == 1
    entry = json.loads(lines[0])
    assert entry == {
        "source": str(sample_result.source),
        "output": str(output_file),
        "language": "en",
        "duration": 3.75,
        "text": "Hello world.",
    }


def test_append_manifest_appends_across_calls(sample_result, tmp_path):
    manifest = tmp_path / "manifest.jsonl"

    append_manifest(manifest, sample_result, tmp_path / "a.txt")
    append_manifest(manifest, sample_result, tmp_path / "b.txt")

    lines = manifest.read_text(encoding="utf-8").splitlines()
    assert len(lines) == 2
    assert json.loads(lines[0])["output"] == str(tmp_path / "a.txt")
    assert json.loads(lines[1])["output"] == str(tmp_path / "b.txt")
