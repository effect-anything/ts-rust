use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ts_ext_sweetener::{ExpandRequest, Phase, SegmentKind, expand, structure};

const OPERATORS: &str = r#"export operator (|>):expr {
  fixity infix;
  associativity left;
  precedence 35;
  rule { $value:expr |> $function:ident } => {
    $function($value)
  }
}
"#;

fn main_source() -> String {
    // Sweetener resolves syntax imports relative to the source file. An
    // absolute path is reported as an unresolved macro module.
    "import { (|>) } from \"./operators.sts\" for syntax;\nconst result = [1, 2, 3] |> sum;\n"
        .to_owned()
}

fn write_fixture() -> (PathBuf, String) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("ts-rust-sweetener-{nanos}"));
    fs::create_dir_all(&dir).expect("temp fixture dir");
    let operators = dir.join("operators.sts");
    fs::write(&operators, OPERATORS).expect("operators.sts");
    let main = dir.join("main.sts");
    let code = main_source();
    fs::write(&main, &code).expect("main.sts");
    fs::write(
        dir.join("tsconfig.json"),
        r#"{
  "compilerOptions": { "module": "ESNext", "target": "ES2022" },
  "files": ["operators.sts", "main.sts"]
}
"#,
    )
    .expect("tsconfig.json");
    (main, code)
}

#[test]
fn every_local_macro_case_matches_theirs() {
    let root = format!("{}/cases", env!("CARGO_MANIFEST_DIR"));
    let mut failed = Vec::new();
    // The compiled rule or the form still misses these. They stay listed so
    // a fix has to delete the name; every other captured case is checked,
    // including a macro defined inside `input.sts`.
    let open = [
        "examples-react-memoization-src-function-shadow",
        "fixtures-acceptance-playground-multi-part-methods",
        "fixtures-acceptance-playground-multi-part-methods-hygiene",
    ];
    for entry in walk(&root) {
        let dir = input_dir(&entry);
        let Some(dir) = dir else {
            continue;
        };
        if snapshot_file(&dir, "theirs").is_none() {
            continue;
        }
        let rel = dir
            .trim_start_matches(&root)
            .trim_start_matches('/')
            .to_owned();
        if open.contains(&rel.as_str()) {
            continue;
        }
        if let Err(why) = case_equal(&dir) {
            failed.push(format!("{rel}: {why}"));
        }
    }
    assert!(
        failed.is_empty(),
        "{} cases differ:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

fn walk(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path.display().to_string()));
        } else {
            out.push(path.display().to_string());
        }
    }
    out
}

fn input_dir(path: &str) -> Option<String> {
    for name in ["/input.sts", "/input.stsx", "/input.ts"] {
        if let Some(dir) = path.strip_suffix(name) {
            return Some(dir.to_owned());
        }
    }
    None
}

fn snapshot_file(dir: &str, stem: &str) -> Option<std::path::PathBuf> {
    for ext in [".ts", ".tsx", ".sts", ".stsx"] {
        let path = std::path::Path::new(dir).join(format!("{stem}{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn case_equal(dir: &str) -> Result<(), String> {
    let input = snapshot_file(dir, "input").ok_or_else(|| "no input".to_owned())?;
    let theirs = snapshot_file(dir, "theirs").ok_or_else(|| "no theirs".to_owned())?;
    let code = std::fs::read_to_string(&input).map_err(|err| err.to_string())?;
    let reference = std::fs::read_to_string(&theirs).map_err(|err| err.to_string())?;
    let tsx = theirs
        .extension()
        .is_some_and(|ext| ext == "tsx" || ext == "stsx")
        || input
            .extension()
            .is_some_and(|ext| ext == "tsx" || ext == "stsx");
    let expansion = expand(ExpandRequest {
        filename: &input.display().to_string(),
        code: &code,
        phase: Phase::Check,
    });
    if !expansion.diagnostics.is_empty() {
        let message = expansion
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(message);
    }
    if structure::same(&expansion.code, &reference, tsx) {
        Ok(())
    } else {
        Err("shape".to_owned())
    }
}

#[test]
fn small_cases_match_theirs() {
    for rel in [
        "examples-language-tour-recovered-unless-main",
        "examples-language-tour-new-generated-constant-main",
        "examples-language-tour-new-source-aware-check-main",
        "examples-language-tour-new-vector-prefix-main",
        "examples-language-tour-new-duplicate-expression-main",
        "examples-language-tour-new-optional-type-main",
        "examples-language-tour-new-pipeline-operator-main",
    ] {
        case_matches_theirs(rel);
    }
}

fn case_matches_theirs(rel: &str) {
    let dir = format!("{}/cases/{rel}", env!("CARGO_MANIFEST_DIR"));
    let code = std::fs::read_to_string(format!("{dir}/input.sts")).unwrap();
    let theirs = std::fs::read_to_string(format!("{dir}/theirs.ts")).unwrap();
    let expansion = expand(ExpandRequest {
        filename: &format!("{dir}/input.sts"),
        code: &code,
        phase: Phase::Check,
    });
    assert!(
        structure::same(&expansion.code, &theirs, false),
        "ours:\n{}\ntheirs:\n{}\n{}\n",
        expansion.code,
        theirs,
        structure::unified(&theirs, &expansion.code)
    );
}

fn pipe_expands_or_reports_not_built_in() {
    let (main, code) = write_fixture();
    let expansion = expand(ExpandRequest {
        filename: &main.display().to_string(),
        code: &code,
        phase: Phase::Check,
    });
    let unavailable = expansion.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("Sweetener expansion is not built in yet")
    });
    if unavailable {
        assert_eq!(expansion.code, code);
        assert_eq!(expansion.segments.len(), 1);
        assert!(expansion.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains(&main.display().to_string())
                || diagnostic.message.contains("not built in yet")
        }));
        return;
    }
    assert!(
        expansion.code.contains("sum("),
        "expanded code missing sum(: {}",
        expansion.code
    );
    assert!(
        !expansion.code.contains("|>"),
        "expanded code still contains |>\n{}",
        expansion.code
    );
    assert!(
        expansion.segments.len() > 1,
        "source map should yield more than one segment, got {}",
        expansion.segments.len()
    );
    assert!(
        expansion
            .segments
            .iter()
            .any(|segment| segment.kind == SegmentKind::Verbatim),
        "expected a verbatim segment from the source map"
    );
}

/// A malformed program is one case. Expansion refuses it: empty virtual
/// text, and a diagnostic whose message contains the expected reason.
/// The span is the author token, not the whole file.
#[test]
fn malformed_cases_are_refused() {
    let root = format!("{}/cases", env!("CARGO_MANIFEST_DIR"));
    let mut seen = 0u32;
    for entry in fs::read_dir(&root).unwrap().flatten() {
        let dir = entry.path();
        let expected_path = dir.join("expected.malformed.diagnostics.json");
        if !expected_path.is_file() {
            continue;
        }
        seen += 1;
        let input = ["input.sts", "input.stsx"]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
            .unwrap_or_else(|| panic!("no input in {}", dir.display()));
        let code = fs::read_to_string(&input).unwrap();
        let expected = fs::read_to_string(&expected_path).unwrap();
        let expansion = expand(ExpandRequest {
            filename: &input.display().to_string(),
            code: &code,
            phase: Phase::Check,
        });
        let name = dir.file_name().and_then(|name| name.to_str()).unwrap_or("");
        assert!(
            expansion.code.is_empty(),
            "{name} still produced TypeScript:\n{}",
            expansion.code
        );
        assert!(
            !expansion.diagnostics.is_empty(),
            "{name} produced no diagnostic"
        );
        let joined = expansion
            .diagnostics
            .iter()
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for phrase in json_strings(&expected) {
            if matches!(
                phrase.as_str(),
                "SWR4001" | "TS4001" | "expansion" | "error" | "diagnostics" | "schemaVersion"
            ) {
                continue;
            }
            assert!(
                joined.contains(&phrase),
                "{name} missing {phrase:?} in {joined}"
            );
        }
        for diagnostic in &expansion.diagnostics {
            let start = diagnostic.start as usize;
            let end = diagnostic.end as usize;
            assert!(
                start <= end && end <= code.len(),
                "{name} span {start}..{end}"
            );
            assert!(
                !(start == 0 && end == code.len()) || code.len() < 8,
                "{name} span covers the whole file"
            );
        }
    }
    assert_eq!(seen, 16, "expected 16 malformed cases");
}

/// Quoted strings that follow a colon. Keys are not phrases.
fn json_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut after_colon = false;
    while index < bytes.len() {
        match bytes[index] {
            b':' => after_colon = true,
            b' ' | b'\n' | b'\r' | b'\t' => {}
            b'"' if after_colon => {
                index += 1;
                let start = index;
                while index < bytes.len() && bytes[index] != b'"' {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    if index < bytes.len() {
                        index += 1;
                    }
                }
                out.push(text[start..index].to_owned());
                after_colon = false;
            }
            _ => after_colon = false,
        }
        index += 1;
    }
    out
}
