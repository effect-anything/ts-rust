use std::fs;

use ts_ext_sweetener::{ExpandRequest, Phase, Severity, expand, structure};

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
    // `function` is a core item form and this definition does not write
    // `shadows core`, so the use stays core (as the upstream CLI prints it).
    assert!(
        prefix_out.code.contains("function plain() { }"),
        "an unauthorized core spelling was expanded:\n{}",
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

/// A core spelling dispatches as core unless the definition writes
/// `shadows core`; a definition that does write it intercepts the form.
#[test]
fn core_spelling_needs_shadows_core() {
    let plain = "\
export syntax module:item {\n\
  rule { module $name:ident { } } => { namespace $name { } }\n\
}\n\
module A { }\n";
    let out = expand(ExpandRequest {
        filename: "core.sts",
        code: plain,
        phase: Phase::Check,
    });
    assert!(out.code.contains("module A"), "{}", out.code);
    assert!(!out.code.contains("namespace A"), "{}", out.code);

    let shadowed = plain.replace("module:item {", "module:item shadows core {");
    let out = expand(ExpandRequest {
        filename: "core.sts",
        code: &shadowed,
        phase: Phase::Check,
    });
    assert!(out.code.contains("namespace A"), "{}", out.code);
}

/// `shadows core` on a spelling that is not a core form is refused.
#[test]
fn shadowing_a_non_core_spelling_is_refused() {
    let code = "\
export syntax hello:expr shadows core {\n\
  rule { hello } => { 1 }\n\
}\n\
const x = hello;\n";
    let out = expand(ExpandRequest {
        filename: "noncore.sts",
        code,
        phase: Phase::Check,
    });
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message.contains("SWR4002")),
        "{:?}",
        out.diagnostics
    );
}

/// `spelling in`, `starts-with-uppercase`, `token-kind`, the length
/// comparisons and `form [not] in`, as the upstream CLI picks the rules.
#[test]
fn refinements_select_rules_like_upstream() {
    let code = "export syntax kind:expr {\n\
  rule { kind $x:token } refine $x spelling in (\"a\", \"b\"); => { \"ab\" }\n\
  rule { kind $x:token } refine $x spelling starts-with-uppercase; => { \"upper\" }\n\
  rule { kind $x:token } refine $x token-kind (numeric-literal); => { \"num\" }\n\
  rule { kind $x:token } => { \"other\" }\n\
}\n\
export syntax count:expr {\n\
  rule { count [$($xs:expr),*] } refine $xs length at-least 3; => { \"many\" }\n\
  rule { count [$($xs:expr),*] } refine $xs length less-than 1; => { \"none\" }\n\
  rule { count [$($xs:expr),*] } => { \"few\" }\n\
}\n\
export syntax shape:expr {\n\
  rule { shape($e:expr) } refine $e form in (arrow); => { \"arrow\" }\n\
  rule { shape($e:expr) } refine $e form not in (conditional, assignment); => { \"plain\" }\n\
  rule { shape($e:expr) } => { \"cond\" }\n\
}\n\
const k = [kind a, kind b, kind Zed, kind 42, kind q];\n\
const c = [count [1, 2, 3], count [], count [1]];\n\
const s = [shape(x => x), shape(a + b), shape(a ? b : c), shape(y = 2)];\n\
";
    let out = expand(ExpandRequest {
        filename: "refine.sts",
        code,
        phase: Phase::Check,
    });
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    for line in [
        r#"const k = ["ab", "ab", "upper", "num", "other"];"#,
        r#"const c = ["many", "none", "few"];"#,
        r#"const s = ["arrow", "plain", "cond", "cond"];"#,
    ] {
        assert!(out.code.contains(line), "missing {line}:\n{}", out.code);
    }
}

fn codes(out: &ts_ext_sweetener::Expansion) -> Vec<String> {
    out.diagnostics
        .iter()
        .map(|d| d.message.split(':').next().unwrap_or("").to_owned())
        .collect()
}

/// SWR2016 at the `$` of a repetition with no quantifier and at the capture
/// of an `#if` that is not `present $capture`; SWR4002 at the definition.
#[test]
fn definition_diagnostics_point_where_upstream_does() {
    let code = "\
export syntax twice:expr {\n\
  rule { twice($($x:expr),*) } => { [$($x),] }\n\
}\n\
export syntax maybe:expr {\n\
  rule { maybe $($x:expr)* } => { #if(absent $x) { 1 } #else { 2 } }\n\
}\n\
export syntax hello:expr shadows core {\n\
  rule { hello } => { 1 }\n\
}\n";
    let out = expand(ExpandRequest {
        filename: "malformed.sts",
        code,
        phase: Phase::Check,
    });
    let find = |needle: &str| {
        out.diagnostics
            .iter()
            .filter(|d| d.message.contains(needle))
            .map(|d| d.start as usize)
            .collect::<Vec<_>>()
    };
    let repetition = code.find("$($x),]").unwrap();
    assert_eq!(find("repetition requires * or +"), vec![repetition]);
    let conditional = code.find("$x) { 1 }").unwrap();
    assert_eq!(
        find("conditional requires present $capture"),
        vec![conditional]
    );
    let definition = code.find("syntax hello").unwrap();
    assert_eq!(find("SWR4002"), vec![definition]);
}

/// A `for syntax` import of a file that does not exist is SWR5002 at the
/// first imported name.
#[test]
fn unresolved_syntax_module_is_swr5002() {
    let code = "import { foo } from \"./missing-module.sts\" for syntax;\nconst a = 1;\n";
    let out = expand(ExpandRequest {
        filename: "/nonexistent-dir/user.sts",
        code,
        phase: Phase::Check,
    });
    assert_eq!(codes(&out), vec!["SWR5002"]);
    assert_eq!(out.diagnostics[0].severity, Severity::Error);
    assert_eq!(out.diagnostics[0].start as usize, code.find("foo").unwrap());
    assert!(
        out.diagnostics[0]
            .message
            .contains("Cannot resolve macro module ./missing-module.sts from"),
        "{}",
        out.diagnostics[0].message
    );
}

/// Legacy `let name = macro { ... }` items are passed through with the
/// SWR4025 warning, as upstream's reader cannot read them.
#[test]
fn unread_legacy_items_warn_swr4025() {
    let code = "\
let function = macro {\n  rule { $x } => { $x }\n}\n\
let go = macro {\n  rule { $x } => { $x }\n}\n\
let ok = 1;\n";
    let out = expand(ExpandRequest {
        filename: "legacy.sts",
        code,
        phase: Phase::Check,
    });
    assert_eq!(codes(&out), vec!["SWR4025", "SWR4025"]);
    assert!(
        out.diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning)
    );
    assert_eq!(Severity::Warning.as_str(), "warning");
    assert!(
        out.diagnostics[0]
            .message
            .contains("expected variable binding")
    );
    assert!(
        out.diagnostics[1]
            .message
            .contains("expected ';' or automatic terminator")
    );
    assert_eq!(
        out.diagnostics[1].start as usize,
        code.find("let go").unwrap()
    );
}

fn expand_ok(filename: &str, code: &str) -> String {
    let out = expand(ExpandRequest {
        filename,
        code,
        phase: Phase::Check,
    });
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    out.code.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `$( ... )?` matches at most once and a spaced `|` separates ordered
/// alternatives; the alternative that fills its group is taken.
#[test]
fn optional_and_choice_patterns() {
    let code = expand_ok(
        "choice.sts",
        "export syntax opt:expr {\n\
  rule { opt($x:expr $(, $y:expr)?) } => { [$x $(, $y)*] }\n\
}\n\
export syntax pick:expr {\n\
  rule { pick(yes) } => { true }\n\
  rule { pick($a:ident | $a:ident !) } => { $a }\n\
}\n\
const a = opt(1);\n\
const b = opt(1, 2);\n\
const c = pick(yes);\n\
const d = pick(foo);\n\
const e = pick(bar !);\n\
",
    );
    for line in [
        "const a = [1 ];",
        "const b = [1 , 2];",
        "const c = true;",
        "const d = foo;",
        "const e = bar;",
    ] {
        assert!(code.contains(line), "missing {line}: {code}");
    }
}

/// `:classElement`, `:typeMember` and destructuring `:binding` holes.
#[test]
fn member_and_binding_holes() {
    let code = expand_ok(
        "holes.sts",
        "export syntax klass:item {\n\
  rule { klass $name:ident { $($member:classElement)* } } => { class $name { $($member)* } }\n\
}\n\
export syntax shape:item {\n\
  rule { shape $name:ident { $($field:typeMember)* } } => { interface $name { $($field)* } }\n\
}\n\
export syntax bind:stmt {\n\
  rule { bind $target:binding = $value:expr; } => { const $target = $value; }\n\
}\n\
export syntax twice:stmt {\n\
  rule { twice $body:item } => { $body $body }\n\
}\n\
klass Point { x = 1; move(dx: number) { this.x += dx; } }\n\
shape Size { width: number; height?: number; }\n\
bind { a, b } = pair;\n\
bind [c, d] = list;\n\
",
    );
    for piece in [
        "class Point { x = 1;move(dx: number) { this.x += dx; } }",
        "interface Size { width: number;height?: number; }",
        "const { a, b } = pair;",
        "const [c, d] = list;",
    ] {
        assert!(code.contains(piece), "missing {piece}: {code}");
    }
}

/// A `fixity postfix` operator takes the expression written before it.
#[test]
fn postfix_operator() {
    let code = expand_ok(
        "postfix.sts",
        "export operator (::):expr {\n\
  fixity postfix;\n\
  precedence 80;\n\
  rule { $value:expr :: } => { assertDefined($value) }\n\
}\n\
const a = load() ::;\n\
const b = 1 + x ::;\n\
",
    );
    assert!(code.contains("const a = assertDefined(load());"), "{code}");
    assert!(code.contains("const b = assertDefined(1 + x);"), "{code}");
}

/// A macro can define a multi-character operator spelling the reader does
/// not know (`@@`, `~~`, `%%`): the file is re-read with those spellings as
/// one token. A prefix operator binds tighter than an infix of lower
/// precedence, and a postfix operator applies at its own precedence.
#[test]
fn macro_defined_operator_spellings_and_precedence() {
    let code = expand_ok(
        "ops.sts",
        "export operator (@@):expr {\n\
  fixity infix;\n\
  associativity left;\n\
  precedence 40;\n\
  rule { $left:expr @@ $right:expr } => { pair($left, $right) }\n\
}\n\
export operator (~~):expr {\n\
  fixity prefix;\n\
  precedence 90;\n\
  rule { ~~ $value:expr } => { twice($value) }\n\
}\n\
export operator (%%):expr {\n\
  fixity postfix;\n\
  precedence 95;\n\
  rule { $value:expr %% } => { bang($value) }\n\
}\n\
const a = ~~ y @@ z;\n\
const b = x @@ y %%;\n\
const d = ~~ y %%;\n\
const g = x %% @@ y;\n\
const h = [x %%];\n\
",
    );
    for piece in [
        "const a = pair(twice(y), z);",
        "const b = pair(x, bang(y));",
        "const d = twice(bang(y));",
        "const g = pair(bang(x), y);",
        "const h = [bang(x)];",
    ] {
        assert!(code.contains(piece), "missing {piece}: {code}");
    }
}

/// `:item` reads a module item, including an import or export declaration;
/// `:stmt` does not, as upstream.
#[test]
fn item_hole_reads_exports_and_stmt_hole_does_not() {
    let source = |kind: &str| {
        format!(
            "export syntax wrap:item {{\n  rule {{ wrap $x:{kind} }} => {{ $x }}\n}}\nwrap export const a = 1;\n"
        )
    };
    let item = expand_ok("item.sts", &source("item"));
    assert!(item.contains("export const a = 1;"), "{item}");
    assert!(!item.contains("wrap"), "{item}");
    let out = expand(ExpandRequest {
        filename: "stmt.sts",
        code: &source("stmt"),
        phase: Phase::Check,
    });
    assert!(
        out.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.starts_with("SWR4001")),
        "{:?}",
        out.diagnostics
    );
    let call = expand_ok(
        "stmt.sts",
        "export syntax wrap:item {\n  rule { wrap $x:stmt } => { $x }\n}\nwrap import(\"y\");\n",
    );
    assert!(call.contains("import(\"y\");"), "{call}");
}

const HACK_PIPE: &str = "export syntax parameter (%):expr;\n\
export operator (|>):expr {\n\
  fixity infix;\n\
  associativity right;\n\
  precedence 20;\n\
  rule { $value:expr |> $body:expr } => {\n\
    // a note the output never prints\n\
    #let(topic = $value) {\n\
      #parameterize(required % = topic) { $body }\n\
    }\n\
  }\n\
}\n";

/// `#let` binds a value once to a fresh name; `#parameterize` gives the
/// syntax parameter `%` that name inside the body. An inner pipe rebinds
/// `%` for its own body only, `% % 5` keeps the remainder, and a body that
/// awaits or yields assigns a binder declared at the top of its function.
#[test]
fn let_and_parameterize() {
    let code = expand_ok(
        "pipe.sts",
        &format!(
            "{HACK_PIPE}const a = xs |> %.map(f) |> %.join(\" \") |> `$ ${{%}}`;\n\
const b = next() |> [%, % * 10];\n\
const c = 17 |> % % 5;\n\
async function load(id) {{\n  return id |> await get(%) |> %.name;\n}}\n"
        ),
    );
    for piece in [
        "const a = ((topic) => ((topic_1) => ((topic_2) => `$ ${topic_2}`)(topic_1.join(\" \")))(topic.map(f)))(xs);",
        "const b = ((topic_3) => [topic_3, topic_3 * 10])(next());",
        "const c = ((topic_4) => topic_4 % 5)(17);",
        "async function load(id) { let topic_5; return (topic_5 = id, ((topic_6) => topic_6.name)(await get(topic_5)));",
    ] {
        assert!(code.contains(piece), "missing {piece}: {code}");
    }
    assert!(!code.contains("note"), "{code}");
}

/// A `required` parameter the body never uses is SWR4022; a parameter used
/// outside every `#parameterize` is SWR4018.
#[test]
fn parameter_misuse_diagnostics() {
    let code = format!("{HACK_PIPE}const a = 1 |> foo + 1;\nconst b = %;\n");
    let out = expand(ExpandRequest {
        filename: "pipe.sts",
        code: &code,
        phase: Phase::Check,
    });
    let codes: Vec<&str> = out
        .diagnostics
        .iter()
        .map(|diagnostic| &diagnostic.message[..7])
        .collect();
    assert_eq!(codes, ["SWR4022", "SWR4018"], "{:?}", out.diagnostics);
    let at = out.diagnostics[1].start as usize;
    assert_eq!(&code[at..at + 1], "%");
}

/// Hygiene asks the output, as upstream does: a binder the user wrote inside
/// input a macro consumed (and did not print again) does not force a rename,
/// one still printed does.
#[test]
fn hygiene_counts_only_names_the_output_spells() {
    let code = "export syntax wrap:item {\n\
  rule { wrap function $name:binding({ id }) { } } => {\n\
    function $name(input: { id: number }) { const { id } = input; return id; }\n\
  }\n\
}\n\
wrap function C({ id }) { }\n\
";
    let consumed = expand_ok("hygiene.sts", code);
    assert!(
        consumed.contains("const { id } = input; return id;"),
        "{consumed}"
    );
    let kept = format!("{code}const id = 1;\n");
    let kept = expand_ok("hygiene.sts", &kept);
    assert!(
        kept.contains("const { id: id_1 } = input; return id_1;"),
        "{kept}"
    );
}

/// Upstream reports `const x = foo 1;` where its reader stopped (`1`), and
/// names that place as a related location. Checked against the CLI.
#[test]
fn unread_item_two_operands_point_at_the_second() {
    let code = "const x = foo 1;\nconst y = 2;\nlet z = a b c\nvar w = 3;\nconst k = foo \"s\";\nconst m = x as any;\n";
    let out = expand(ExpandRequest {
        filename: "t.sts",
        code,
        phase: Phase::Check,
    });
    let lines: Vec<(usize, usize)> = out
        .diagnostics
        .iter()
        .map(|item| {
            let at = item.related[0].start as usize;
            let line = code[..at].matches('\n').count() + 1;
            let column = at - code[..at].rfind('\n').map_or(0, |nl| nl + 1) + 1;
            (line, column)
        })
        .collect();
    assert_eq!(lines, vec![(1, 15), (3, 11), (5, 15)]);
    assert!(
        out.diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning)
    );
    assert_eq!(
        out.diagnostics[0].related[0].message,
        "The reader expected ';' or automatic terminator here."
    );
}

/// A variable that reads but runs into a non-item token without `;` takes
/// everything to the next `;` with it, unreported, as upstream's
/// `incompleteVariable` recovery does. The later `let (-)` is inside it.
#[test]
fn incomplete_variable_swallows_later_unread_items() {
    let code = "var stuff = (f) => (x) => {\n  return f(x);\n}\n\n@stuff\nfunction test(x, y) {\n  return 3 + 3\n}\n\nlet (-) = macro {\n  rule { $x } => { $x }\n}\n";
    let out = expand(ExpandRequest {
        filename: "t.sts",
        code,
        phase: Phase::Check,
    });
    assert_eq!(codes(&out), Vec::<String>::new());
    let alone = "let (-) = macro {\n  rule { $x } => { $x }\n}\n";
    let out = expand(ExpandRequest {
        filename: "t.sts",
        code: alone,
        phase: Phase::Check,
    });
    assert_eq!(codes(&out), vec!["SWR4025"]);
    assert_eq!(out.diagnostics[0].related[0].start, 4);
}

/// A choice backtracks against the rest of the pattern: `x !` is not the
/// first alternative followed by `,`, so the second is taken. Checked
/// against the CLI (`[x, z]`, `[p, q]`).
#[test]
fn choice_backtracks_through_the_rest_of_the_pattern() {
    let code = expand_ok(
        "backtrack.sts",
        "export syntax both:expr {\n\
  rule { both($($a:ident | $a:ident !),*) } => { [$($a),*] }\n\
}\n\
const r = both(x !, z);\n\
const s = both(p, q !);\n",
    );
    assert!(code.contains("const r = [x, z];"), "{code}");
    assert!(code.contains("const s = [p, q];"), "{code}");
}
