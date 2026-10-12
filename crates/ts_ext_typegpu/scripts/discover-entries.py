#!/usr/bin/env python3
"""List TypeGPU plugin cases that are not in cases.jsonl.

A case is a `describe` / `it` / `test` in the pinned plugin tests that owns
a `const code` template. Inner `babel` and `rollup` checks are not cases.
This script does not edit the manifest and does not write case directories.

Usage:
  discover-entries.py            print cases the manifest does not list
  discover-entries.py --all      print every discovered case
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CRATE = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor" / "TypeGPU" / "packages" / "unplugin-typegpu" / "test"
MANIFEST = CRATE / "cases.jsonl"
TITLE = re.compile(r"""(?:describe|it|test)\(\s*(['"`])(.*?)\1""")
SKIP_TITLES = {"babel", "rollup"}


def slug(title: str) -> str:
    text = title.lower().replace("'", "").replace('"', "")
    text = re.sub(r"[^a-z0-9]+", "-", text).strip("-")
    return text


def template_at(text: str, start: int) -> str | None:
    """JS string value of a template that starts at the backtick `start`."""
    if start >= len(text) or text[start] != "`":
        return None
    index = start + 1
    out: list[str] = []
    while index < len(text):
        ch = text[index]
        if ch == "\\":
            nxt = text[index + 1] if index + 1 < len(text) else ""
            if nxt == "\n":
                index += 2
                continue
            if nxt == "`":
                out.append("`")
                index += 2
                continue
            if nxt == "\\":
                out.append("\\")
                index += 2
                continue
            out.append(ch)
            index += 1
            continue
        if ch == "`":
            return "".join(out)
        out.append(ch)
        index += 1
    return None


def cases_in(path: Path) -> list[tuple[str, str]]:
    """(title, code) for each template, titled by the nearest real block."""
    text = path.read_text(encoding="utf-8", errors="replace")
    found: list[tuple[str, str]] = []
    cursor = 0
    needle = "const code = `"
    while True:
        at = text.find(needle, cursor)
        if at < 0:
            break
        code = template_at(text, at + len(needle) - 1)
        if code is None:
            break
        titles = [
            match.group(2)
            for match in TITLE.finditer(text[:at])
            if match.group(2) not in SKIP_TITLES
        ]
        if titles:
            found.append((titles[-1], code))
        cursor = at + len(needle)
    return found


def listed() -> list[dict]:
    if not MANIFEST.is_file():
        return []
    return [json.loads(line) for line in MANIFEST.read_text().splitlines() if line.strip()]


def covered(file_name: str, title: str, rows: list[dict]) -> bool:
    snapshot = file_name.removesuffix(".test.ts")
    title_slug = slug(title)
    for row in rows:
        entry = row.get("entry", "")
        if not entry.endswith("/" + file_name) and not entry.endswith(file_name):
            continue
        if row.get("testTitle") == title:
            return True
        case_id = row.get("id", "")
        if case_id == title_slug or case_id.endswith("-" + title_slug):
            return True
        if row.get("snapshot") == snapshot and case_id.endswith("-" + title_slug):
            return True
    return False


def main() -> int:
    show_all = "--all" in sys.argv
    rows = listed()
    discovered: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()
    for path in sorted(VENDOR.glob("*.test.ts")):
        for title, _code in cases_in(path):
            key = (path.name, title)
            if key in seen:
                continue
            seen.add(key)
            discovered.append(key)
    missing = [(file_name, title) for file_name, title in discovered if not covered(file_name, title, rows)]
    print(
        f"discovered={len(discovered)} listed={len(rows)} "
        f"missing={len(missing)}"
    )
    shown = discovered if show_all else missing
    for file_name, title in shown:
        if show_all and covered(file_name, title, rows):
            continue
        snapshot = file_name.removesuffix(".test.ts")
        print(f"missing {snapshot}.test.ts  {title}")
        print(f"  id {snapshot}-{slug(title)}")
    probes = [row["id"] for row in rows if row.get("sources") == "probe"]
    if probes and not show_all:
        print(f"probes={len(probes)} (kept by hand, not discovered from the plugin tests)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
