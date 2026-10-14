//! The diagnostics catalog (`src/catalog.rs`) is complete, its examples
//! are true, and `ext/docs/diagnostics.md` is generated from it.
//! `TSC_EXT_UPDATE=1` rewrites the page.

use std::collections::BTreeSet;
use std::path::Path;

use ts_ext_sweetener::catalog::{CATALOG, Level, lookup, markdown};
use ts_ext_sweetener::{ExpandRequest, Phase, Severity, expand};

fn codes_in(dir: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            codes_in(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_some_and(|name| name != "catalog.rs")
        {
            let text = std::fs::read_to_string(&path).unwrap();
            let bytes = text.as_bytes();
            for (at, _) in text.match_indices("SWR") {
                let digits = &bytes[at + 3..(at + 7).min(bytes.len())];
                if digits.len() == 4 && digits.iter().all(u8::is_ascii_digit) {
                    out.insert(text[at..at + 7].to_owned());
                }
            }
        }
    }
}

#[test]
fn every_emitted_code_is_in_the_catalog_and_no_other() {
    let mut emitted = BTreeSet::new();
    codes_in(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut emitted,
    );
    let listed: BTreeSet<String> = CATALOG.iter().map(|entry| entry.code.to_owned()).collect();
    let missing: Vec<_> = emitted.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&emitted).collect();
    assert!(
        missing.is_empty(),
        "codes in src/ with no catalog entry: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "catalog entries no source emits: {stale:?}"
    );
    let mut sorted: Vec<&str> = CATALOG.iter().map(|entry| entry.code).collect();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        CATALOG.iter().map(|entry| entry.code).collect::<Vec<_>>(),
        "keep CATALOG in numeric order"
    );
    assert_eq!(lookup("4001").map(|entry| entry.code), Some("SWR4001"));
    assert_eq!(lookup("swr1002").map(|entry| entry.code), Some("SWR1002"));
}

#[test]
fn every_bad_example_reports_its_code() {
    let root = std::env::temp_dir().join(format!("swr-catalog-{}", std::process::id()));
    let mut wrong = Vec::new();
    for entry in CATALOG.iter().filter(|entry| !entry.bad.is_empty()) {
        let dir = root.join(entry.code);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in entry.files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let main = dir.join("main.sts");
        std::fs::write(&main, entry.bad).unwrap();
        let out = expand(ExpandRequest {
            filename: main.to_str().unwrap(),
            code: entry.bad,
            phase: Phase::Check,
        });
        let want = match entry.level {
            Level::Error => Severity::Error,
            Level::Warning => Severity::Warning,
        };
        let found = out.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.starts_with(entry.code) && diagnostic.severity == want
        });
        if !found {
            let got: Vec<String> = out
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    format!("{} {}", diagnostic.severity.as_str(), diagnostic.message)
                })
                .collect();
            wrong.push(format!(
                "{} ({}): got {got:?}",
                entry.code,
                entry.level.as_str()
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        wrong.is_empty(),
        "examples that do not report their code:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn diagnostics_page_is_generated_from_the_catalog() {
    let page = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ext/docs/diagnostics.md");
    let want = markdown();
    if std::env::var_os("TSC_EXT_UPDATE").is_some() {
        std::fs::write(&page, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&page).unwrap_or_default();
    assert!(
        have == want,
        "ext/docs/diagnostics.md is stale: run TSC_EXT_UPDATE=1 cargo test -p ts_ext_sweetener --test catalog"
    );
}
