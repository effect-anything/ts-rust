#!/usr/bin/env python3
"""Materialise the cases named in cases.jsonl. Nothing else.

Discovery does not write the manifest. Add a line by hand, then run this.

  1. Copy the upstream snippet named by the manifest.
  2. Read the babel `toMatchInlineSnapshot` already written in that test.
     The plugin is not built or run.
  3. Write that snapshot to `theirs.ts`. `capture_ours` compares our build
     with it.

A case that is already on disk is left alone unless `--refresh`.
Probe rows (`sources: probe`) have no vendor test; this script only checks
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
import re
import sys
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
ROOT = Path(__file__).resolve().parents[3]
CASES = CRATE / "cases"
MANIFEST = CRATE / "cases.jsonl"


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


def find_code(text: str, start: int, end: int, last: bool = False) -> int:
    """Index of the opening backtick of a const code template."""
    found = [match.end() - 1 for match in re.finditer(r"const code\s*=\s*`", text[start:end])]
    if not found:
        return -1
    return start + (found[-1] if last else found[0])


def case_from_test(path: Path, title: str) -> tuple[str, str] | None:
    """Upstream `const code` and the babel snapshot already written after it."""
    text = path.read_text(encoding="utf-8", errors="replace")
    at = text.find(title)
    if at < 0:
        at = text.find(title.replace("'", "\\'"))
    if at < 0:
        return None
    nxt = text.find("describe(", at + len(title))
    if nxt < 0:
        nxt = len(text)
    code_at = find_code(text, at, nxt)
    if code_at < 0:
        code_at = find_code(text, 0, at, last=True)
    if code_at < 0:
        return None
    code = DISCOVER.template_at(text, code_at)
    if code is None:
        return None
    nxt_code = find_code(text, code_at + 1, len(text))
    snap = text.find("toMatchInlineSnapshot", code_at)
    if snap < 0 or (nxt_code >= 0 and snap > nxt_code):
        return None
    raw = DISCOVER.template_at(text, text.find("`", snap))
    if raw is None:
        return None
    lines = raw.split("\n")
    indents = [len(line) - len(line.lstrip(" ")) for line in lines if line.strip()]
    cut = min(indents) if indents else 0
    body = "\n".join(line[cut:] for line in lines).strip("\n")
    if len(body) >= 2 and body[0] == '"' and body[-1] == '"':
        body = body[1:-1]
    snapshot = body if body.endswith("\n") or body == "" else body + "\n"
    return code, snapshot


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
        found = case_from_test(vendor_file(row.get("entry", "")), row.get("testTitle", ""))
        if found is None:
            print(f"missing {case_id}: no snippet and babel snapshot for {row.get('testTitle')!r}")
            changes += 1
            continue
        code, snapshot = found
        action = "refresh" if have else "write"
        print(f"{action} {case_id}")
        changes += 1
        if not apply:
            continue
        case.mkdir(parents=True, exist_ok=True)
        input_path.write_text(code if code.endswith("\n") else code + "\n", encoding="utf-8")
        (case / "test.title").write_text(row.get("testTitle", "") + "\n", encoding="utf-8")
        theirs_path.write_text(snapshot, encoding="utf-8")
    if not apply and changes:
        print("dry-run; pass --apply to write")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
