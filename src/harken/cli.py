"""Command-line entry point for harken: batch mode and whatsapp delegation."""

from __future__ import annotations

import argparse
import sys
from importlib.metadata import version
from pathlib import Path

from harken.batch import collect_audio_files, run_batch
from harken.core import Transcriber


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="harken",
        description="Local audio transcription CLI (faster-whisper, CPU)",
    )
    parser.add_argument(
        "--version",
        action="version",
        version=f"%(prog)s {version('harken')}",
    )
    parser.add_argument(
        "inputs", nargs="+", help="Audio files, directories, or globs to transcribe"
    )
    parser.add_argument(
        "--out", default="./transcripts", help="Output directory (default: ./transcripts)"
    )
    parser.add_argument("--model", default="small", help="Whisper model size (default: small)")
    parser.add_argument(
        "--lang",
        default="pt",
        help="Language code, or 'auto' to auto-detect (default: pt)",
    )
    parser.add_argument(
        "--format",
        choices=["txt", "json", "srt"],
        default="txt",
        help="Output format (default: txt)",
    )
    parser.add_argument("--device", default="cpu", help="Device to run on (default: cpu)")
    parser.add_argument(
        "--force", action="store_true", help="Re-transcribe even if output already exists"
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv

    if argv and argv[0] == "whatsapp":
        from harken.whatsapp import main as whatsapp_main

        return whatsapp_main(argv[1:])

    parser = build_parser()
    args = parser.parse_args(argv)

    language = None if args.lang == "auto" else args.lang
    transcriber = Transcriber(
        model_size=args.model, device=args.device, language=language
    )

    files = collect_audio_files(args.inputs)
    out_dir = Path(args.out)
    stats = run_batch(files, out_dir, transcriber, args.format, args.force)

    return 1 if stats.failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
