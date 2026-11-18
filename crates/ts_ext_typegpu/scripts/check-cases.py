#!/usr/bin/env python3
"""Typecheck each case in cases.jsonl against the simulated TypeGPU names.

The package is not installed. `types/sim.d.ts` declares the names the
snippets use, and tsc (the cargo `tsgo` binary) adds the operator prelude itself. A clean case
gets an empty `ours.check.txt`. This does not discover cases.

Usage:
  check-cases.py            print how many cases have diagnostics
  check-cases.py --apply    write ours.check.txt for every listed case
  check-cases.py <id>       one case, or every id with that prefix
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
ROOT = Path(__file__).resolve().parents[3]
CASES = CRATE / "cases"
MANIFEST = CRATE / "cases.jsonl"
STUB = CRATE / "types" / "sim.d.ts"

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

# Module-resolution gaps of the stub, not failures of the rewrite.
STUB_NOISE = ("TS2307", "TS2305", "TS2614", "TS2595", "TS2749", "TS2694")


def titles() -> None:
    for line in MANIFEST.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        case = CASES / row["id"]
        if not case.is_dir():
            continue
        (case / "test.title").write_text(row.get("testTitle", "") + "\n", encoding="utf-8")


# Free names a snippet mentions without binding them. Declared here, in
# that file only, so a different case that binds `fn` itself is not a
# redeclaration.
FREE_NAMES = ("shell", "fn", "cst")


def stage_input(source: str) -> str:
    lines = []
    for name in FREE_NAMES:
        if not re.search(rf"\b{name}\b", source):
            continue
        if re.search(rf"\b(?:const|let|var|function|class)\s+{name}\b", source):
            continue
        if re.search(rf"\bimport\b[^;]*\b{name}\b", source):
            continue
        lines.append(f"declare const {name}: any;")
    if not lines:
        return source
    return "\n".join(lines) + "\n" + source


def check_one(case: Path) -> str:
    stage = Path(tempfile.mkdtemp(prefix="typegpu-check-"))
    try:
        (stage / "input.ts").write_text(stage_input((case / "input.ts").read_text()), encoding="utf-8")
        shutil.copy(STUB, stage / "sim.d.ts")
        (stage / "tsconfig.json").write_text(
            json.dumps(
                {
                    "compilerOptions": {
                        "strict": True,
                        "noEmit": True,
                        "target": "esnext",
                        "module": "esnext",
                        "moduleResolution": "bundler",
                        "lib": ["esnext", "dom"],
                        "skipLibCheck": True,
                        "noImplicitAny": False,
                    },
                    "files": ["input.ts", "sim.d.ts"],
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
        if completed.returncode != 0 and not report.strip():
            report = f"tsc exit {completed.returncode}\n"
        kept = [
            line
            for line in report.splitlines()
            if line.strip() and not any(code in line for code in STUB_NOISE)
        ]
        return "\n".join(kept) + ("\n" if kept else "")
    finally:
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
    titles()
    rows = [json.loads(line) for line in MANIFEST.read_text().splitlines() if line.strip()]
    dirty = 0
    seen = 0
    for row in rows:
        if prefix and not row["id"].startswith(prefix):
            continue
        case = CASES / row["id"]
        if not (case / "input.ts").is_file():
            print(f"skip {row['id']}: no input.ts")
            continue
        seen += 1
        report = check_one(case)
        if report.strip():
            dirty += 1
            print(f"diagnostics {row['id']}")
            print(report.rstrip())
        if apply:
            (case / "ours.check.txt").write_text(report, encoding="utf-8")
    print(f"cases={seen} with_diagnostics={dirty}")
    if not apply:
        print("dry-run; pass --apply to write ours.check.txt")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
