//! Operator overload rewrite. Not part of the typescript-go port.
//!
//! This is the `tsover` half. `+`, `-`, `*`, `/`, and `%` become
//! `__tsover_*` calls so a later runtime can dispatch them. It does not
//! know about GPU functions, metadata, or WGSL. TypeGPU and typenet both
//! use this convention. Neither owns it.
//!
//! Check and LSP see the typed calls, so a library can declare
//! `Operator.plus` without this compiler knowing its types. Build output
//! replaces the operators with the calls and does not include [`PRELUDE`].
//!
//! `"use tsover"` and `"use gpu"` both opt in. The GPU wrapper only runs
//! for `"use gpu"`.

use crate::astdata::SyntaxKind;

/// Declarations for the five calls. Check needs them. Build output must
/// not include them.
pub const PRELUDE: &str = "\
declare function __tsover_add(a: number, b: number): number;\n\
declare function __tsover_sub(a: number, b: number): number;\n\
declare function __tsover_mul(a: number, b: number): number;\n\
declare function __tsover_div(a: number, b: number): number;\n\
declare function __tsover_mod(a: number, b: number): number;\n";

/// The runtime call for an operator, and whether the source was an
/// assignment (`+=` stays an assignment of the call).
#[must_use]
pub fn name(kind: SyntaxKind) -> Option<(&'static str, bool)> {
    let (name, assign) = match kind {
        SyntaxKind::PlusToken => ("__tsover_add", false),
        SyntaxKind::MinusToken => ("__tsover_sub", false),
        SyntaxKind::AsteriskToken => ("__tsover_mul", false),
        SyntaxKind::SlashToken => ("__tsover_div", false),
        SyntaxKind::PercentToken => ("__tsover_mod", false),
        SyntaxKind::PlusEqualsToken => ("__tsover_add", true),
        SyntaxKind::MinusEqualsToken => ("__tsover_sub", true),
        SyntaxKind::AsteriskEqualsToken => ("__tsover_mul", true),
        SyntaxKind::SlashEqualsToken => ("__tsover_div", true),
        SyntaxKind::PercentEqualsToken => ("__tsover_mod", true),
        _ => return None,
    };
    Some((name, assign))
}

/// True when this file asked for the operator rewrite.
///
/// typenet writes `"use tsover"`. TypeGPU writes `"use gpu"`. Both mean
/// the same operator calls. A file with neither is unchanged.
#[must_use]
pub fn enabled(code: &str) -> bool {
    code.contains("use tsover") || code.contains("use gpu")
}
