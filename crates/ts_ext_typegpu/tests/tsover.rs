//! The operator-overload layer on its own.
//!
//! `tsover` rewrites `+ - * / %` (and their assignments) into `__tsover_*`
//! calls, adds `**` for its own `"use tsover"` directive, and leaves the rest
//! of the file alone. `typegpu` is a consumer: it wraps marked functions and
//! records the tinyest AST of what this layer produced. Tests for that layer
//! live in `tests/typegpu.rs`; these ask only about the rewrite.

use ts_goport::astdata::SyntaxKind;
use ts_goport::ext::tsover::{self, Rewrite};
use ts_goport::spanmap::Kind;

fn rewrite(code: &str) -> Rewrite {
    tsover::rewrite_text("a.ts", code).expect("the file asks for the rewrite")
}

#[test]
fn the_table_matches_the_consumers() {
    // The pinned TypeGPU plugin's table.
    for (kind, name) in [
        (SyntaxKind::PlusToken, "__tsover_add"),
        (SyntaxKind::MinusToken, "__tsover_sub"),
        (SyntaxKind::AsteriskToken, "__tsover_mul"),
        (SyntaxKind::SlashToken, "__tsover_div"),
        (SyntaxKind::PercentToken, "__tsover_mod"),
    ] {
        assert_eq!(tsover::name(kind), Some((name, false)), "{kind:?}");
        assert_eq!(tsover::name_with_pow(kind), Some((name, false)), "{kind:?}");
    }
    for (kind, name) in [
        (SyntaxKind::PlusEqualsToken, "__tsover_add"),
        (SyntaxKind::MinusEqualsToken, "__tsover_sub"),
        (SyntaxKind::AsteriskEqualsToken, "__tsover_mul"),
        (SyntaxKind::SlashEqualsToken, "__tsover_div"),
        (SyntaxKind::PercentEqualsToken, "__tsover_mod"),
    ] {
        assert_eq!(tsover::name(kind), Some((name, true)), "{kind:?}");
    }
    // `**` is `"use tsover"` only: the plugin's table has no power.
    assert_eq!(tsover::name(SyntaxKind::AsteriskAsteriskToken), None);
    assert_eq!(
        tsover::name_with_pow(SyntaxKind::AsteriskAsteriskToken),
        Some(("__tsover_pow", false))
    );
    assert_eq!(
        tsover::name_with_pow(SyntaxKind::AsteriskAsteriskEqualsToken),
        Some(("__tsover_pow", true))
    );
    assert_eq!(tsover::name(SyntaxKind::LessThanToken), None);
}

#[test]
fn either_directive_enables_the_pass() {
    assert!(tsover::enabled("\"use tsover\";\n"));
    assert!(tsover::enabled("function f() {\n  \"use gpu\";\n}\n"));
    assert!(!tsover::enabled("const a = 1 + 2;\n"));
}

#[test]
fn the_prelude_declares_every_call() {
    for name in [
        "__tsover_add",
        "__tsover_sub",
        "__tsover_mul",
        "__tsover_div",
        "__tsover_mod",
        "__tsover_pow",
    ] {
        assert!(
            tsover::PRELUDE.contains(&format!("declare function {name}(")),
            "{name} missing from the prelude"
        );
    }
}

#[test]
fn a_use_tsover_file_rewrites_every_operator() {
    let out = rewrite("\"use tsover\";\nexport const t = (1 + 2) * 3 % 4 - 5 / 6;\n");
    assert!(out.code.contains("__tsover_add(1, 2)"), "{}", out.code);
    assert!(out.code.contains("__tsover_mul"), "{}", out.code);
    assert!(out.code.contains("__tsover_mod"), "{}", out.code);
    assert!(out.code.contains("__tsover_sub"), "{}", out.code);
    assert!(out.code.contains("__tsover_div"), "{}", out.code);
    // The host text is what check sees: the calls, and no declarations.
    assert!(!out.code.contains("declare function"), "{}", out.code);
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
}

#[test]
fn a_use_tsover_file_rewrites_power_both_ways() {
    let out = rewrite(
        "\"use tsover\";\nexport function f(a, b) {\n  const x = a ** b;\n  a **= b;\n  return x;\n}\n",
    );
    assert!(out.code.contains("__tsover_pow(a, b)"), "{}", out.code);
    assert!(out.code.contains("a = __tsover_pow(a, b)"), "{}", out.code);
}

#[test]
fn a_use_gpu_scope_keeps_power() {
    let code = "export function f(a, b) {\n  \"use gpu\";\n  return a ** b + a / b;\n}\n";
    let out = rewrite(code);
    assert!(out.code.contains("a ** b"), "{}", out.code);
    assert!(!out.code.contains("__tsover_pow"), "{}", out.code);
    assert!(out.code.contains("__tsover_div"), "{}", out.code);
}

#[test]
fn only_the_opted_in_scope_is_rewritten() {
    let code = concat!(
        "export function gpu(a, b) {\n  \"use gpu\";\n  return a + b;\n}\n",
        "export function plain(c, d) {\n  return c + d;\n}\n",
    );
    let out = rewrite(code);
    assert!(out.code.contains("__tsover_add(a, b)"), "{}", out.code);
    assert!(out.code.contains("return c + d;"), "{}", out.code);
    assert!(!out.code.contains("__tsover_add(c, d)"), "{}", out.code);
}

#[test]
fn a_file_without_a_directive_is_not_rewritten() {
    assert!(tsover::rewrite_text("a.ts", "export const t = 1 + 2;\n").is_none());
}

#[test]
fn the_span_map_is_an_atom_on_the_operator() {
    let input = "\"use tsover\";\nexport const t = 1 + 2;\n";
    let operator = input.find('+').expect("the operator") as i32;
    let out = rewrite(input);
    let call = out.code.find("__tsover_add").expect("the call") as i32;
    let atom = out
        .segments
        .iter()
        .find(|segment| {
            segment.kind == Kind::ATOM
                && segment.virtual_start <= call
                && call < segment.virtual_end
        })
        .expect("an atom covering the call");
    // The whole rewritten call maps back to the operator alone, which is what
    // a diagnostic on it uses.
    assert_eq!(
        (atom.original_start, atom.original_end),
        (operator, operator + 1),
        "atom {atom:?} for the operator at {operator}"
    );
}

#[test]
fn a_nested_marked_function_reports_nesting() {
    let code = concat!(
        "export function outer(a, b) {\n  \"use gpu\";\n",
        "  function inner() {\n    \"use gpu\";\n    return 1;\n  }\n  return inner();\n}\n",
    );
    let out = rewrite(code);
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message == "Nesting 'use gpu' functions is not allowed"),
        "{:?}",
        out.diagnostics
    );
}
