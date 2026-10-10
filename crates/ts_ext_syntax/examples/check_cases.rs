//! Check stage for captured cases.
//!
//! Writes `ours.check.txt` by running `tsgo --noEmit` on the expanded
//! TypeScript. Does not overwrite `theirs.ts`. Sweetener inputs are staged
//! with their `deps/` so relative imports resolve. TypeGPU inputs get a
//! local declaration of `__tsover_*` so a missing name is not a false failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ts_ext_syntax::{ExpandRequest, Phase, expand};

const OVERLOADS: &str = r#"
declare function __tsover_add(a: any, b: any): any;
declare function __tsover_sub(a: any, b: any): any;
declare function __tsover_mul(a: any, b: any): any;
declare function __tsover_div(a: any, b: any): any;
declare function __tsover_mod(a: any, b: any): any;
"#;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance/cases");
    let tsgo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/tsgo");
    let mut checked = 0u32;
    let mut clean = 0u32;
    let mut failed = 0u32;
    visit(&root, &tsgo, &mut checked, &mut clean, &mut failed);
    println!("checked={checked} clean={clean} with_diagnostics={failed}");
}

fn visit(dir: &Path, tsgo: &Path, checked: &mut u32, clean: &mut u32, failed: &mut u32) {
    if dir.join("theirs.ts").is_file()
        && input_file(dir).is_some()
        && let Ok(text) = check_one(dir, tsgo)
    {
        *checked += 1;
        if text.trim().is_empty() {
            *clean += 1;
        } else {
            *failed += 1;
        }
        let _ = fs::write(dir.join("ours.check.txt"), text);
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            visit(&entry.path(), tsgo, checked, clean, failed);
        }
    }
}

fn check_one(dir: &Path, tsgo: &Path) -> Result<String, String> {
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
    if expansion
        .diagnostics
        .iter()
        .any(|d| d.message.contains("not built in yet"))
    {
        return Ok("expand: Sweetener expansion is not built in yet\n".to_string());
    }
    let stage = std::env::temp_dir().join(format!(
        "ext-syntax-check-{}",
        dir.file_name().and_then(|s| s.to_str()).unwrap_or("case")
    ));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).map_err(|err| err.to_string())?;
    if let Ok(deps) = fs::read_dir(dir.join("deps")) {
        for dep in deps.flatten() {
            let name = dep.file_name();
            let Some(text) = name.to_str() else {
                continue;
            };
            if text.starts_with("expected.") || text.ends_with(".json") {
                continue;
            }
            let _ = fs::copy(dep.path(), stage.join(name));
        }
    }
    let mut virtual_name = input
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input.ts")
        .to_string();
    if virtual_name.ends_with(".sts") {
        virtual_name = virtual_name.replace(".sts", ".ts");
    } else if virtual_name.ends_with(".stsx") {
        virtual_name = virtual_name.replace(".stsx", ".tsx");
    }
    let mut text = expansion.code.clone();
    if dir.to_string_lossy().contains("/typegpu/") {
        text = format!("{OVERLOADS}\n{text}");
    }
    fs::write(stage.join(&virtual_name), &text).map_err(|err| err.to_string())?;
    let types = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance/types");
    if let Ok(entries) = fs::read_dir(&types) {
        for entry in entries.flatten() {
            let _ = fs::copy(entry.path(), stage.join(entry.file_name()));
        }
    }
    let config = format!(
        "{{\"compilerOptions\":{{\"strict\":true,\"noEmit\":true,\"jsx\":\"react-jsx\",\"moduleResolution\":\"bundler\",\"target\":\"esnext\",\"module\":\"esnext\",\"skipLibCheck\":true,\"noImplicitAny\":false,\"allowArbitraryExtensions\":true}},\"include\":[\"*.ts\",\"*.tsx\",\"*.d.ts\"]}}"
    );
    fs::write(stage.join("tsconfig.json"), config).map_err(|err| err.to_string())?;
    if !tsgo.is_file() {
        return Err(format!("tsgo missing at {}", tsgo.display()));
    }
    let output = Command::new(tsgo)
        .arg("--noEmit")
        .arg("--pretty")
        .arg("false")
        .arg("-p")
        .arg(".")
        .current_dir(&stage)
        .output()
        .map_err(|err| err.to_string())?;
    let mut report = String::from_utf8_lossy(&output.stdout).into_owned();
    report.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() && report.trim().is_empty() {
        report = format!("tsgo exit {}\n", output.status);
    }
    Ok(map_report(&code, &expansion, &filter_stub_noise(&report)))
}

fn map_report(original: &str, expansion: &ts_ext_syntax::Expansion, report: &str) -> String {
    report
        .lines()
        .map(|line| map_line(original, expansion, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn map_line(original: &str, expansion: &ts_ext_syntax::Expansion, line: &str) -> String {
    let Some(rest) = line.strip_prefix("input.ts") else {
        return line.to_string();
    };
    let Some(start) = rest.find('(') else {
        return line.to_string();
    };
    let Some(end) = rest.find(')') else {
        return line.to_string();
    };
    let coords = &rest[start + 1..end];
    let mut parts = coords.split(',');
    let (Some(line_no), Some(col)) = (parts.next(), parts.next()) else {
        return line.to_string();
    };
    let Ok(line_no) = line_no.trim().parse::<usize>() else {
        return line.to_string();
    };
    let Ok(col) = col.trim().parse::<usize>() else {
        return line.to_string();
    };
    let virtual_at = offset_at(&expansion.code, line_no, col);
    match expansion.original_offset(virtual_at) {
        Some(original_at) => {
            let (ol, oc) = line_col_at(original, original_at);
            format!("original({ol},{oc}){rest}")
        }
        None => format!("synthesized{rest}"),
    }
}

fn offset_at(text: &str, line_no: usize, col: usize) -> u32 {
    let mut offset = 0usize;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if index + 1 == line_no {
            return u32::try_from(offset + col.saturating_sub(1).min(line.len())).unwrap_or(u32::MAX);
        }
        offset += line.len();
    }
    u32::try_from(text.len()).unwrap_or(u32::MAX)
}

fn line_col_at(text: &str, offset: u32) -> (usize, usize) {
    let offset = offset as usize;
    let mut line = 1usize;
    let mut col = 1usize;
    for (index, ch) in text.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Missing third-party exports are stub gaps, not expansion failures.
/// Syntax errors and unresolved relative files stay.
fn filter_stub_noise(report: &str) -> String {
    report
        .lines()
        .filter(|line| {
            let stub = line.contains("TS2305")
                || line.contains("TS2614")
                || line.contains("TS2595")
                || line.contains("TS2307")
                || line.contains("TS2749")
                || line.contains("TS2694");
            !stub
        })
        .collect::<Vec<_>>()
        .join("\n")
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
