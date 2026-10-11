#!/usr/bin/env python3
"""List Sweetener program entries the conformance tree has not captured.

An entry is not guessed from a file name. The sources are:

- fixtures: `case.json` field `entry`, plus sibling `hygiene.*` and
  `malformed.*` when those files exist
- programs: a file that contains `import ... for syntax`
- a `run.mjs` / `package.json` script that names another `.sts` file

Macro definition files are dependencies, not entries. A file that only
exports `syntax` or `operator` and does not import syntax is skipped.

Usage:
  discover-entries.py            print missing entries
  discover-entries.py --all      print every discovered entry
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
VENDOR = (ROOT / "vendor" / "sweetener").resolve()
CASES = ROOT / "crates" / "ts_ext_syntax" / "conformance" / "cases" / "sweetener"
EXTS = {".sts", ".stsx", ".sjs", ".sjsx"}
SYNTAX_IMPORT = re.compile(r"""from\s+['"](\.[^'"]+)['"]\s+for\s+syntax""")
NAMED_SOURCE = re.compile(r"""['"]([^'"]+\.(?:sts|stsx|sjs|sjsx))['"]""")
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


def fixture_entries(fixtures: Path) -> list[tuple[str, Path]]:
    found: list[tuple[str, Path]] = []
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
            found.append((f"case.json:{entry_name}", entry))
        for sibling in case.parent.iterdir():
            if not sibling.is_file() or sibling.suffix not in EXTS:
                continue
            if sibling.stem in {"hygiene", "malformed"}:
                found.append((f"sibling:{sibling.stem}", sibling))
    return found


def program_entries(root: Path) -> list[tuple[str, Path]]:
    found: list[tuple[str, Path]] = []
    if not root.exists():
        return found
    for path in root.rglob("*"):
        if not path.is_file() or path.suffix not in EXTS or skip(path):
            continue
        if "fixtures" in path.parts:
            continue
        text = read(path)
        if SYNTAX_IMPORT.search(text):
            found.append(("syntax-import", path))
    for script in list(root.rglob("run.mjs")) + list(root.rglob("package.json")):
        if skip(script) or "fixtures" in script.parts:
            continue
        text = read(script)
        base = script.parent
        for name in NAMED_SOURCE.findall(text):
            candidate = (base / name).resolve()
            if candidate.is_file() and candidate.suffix in EXTS and not is_macro_only(candidate):
                found.append((f"script:{script.name}", candidate))
    return found


def captured_names() -> set[tuple[str, str]]:
    """(project-relative dir, original file name) for each existing case."""
    found: set[tuple[str, str]] = set()
    if not CASES.exists():
        return found
    for entry_name in CASES.rglob("entry.name"):
        original = entry_name.read_text(encoding="utf-8").strip()
        parent = entry_name.parent.relative_to(CASES).as_posix()
        found.add((parent, original))
    return found


def vendor_rel(path: Path) -> str:
    """Path under examples, fixtures, or samples, after symlink resolution."""
    parts = path.parts
    for label in ("examples", "fixtures", "samples"):
        if label in parts:
            return Path(*parts[parts.index(label):]).as_posix()
    return path.name


def covered(path: Path, captured: set[tuple[str, str]]) -> bool:
    rel = vendor_rel(path)
    # examples/language-tour/new/power-operator/main.sts
    # becomes cases/.../examples/language-tour/new/power-operator/main
    without_vendor = rel
    for parent, original in captured:
        if original != path.name:
            continue
        folder = str(Path(without_vendor).parent).replace("\\", "/")
        if parent.endswith(folder) or parent.endswith(folder + "/" + path.stem):
            return True
    return False


def main() -> int:
    show_all = "--all" in sys.argv
    entries = []
    entries.extend(fixture_entries(VENDOR / "fixtures"))
    entries.extend(program_entries(VENDOR / "examples"))
    entries.extend(program_entries(VENDOR / "samples"))
    unique: dict[Path, str] = {}
    for reason, path in entries:
        unique.setdefault(path.resolve(), reason)
    captured = captured_names()
    missing = []
    for path, reason in sorted(unique.items(), key=lambda item: str(item[0])):
        if is_macro_only(path):
            continue
        hit = covered(path, captured)
        if show_all or not hit:
            missing.append((path, reason, hit))
    print(f"discovered={len(unique)} captured_dirs={len(captured)} missing={sum(1 for _, _, hit in missing if not hit)}")
    for path, reason, hit in missing:
        if show_all and hit:
            continue
        print(f"missing {vendor_rel(path)} via {reason}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
