use std::fs;

use ts_ext_sweetener::{ExpandRequest, Phase, expand, structure};

#[test]
fn every_local_macro_case_matches_theirs() {
    let root = format!("{}/cases", env!("CARGO_MANIFEST_DIR"));
    let mut failed = Vec::new();
    // Every captured case is checked, including a macro defined inside
    // `input.sts`.
    let open: [&str; 0] = [];
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
        Err(structure::unified(&reference, &expansion.code))
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

/// A malformed program is one case. The item that failed stays as written
/// and a diagnostic names the expected reason. The span is the author
/// token, not the whole file. Other items in the file still expand.
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
            !expansion.code.is_empty(),
            "{name} produced no TypeScript for the items it could read"
        );
        assert!(
            expansion.segments.iter().any(|segment| {
                segment.kind == ts_ext_sweetener::SegmentKind::Verbatim
                    && segment.virtual_end > segment.virtual_start
            }),
            "{name} kept no verbatim segment for the item it could not read"
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
                "SWR4001" | "expansion" | "error" | "diagnostics" | "schemaVersion"
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

/// The matcher VM compares a literal with the token under the cursor. It does
/// not treat `async` as an optional prefix of `function`, and it does not
/// search ahead for a later copy of the same literal.
#[test]
fn literal_prefix_is_not_optional_and_a_repeated_literal_is_not_the_first() {
    let prefix = "\
export syntax function:item {\n\
  rule { function $name:binding() { } } => {\n\
    const $name = 1;\n\
  }\n\
}\n\
function plain() { }\n\
async function later() { }\n";
    let prefix_out = expand(ExpandRequest {
        filename: "prefix.sts",
        code: prefix,
        phase: Phase::Check,
    });
    assert!(
        prefix_out.diagnostics.is_empty(),
        "{:?}",
        prefix_out.diagnostics
    );
    assert!(
        prefix_out.code.contains("const plain = 1"),
        "plain function was not expanded:\n{}",
        prefix_out.code
    );
    assert!(
        prefix_out.code.contains("async function later"),
        "async function matched a bare function rule:\n{}",
        prefix_out.code
    );

    // Two `function` literals. The call that writes the word once must not
    // succeed by binding the first copy to both pattern positions, and the
    // call that writes it twice must expand.
    let once = "\
export syntax keep:item {\n\
  rule { keep function $name:binding() { function } } => {\n\
    const $name = 1;\n\
  }\n\
}\n\
keep function only() { }\n";
    let once_out = expand(ExpandRequest {
        filename: "once.sts",
        code: once,
        phase: Phase::Check,
    });
    assert!(
        once_out.code.contains("keep function only()"),
        "a refused `keep` was not left as written:\n{}",
        once_out.code
    );
    assert!(
        !once_out.code.contains("const only = 1"),
        "one `function` satisfied a pattern that writes it twice:\n{}",
        once_out.code
    );
    assert!(
        once_out
            .diagnostics
            .iter()
            .any(|item| item.message.contains("macro keep")),
        "missing keep diagnostic: {:?}",
        once_out.diagnostics
    );

    let twice = "\
export syntax keep:item {\n\
  rule { keep function $name:binding() { function } } => {\n\
    const $name = 1;\n\
  }\n\
}\n\
keep function both() { function }\n";
    let twice_out = expand(ExpandRequest {
        filename: "twice.sts",
        code: twice,
        phase: Phase::Check,
    });
    assert!(
        twice_out.diagnostics.is_empty(),
        "{:?}",
        twice_out.diagnostics
    );
    assert!(
        twice_out.code.contains("const both = 1"),
        "a pattern with two `function` literals did not match:\n{}",
        twice_out.code
    );
}

/// `:stmt` ends where the enforester inserts a semicolon: a line break, a
/// `}` that closes the statement list, or the end of the file. `go()` has
/// none of those written as `;`, and it is still the body.
#[test]
fn statement_body_ends_by_automatic_semicolon() {
    let macros = "\
export syntax unless:stmt {\n\
  rule { unless ($condition:expr) { $($body:stmt)* } } => {\n\
    if (!($condition)) { $($body)* }\n\
  }\n\
}\n";
    let cases = [
        ("file end", "unless(ok){go()}\n"),
        ("before brace", "function f() { unless(ok){go()} }\n"),
        ("line break", "unless(ok){\ngo()\n}\n"),
    ];
    for (label, code) in cases {
        let expansion = expand(ExpandRequest {
            filename: "main.sts",
            code: &format!("{macros}{code}"),
            phase: Phase::Check,
        });
        assert!(
            expansion.diagnostics.is_empty(),
            "{label}: {:?}",
            expansion.diagnostics
        );
        assert!(
            expansion.code.contains("if (!(ok))"),
            "{label} did not expand the statement body:\n{}",
            expansion.code
        );
        assert!(
            expansion.code.contains("go()"),
            "{label} dropped the statement body:\n{}",
            expansion.code
        );
    }
}

/// One macro no rule accepts is left as written. A later macro in the same
/// file still expands, and the kept text is a verbatim segment.
#[test]
fn one_failed_macro_does_not_drop_the_rest_of_the_file() {
    let code = "\
export syntax unless:stmt {\n\
  rule { unless ($condition:expr) { $($body:stmt)* } } => {\n\
    if (!($condition)) { $($body)* }\n\
  }\n\
}\n\
export syntax twice:expr {\n\
  rule { twice($value:expr) } => { [$value, $value] }\n\
}\n\
unless (not a call;\n\
const n = twice(1);\n";
    let expansion = expand(ExpandRequest {
        filename: "mixed.sts",
        code,
        phase: Phase::Check,
    });
    assert!(
        expansion
            .diagnostics
            .iter()
            .any(|item| item.message.contains("macro unless")),
        "missing unless diagnostic: {:?}",
        expansion.diagnostics
    );
    assert!(
        expansion.code.contains("[1, 1]"),
        "twice was not expanded:\n{}",
        expansion.code
    );
    assert!(
        expansion.code.contains("unless"),
        "the failed unless was erased:\n{}",
        expansion.code
    );
    assert!(
        expansion
            .segments
            .iter()
            .any(|segment| segment.kind == ts_ext_sweetener::SegmentKind::Verbatim),
        "no verbatim segment for the kept item"
    );
}

/// An unmatched delimiter is a reader diagnostic. The file is still printed.
#[test]
fn reader_error_still_prints_the_file() {
    let code = "function f( {\nconst n = 1;\n";
    let expansion = expand(ExpandRequest {
        filename: "reader.sts",
        code,
        phase: Phase::Check,
    });
    assert!(
        expansion
            .diagnostics
            .iter()
            .any(|item| item.message.contains("SWR1003")),
        "missing reader diagnostic: {:?}",
        expansion.diagnostics
    );
    assert!(
        !expansion.code.is_empty(),
        "reader error discarded the file"
    );
    assert!(
        expansion.code.contains("const n = 1"),
        "text after the unmatched delimiter was dropped:\n{}",
        expansion.code
    );
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
