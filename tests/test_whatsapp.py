"""Tests for hark.whatsapp: chat-export parsing and transcription flow."""

from __future__ import annotations

import json
import zipfile
from datetime import date
from pathlib import Path

import pytest

from hark.whatsapp import (
    Message,
    build_merged_chat,
    build_parser,
    default_out_dir,
    extract_attachment,
    find_attachment_member,
    find_chat_entry,
    is_audio_attachment,
    main,
    parse_chat,
    select_audio_messages,
)

U200E = "‎"


# --- find_chat_entry -------------------------------------------------------


def test_find_chat_entry_prefers_name_ending_in_chat_txt():
    names = ["WhatsApp Chat with Bob/_chat.txt", "WhatsApp Chat with Bob/00001.opus"]

    assert find_chat_entry(names) == "WhatsApp Chat with Bob/_chat.txt"


def test_find_chat_entry_falls_back_to_single_root_txt():
    names = ["notes.txt", "00001.opus"]

    assert find_chat_entry(names) == "notes.txt"


def test_find_chat_entry_raises_when_ambiguous():
    names = ["a.txt", "b.txt"]

    with pytest.raises(ValueError):
        find_chat_entry(names)


def test_find_chat_entry_raises_when_none_found():
    with pytest.raises(ValueError):
        find_chat_entry(["00001.opus"])


# --- find_attachment_member --------------------------------------------------


def test_find_attachment_member_exact_match():
    names = ["_chat.txt", "00001.opus"]

    assert find_attachment_member(names, "00001.opus") == "00001.opus"


def test_find_attachment_member_nested_path():
    names = ["export/_chat.txt", "export/00001.opus"]

    assert find_attachment_member(names, "00001.opus") == "export/00001.opus"


def test_find_attachment_member_missing_returns_none():
    names = ["_chat.txt"]

    assert find_attachment_member(names, "00001.opus") is None


# --- extract_attachment ------------------------------------------------------


def test_extract_attachment_anexado():
    assert extract_attachment("‎<anexado: 00001-AUDIO.opus>") == "00001-AUDIO.opus"


def test_extract_attachment_attached():
    assert extract_attachment("<attached: 00001-AUDIO.opus>") == "00001-AUDIO.opus"


def test_extract_attachment_none_when_absent():
    assert extract_attachment("fica no aguardo ai") is None


# --- is_audio_attachment ------------------------------------------------------


def test_is_audio_attachment_true_for_opus():
    assert is_audio_attachment("00001-AUDIO.opus") is True


def test_is_audio_attachment_false_for_image():
    assert is_audio_attachment("00004-IMG.jpg") is False


# --- parse_chat ---------------------------------------------------------------


def test_parse_chat_single_message():
    text = f"{U200E}[10/07/2026, 09:00:00] Alice: fica no aguardo ai"

    messages = parse_chat(text)

    assert messages == [
        Message(
            date=date(2026, 7, 10),
            time="09:00:00",
            sender="Alice",
            body="fica no aguardo ai",
            line_index=0,
        )
    ]


def test_parse_chat_strips_invisible_marks_around_attachment():
    text = f"{U200E}[13/07/2026, 21:04:39] Bob: {U200E}<anexado: 00001935-AUDIO.opus>"

    messages = parse_chat(text)

    # matching (and the captured groups) happen on the invisible-mark-stripped
    # line, so the parsed body is clean even though the raw line wasn't.
    assert len(messages) == 1
    assert messages[0].body == "<anexado: 00001935-AUDIO.opus>"
    assert extract_attachment(messages[0].body) == "00001935-AUDIO.opus"


def test_parse_chat_joins_continuation_lines():
    text = (
        "[11/07/2026, 10:00:00] Bob: primeira parte\n"
        "segunda parte (continuacao)"
    )

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].body == "primeira parte\nsegunda parte (continuacao)"


def test_parse_chat_leading_system_line_is_ignored():
    text = (
        "Messages and calls are end-to-end encrypted.\n"
        "[10/07/2026, 09:00:00] Alice: oi"
    )

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].line_index == 1


# --- parse_chat: Android format -------------------------------------------------


def test_parse_chat_android_single_message():
    text = "10/07/2026, 21:04 - Bob: fica no aguardo ai"

    messages = parse_chat(text)

    assert messages == [
        Message(
            date=date(2026, 7, 10),
            time="21:04",
            sender="Bob",
            body="fica no aguardo ai",
            line_index=0,
        )
    ]


def test_parse_chat_android_without_comma_after_date():
    text = "10/07/2026 21:04 - Bob: oi"

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].date == date(2026, 7, 10)
    assert messages[0].sender == "Bob"


def test_parse_chat_android_two_digit_year():
    text = "10/07/26, 21:04 - Bob: oi"

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].date == date(2026, 7, 10)


def test_parse_chat_android_12h_time_with_narrow_nbsp():
    narrow_nbsp = " "
    text = f"13/07/26, 9:05{narrow_nbsp}PM - Bob: hi"

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].date == date(2026, 7, 13)
    assert messages[0].time == f"9:05{narrow_nbsp}PM"


def test_parse_chat_android_day_first_when_first_component_exceeds_12():
    text = (
        "10/07/2026, 21:04 - Bob: oi\n"
        "13/07/2026, 21:05 - Bob: tudo certo"
    )

    messages = parse_chat(text)

    assert [m.date for m in messages] == [date(2026, 7, 10), date(2026, 7, 13)]


def test_parse_chat_android_month_first_when_second_component_exceeds_12():
    text = (
        "7/10/26, 9:04 PM - Bob: hi\n"
        "7/13/26, 9:05 PM - Bob: all good"
    )

    messages = parse_chat(text)

    assert [m.date for m in messages] == [date(2026, 7, 10), date(2026, 7, 13)]


def test_parse_chat_android_ambiguous_dates_default_day_first():
    text = "05/07/2026, 21:04 - Bob: oi"

    messages = parse_chat(text)

    assert messages[0].date == date(2026, 7, 5)


def test_parse_chat_android_joins_continuation_lines():
    text = (
        "11/07/2026, 10:00 - Bob: primeira parte\n"
        "segunda parte (continuacao)"
    )

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].body == "primeira parte\nsegunda parte (continuacao)"


def test_parse_chat_android_leading_system_line_is_ignored():
    text = (
        "10/07/2026, 21:00 - Messages and calls are end-to-end encrypted. "
        "Tap to learn more.\n"
        "10/07/2026, 21:04 - Bob: oi"
    )

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].sender == "Bob"
    assert messages[0].line_index == 1


def test_parse_chat_does_not_mix_formats():
    """Format is detected once per chat: after an android header, an
    iOS-style line in a message body is a continuation, not a header."""
    text = (
        "10/07/2026, 21:04 - Bob: olha esse formato:\n"
        "[10/07/2026, 09:00:00] Alice: nao sou uma mensagem"
    )

    messages = parse_chat(text)

    assert len(messages) == 1
    assert messages[0].sender == "Bob"
    assert messages[0].line_index == 0
    assert "nao sou uma mensagem" in messages[0].body


# --- extract_attachment: Android format ------------------------------------------


def test_extract_attachment_android_file_attached():
    assert (
        extract_attachment("PTT-20260710-WA0001.opus (file attached)")
        == "PTT-20260710-WA0001.opus"
    )


def test_extract_attachment_android_arquivo_anexado():
    assert (
        extract_attachment("PTT-20260710-WA0001.opus (arquivo anexado)")
        == "PTT-20260710-WA0001.opus"
    )


def test_extract_attachment_android_media_omitted_returns_none():
    assert extract_attachment("<Media omitted>") is None


# --- select_audio_messages -----------------------------------------------------


def _msg(day, filename=None, sender="Bob"):
    body = f"<anexado: {filename}>" if filename else "texto qualquer"
    return Message(date=date(2026, 7, day), time="09:00:00", sender=sender, body=body, line_index=day)


def test_select_audio_messages_filters_non_attachment():
    messages = [_msg(10, filename=None)]

    assert select_audio_messages(messages) == []


def test_select_audio_messages_filters_non_audio_attachment():
    messages = [_msg(10, filename="photo.jpg")]

    assert select_audio_messages(messages) == []


def test_select_audio_messages_inclusive_date_bounds():
    messages = [
        _msg(9, filename="a.opus"),
        _msg(10, filename="b.opus"),
        _msg(12, filename="c.opus"),
        _msg(13, filename="d.opus"),
    ]

    selected = select_audio_messages(
        messages, date_from=date(2026, 7, 10), date_to=date(2026, 7, 12)
    )

    assert [m.body for m in selected] == [
        "<anexado: b.opus>",
        "<anexado: c.opus>",
    ]


def test_select_audio_messages_no_bounds_selects_all_audio():
    messages = [_msg(9, filename="a.opus"), _msg(20, filename="b.opus")]

    selected = select_audio_messages(messages)

    assert len(selected) == 2


# --- default_out_dir -----------------------------------------------------------


def test_default_out_dir_from_zip_stem():
    assert default_out_dir(Path("chat_export.zip")) == Path("./chat_export-transcripts")


# --- build_merged_chat ----------------------------------------------------------


def test_build_merged_chat_inserts_transcript_after_attachment_line_only():
    raw_text = (
        "[10/07/2026, 09:00:00] Alice: oi\n"
        "[10/07/2026, 09:05:00] Bob: <anexado: a.opus>\n"
        "[20/07/2026, 09:00:00] Bob: <anexado: b.opus>\n"
    )
    messages = parse_chat(raw_text)
    audio_dir = Path("/tmp/out/audio")
    transcripts = {str(audio_dir / "a.opus"): "oi tudo bem"}

    merged = build_merged_chat(raw_text, messages, transcripts, audio_dir)

    lines = merged.splitlines()
    assert lines[0] == "[10/07/2026, 09:00:00] Alice: oi"
    assert lines[1] == "[10/07/2026, 09:05:00] Bob: <anexado: a.opus>"
    assert lines[2] == "    >> [transcript] oi tudo bem"
    assert lines[3] == "[20/07/2026, 09:00:00] Bob: <anexado: b.opus>"
    assert len(lines) == 4


def test_build_merged_chat_normalizes_attachment_path_component():
    """The marker filename can carry a path component (e.g. `media/foo.opus`)
    while transcripts are keyed by `audio_dir / <basename>` (extraction
    strips the path via `Path(filename).name`). Lookup must normalize the
    same way so the transcript still gets inlined."""
    raw_text = (
        "[10/07/2026, 09:05:00] Bob: <anexado: media/a.opus>\n"
    )
    messages = parse_chat(raw_text)
    audio_dir = Path("/tmp/out/audio")
    transcripts = {str(audio_dir / "a.opus"): "oi tudo bem"}

    merged = build_merged_chat(raw_text, messages, transcripts, audio_dir)

    lines = merged.splitlines()
    assert lines[0] == "[10/07/2026, 09:05:00] Bob: <anexado: media/a.opus>"
    assert lines[1] == "    >> [transcript] oi tudo bem"


def test_build_merged_chat_preserves_continuation_lines_untouched():
    raw_text = (
        "[11/07/2026, 10:00:00] Bob: primeira parte\n"
        "segunda parte (continuacao)\n"
    )
    messages = parse_chat(raw_text)

    merged = build_merged_chat(raw_text, messages, {}, Path("/tmp/out/audio"))

    assert merged == raw_text


# --- build_parser ----------------------------------------------------------------


def test_parser_defaults():
    parser = build_parser()
    args = parser.parse_args(["export.zip"])

    assert args.export_zip == "export.zip"
    assert args.out is None
    assert args.date_from is None
    assert args.date_to is None
    assert args.merge is False
    assert args.model == "small"
    assert args.lang == "pt"
    assert args.device == "cpu"
    assert args.force is False


def test_parser_accepts_all_flags():
    parser = build_parser()
    args = parser.parse_args(
        [
            "export.zip",
            "--out",
            "/tmp/out",
            "--from",
            "2026-07-01",
            "--to",
            "2026-07-31",
            "--merge",
            "--model",
            "medium",
            "--lang",
            "auto",
            "--device",
            "cuda",
            "--force",
        ]
    )

    assert args.out == "/tmp/out"
    assert args.date_from == "2026-07-01"
    assert args.date_to == "2026-07-31"
    assert args.merge is True
    assert args.model == "medium"
    assert args.lang == "auto"
    assert args.device == "cuda"
    assert args.force is True


# --- main(): end-to-end with a synthetic zip -------------------------------------


def _build_export_zip(zip_path: Path) -> None:
    chat_text = (
        "Messages and calls are end-to-end encrypted.\n"
        f"{U200E}[10/07/2026, 09:00:00] Alice: fica no aguardo ai\n"
        f"{U200E}[10/07/2026, 09:05:00] Bob: {U200E}<anexado: 00001-AUDIO-2026-07-10.opus>\n"
        "[11/07/2026, 10:00:00] Bob: primeira parte\n"
        "segunda parte (continuacao)\n"
        f"{U200E}[12/07/2026, 08:00:00] Alice: {U200E}<anexado: 00002-AUDIO-2026-07-12.opus>\n"
        f"{U200E}[20/07/2026, 08:00:00] Bob: {U200E}<anexado: 00003-AUDIO-2026-07-20.opus>\n"
        f"{U200E}[10/07/2026, 09:10:00] Bob: {U200E}<anexado: 00004-IMG-2026-07-10.jpg>\n"
    )
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.writestr("_chat.txt", chat_text)
        zf.writestr("00001-AUDIO-2026-07-10.opus", b"fake-audio-1")
        zf.writestr("00002-AUDIO-2026-07-12.opus", b"fake-audio-2")
        zf.writestr("00003-AUDIO-2026-07-20.opus", b"fake-audio-3")
        zf.writestr("00004-IMG-2026-07-10.jpg", b"fake-image")


class FakeTranscriber:
    """Stand-in for hark.core.Transcriber. No faster_whisper involved."""

    def __init__(self, **kwargs):
        self.kwargs = kwargs

    def transcribe(self, path: Path):
        from hark.core import Segment, TranscriptionResult

        text = f"transcript of {path.name}"
        return TranscriptionResult(
            source=path,
            text=text,
            segments=[Segment(0.0, 1.0, text)],
            language="pt",
            duration=1.0,
        )


def test_main_end_to_end_selects_extracts_transcribes_and_merges(monkeypatch, tmp_path):
    """Drives the real main() -> real zip parsing -> real run_batch chain.

    Only Transcriber is faked (no faster_whisper); collection, extraction,
    batch orchestration, writers, and merge all run for real against a
    temp directory.
    """
    import hark.whatsapp as whatsapp

    monkeypatch.setattr(whatsapp, "Transcriber", FakeTranscriber)

    zip_path = tmp_path / "chat_export.zip"
    _build_export_zip(zip_path)
    out_dir = tmp_path / "out"

    exit_code = main(
        [
            str(zip_path),
            "--out",
            str(out_dir),
            "--from",
            "2026-07-10",
            "--to",
            "2026-07-12",
            "--merge",
        ]
    )

    assert exit_code == 0

    audio_dir = out_dir / "audio"
    assert (audio_dir / "00001-AUDIO-2026-07-10.opus").read_bytes() == b"fake-audio-1"
    assert (audio_dir / "00002-AUDIO-2026-07-12.opus").read_bytes() == b"fake-audio-2"
    assert not (audio_dir / "00003-AUDIO-2026-07-20.opus").exists()
    assert not (audio_dir / "00004-IMG-2026-07-10.jpg").exists()

    assert (
        out_dir / "00001-AUDIO-2026-07-10.txt"
    ).read_text() == "transcript of 00001-AUDIO-2026-07-10.opus\n"
    assert (
        out_dir / "00002-AUDIO-2026-07-12.txt"
    ).read_text() == "transcript of 00002-AUDIO-2026-07-12.opus\n"

    manifest_lines = (out_dir / "manifest.jsonl").read_text().splitlines()
    assert len(manifest_lines) == 2
    sources = {json.loads(line)["source"] for line in manifest_lines}
    assert sources == {
        str(audio_dir / "00001-AUDIO-2026-07-10.opus"),
        str(audio_dir / "00002-AUDIO-2026-07-12.opus"),
    }

    merged = (out_dir / "_chat.transcribed.txt").read_text()
    merged_lines = merged.splitlines()
    idx_a = merged_lines.index(
        f"{U200E}[10/07/2026, 09:05:00] Bob: {U200E}<anexado: 00001-AUDIO-2026-07-10.opus>"
    )
    assert merged_lines[idx_a + 1] == (
        "    >> [transcript] transcript of 00001-AUDIO-2026-07-10.opus"
    )
    idx_b = merged_lines.index(
        f"{U200E}[12/07/2026, 08:00:00] Alice: "
        f"{U200E}<anexado: 00002-AUDIO-2026-07-12.opus>"
    )
    assert merged_lines[idx_b + 1] == (
        "    >> [transcript] transcript of 00002-AUDIO-2026-07-12.opus"
    )
    idx_c = merged_lines.index(
        f"{U200E}[20/07/2026, 08:00:00] Bob: {U200E}<anexado: 00003-AUDIO-2026-07-20.opus>"
    )
    # out-of-range attachment stays untouched: no transcript line follows it
    assert merged_lines[idx_c + 1] == (
        f"{U200E}[10/07/2026, 09:10:00] Bob: "
        f"{U200E}<anexado: 00004-IMG-2026-07-10.jpg>"
    )
    # continuation line preserved verbatim
    assert "segunda parte (continuacao)" in merged_lines


def _build_export_zip_android(zip_path: Path) -> None:
    chat_text = (
        "10/07/2026, 08:55 - Messages and calls are end-to-end encrypted. "
        "Tap to learn more.\n"
        "10/07/2026, 09:00 - Alice: fica no aguardo ai\n"
        "10/07/2026, 09:05 - Bob: PTT-20260710-WA0001.opus (file attached)\n"
        "11/07/2026, 10:00 - Bob: primeira parte\n"
        "segunda parte (continuacao)\n"
        "12/07/2026, 08:00 - Alice: PTT-20260712-WA0002.opus (arquivo anexado)\n"
        "20/07/2026, 08:00 - Bob: PTT-20260720-WA0003.opus (file attached)\n"
        "10/07/2026, 09:10 - Bob: IMG-20260710-WA0004.jpg (file attached)\n"
        "10/07/2026, 09:11 - Bob: <Media omitted>\n"
    )
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.writestr("WhatsApp Chat with Bob.txt", chat_text)
        zf.writestr("PTT-20260710-WA0001.opus", b"fake-audio-1")
        zf.writestr("PTT-20260712-WA0002.opus", b"fake-audio-2")
        zf.writestr("PTT-20260720-WA0003.opus", b"fake-audio-3")
        zf.writestr("IMG-20260710-WA0004.jpg", b"fake-image")


def test_main_end_to_end_android_export(monkeypatch, tmp_path):
    """Android-format zip through the real main(): chat located via the
    single-root-txt fallback, android headers parsed, `(file attached)`
    markers extracted, date filter applied, merge inlined."""
    import hark.whatsapp as whatsapp

    monkeypatch.setattr(whatsapp, "Transcriber", FakeTranscriber)

    zip_path = tmp_path / "chat_export.zip"
    _build_export_zip_android(zip_path)
    out_dir = tmp_path / "out"

    exit_code = main(
        [
            str(zip_path),
            "--out",
            str(out_dir),
            "--from",
            "2026-07-10",
            "--to",
            "2026-07-12",
            "--merge",
        ]
    )

    assert exit_code == 0

    audio_dir = out_dir / "audio"
    assert (audio_dir / "PTT-20260710-WA0001.opus").read_bytes() == b"fake-audio-1"
    assert (audio_dir / "PTT-20260712-WA0002.opus").read_bytes() == b"fake-audio-2"
    assert not (audio_dir / "PTT-20260720-WA0003.opus").exists()
    assert not (audio_dir / "IMG-20260710-WA0004.jpg").exists()

    manifest_lines = (out_dir / "manifest.jsonl").read_text().splitlines()
    assert len(manifest_lines) == 2

    merged_lines = (out_dir / "_chat.transcribed.txt").read_text().splitlines()
    idx_a = merged_lines.index(
        "10/07/2026, 09:05 - Bob: PTT-20260710-WA0001.opus (file attached)"
    )
    assert merged_lines[idx_a + 1] == (
        "    >> [transcript] transcript of PTT-20260710-WA0001.opus"
    )
    idx_c = merged_lines.index(
        "20/07/2026, 08:00 - Bob: PTT-20260720-WA0003.opus (file attached)"
    )
    # out-of-range attachment stays untouched: no transcript line follows it
    assert merged_lines[idx_c + 1] == (
        "10/07/2026, 09:10 - Bob: IMG-20260710-WA0004.jpg (file attached)"
    )
    assert "segunda parte (continuacao)" in merged_lines


def test_main_returns_1_when_a_transcription_fails(monkeypatch, tmp_path):
    class FailingTranscriber:
        def __init__(self, **kwargs):
            pass

        def transcribe(self, path):
            raise RuntimeError("boom")

    import hark.whatsapp as whatsapp

    monkeypatch.setattr(whatsapp, "Transcriber", FailingTranscriber)

    zip_path = tmp_path / "chat_export.zip"
    _build_export_zip(zip_path)
    out_dir = tmp_path / "out"

    exit_code = main([str(zip_path), "--out", str(out_dir)])

    assert exit_code == 1


def test_main_missing_zip_returns_2(tmp_path, capsys):
    missing = tmp_path / "nope.zip"

    exit_code = main([str(missing), "--out", str(tmp_path / "out")])

    assert exit_code == 2
    assert "nope.zip" in capsys.readouterr().err


def test_main_corrupt_zip_returns_2(tmp_path, capsys):
    bad_zip = tmp_path / "chat_export.zip"
    bad_zip.write_bytes(b"not actually a zip file")

    exit_code = main([str(bad_zip), "--out", str(tmp_path / "out")])

    err = capsys.readouterr().err
    assert exit_code == 2
    assert err.startswith("error: ")
    assert "Traceback" not in err


def test_main_no_chat_entry_leaves_no_output_dir(tmp_path, capsys):
    """A zip without a locatable chat file (*_chat.txt or single root .txt)
    exits 2 before any output directory is created -- no empty <out>/audio/
    left behind."""
    zip_path = tmp_path / "chat_export.zip"
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.writestr("a.txt", "irrelevant")
        zf.writestr("b.txt", "irrelevant")
    out_dir = tmp_path / "out"

    exit_code = main([str(zip_path), "--out", str(out_dir)])

    assert exit_code == 2
    assert "could not locate a chat log" in capsys.readouterr().err
    assert not out_dir.exists()


def test_main_malformed_date_returns_2(tmp_path, capsys):
    zip_path = tmp_path / "chat_export.zip"
    _build_export_zip(zip_path)

    exit_code = main(
        [str(zip_path), "--out", str(tmp_path / "out"), "--from", "not-a-date"]
    )

    assert exit_code == 2
    assert "not-a-date" in capsys.readouterr().err


def test_main_merge_does_not_leak_stale_manifest_entries_across_runs(
    monkeypatch, tmp_path
):
    """A second run with a narrower date range, reusing the same --out,
    must not inline a transcript left over in manifest.jsonl from a wider
    first run onto a line that's outside *this* run's filter.
    """
    import hark.whatsapp as whatsapp

    monkeypatch.setattr(whatsapp, "Transcriber", FakeTranscriber)

    zip_path = tmp_path / "chat_export.zip"
    _build_export_zip(zip_path)
    out_dir = tmp_path / "out"

    # Run 1: wide range, no merge -- populates manifest.jsonl with both
    # 00001 (07-10) and 00003 (07-20).
    exit_code = main([str(zip_path), "--out", str(out_dir)])
    assert exit_code == 0
    manifest_lines = (out_dir / "manifest.jsonl").read_text().splitlines()
    assert len(manifest_lines) == 3  # 00001, 00002, 00003

    # Run 2: narrow range covering only 00001, with --merge.
    exit_code = main(
        [
            str(zip_path),
            "--out",
            str(out_dir),
            "--from",
            "2026-07-10",
            "--to",
            "2026-07-10",
            "--merge",
        ]
    )
    assert exit_code == 0

    merged_lines = (out_dir / "_chat.transcribed.txt").read_text().splitlines()
    idx_a = merged_lines.index(
        f"{U200E}[10/07/2026, 09:05:00] Bob: {U200E}<anexado: 00001-AUDIO-2026-07-10.opus>"
    )
    assert merged_lines[idx_a + 1] == (
        "    >> [transcript] transcript of 00001-AUDIO-2026-07-10.opus"
    )
    idx_c = merged_lines.index(
        f"{U200E}[20/07/2026, 08:00:00] Bob: {U200E}<anexado: 00003-AUDIO-2026-07-20.opus>"
    )
    # 00003 was transcribed in run 1 (it's in the manifest) but is outside
    # run 2's filter -- it must stay untouched, not get a stale transcript
    # line inlined from the old manifest entry.
    assert merged_lines[idx_c + 1] != (
        "    >> [transcript] transcript of 00003-AUDIO-2026-07-20.opus"
    )
