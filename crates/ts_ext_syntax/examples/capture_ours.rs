//! Write `ours.ts` next to each captured `theirs.ts`. Does not overwrite
//! `theirs.ts`. Sweetener cases call the live bridge. TypeGPU cases compare
//! operator rewrites after stripping `__TYPEGPU_META__` wrappers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ts_ext_syntax::{ExpandRequest, Phase, expand};

fn main() {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance/cases");
    // `capture_ours sweetener/fixtures/acceptance` only visits that subtree.
    // Cases already equal are skipped unless `--all` is passed.
    let mut all = false;
    for arg in std::env::args().skip(1) {
        if arg == "--all" {
            all = true;
        } else {
            root = root.join(arg);
        }
    }
    let mut wrote = 0u32;
    let mut equal = 0u32;
    let mut shape = 0u32;
    let mut differ = 0u32;
    let mut skipped = 0u32;
    visit(
        &root,
        all,
        &mut wrote,
        &mut equal,
        &mut shape,
        &mut differ,
        &mut skipped,
    );
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
    let theirs = dir.join("theirs.ts");
    let needs = all || dir.join("diff.txt").is_file() || !dir.join("ours.ts").is_file();
    if theirs.is_file() && needs {
        match capture(dir, &theirs) {
            Ok(kind) => {
                *wrote += 1;
                match kind {
                    Same::Bytes => {
                        *equal += 1;
                        let _ = fs::remove_file(dir.join("diff.txt"));
                    }
                    Same::Shape => {
                        *shape += 1;
                        let _ = fs::remove_file(dir.join("diff.txt"));
                    }
                    Same::Differ => *differ += 1,
                }
            }
            Err(err) => {
                *skipped += 1;
                eprintln!("skip {}: {err}", dir.display());
            }
        }
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, all, wrote, equal, shape, differ, skipped);
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
    let expansion = if dir.to_string_lossy().contains("/typegpu/") {
        // Build is the pass that embeds `__TYPEGPU_META__`, the tinyest AST,
        // and externals. Check rewrites operators and stops there.
        let built = ts_ext_syntax::TransformSession::build().transform(&filename, &code);
        fs::write(dir.join("ours.ts"), &built.code).map_err(|err| err.to_string())?;
        return compare_payload(&built.code, theirs);
    } else {
        expand(ExpandRequest {
            filename: &filename,
            code: &code,
            phase: Phase::Check,
        })
    };
    fs::write(dir.join("ours.ts"), &expansion.code).map_err(|err| err.to_string())?;
    let reference = fs::read_to_string(theirs).map_err(|err| err.to_string())?;
    let left = normalize_theirs(&reference);
    let right = normalize_ours(&expansion.code);
    if left == right {
        return Ok(Same::Bytes);
    }
    // Both sides go through the same canonical form, so an old captured
    // print with extra parentheses or wrapped lines stays equal.
    if canonical(&left) == canonical(&right) {
        return Ok(Same::Shape);
    }
    let diff = unified(&canonical(&left), &canonical(&right));
    fs::write(dir.join("diff.txt"), diff).map_err(|err| err.to_string())?;
    Ok(Same::Differ)
}

/// The form both sides are compared in. Whitespace and parentheses that do
/// not change an operator, a type, or a call are removed. A missing
/// expansion still differs because its identifiers differ.
/// Operators, metadata name, AST nodes, and externals. Whitespace is not
/// part of the comparison.
fn compare_payload(ours: &str, theirs: &Path) -> Result<Same, String> {
    let reference = fs::read_to_string(theirs).map_err(|err| err.to_string())?;
    let left = payload(&reference);
    let right = payload(ours);
    if left == right {
        let _ = fs::remove_file(theirs.parent().unwrap().join("diff.txt"));
        return Ok(Same::Shape);
    }
    fs::write(
        theirs.parent().unwrap().join("diff.txt"),
        format!("--- theirs payload\n{left}\n--- ours payload\n{right}\n"),
    )
    .map_err(|err| err.to_string())?;
    Ok(Same::Differ)
}

fn payload(text: &str) -> String {
    let mut out = String::new();
    for needle in ["__tsover_add", "__tsover_sub", "__tsover_mul", "__tsover_div", "__tsover_mod"] {
        let count = text.matches(needle).count();
        if count > 0 {
            out.push_str(&format!("{needle}={count}\n"));
        }
    }
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("name:") || line.starts_with("body:") || line.starts_with("externals:") || line.contains("() =>") {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn canonical(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '(' && *ch != ')')
        .collect()
}

fn input_file(dir: &Path) -> Option<PathBuf> {
    for name in ["input.ts", "input.sts", "input.stsx", "input.sjs", "input.sjsx"] {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Drop TypeGPU's build-time metadata wrapper so the expand comparison is the
/// operator rewrite. Sweetener output has no such wrapper and is unchanged.
fn normalize_theirs(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("/*#__PURE__*/") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(fn_at) = after.find("$.f = ") else {
            out.push_str(after);
            return collapse(&out);
        };
        let body = &after[fn_at + "$.f = ".len()..];
        let Some(end) = body.find("}, {") else {
            out.push_str(after);
            return collapse(&out);
        };
        let mut function = body[..end].trim().to_string();
        if function.starts_with('(') && function.ends_with(')') {
            function = function[1..function.len() - 1].to_string();
        }
        out.push_str(function.trim_end_matches([',', ' ']));
        let Some(close) = body[end..].find(")({});") else {
            rest = &body[end..];
            continue;
        };
        rest = &body[end + close + ")({});".len()..];
    }
    out.push_str(rest);
    collapse(&out)
}

fn normalize_ours(text: &str) -> String {
    collapse(text)
}

fn collapse(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn unified(theirs: &str, ours: &str) -> String {
    let dir = std::env::temp_dir();
    let a = dir.join("ext-syntax-theirs.txt");
    let b = dir.join("ext-syntax-ours.txt");
    let _ = fs::write(&a, theirs);
    let _ = fs::write(&b, ours);
    let output = Command::new("diff")
        .args(["-u", a.to_str().unwrap_or("a"), b.to_str().unwrap_or("b")])
        .output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
        Err(err) => format!("diff failed: {err}\n--- theirs\n{theirs}\n--- ours\n{ours}\n"),
    }
}
