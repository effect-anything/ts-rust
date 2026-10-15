#!/usr/bin/env python3
"""Typecheck each expanded case with the debug tsc (the cargo `tsgo` binary).

The input is Sweetener. tsc sees `ours.ts` or `ours.tsx`, the snapshot
`capture_ours` wrote. A clean case gets an empty `ours.check.txt`. This does
not discover cases and does not expand them again.

Usage:
  check-cases.py            print how many cases have diagnostics
  check-cases.py --apply    write ours.check.txt for every listed case
  check-cases.py <id>       one case, or every id with that prefix
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
ROOT = Path(__file__).resolve().parents[3]
CASES = CRATE / "cases"
MANIFEST = CRATE / "cases.jsonl"

def find_tsc() -> Path:
    """The debug tsc (the cargo `tsgo` binary).

    `TSC_BIN` when set, else `<target>/debug/tsgo`, where the target directory
    is `CARGO_TARGET_DIR`, else `cargo metadata`'s `target_directory`, else
    `<root>/target`.
    """
    override = os.environ.get("TSC_BIN")
    if override:
        return Path(override)
    target = os.environ.get("CARGO_TARGET_DIR")
    if target:
        base = ROOT / target
    else:
        base = ROOT / "target"
        try:
            meta = subprocess.run(
                ["cargo", "metadata", "--format-version", "1", "--no-deps"],
                cwd=ROOT,
                text=True,
                capture_output=True,
                check=True,
            )
            base = Path(json.loads(meta.stdout)["target_directory"])
        except (OSError, subprocess.CalledProcessError, KeyError, ValueError):
            pass
    return base / "debug" / ("tsgo.exe" if os.name == "nt" else "tsgo")


TSC = find_tsc()


def ours_file(case: Path) -> Path | None:
    for name in ("ours.tsx", "ours.ts"):
        path = case / name
        if path.is_file():
            return path
    return None


def check_one(case: Path, source: Path) -> str:
    stage = Path(tempfile.mkdtemp(prefix="sweetener-check-"))
    try:
        staged = stage / source.name
        staged.write_text(source.read_text(encoding="utf-8"), encoding="utf-8")
        (stage / "tsconfig.json").write_text(
            json.dumps(
                {
                    "compilerOptions": {
                        "strict": True,
                        "noEmit": True,
                        "target": "esnext",
                        "module": "esnext",
                        "moduleResolution": "bundler",
                        "jsx": "react-jsx",
                        "skipLibCheck": True,
                    },
                    "files": [source.name],
                }
            ),
            encoding="utf-8",
        )
        completed = subprocess.run(
            [str(TSC), "--noEmit", "--pretty", "false", "-p", "."],
            cwd=stage,
            text=True,
            capture_output=True,
            check=False,
        )
        report = completed.stdout + completed.stderr
        if completed.returncode not in (0, 2) and not report.strip():
            report = f"tsc exit {completed.returncode}\n"
        kept = [line for line in report.splitlines() if line.strip()]
        return "\n".join(kept) + ("\n" if kept else "")
    finally:
        import shutil

        shutil.rmtree(stage, ignore_errors=True)


def main() -> int:
    if not TSC.is_file():
        raise SystemExit(
            f"tsc (the cargo `tsgo` binary) missing at {TSC}: "
            "build it with `cargo build -p ts_goport --bin tsgo`, or set TSC_BIN"
        )
    args = [arg for arg in sys.argv[1:] if arg != "--apply"]
    apply = "--apply" in sys.argv
    prefix = args[0] if args else ""
    rows = [json.loads(line) for line in MANIFEST.read_text().splitlines() if line.strip()]
    dirty = 0
    seen = 0
    for row in rows:
        case_id = row["id"]
        if prefix and not case_id.startswith(prefix):
            continue
        case = CASES / case_id
        source = ours_file(case)
        if source is None:
            print(f"skip {case_id}: no ours.ts")
            continue
        seen += 1
        report = check_one(case, source)
        if report.strip():
            dirty += 1
            print(f"diagnostics {case_id}")
            print(report.rstrip())
        if apply:
            (case / "ours.check.txt").write_text(report, encoding="utf-8")
    print(f"cases={seen} with_diagnostics={dirty}")
    if not apply:
        print("dry-run; pass --apply to write ours.check.txt")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
