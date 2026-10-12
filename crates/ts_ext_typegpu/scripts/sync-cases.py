#!/usr/bin/env python3
"""Materialise the cases named in cases.jsonl. Nothing else.

Discovery does not write the manifest. Add a line by hand, then run this.
A case that is already on disk is left alone unless `--refresh` is set.
Probe cases (`sources: probe`) have no vendor test; this script only checks
that their files are present.

Usage:
  sync-cases.py                 print what would be written
  sync-cases.py --apply         write cases that have no input.ts yet
  sync-cases.py --apply --refresh
                                pull every pinned case again from the vendor pin
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
ROOT = Path(__file__).resolve().parents[3]
CASES = CRATE / "cases"
MANIFEST = CRATE / "cases.jsonl"
UPSTREAM = ROOT / "scripts" / "ext-bench" / "upstream-typegpu.mjs"


def load_discover():
    path = Path(__file__).resolve().parent / "discover-entries.py"
    spec = importlib.util.spec_from_file_location("discover_entries", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DISCOVER = load_discover()


def vendor_file(entry: str) -> Path:
    path = Path(entry)
    if path.is_file():
        return path
    return ROOT / entry


def code_for(row: dict) -> str | None:
    path = vendor_file(row.get("entry", ""))
    if not path.is_file():
        return None
    title = row.get("testTitle", "")
    wanted = DISCOVER.slug(title)
    for found, code in DISCOVER.cases_in(path):
        if found == title or DISCOVER.slug(found) == wanted:
            return code
    return None


def theirs_text(code: str, auto_naming: bool, externals_only: bool) -> str:
    payload = json.dumps(
        [{"name": "case", "code": code, "options": {"autoNamingEnabled": auto_naming}}]
    )
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        handle.write(payload)
        spec = handle.name
    completed = subprocess.run(
        ["node", str(UPSTREAM), spec],
        text=True,
        capture_output=True,
        check=False,
    )
    Path(spec).unlink(missing_ok=True)
    if completed.returncode != 0:
        raise SystemExit(completed.stderr.strip() or "upstream plugin failed")
    text = completed.stdout
    body = text.split("\n", 1)[1] if text.startswith("### ") else text
    if body.startswith("<<threw>>"):
        raise SystemExit(body.strip())
    if not externals_only:
        return body if body.endswith("\n") else body + "\n"
    start = body.find("externals:")
    end = body.find("}) && $.f")
    if start < 0 or end < 0:
        raise SystemExit("upstream output has no externals object")
    chunk = body[start + len("externals:") : end].strip()
    return chunk + "\n"


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
        input_path = case / "input.ts"
        theirs_path = case / "theirs.ts"
        if row.get("sources") == "probe":
            if input_path.is_file() and theirs_path.is_file():
                print(f"kept {case_id} (probe)")
            else:
                print(f"missing {case_id}: probe cases are written by hand")
                changes += 1
            continue
        have = input_path.is_file() and theirs_path.is_file()
        if have and not refresh:
            if apply:
                (case / "test.title").write_text(row.get("testTitle", "") + "\n", encoding="utf-8")
            print(f"kept {case_id}")
            continue
        code = code_for(row)
        if code is None:
            print(f"missing {case_id}: no `const code` for {row.get('testTitle')!r} in {row.get('entry')}")
            changes += 1
            continue
        action = "refresh" if have else "write"
        print(f"{action} {case_id}")
        changes += 1
        if not apply:
            continue
        case.mkdir(parents=True, exist_ok=True)
        input_path.write_text(code if code.endswith("\n") else code + "\n", encoding="utf-8")
        (case / "test.title").write_text(row.get("testTitle", "") + "\n", encoding="utf-8")
        theirs_path.write_text(
            theirs_text(code, row.get("autoNaming") is True, case_id.startswith("externals-")),
            encoding="utf-8",
        )
    if not apply and changes:
        print("dry-run; pass --apply to write")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
