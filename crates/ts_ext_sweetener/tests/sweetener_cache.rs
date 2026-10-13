//! Incremental expansion caching: a memoized expansion equals a fresh one,
//! and editing a macro module the file imports invalidates it.

use std::fs;

use ts_ext_sweetener::{ExpandRequest, Phase, expand, expand_unchecked};

#[test]
fn file_memo_matches_fresh_expansion_and_tracks_module_edits() {
    let dir = std::env::temp_dir().join(format!("swcache-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let module = dir.join("m.sts");
    fs::write(
        &module,
        "export syntax one:expr {\n  rule { one() } => { 1 }\n}\n",
    )
    .unwrap();
    let filename = dir.join("main.sts");
    let code = "import { one } from \"./m.sts\" for syntax;\nconst a = one();\n";
    let run = || {
        expand(ExpandRequest {
            filename: filename.to_str().unwrap(),
            code,
            phase: Phase::Check,
        })
    };
    let first = run();
    assert!(first.code.contains("const a = 1;"), "{}", first.code);
    assert_eq!(first.dependencies.len(), 1);
    let second = run();
    assert_eq!(first, second);

    fs::write(
        &module,
        "export syntax one:expr {\n  rule { one() } => { 100 }\n}\n",
    )
    .unwrap();
    let edited = run();
    assert!(edited.code.contains("const a = 100;"), "{}", edited.code);
    assert_eq!(edited, run());

    fs::remove_file(&module).unwrap();
    let missing = run();
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|d| d.message.starts_with("SWR5002")),
        "{:?}",
        missing.diagnostics
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn memoized_expansion_of_a_plain_file_equals_an_uncached_one() {
    let code = "syntax two:expr {\n  rule { two($x:expr) } => { [$x, $x] }\n}\nconst b = two(`a${two(1)}`);\n";
    let request = || ExpandRequest {
        filename: "fuzz.sts",
        code,
        phase: Phase::Check,
    };
    let cold = expand(request());
    let warm = expand(request());
    assert_eq!(cold, warm);
    assert_eq!(warm, expand_unchecked(code));
}
