#!/usr/bin/env python3
"""Keep each Sweetener case to the entry plus the files it imports.

A case needs:

- `input.<ext>` — the program entry
- `entry.name` — the upstream file name, so `./macros.sts` still resolves
- `deps/` — only files reached by `import ... for syntax`, recursively

`expected.*`, `intent.json`, `types.ts`, `runtime.ts`, and `case.json` are
upstream oracles. They are not program input. A second entry such as
`workflow.sts` or `hygiene.sts` is its own case, not a dependency of `main`.

Usage:
  sync-cases.py            print what would be removed or added
  sync-cases.py --apply    rewrite deps and add missing entries
"""

from __future__ import annotations

import hashlib
import importlib.util
import sys
from pathlib import Path


def _load_discover():
    import importlib.util
    path = Path(__file__).resolve().parent / "discover-entries.py"
    spec = importlib.util.spec_from_file_location("discover_entries", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


_discover = _load_discover()
CASES = _discover.CASES
EXTS = _discover.EXTS
SYNTAX_IMPORT = _discover.SYNTAX_IMPORT
VENDOR = _discover.VENDOR
fixture_entries = _discover.fixture_entries
is_macro_only = _discover.is_macro_only
program_entries = _discover.program_entries
read = _discover.read
vendor_rel = _discover.vendor_rel


KEEP_CASE_FILES = {
    "entry.name",
    "input.pin",
    "deps.blob",
    "theirs.ts",
    "ours.ts",
    "diff.txt",
}


def syntax_deps(source: Path) -> list[Path]:
    """Files imported `for syntax`, relative to the importer, recursively."""
    seen: list[Path] = []
    pending = [source.resolve()]
    known = {source.resolve()}
    while pending:
        current = pending.pop()
        if not current.is_file():
            continue
        base = current.parent
        for spec in SYNTAX_IMPORT.findall(read(current)):
            body = spec.split("?", 1)[0]
            dep = (base / body).resolve()
            if dep in known or not dep.is_file():
                continue
            known.add(dep)
            seen.append(dep)
            pending.append(dep)
    return seen


def discovered() -> dict[str, Path]:
    entries = []
    entries.extend(fixture_entries(VENDOR / "fixtures"))
    entries.extend(program_entries(VENDOR / "examples"))
    entries.extend(program_entries(VENDOR / "samples"))
    found: dict[str, Path] = {}
    for _reason, path in entries:
        path = path.resolve()
        if is_macro_only(path):
            continue
        found.setdefault(vendor_rel(path), path)
    return found


def case_dir_for(rel: str) -> Path:
    path = Path(rel)
    return CASES / path.parent / path.stem


def wanted_deps(entry: Path) -> dict[str, Path]:
    """Map of file name copied next to the staged input, to its source."""
    wanted: dict[str, Path] = {}
    for dep in syntax_deps(entry):
        name = dep.name
        if name in wanted and wanted[name].read_bytes() != dep.read_bytes():
            raise SystemExit(f"two imported files share the name {name}: {wanted[name]} and {dep}")
        wanted[name] = dep
    return wanted


def prune_case(case: Path, wanted: dict[str, Path], apply: bool) -> list[str]:
    notes: list[str] = []
    deps = case / "deps"
    if deps.is_dir():
        for child in sorted(deps.rglob("*")):
            if not child.is_file():
                continue
            notes.append(f"remove {child.relative_to(CASES)}")
            if apply:
                child.unlink()
        if apply:
            for child in sorted(deps.rglob("*"), reverse=True):
                if child.is_dir() and not any(child.iterdir()):
                    child.rmdir()
            if deps.is_dir() and not any(deps.iterdir()):
                deps.rmdir()
    for name, source in sorted(wanted.items()):
        dest = case / name
        # The expander resolves `./macros.sts` beside `input.sts`, not in deps/.
        if not dest.is_file() or dest.is_symlink():
            notes.append(f"write {dest.relative_to(CASES)}")
            if apply:
                dest.parent.mkdir(parents=True, exist_ok=True)
                if dest.is_symlink() or dest.exists():
                    dest.unlink()
                dest.write_bytes(source.read_bytes())
        elif dest.read_bytes() != source.read_bytes():
            notes.append(f"update {dest.relative_to(CASES)}")
            if apply:
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(source.read_bytes())
    return notes


def add_case(rel: str, entry: Path, apply: bool) -> list[str]:
    case = case_dir_for(rel)
    notes = [f"add {case.relative_to(CASES)} from {entry.name}"]
    if not apply:
        return notes
    case.mkdir(parents=True, exist_ok=True)
    ext = entry.suffix
    (case / "entry.name").write_text(entry.name + "\n", encoding="utf-8")
    (case / f"input{ext}").write_bytes(entry.read_bytes())
    (case / "input.pin").write_text("vendor\n", encoding="utf-8")
    blob = [f"ENTRY {entry.name} {hashlib.sha1(entry.read_bytes()).hexdigest()}"]
    for dep in syntax_deps(entry):
        blob.append(f"{dep.name} {hashlib.sha1(dep.read_bytes()).hexdigest()}")
        (case / dep.name).write_bytes(dep.read_bytes())
    (case / "deps.blob").write_text("\n".join(blob) + "\n", encoding="utf-8")
    return notes


def existing_cases() -> list[Path]:
    found = []
    for entry_name in CASES.rglob("entry.name"):
        found.append(entry_name.parent)
    return found


def match_entry(case: Path, entries: dict[str, Path]) -> Path | None:
    original = (case / "entry.name").read_text(encoding="utf-8").strip()
    parent = case.relative_to(CASES).as_posix()
    for rel, path in entries.items():
        if path.name != original:
            continue
        folder = Path(rel).parent.as_posix()
        expected = f"{folder}/{Path(rel).stem}"
        # cases/.../power-operator/main matches examples/.../power-operator/main.sts.
        # Older fixture captures used the fixture directory itself.
        legacy = folder if original == path.name else None
        if parent in {expected, legacy} or parent.endswith("/" + expected):
            return path
    return None


def main() -> int:
    apply = "--apply" in sys.argv
    entries = discovered()
    matched: set[str] = set()
    changes = 0
    for case in existing_cases():
        entry = match_entry(case, entries)
        if entry is None:
            print(f"unmatched {case.relative_to(CASES)}")
            continue
        matched.add(vendor_rel(entry))
        wanted = wanted_deps(entry)
        notes = prune_case(case, wanted, apply)
        for child in case.iterdir():
            if child.is_dir() or child.name.startswith("input."):
                continue
            if child.name in KEEP_CASE_FILES or child.name in wanted:
                continue
            if child.is_symlink() or child.suffix in EXTS or child.name.startswith("expected.") or child.name in {"case.json", "intent.json", "types.ts", "runtime.ts"}:
                notes.append(f"remove {child.relative_to(CASES)}")
                if apply:
                    child.unlink()
        if apply and (case / "deps.blob").is_file():
            blob = [f"ENTRY {entry.name} {__import__('hashlib').sha1(entry.read_bytes()).hexdigest()}"]
            for name, source in sorted(wanted.items()):
                blob.append(f"{name} {__import__('hashlib').sha1(source.read_bytes()).hexdigest()}")
            (case / "deps.blob").write_text("\n".join(blob) + "\n", encoding="utf-8")
        if notes:
            changes += 1
            print(f"{case.relative_to(CASES)}")
            for note in notes:
                print(f"  {note}")
    missing = [rel for rel in sorted(entries) if rel not in matched]
    for rel in missing:
        for note in add_case(rel, entries[rel], apply):
            print(note)
        changes += 1
    print(f"cases={len(existing_cases())} changed={changes} missing={len(missing)} apply={apply}")
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    raise SystemExit(main())
