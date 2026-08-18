"""WhatsApp chat-export transcription mode.

Parses a WhatsApp chat-export zip, selects audio-attachment messages within
an optional date range, extracts and transcribes them (reusing the batch
pipeline), and optionally writes a merged chat transcript with transcript
lines inlined after each attachment line.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import zipfile
from dataclasses import dataclass
from datetime import date, datetime
from pathlib import Path

from hark.batch import AUDIO_EXTENSIONS, run_batch
from hark.core import Transcriber

_MESSAGE_RE = re.compile(
    r"^‎?\[(\d{2}/\d{2}/\d{4}), (\d{2}:\d{2}:\d{2})\] ([^:]+): (.*)$"
)
_ATTACHMENT_RE = re.compile(r"<(?:anexado|attached): ([^>]+)>")
_INVISIBLE_CHARS = "‎‏"


@dataclass
class Message:
    date: date
    time: str
    sender: str
    body: str
    line_index: int


def _strip_invisible(line: str) -> str:
    return line.translate({ord(c): None for c in _INVISIBLE_CHARS})


def parse_chat(text: str) -> list[Message]:
    """Parse raw WhatsApp chat-export text into a list of Message.

    Lines that don't match the message-header pattern are continuations of
    the previous message and get folded into its body. Any lines before the
    first recognized message header (e.g. the encryption notice) are
    dropped.
    """
    messages: list[Message] = []

    for i, raw_line in enumerate(text.splitlines()):
        stripped = _strip_invisible(raw_line)
        match = _MESSAGE_RE.match(stripped)
        if match:
            date_str, time_str, sender, body = match.groups()
            msg_date = datetime.strptime(date_str, "%d/%m/%Y").date()
            messages.append(
                Message(
                    date=msg_date,
                    time=time_str,
                    sender=sender,
                    body=body,
                    line_index=i,
                )
            )
        elif messages:
            messages[-1].body += "\n" + raw_line

    return messages


def extract_attachment(body: str) -> str | None:
    """Return the attachment filename in `body`, if any (anexado/attached)."""
    match = _ATTACHMENT_RE.search(body)
    return match.group(1) if match else None


def is_audio_attachment(filename: str) -> bool:
    return Path(filename).suffix.lower() in AUDIO_EXTENSIONS


def select_audio_messages(
    messages: list[Message],
    date_from: date | None = None,
    date_to: date | None = None,
) -> list[Message]:
    """Return messages carrying an audio attachment, within [date_from, date_to]."""
    selected = []
    for message in messages:
        filename = extract_attachment(message.body)
        if filename is None or not is_audio_attachment(filename):
            continue
        if date_from is not None and message.date < date_from:
            continue
        if date_to is not None and message.date > date_to:
            continue
        selected.append(message)
    return selected


def find_chat_entry(names: list[str]) -> str:
    """Locate the chat-log member inside a WhatsApp export zip's namelist."""
    chat_names = [n for n in names if n.endswith("_chat.txt")]
    if chat_names:
        return chat_names[0]

    root_txts = [n for n in names if n.lower().endswith(".txt") and "/" not in n]
    if len(root_txts) == 1:
        return root_txts[0]

    raise ValueError("could not locate a chat log (*_chat.txt or a single root .txt)")


def find_attachment_member(names: list[str], filename: str) -> str | None:
    """Find the zip member matching an attachment filename referenced in the chat."""
    for name in names:
        if name == filename or name.endswith("/" + filename):
            return name
    return None


def build_merged_chat(
    raw_text: str,
    messages: list[Message],
    transcripts: dict[str, str],
    audio_dir: Path,
) -> str:
    """Rebuild the chat text, inserting a transcript line after each
    attachment line whose extracted audio has an entry in `transcripts`
    (keyed by the extracted file's path). Everything else is untouched.
    """
    inserts: dict[int, str] = {}
    for message in messages:
        filename = extract_attachment(message.body)
        if filename is None:
            continue
        text = transcripts.get(str(audio_dir / Path(filename).name))
        if text is not None:
            inserts[message.line_index] = text

    lines = raw_text.splitlines()
    out_lines: list[str] = []
    for i, line in enumerate(lines):
        out_lines.append(line)
        if i in inserts:
            out_lines.append(f"    >> [transcript] {inserts[i]}")

    return "\n".join(out_lines) + "\n"


def default_out_dir(export_zip: Path) -> Path:
    return Path(f"./{export_zip.stem}-transcripts")


def _parse_date_arg(value: str) -> date:
    return datetime.strptime(value, "%Y-%m-%d").date()


def _load_manifest_texts(manifest: Path) -> dict[str, str]:
    if not manifest.exists():
        return {}
    texts: dict[str, str] = {}
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        entry = json.loads(line)
        texts[entry["source"]] = entry["text"]
    return texts


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="hark whatsapp",
        description="Transcribe audio attachments from a WhatsApp chat export",
    )
    parser.add_argument("export_zip", help="Path to the WhatsApp chat export .zip")
    parser.add_argument(
        "--out", default=None, help="Output directory (default: ./<zip-stem>-transcripts)"
    )
    parser.add_argument(
        "--from",
        dest="date_from",
        default=None,
        help="Only include messages on/after this date (YYYY-MM-DD)",
    )
    parser.add_argument(
        "--to",
        dest="date_to",
        default=None,
        help="Only include messages on/before this date (YYYY-MM-DD)",
    )
    parser.add_argument(
        "--merge",
        action="store_true",
        help="Write out_dir/_chat.transcribed.txt with transcripts inlined",
    )
    parser.add_argument("--model", default="small", help="Whisper model size (default: small)")
    parser.add_argument(
        "--lang",
        default="pt",
        help="Language code, or 'auto' to auto-detect (default: pt)",
    )
    parser.add_argument("--device", default="cpu", help="Device to run on (default: cpu)")
    parser.add_argument(
        "--force", action="store_true", help="Re-transcribe even if output already exists"
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    parser = build_parser()
    args = parser.parse_args(argv)

    export_zip = Path(args.export_zip)
    if not export_zip.exists():
        print(f"error: file not found: {export_zip}", file=sys.stderr)
        return 2

    try:
        date_from = _parse_date_arg(args.date_from) if args.date_from else None
        date_to = _parse_date_arg(args.date_to) if args.date_to else None
    except ValueError:
        print(
            f"error: invalid date (expected YYYY-MM-DD): {args.date_from!r} / {args.date_to!r}",
            file=sys.stderr,
        )
        return 2

    out_dir = Path(args.out) if args.out else default_out_dir(export_zip)
    audio_dir = out_dir / "audio"

    try:
        zf = zipfile.ZipFile(export_zip)
    except zipfile.BadZipFile as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    with zf:
        names = zf.namelist()
        try:
            chat_name = find_chat_entry(names)
        except ValueError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 2

        # No output directory is created until the chat entry is located --
        # a zip that fails to locate one exits 2 without leaving an empty
        # <out>/audio/ behind.
        audio_dir.mkdir(parents=True, exist_ok=True)

        raw_text = zf.read(chat_name).decode("utf-8-sig")
        messages = parse_chat(raw_text)
        selected = select_audio_messages(messages, date_from, date_to)

        extracted: list[Path] = []
        for message in selected:
            filename = extract_attachment(message.body)
            member = find_attachment_member(names, filename)
            if member is None:
                print(f"warning: attachment not found in zip: {filename}", file=sys.stderr)
                continue
            dest = audio_dir / Path(filename).name
            dest.write_bytes(zf.read(member))
            extracted.append(dest)

    language = None if args.lang == "auto" else args.lang
    transcriber = Transcriber(model_size=args.model, device=args.device, language=language)
    stats = run_batch(extracted, out_dir, transcriber, "txt", args.force)

    if args.merge:
        # Gate by *this run's* selection, not the full accumulated manifest --
        # out_dir/manifest.jsonl persists across runs, so on a reused --out
        # with a narrower date range it could otherwise carry stale entries
        # from a previous, wider-range run and inline a transcript onto a
        # line outside the current filter.
        selected_paths = {
            str(audio_dir / Path(extract_attachment(m.body)).name) for m in selected
        }
        transcripts = {
            path: text
            for path, text in _load_manifest_texts(out_dir / "manifest.jsonl").items()
            if path in selected_paths
        }
        merged = build_merged_chat(raw_text, messages, transcripts, audio_dir)
        (out_dir / "_chat.transcribed.txt").write_text(merged, encoding="utf-8")

    return 1 if stats.failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
