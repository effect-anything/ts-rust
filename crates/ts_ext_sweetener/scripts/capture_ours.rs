//! Write `ours.ts` next to each captured `theirs.ts`. Does not overwrite
//! `theirs.ts`. Comparison is this crate's syntax tree, not TypeGPU's.
//!
//! `cargo run -q -p ts_ext_sweetener --example capture_ours -- --all`
//! `cargo run -q -p ts_ext_sweetener --example capture_ours -- fixtures-acceptance`
//! visits flat ids with that prefix. Cases already equal are skipped unless
//! `--all` is passed.

use std::fs;
use std::path::{Path, PathBuf};

use ts_ext_sweetener::{ExpandRequest, Phase, expand, structure};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("cases");
    let mut all = false;
    let mut prefix = String::new();
    for arg in std::env::args().skip(1) {
        if arg == "--all" {
            all = true;
        } else {
            prefix = arg;
        }
    }
    let mut wrote = 0u32;
    let mut equal = 0u32;
    let mut shape = 0u32;
    let mut differ = 0u32;
    let mut skipped = 0u32;
    let Ok(entries) = fs::read_dir(&root) else {
        eprintln!("no cases at {}", root.display());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        if !path.is_dir() || (!prefix.is_empty() && !name.starts_with(&prefix)) {
            continue;
        }
        visit(
            &path,
            all,
            &mut wrote,
            &mut equal,
            &mut shape,
            &mut differ,
            &mut skipped,
        );
    }
    println!("wrote={wrote} equal={equal} shape={shape} differ={differ} skipped={skipped}");
}

fn visit(
    dir: &Path,
    all: bool,
    wrote: &mut u32,
    equal: &mut u32,
    shape: &mut u32,
    differ: &mut u32,
    skipped: &mut u32,
) {
    let theirs = snapshot_path(dir, "theirs");
    let needs = all || dir.join("diff.txt").is_file() || !snapshot_path(dir, "ours").is_file();
    if theirs.is_file() && needs {
        match capture(dir, &theirs) {
            Ok(Same::Bytes) => {
                *wrote += 1;
                *equal += 1;
                let _ = fs::remove_file(dir.join("diff.txt"));
            }
            Ok(Same::Shape) => {
                *wrote += 1;
                *shape += 1;
                let _ = fs::remove_file(dir.join("diff.txt"));
            }
            Ok(Same::Differ) => {
                *wrote += 1;
                *differ += 1;
            }
            Err(err) => {
                *skipped += 1;
                eprintln!("skip {}: {err}", dir.display());
            }
        }
    }
}

enum Same {
    Bytes,
    Shape,
    Differ,
}

fn capture(dir: &Path, theirs: &Path) -> Result<Same, String> {
    let input = input_file(dir).ok_or_else(|| "no input".to_string())?;
    let code = fs::read_to_string(&input).map_err(|err| err.to_string())?;
    let filename = input
        .canonicalize()
        .unwrap_or(input.clone())
        .to_string_lossy()
        .into_owned();
    let expansion = expand(ExpandRequest {
        filename: &filename,
        code: &code,
        phase: Phase::Check,
    });
    fs::write(snapshot_path(dir, "ours"), &expansion.code).map_err(|err| err.to_string())?;
    let reference = fs::read_to_string(theirs).map_err(|err| err.to_string())?;
    if collapse(&reference) == collapse(&expansion.code) {
        return Ok(Same::Bytes);
    }
    let tsx = theirs.extension().is_some_and(|ext| ext == "tsx");
    if structure::same(&reference, &expansion.code, tsx) {
        return Ok(Same::Shape);
    }
    let diff = structure::unified(&reference, &expansion.code);
    fs::write(dir.join("diff.txt"), diff).map_err(|err| err.to_string())?;
    Ok(Same::Differ)
}

fn input_file(dir: &Path) -> Option<PathBuf> {
    for name in ["input.sts", "input.stsx", "input.ts"] {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn snapshot_path(dir: &Path, stem: &str) -> PathBuf {
    let name = fs::read_to_string(dir.join("entry.name")).unwrap_or_default();
    let ext = virtual_extension(name.trim());
    let path = dir.join(format!("{stem}{ext}"));
    if path.is_file() {
        return path;
    }
    let legacy = dir.join(format!("{stem}.ts"));
    if legacy.is_file() {
        return legacy;
    }
    path
}

fn virtual_extension(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("tsx" | "stsx") => ".tsx",
        _ => ".ts",
    }
}

fn collapse(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
