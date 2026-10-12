#!/usr/bin/env python3
"""Materialise the cases named in cases.jsonl. Nothing else.

Discovery does not write the manifest. Add a line by hand, then run this.
One row is one program.

  1. Copy the upstream source, and the syntax files it imports.
  2. Run the sweetener CLI (`sweetener expand`) on that file.
  3. Write its stdout to `theirs.ts` or `theirs.tsx`. `capture_ours`
     compares our expansion with that snapshot.

A case that is already on disk is left alone unless `--refresh`.
`--refresh` uses the vendor checkout. That tree is not the pin in
pins.json unless the checkout is. A malformed case has no expansion
snapshot; only its input is copied.

Usage:
  sync-cases.py                 print what would be written
  sync-cases.py --apply         write cases that have no input yet
  sync-cases.py --apply --refresh
                                pull every listed case again from vendor
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
ROOT = Path(__file__).resolve().parents[3]
CASES = CRATE / "cases"
MANIFEST = CRATE / "cases.jsonl"
SWEETENER = ROOT / "vendor" / "sweetener" / "packages" / "cli" / "bin" / "sweetener.mjs"


def load_discover():
    path = Path(__file__).resolve().parent / "discover-entries.py"
    spec = importlib.util.spec_from_file_location("discover_entries", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DISCOVER = load_discover()


def syntax_deps(source: Path) -> list[Path]:
    seen: list[Path] = []
    pending = [source.resolve()]
    known = {source.resolve()}
    while pending:
        current = pending.pop()
        if not current.is_file():
            continue
        for spec in DISCOVER.SYNTAX_IMPORT.findall(DISCOVER.read(current)):
            body = spec.split("?", 1)[0]
            dep = (current.parent / body).resolve()
            if dep in known or not dep.is_file():
                continue
            known.add(dep)
            seen.append(dep)
            pending.append(dep)
    return seen


def theirs_name(input_name: str) -> str:
    if input_name.endswith("stsx") or input_name.endswith("tsx"):
        return "theirs.tsx"
    return "theirs.ts"


def expand_upstream(source: Path) -> str:
    if not SWEETENER.is_file():
        raise SystemExit(f"sweetener CLI missing: {SWEETENER}")
    completed = subprocess.run(
        ["node", str(SWEETENER), "expand", str(source)],
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        print(f"expand failed {source.name}: {detail}")
        return ""
    text = completed.stdout
    return text if text.endswith("\n") or text == "" else text + "\n"


def vendor_file(entry: str) -> Path:
    path = Path(entry)
    if path.is_file():
        return path
    return ROOT / entry


def main() -> int:
    apply = "--apply" in sys.argv
    refresh = "--refresh" in sys.argv
    rows = [json.loads(line) for line in MANIFEST.read_text().splitlines() if line.strip()]
    ids = {row["id"] for row in rows}
    if CASES.is_dir():
        for child in sorted(CASES.iterdir()):
            if child.is_dir() and child.name not in ids:
                print(f"orphan {child.name} (not in cases.jsonl; left in place)")
    changes = 0
    for row in rows:
        case_id = row["id"]
        case = CASES / case_id
        input_name = row.get("input", "input.sts")
        input_path = case / input_name
        have = input_path.is_file()
        if have and not refresh:
            if apply:
                (case / "test.title").write_text(row.get("testTitle", case_id) + "\n", encoding="utf-8")
            print(f"kept {case_id}")
            continue
        source = vendor_file(row.get("entry", ""))
        if not source.is_file():
            print(f"missing {case_id}: {row.get('entry')} is not a file")
            changes += 1
            continue
        action = "refresh" if have else "write"
        print(f"{action} {case_id}")
        changes += 1
        if not apply:
            continue
        case.mkdir(parents=True, exist_ok=True)
        text = source.read_bytes()
        input_path.write_bytes(text)
        (case / "entry.name").write_text(source.name + "\n", encoding="utf-8")
        (case / "test.title").write_text(row.get("testTitle", case_id) + "\n", encoding="utf-8")
        for dep in syntax_deps(source):
            dest = case / dep.name
            if dest.exists() and dest.resolve() == dep.resolve():
                continue
            dest.write_bytes(dep.read_bytes())
        if "malformed" in case_id:
            continue
        expanded = expand_upstream(input_path)
        if not expanded:
            continue
        (case / theirs_name(input_name)).write_text(expanded, encoding="utf-8")
    if not apply and changes:
        print("dry-run; pass --apply to write")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
