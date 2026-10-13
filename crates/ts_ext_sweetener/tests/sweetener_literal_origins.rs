//! A template word that spells one of its rule's pattern literals maps to
//! the input token that literal matched, found through the matcher (not by
//! searching the source for the spelling).

use ts_ext_sweetener::{ExpandRequest, Expansion, Phase, expand};

fn run(code: &str) -> Expansion {
    let out = expand(ExpandRequest {
        filename: "literals.sts",
        code,
        phase: Phase::Check,
    });
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    out
}

/// Where the `nth` printed `needle` came from, if it has an exact origin.
fn origin_of(out: &Expansion, needle: &str, nth: usize) -> Option<u32> {
    let at = out.code.match_indices(needle).nth(nth)?.0 as u32;
    out.origins
        .iter()
        .find(|o| o.virtual_start == at && !o.introduced)
        .map(|o| o.original_start)
}

fn nth_in(code: &str, needle: &str, nth: usize) -> u32 {
    code.match_indices(needle).nth(nth).unwrap().0 as u32
}

#[test]
fn rule_literals_class_literals_and_separators_map_to_the_matched_token() {
    let defs = "syntax class Filter {\n  fields { condition: expr; }\n  rule { where $condition:expr }\n}\nsyntax query:expr {\n  rule { query($db:expr) { select $shape:tt from $table:ident $($filter:Filter)? } } => {\n    $db.select($shape).from($table)$(.where($filter.condition))*\n  }\n}\nsyntax all:expr {\n  rule { all($($x:ident) and +) } => { and($($x),*) }\n}\n";
    let body = "const q = query(db) { select { id: 1 } from users where ok };\nconst r = all(a and b and c);\n";
    let code = format!("{defs}{body}");
    let out = run(&code);
    assert!(
        out.code
            .contains("db.select({ id: 1 }).from(users).where(ok)"),
        "{}",
        out.code
    );
    let body_at = defs.len();
    let user = |needle: &str| body_at as u32 + nth_in(body, needle, 0);
    assert_eq!(origin_of(&out, "select", 0), Some(user("select")));
    assert_eq!(origin_of(&out, "from", 0), Some(user("from")));
    // `where` is the `Filter` class's literal, inside a repetition row.
    assert_eq!(origin_of(&out, "where", 0), Some(user("where")));
    // `and(a, b, c)` stands for every separator; it maps to the first.
    assert_eq!(origin_of(&out, "and(", 0), Some(user("and")));
}

/// Two rows each matched the class literal: a template word outside the
/// rows cannot tell which one it means, so it maps to neither.
#[test]
fn a_literal_matched_by_two_rows_is_ambiguous_outside_them() {
    let defs = "syntax class Step {\n  fields { value: ident; }\n  rule { then $value:ident }\n}\nsyntax chain:expr {\n  rule { chain($($steps:Step)*) } => { [then, $($steps.value),*] }\n}\n";
    let body = "const c = chain(then a then b);\n";
    let code = format!("{defs}{body}");
    let out = run(&code);
    assert!(out.code.contains("[then, a, b]"), "{}", out.code);
    assert_eq!(origin_of(&out, "then", 0), None);
}
