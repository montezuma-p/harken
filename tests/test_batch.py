"""Tests for hark.batch: file collection and batch transcription."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from hark.batch import BatchStats, collect_audio_files, run_batch
from hark.core import Segment, TranscriptionResult


class FakeTranscriber:
    """Stand-in for hark.core.Transcriber. No faster_whisper involved."""

    def __init__(self, fail_on: set[str] | None = None):
        self.fail_on = fail_on or set()
        self.calls: list[Path] = []

    def transcribe(self, path: Path) -> TranscriptionResult:
        self.calls.append(path)
        if path.name in self.fail_on:
            raise RuntimeError(f"boom on {path.name}")
        text = f"transcript of {path.name}"
        return TranscriptionResult(
            source=path,
            text=text,
            segments=[Segment(0.0, 1.0, text)],
            language="pt",
            duration=1.0,
        )


# --- collect_audio_files -----------------------------------------------


def test_collect_recurses_into_directory(tmp_path):
    (tmp_path / "sub").mkdir()
    a = tmp_path / "sub" / "a.mp3"
    a.write_bytes(b"x")
    b = tmp_path / "b.wav"
    b.write_bytes(b"x")
    (tmp_path / "note.txt").write_text("not audio")

    result = collect_audio_files([str(tmp_path)])

    assert result == sorted([a, b])


def test_collect_expands_glob_and_filters_extensions(tmp_path):
    x = tmp_path / "x.opus"
    x.write_bytes(b"x")
    y = tmp_path / "y.opus"
    y.write_bytes(b"x")
    (tmp_path / "z.txt").write_text("not audio")

    result = collect_audio_files([str(tmp_path / "*")])

    assert result == sorted([x, y])


def test_collect_includes_explicit_non_audio_file(tmp_path):
    weird = tmp_path / "weird.xyz"
    weird.write_bytes(b"x")

    result = collect_audio_files([str(weird)])

    assert result == [weird]


def test_collect_dedups_file_reachable_two_ways(tmp_path):
    a = tmp_path / "a.mp3"
    a.write_bytes(b"x")

    result = collect_audio_files([str(tmp_path), str(a)])

    assert result == [a]


def test_collect_missing_path_exits_2(tmp_path, capsys):
    missing = tmp_path / "nope.mp3"

    with pytest.raises(SystemExit) as exc_info:
        collect_audio_files([str(missing)])

    assert exc_info.value.code == 2
    assert "nope.mp3" in capsys.readouterr().err


# --- run_batch -----------------------------------------------------------


def test_run_batch_writes_output_and_manifest(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    a = src / "a.wav"
    a.write_bytes(b"x")
    out_dir = tmp_path / "out"
    transcriber = FakeTranscriber()

    stats = run_batch([a], out_dir, transcriber, fmt="txt", force=False)

    assert stats == BatchStats(total=1, done=1, skipped=0, failed=0)
    assert (out_dir / "a.txt").read_text() == "transcript of a.wav\n"
    manifest_lines = (out_dir / "manifest.jsonl").read_text().splitlines()
    assert len(manifest_lines) == 1
    entry = json.loads(manifest_lines[0])
    assert entry["source"] == str(a)
    assert entry["output"] == str(out_dir / "a.txt")
    assert entry["text"] == "transcript of a.wav"


def test_run_batch_skips_existing_output_without_force(tmp_path):
    a = tmp_path / "a.wav"
    a.write_bytes(b"x")
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "a.txt").write_text("already here\n")
    transcriber = FakeTranscriber()

    stats = run_batch([a], out_dir, transcriber, fmt="txt", force=False)

    assert stats == BatchStats(total=1, done=0, skipped=1, failed=0)
    assert (out_dir / "a.txt").read_text() == "already here\n"
    assert transcriber.calls == []
    assert not (out_dir / "manifest.jsonl").exists()


def test_run_batch_force_reprocesses_existing_output(tmp_path):
    a = tmp_path / "a.wav"
    a.write_bytes(b"x")
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "a.txt").write_text("already here\n")
    transcriber = FakeTranscriber()

    stats = run_batch([a], out_dir, transcriber, fmt="txt", force=True)

    assert stats == BatchStats(total=1, done=1, skipped=0, failed=0)
    assert (out_dir / "a.txt").read_text() == "transcript of a.wav\n"
    assert transcriber.calls == [a]


def test_run_batch_collision_suffix_independent_of_skip(tmp_path):
    """Stem-collision numbering must not be perturbed by pre-existing outputs.

    First source (dir1/note.wav) maps to note.txt, which already exists on
    disk -> skipped. The second source (dir2/note.wav) must still receive
    the *next* free suffix (note-2.txt) and, since that path is free, must
    be processed rather than itself being skipped or renamed further.
    """
    dir1 = tmp_path / "dir1"
    dir2 = tmp_path / "dir2"
    dir1.mkdir()
    dir2.mkdir()
    note1 = dir1 / "note.wav"
    note1.write_bytes(b"x")
    note2 = dir2 / "note.wav"
    note2.write_bytes(b"x")
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "note.txt").write_text("pre-existing\n")
    transcriber = FakeTranscriber()

    stats = run_batch([note1, note2], out_dir, transcriber, fmt="txt", force=False)

    assert stats == BatchStats(total=2, done=1, skipped=1, failed=0)
    assert (out_dir / "note.txt").read_text() == "pre-existing\n"
    assert (out_dir / "note-2.txt").read_text() == "transcript of note.wav\n"
    assert transcriber.calls == [note2]


def test_run_batch_continues_after_failure_and_reports_it(tmp_path, capsys):
    good = tmp_path / "good.wav"
    good.write_bytes(b"x")
    bad = tmp_path / "bad.wav"
    bad.write_bytes(b"x")
    out_dir = tmp_path / "out"
    transcriber = FakeTranscriber(fail_on={"bad.wav"})

    stats = run_batch([bad, good], out_dir, transcriber, fmt="txt", force=False)

    assert stats == BatchStats(total=2, done=1, skipped=0, failed=1)
    assert (out_dir / "good.txt").exists()
    assert not (out_dir / "bad.txt").exists()
    err = capsys.readouterr().err
    assert "bad.wav" in err
    assert "FAILED" in err
    manifest_lines = (out_dir / "manifest.jsonl").read_text().splitlines()
    assert len(manifest_lines) == 1


def test_run_batch_creates_out_dir(tmp_path):
    a = tmp_path / "a.wav"
    a.write_bytes(b"x")
    out_dir = tmp_path / "does" / "not" / "exist"
    transcriber = FakeTranscriber()

    run_batch([a], out_dir, transcriber, fmt="txt", force=False)

    assert out_dir.is_dir()
