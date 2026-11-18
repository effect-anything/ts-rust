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

fn expand_in(dir: &std::path::Path, code: &str) -> ts_ext_sweetener::Expansion {
    expand(ExpandRequest {
        filename: dir.join("main.sts").to_str().unwrap(),
        code,
        phase: Phase::Check,
    })
}

const ONE: &str = "export syntax one:expr {\n  rule { one() } => { 1 }\n}\n";
const TWO: &str = "export syntax one:expr {\n  rule { one() } => { 2 }\n}\n";

/// A miss is part of the resolution too: an SWR5002 result is memoized,
/// and creating the module it could not find invalidates it.
#[test]
fn creating_a_missing_module_invalidates_the_memo() {
    let dir = std::env::temp_dir().join(format!("swcache-miss-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let code = "import { one } from \"./later.sts\" for syntax;\nconst a = one();\n";
    let missing = expand_in(&dir, code);
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|d| d.message.starts_with("SWR5002"))
    );
    assert_eq!(missing, expand_in(&dir, code));
    fs::write(dir.join("later.sts"), ONE).unwrap();
    let found = expand_in(&dir, code);
    assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    assert!(found.code.contains("const a = 1;"), "{}", found.code);
    fs::remove_file(dir.join("later.sts")).unwrap();
    assert!(
        expand_in(&dir, code)
            .diagnostics
            .iter()
            .any(|d| d.message.starts_with("SWR5002"))
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// New candidate files that the specifier does not name (`m/index.sts`
/// beside `m.sts`) do not change resolution, so the memo stays valid and
/// still expands `m.sts`.
#[test]
fn unrelated_candidates_keep_the_memo_and_match_a_fresh_expansion() {
    let dir = std::env::temp_dir().join(format!("swcache-cand-{}", std::process::id()));
    fs::create_dir_all(dir.join("m")).unwrap();
    fs::write(dir.join("m.sts"), ONE).unwrap();
    let code = "import { one } from \"./m.sts\" for syntax;\nconst a = one();\n";
    let before = expand_in(&dir, code);
    fs::write(dir.join("m").join("index.sts"), TWO).unwrap();
    let after = expand_in(&dir, code);
    assert_eq!(before, after);
    assert!(after.code.contains("const a = 1;"), "{}", after.code);
    fs::remove_dir_all(&dir).unwrap();
}

/// Retargeting a symlinked module changes what resolution reaches, even
/// when the text is identical: dependency names follow the target.
#[cfg(unix)]
#[test]
fn retargeting_a_symlinked_module_invalidates_the_memo() {
    let dir = std::env::temp_dir().join(format!("swcache-link-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.sts"), ONE).unwrap();
    fs::write(dir.join("b.sts"), ONE).unwrap();
    std::os::unix::fs::symlink(dir.join("a.sts"), dir.join("m.sts")).unwrap();
    let code = "import { one } from \"./m.sts\" for syntax;\nconst a = one();\n";
    let first = expand_in(&dir, code);
    assert!(
        first.dependencies[0].ends_with("a.sts"),
        "{:?}",
        first.dependencies
    );
    fs::remove_file(dir.join("m.sts")).unwrap();
    std::os::unix::fs::symlink(dir.join("b.sts"), dir.join("m.sts")).unwrap();
    let second = expand_in(&dir, code);
    assert!(
        second.dependencies[0].ends_with("b.sts"),
        "{:?}",
        second.dependencies
    );
    fs::remove_dir_all(&dir).unwrap();
}
