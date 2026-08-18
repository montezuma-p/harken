"""Tests for hark.cli: argument parsing and whatsapp delegation."""

from __future__ import annotations

import sys
import types

import pytest

from hark import cli


# --- build_parser ---------------------------------------------------------


def test_parser_defaults():
    parser = cli.build_parser()
    args = parser.parse_args(["a.wav"])

    assert args.inputs == ["a.wav"]
    assert args.out == "./transcripts"
    assert args.model == "small"
    assert args.lang == "pt"
    assert args.format == "txt"
    assert args.device == "cpu"
    assert args.force is False


def test_parser_accepts_multiple_inputs_and_flags():
    parser = cli.build_parser()
    args = parser.parse_args(
        [
            "a.wav",
            "b.mp3",
            "--out",
            "/tmp/out",
            "--model",
            "medium",
            "--lang",
            "auto",
            "--format",
            "srt",
            "--device",
            "cuda",
            "--force",
        ]
    )

    assert args.inputs == ["a.wav", "b.mp3"]
    assert args.out == "/tmp/out"
    assert args.model == "medium"
    assert args.lang == "auto"
    assert args.format == "srt"
    assert args.device == "cuda"
    assert args.force is True


def test_parser_rejects_bad_format():
    parser = cli.build_parser()
    with pytest.raises(SystemExit):
        parser.parse_args(["a.wav", "--format", "mp3"])


# --- main(): whatsapp delegation ------------------------------------------


def test_main_delegates_whatsapp_subcommand(monkeypatch):
    calls = []
    fake_module = types.ModuleType("hark.whatsapp")

    def fake_main(argv):
        calls.append(argv)
        return 0

    fake_module.main = fake_main
    monkeypatch.setitem(sys.modules, "hark.whatsapp", fake_module)

    exit_code = cli.main(["whatsapp", "export.zip", "--merge"])

    assert calls == [["export.zip", "--merge"]]
    assert exit_code == 0


def test_main_batch_mode_does_not_require_whatsapp_module(monkeypatch):
    monkeypatch.delitem(sys.modules, "hark.whatsapp", raising=False)
    monkeypatch.setattr(cli, "collect_audio_files", lambda inputs: [])

    exit_code = cli.main(["a.wav"])

    assert exit_code == 0
    assert "hark.whatsapp" not in sys.modules


# --- main(): lang auto -> language=None -----------------------------------


def test_main_lang_auto_passes_language_none_to_transcriber(monkeypatch, tmp_path):
    captured = {}

    class CapturingTranscriber:
        def __init__(self, **kwargs):
            captured.update(kwargs)

        def transcribe(self, path):  # pragma: no cover - not exercised
            raise AssertionError("should not be called")

    monkeypatch.setattr(cli, "Transcriber", CapturingTranscriber)
    monkeypatch.setattr(cli, "collect_audio_files", lambda inputs: [])

    exit_code = cli.main(["--lang", "auto", "--out", str(tmp_path / "out"), "a.wav"])

    assert captured["language"] is None
    assert exit_code == 0


def test_main_default_lang_pt_passed_through(monkeypatch, tmp_path):
    captured = {}

    class CapturingTranscriber:
        def __init__(self, **kwargs):
            captured.update(kwargs)

        def transcribe(self, path):  # pragma: no cover - not exercised
            raise AssertionError("should not be called")

    monkeypatch.setattr(cli, "Transcriber", CapturingTranscriber)
    monkeypatch.setattr(cli, "collect_audio_files", lambda inputs: [])

    cli.main(["--out", str(tmp_path / "out"), "a.wav"])

    assert captured["language"] == "pt"


def test_main_end_to_end_success_writes_real_output(monkeypatch, tmp_path):
    """Drive real main() -> real collect_audio_files -> real run_batch.

    Only Transcriber is faked (no faster_whisper); everything else -- CLI
    parsing, file collection, batch orchestration, writers -- runs for
    real against a temp directory, so this exercises the actual wiring
    rather than a chain of mocked-out seams.
    """
    from hark.core import Segment, TranscriptionResult

    class FakeTranscriber:
        def __init__(self, **kwargs):
            pass

        def transcribe(self, path):
            text = f"transcript of {path.name}"
            return TranscriptionResult(
                source=path,
                text=text,
                segments=[Segment(0.0, 1.0, text)],
                language="pt",
                duration=1.0,
            )

    monkeypatch.setattr(cli, "Transcriber", FakeTranscriber)

    audio = tmp_path / "sample.wav"
    audio.write_bytes(b"x")
    out_dir = tmp_path / "out"

    exit_code = cli.main(["--out", str(out_dir), str(audio)])

    assert exit_code == 0
    assert (out_dir / "sample.txt").read_text() == "transcript of sample.wav\n"
    assert (out_dir / "manifest.jsonl").exists()


def test_main_returns_1_when_a_file_fails(monkeypatch, tmp_path):
    from hark.batch import BatchStats

    monkeypatch.setattr(cli, "collect_audio_files", lambda inputs: [tmp_path / "a.wav"])
    monkeypatch.setattr(
        cli,
        "run_batch",
        lambda files, out_dir, transcriber, fmt, force: BatchStats(
            total=1, done=0, skipped=0, failed=1
        ),
    )

    exit_code = cli.main(["--out", str(tmp_path / "out"), "a.wav"])

    assert exit_code == 1
