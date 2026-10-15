//! Write `ours.ts` for each captured TypeGPU case. Does not overwrite
//! `theirs.ts`. Sweetener cases are not in this tree.
//!
//! Compare every case named in `cases.jsonl`. It does not discover cases.
//! Add a line with `scripts/discover-entries.py`, then materialise it with
//! `scripts/sync-cases.py --apply`, then run this.
//!
//! `cargo run -q -p ts_ext_typegpu --example capture_ours -- --all`
//! `cargo run -q -p ts_ext_typegpu --example capture_ours -- use-gpu-directive`
//! limits to ids that start with that prefix. One id checks one case.

use std::fs;
use std::path::{Path, PathBuf};

use ts_ext_typegpu::{TypegpuSession, structure};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut all = false;
    let mut prefix = String::new();
    for arg in std::env::args().skip(1) {
        if arg == "--all" {
            all = true;
        } else {
            prefix = arg;
        }
    }
    let manifest = fs::read_to_string(root.join("cases.jsonl")).unwrap_or_default();
    let mut wrote = 0u32;
    let mut equal = 0u32;
    let mut shape = 0u32;
    let mut differ = 0u32;
    let mut skipped = 0u32;
    for line in manifest.lines() {
        let id = id_of(line);
        if id.is_empty() || (!prefix.is_empty() && !id.starts_with(&prefix)) {
            continue;
        }
        let dir = root.join("cases").join(&id);
        let theirs = dir.join("theirs.ts");
        let needs = all
            || !prefix.is_empty()
            || dir.join("diff.txt").is_file()
            || !dir.join("ours.ts").is_file();
        if !theirs.is_file() || !needs {
            if !theirs.is_file() {
                skipped += 1;
                eprintln!("skip {id}: no theirs.ts");
            }
            continue;
        }
        match capture(&dir, &theirs, line) {
            Ok(Same::Bytes) => {
                wrote += 1;
                equal += 1;
                let _ = fs::remove_file(dir.join("diff.txt"));
            }
            Ok(Same::Shape) => {
                wrote += 1;
                shape += 1;
                let _ = fs::remove_file(dir.join("diff.txt"));
            }
            Ok(Same::Differ) => {
                wrote += 1;
                differ += 1;
            }
            Err(err) => {
                skipped += 1;
                eprintln!("skip {id}: {err}");
            }
        }
    }
    println!("wrote={wrote} equal={equal} shape={shape} differ={differ} skipped={skipped}");
}

fn id_of(line: &str) -> String {
    let Some(start) = line.find("\"id\":\"") else {
        return String::new();
    };
    let rest = &line[start + "\"id\":\"".len()..];
    rest.split('"').next().unwrap_or("").to_owned()
}

enum Same {
    Bytes,
    Shape,
    Differ,
}

fn capture(dir: &Path, theirs: &Path, manifest_line: &str) -> Result<Same, String> {
    let input = dir.join("input.ts");
    if !input.is_file() {
        return Err("no input".to_owned());
    }
    let code = fs::read_to_string(&input).map_err(|err| err.to_string())?;
    let built =
        TypegpuSession::build_with_auto_naming(manifest_line.contains("\"autoNaming\":true"))
            .transform(&dir.to_string_lossy(), &code);
    fs::write(dir.join("ours.ts"), &built.code).map_err(|err| err.to_string())?;
    compare(dir, &built.code, theirs)
}

fn compare(dir: &Path, ours: &str, theirs: &Path) -> Result<Same, String> {
    let reference = fs::read_to_string(theirs).map_err(|err| err.to_string())?;
    let externals_only = theirs.parent().is_some_and(|dir| {
        dir.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("externals-"))
    }) && reference.trim_start().starts_with('{')
        && reference.contains(": () =>")
        && !reference.contains("function")
        && !reference.contains("__TYPEGPU_META__");
    let left = if externals_only {
        reference.trim().to_owned()
    } else {
        reference
    };
    let right = if externals_only {
        externals_object(ours).to_owned()
    } else {
        ours.to_owned()
    };
    if left == right {
        return Ok(Same::Bytes);
    }
    if structure::same(&left, &right, false)
        && left.matches("#__PURE__").count() == right.matches("#__PURE__").count()
    {
        return Ok(Same::Shape);
    }
    let text = match (
        structure::dump(&left, false),
        structure::dump(&right, false),
    ) {
        (Some(theirs_tree), Some(ours_tree)) => structure::unified(&theirs_tree, &ours_tree),
        _ => "one side does not parse\n".to_owned(),
    };
    fs::write(dir.join("diff.txt"), text).map_err(|err| err.to_string())?;
    Ok(Same::Differ)
}

fn externals_object(text: &str) -> &str {
    let Some(at) = text.find("externals:") else {
        return text;
    };
    let Some(open) = text[at..].find('{') else {
        return text;
    };
    let start = at + open;
    let mut depth = 0i32;
    for (index, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[start..start + index + 1];
                }
            }
            _ => {}
        }
    }
    text
}
