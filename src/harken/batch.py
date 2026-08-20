"""Audio file collection and batch transcription."""

from __future__ import annotations

import glob
import sys
from dataclasses import dataclass
from pathlib import Path

from harken.core import Transcriber
from harken.writers import append_manifest, write_json, write_srt, write_txt

AUDIO_EXTENSIONS = {
    ".opus",
    ".ogg",
    ".oga",
    ".mp3",
    ".m4a",
    ".wav",
    ".flac",
    ".mp4",
    ".webm",
    ".aac",
    ".wma",
    ".amr",
}

_WRITERS = {"txt": write_txt, "json": write_json, "srt": write_srt}

_GLOB_CHARS = set("*?[")


def _is_audio(path: Path) -> bool:
    return path.suffix.lower() in AUDIO_EXTENSIONS


def collect_audio_files(inputs: list[str]) -> list[Path]:
    """Resolve CLI input arguments to a sorted, de-duplicated list of files.

    Each input may be:
    - an explicit file path: included verbatim, regardless of extension;
    - a directory: recursed, filtered to AUDIO_EXTENSIONS;
    - a glob pattern: expanded, filtered to AUDIO_EXTENSIONS (files matched
      by a glob are not "explicit" the way a bare path is).

    A missing explicit path or directory is a hard error (exit 2).
    """
    collected: set[Path] = set()

    for raw in inputs:
        if any(ch in raw for ch in _GLOB_CHARS):
            for match in glob.glob(raw, recursive=True):
                match_path = Path(match)
                if match_path.is_dir():
                    collected.update(
                        p for p in match_path.rglob("*") if p.is_file() and _is_audio(p)
                    )
                elif _is_audio(match_path):
                    collected.add(match_path)
            continue

        path = Path(raw)
        if not path.exists():
            print(f"error: path not found: {raw}", file=sys.stderr)
            raise SystemExit(2)

        if path.is_dir():
            collected.update(p for p in path.rglob("*") if p.is_file() and _is_audio(p))
        else:
            collected.add(path)

    return sorted(collected)


@dataclass
class BatchStats:
    total: int
    done: int = 0
    skipped: int = 0
    failed: int = 0


def _next_output_path(source: Path, out_dir: Path, fmt: str, seen: dict[str, int]) -> Path:
    """Assign a unique-in-run output path for `source`'s stem.

    Numbering is driven purely by how many times this stem has already been
    assigned in this run, never by filesystem existence -- that is the
    skip/force decision's job, made independently below.
    """
    stem = source.stem
    count = seen.get(stem, 0) + 1
    seen[stem] = count
    name = stem if count == 1 else f"{stem}-{count}"
    return out_dir / f"{name}.{fmt}"


def run_batch(
    files: list[Path],
    out_dir: Path,
    transcriber: Transcriber,
    fmt: str,
    force: bool,
) -> BatchStats:
    """Transcribe every file with one shared Transcriber, writing outputs.

    Returns BatchStats; the caller decides the process exit code from it
    (0 if stats.failed == 0, else 1).
    """
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest = out_dir / "manifest.jsonl"
    write = _WRITERS[fmt]
    seen: dict[str, int] = {}
    stats = BatchStats(total=len(files))

    for i, source in enumerate(files, start=1):
        output_path = _next_output_path(source, out_dir, fmt, seen)
        name = source.name

        if output_path.exists() and not force:
            stats.skipped += 1
            print(f"[{i}/{stats.total}] {name} ... skipped (exists)", file=sys.stderr)
            continue

        try:
            result = transcriber.transcribe(source)
        except Exception as exc:
            stats.failed += 1
            print(f"[{i}/{stats.total}] {name} ... FAILED: {exc}", file=sys.stderr)
            continue

        write(result, output_path)
        append_manifest(manifest, result, output_path)
        stats.done += 1
        print(
            f"[{i}/{stats.total}] {name} ... done ({result.duration:.1f}s audio)",
            file=sys.stderr,
        )

    print(
        f"batch complete: {stats.done} done, {stats.skipped} skipped, "
        f"{stats.failed} failed (of {stats.total})",
        file=sys.stderr,
    )
    return stats
