use ts_ext_syntax::{ExpandRequest, Phase, expand};

fn check(code: &str) -> ts_ext_syntax::Expansion {
    expand(ExpandRequest {
        filename: "a.ts",
        code,
        phase: Phase::Check,
    })
}

#[test]
fn use_gpu_is_not_expanded_here() {
    let src = "function f(a, b) { \"use gpu\"; return a + b }";
    let exp = check(src);
    assert_eq!(exp.code, src);
    assert!(exp.diagnostics.is_empty(), "{:?}", exp.diagnostics);
    assert!(!exp.code.contains("__tsover_add"));
}

#[test]
fn use_gpu_assign_is_not_expanded_here() {
    let src = "function f(a, b) { \"use gpu\"; a += b }";
    assert_eq!(check(src).code, src);
}
