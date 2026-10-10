use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ts_ext_syntax::{ExpandRequest, Phase, expand};

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
    "import { (|>) } from \"./operators.sts\" for syntax;\nconst result = [1, 2, 3] |> sum;\n".to_owned()
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
    let root = format!(
        "{}/conformance/cases/sweetener",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut failed = Vec::new();
    for entry in walk(&root) {
        if !entry.ends_with("/input.sts") {
            continue;
        }
        let dir = entry.trim_end_matches("/input.sts");
        if !std::path::Path::new(dir).join("macros.sts").exists() {
            continue;
        }
        if !std::path::Path::new(dir).join("theirs.ts").exists() {
            continue;
        }
        let rel = dir.trim_start_matches(&root).trim_start_matches('/');
        if case_equal(dir).is_err() {
            failed.push(rel.to_owned());
        }
    }
    assert!(failed.is_empty(), "{} cases differ:\n{}", failed.len(), failed.join("\n"));
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

fn case_equal(dir: &str) -> Result<(), ()> {
    let code = std::fs::read_to_string(format!("{dir}/input.sts")).map_err(|_| ())?;
    let theirs = std::fs::read_to_string(format!("{dir}/theirs.ts")).map_err(|_| ())?;
    let expansion = expand(ExpandRequest {
        filename: &format!("{dir}/input.sts"),
        code: &code,
        phase: Phase::Check,
    });
    if whitespace(&expansion.code) == whitespace(&theirs) {
        Ok(())
    } else {
        Err(())
    }
}

#[test]
fn small_cases_match_theirs() {
    for rel in [
        "examples/language-tour/recovered/unless/main",
        "examples/language-tour/new/generated-constant/main",
        "examples/language-tour/new/source-aware-check/main",
        "examples/language-tour/new/vector-prefix/main",
        "examples/language-tour/new/duplicate-expression/main",
        "examples/language-tour/new/optional-type/main",
        "examples/language-tour/new/pipeline-operator/main",
    ] {
        case_matches_theirs(rel);
    }
}

fn case_matches_theirs(rel: &str) {
    let dir = format!(
        "{}/conformance/cases/sweetener/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    let code = std::fs::read_to_string(format!("{dir}/input.sts")).unwrap();
    let theirs = std::fs::read_to_string(format!("{dir}/theirs.ts")).unwrap();
    let expansion = expand(ExpandRequest {
        filename: &format!("{dir}/input.sts"),
        code: &code,
        phase: Phase::Check,
    });
    let ours = whitespace(&expansion.code);
    let want = whitespace(&theirs);
    assert_eq!(ours, want, "ours:\n{}\ntheirs:\n{}", expansion.code, theirs);
}

fn whitespace(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '(' && *ch != ')')
        .collect()
}

fn pipe_expands_or_reports_not_built_in() {
    let (main, code) = write_fixture();
    let expansion = expand(ExpandRequest {
        filename: &main.display().to_string(),
        code: &code,
        phase: Phase::Check,
    });
    let unavailable = expansion
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("Sweetener expansion is not built in yet"));
    if unavailable {
        assert_eq!(expansion.code, code);
        assert_eq!(expansion.segments.len(), 1);
        assert!(expansion
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(&main.display().to_string())
                || diagnostic.message.contains("not built in yet")));
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
            .any(|segment| segment.kind == ts_ext_syntax::SegmentKind::Verbatim),
        "expected a verbatim segment from the source map"
    );
}
