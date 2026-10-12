//! Diff `TypegpuSession::build()` against captured TypeGPU oracles.
//!
//! Reads `use-gpu-directive-*` lines from `cases.jsonl`, writes
//! `ours.ts` from `Transformed.code`, and raw-byte-compares it with
//! `theirs.ts`. Does not write `theirs.ts`.

use std::fs;
use std::path::PathBuf;

use ts_ext_typegpu::TypegpuSession;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("cases.jsonl")).unwrap_or_default();
    let mut cases = 0u32;
    let mut equal = 0u32;
    let mut differ = 0u32;
    let mut classes = ClassCounts::default();
    let mut ids: Vec<String> = manifest
        .lines()
        .filter_map(|line| {
            let start = line.find("\"id\":\"")? + "\"id\":\"".len();
            let id = line[start..].split('"').next()?;
            id.starts_with("use-gpu-directive-").then(|| id.to_owned())
        })
        .collect();
    ids.sort();
    for id in ids {
        let dir = root.join("cases").join(&id);
        if !dir.join("input.ts").is_file() || !dir.join("theirs.ts").is_file() {
            eprintln!("skip {id}: not materialised");
            continue;
        }
        cases += 1;
        let name = id.as_str();
        let input = fs::read_to_string(dir.join("input.ts")).unwrap_or_default();
        let theirs = fs::read_to_string(dir.join("theirs.ts")).unwrap_or_default();
        // `use-gpu-directive.test.ts` runs the plugin with its test helper's
        // default, which has auto-naming off.
        let out = TypegpuSession::build_with_auto_naming(false).transform("input.ts", &input);
        fs::write(dir.join("ours.ts"), &out.code).expect("write ours.ts");
        if out.code.as_bytes() == theirs.as_bytes() {
            equal += 1;
            println!("equal {name}");
            continue;
        }
        differ += 1;
        let class = classify(&theirs, &out.code);
        classes.bump(class);
        println!("differ {name} {class:?}");
        let diff = unified(&theirs, &out.code);
        let head = diff.lines().take(24).collect::<Vec<_>>().join("\n");
        println!("{head}\n");
    }
    println!(
        "ran TypegpuSession::build cases={cases} equal={equal} differ={differ} \
         wrapper-missing={} ast-empty={} hoist-or-placement={} operator-rewrite={} other={}",
        classes.wrapper_missing,
        classes.ast_empty,
        classes.hoist_or_placement,
        classes.operator_rewrite,
        classes.other,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    WrapperMissing,
    AstEmpty,
    HoistOrPlacement,
    OperatorRewrite,
    Other,
}

#[derive(Default)]
struct ClassCounts {
    wrapper_missing: u32,
    ast_empty: u32,
    hoist_or_placement: u32,
    operator_rewrite: u32,
    other: u32,
}

impl ClassCounts {
    fn bump(&mut self, class: Class) {
        match class {
            Class::WrapperMissing => self.wrapper_missing += 1,
            Class::AstEmpty => self.ast_empty += 1,
            Class::HoistOrPlacement => self.hoist_or_placement += 1,
            Class::OperatorRewrite => self.operator_rewrite += 1,
            Class::Other => self.other += 1,
        }
    }
}

/// One primary class per case. Wrapper absence wins, then a missing
/// operator rewrite, then an empty AST, then hoist or placement.
fn classify(theirs: &str, ours: &str) -> Class {
    let wants_meta = theirs.contains("__TYPEGPU_META__");
    let has_meta = ours.contains("__TYPEGPU_META__");
    if wants_meta && !has_meta {
        return Class::WrapperMissing;
    }
    if operator_missing(theirs, ours) {
        return Class::OperatorRewrite;
    }
    if wants_meta && has_meta && ast_empty(ours) && !ast_empty(theirs) {
        if placement_differs(theirs, ours) {
            return Class::HoistOrPlacement;
        }
        return Class::AstEmpty;
    }
    if wants_meta && has_meta && placement_differs(theirs, ours) {
        return Class::HoistOrPlacement;
    }
    Class::Other
}

fn ast_empty(text: &str) -> bool {
    text.contains("body: [0, []]") || text.contains("\"body\":[0,[]]")
}

/// The oracle lifts a `function` statement to a `const` binding. We still
/// emit the original `function` text, so the binding count differs.
fn placement_differs(theirs: &str, ours: &str) -> bool {
    theirs.matches("const ").count() != ours.matches("const ").count()
        && (theirs.contains("function ") || ours.contains("function "))
}

fn operator_missing(theirs: &str, ours: &str) -> bool {
    for name in [
        "__tsover_add",
        "__tsover_sub",
        "__tsover_mul",
        "__tsover_div",
        "__tsover_mod",
    ] {
        if theirs.contains(name) && !ours.contains(name) {
            return true;
        }
    }
    false
}

fn strip_meta(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("/*#__PURE__*/") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(fn_at) = after.find("$.f = ") else {
            out.push_str(after);
            return out;
        };
        let body = &after[fn_at + "$.f = ".len()..];
        let Some(end) = body.find("}, {") else {
            out.push_str(after);
            return out;
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
        let mut after_wrap = &body[end + close + ")({});".len()..];
        if after_wrap.starts_with(';') {
            after_wrap = &after_wrap[1..];
        }
        rest = after_wrap;
    }
    out.push_str(rest);
    out
}

fn collapse(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("declare function __tsover_"))
        .filter(|line| !line.starts_with("// extracted"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn unified(theirs: &str, ours: &str) -> String {
    ts_ext_typegpu::structure::unified(theirs, ours)
}
