#!/usr/bin/env python3
"""List Sweetener entries that are not in cases.jsonl.

An entry is one program, not a project. `hygiene.sts` and `malformed.sts`
beside a fixture are their own entries. A file that only defines macros is
not an entry.

This script does not edit the manifest and does not write case directories.

Usage:
  discover-entries.py            print entries the manifest does not list
  discover-entries.py --all      print every discovered entry
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CRATE = Path(__file__).resolve().parents[1]
VENDOR = (ROOT / "vendor" / "sweetener").resolve()
MANIFEST = CRATE / "cases.jsonl"
EXTS = {".sts", ".stsx"}
SYNTAX_IMPORT = re.compile(r"""from\s+['"](\.[^'"]+)['"]\s+for\s+syntax""")
NAMED_SOURCE = re.compile(r"""['"]([^'"]+\.(?:sts|stsx))['"]""")
SKIP_PARTS = {"node_modules", ".worktrees", "dist", "target"}


def skip(path: Path) -> bool:
    return any(part in SKIP_PARTS for part in path.parts)


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def is_macro_only(path: Path) -> bool:
    text = read(path)
    if SYNTAX_IMPORT.search(text):
        return False
    return "export syntax" in text or "export operator" in text


def vendor_rel(path: Path) -> str:
    parts = path.resolve().parts
    for label in ("examples", "fixtures", "samples"):
        if label in parts:
            return Path(*parts[parts.index(label) :]).as_posix()
    return path.name


def fixture_entries(fixtures: Path) -> list[Path]:
    found: list[Path] = []
    if not fixtures.exists():
        return found
    for case in fixtures.rglob("case.json"):
        if skip(case):
            continue
        data = json.loads(read(case))
        entry_name = data.get("entry")
        if not entry_name:
            continue
        entry = case.parent / entry_name
        if entry.is_file():
            found.append(entry)
        for sibling in case.parent.iterdir():
            if sibling.is_file() and sibling.suffix in EXTS and sibling.stem in {"hygiene", "malformed"}:
                found.append(sibling)
    return found


def program_entries(root: Path) -> list[Path]:
    found: list[Path] = []
    if not root.exists():
        return found
    for path in root.rglob("*"):
        if not path.is_file() or path.suffix not in EXTS or skip(path):
            continue
        if "fixtures" in path.parts:
            continue
        text = read(path)
        if SYNTAX_IMPORT.search(text) and not is_macro_only(path):
            found.append(path)
    for script in list(root.rglob("run.mjs")) + list(root.rglob("package.json")):
        if skip(script) or "fixtures" in script.parts:
            continue
        text = read(script)
        base = script.parent
        for name in NAMED_SOURCE.findall(text):
            candidate = (base / name).resolve()
            if candidate.is_file() and candidate.suffix in EXTS and not is_macro_only(candidate):
                found.append(candidate)
    return found


def discovered() -> list[Path]:
    found: list[Path] = []
    found.extend(fixture_entries(VENDOR / "fixtures"))
    found.extend(program_entries(VENDOR / "examples"))
    found.extend(program_entries(VENDOR / "samples"))
    unique: dict[Path, None] = {}
    for path in found:
        if is_macro_only(path):
            continue
        unique.setdefault(path.resolve(), None)
    return sorted(unique, key=lambda path: vendor_rel(path))


def listed() -> list[dict]:
    if not MANIFEST.is_file():
        return []
    return [json.loads(line) for line in MANIFEST.read_text().splitlines() if line.strip()]


def covered(path: Path, rows: list[dict]) -> bool:
    rel = vendor_rel(path)
    # The recovered language tour repeats the playground fixtures. The
    # playground row is the one the manifest keeps.
    recovered = "examples/language-tour/recovered/"
    if rel.startswith(recovered):
        feature = rel[len(recovered) :].split("/")[0]
        playground = f"fixtures/acceptance/playground/{feature}/"
        if any(playground in row.get("entry", "") for row in rows):
            return True
    for row in rows:
        entry = row.get("entry", "")
        if entry.endswith("/" + rel) or entry.endswith(rel):
            return True
    return False


def flat_id(path: Path) -> str:
    rel = vendor_rel(path)
    stem = Path(rel).with_suffix("")
    return stem.as_posix().replace("/", "-")


def main() -> int:
    show_all = "--all" in sys.argv
    rows = listed()
    entries = discovered()
    missing = [path for path in entries if not covered(path, rows)]
    print(f"discovered={len(entries)} listed={len(rows)} missing={len(missing)}")
    shown = entries if show_all else missing
    for path in shown:
        if show_all and covered(path, rows):
            continue
        print(f"missing {vendor_rel(path)}")
        print(f"  id {flat_id(path)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
