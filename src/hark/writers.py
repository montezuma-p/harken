"""Output writers for transcription results: txt, json, srt, and manifest."""

from __future__ import annotations

import json
from pathlib import Path

from hark.core import TranscriptionResult


def write_txt(result: TranscriptionResult, dest: Path) -> None:
    dest.write_text(result.text + "\n", encoding="utf-8")


def write_json(result: TranscriptionResult, dest: Path) -> None:
    data = {
        "source": str(result.source),
        "language": result.language,
        "duration": result.duration,
        "text": result.text,
        "segments": [
            {"start": s.start, "end": s.end, "text": s.text} for s in result.segments
        ],
    }
    dest.write_text(
        json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )


def _srt_timestamp(seconds: float) -> str:
    total_ms = round(seconds * 1000)
    hours, rem_ms = divmod(total_ms, 3_600_000)
    minutes, rem_ms = divmod(rem_ms, 60_000)
    secs, millis = divmod(rem_ms, 1000)
    return f"{hours:02d}:{minutes:02d}:{secs:02d},{millis:03d}"


def write_srt(result: TranscriptionResult, dest: Path) -> None:
    blocks = []
    for i, segment in enumerate(result.segments, start=1):
        start = _srt_timestamp(segment.start)
        end = _srt_timestamp(segment.end)
        blocks.append(f"{i}\n{start} --> {end}\n{segment.text}\n\n")
    dest.write_text("".join(blocks), encoding="utf-8")


def append_manifest(
    manifest: Path, result: TranscriptionResult, output_file: Path
) -> None:
    entry = {
        "source": str(result.source),
        "output": str(output_file),
        "language": result.language,
        "duration": result.duration,
        "text": result.text,
    }
    with manifest.open("a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")
