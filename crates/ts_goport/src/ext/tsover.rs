//! Operator overload rewrite. Not part of the typescript-go port.
//!
//! This is the `tsover` half, written into this compiler instead of shipping
//! a forked `tsc`. `+`, `-`, `*`, `/`, `%`, and `**` become `__tsover_*`
//! calls. The runtime then dispatches `Symbol.operatorPlus` and the rest.
//! TypeGPU and typenet are consumers. This file does not know about GPU
//! functions, metadata, or WGSL.
//!
//! Upstream's plugin skips `"use gpu"` because the TypeGPU plugin used to
//! own that scope. Here both directives opt into the same calls. The GPU
//! wrapper still runs only for `"use gpu"`.
//!
//! Check and LSP see the calls. Build output replaces the operators and
//! does not include [`PRELUDE`]. A file with neither directive is unchanged.
//! The type-driven skip (leave a primitive `+` alone when neither operand
//! has the operator symbol) belongs in the checker. This pass rewrites the
//! opted-in scope so a library can declare the symbols on its own types.

use crate::astdata::SyntaxKind;

/// Declarations for the six calls. Check needs them when the file does not
/// already import a library that declares them. Build output must not
/// include them.
pub const PRELUDE: &str = "\
declare function __tsover_add(a: number, b: number): number;\n\
declare function __tsover_sub(a: number, b: number): number;\n\
declare function __tsover_mul(a: number, b: number): number;\n\
declare function __tsover_div(a: number, b: number): number;\n\
declare function __tsover_mod(a: number, b: number): number;\n\
declare function __tsover_pow(a: number, b: number): number;\n";

/// The runtime call for an operator, and whether the source was an
/// assignment (`a += b` becomes `a = __tsover_add(a, b)`).
#[must_use]
pub fn name(kind: SyntaxKind) -> Option<(&'static str, bool)> {
    let (name, assign) = match kind {
        SyntaxKind::PlusToken => ("__tsover_add", false),
        SyntaxKind::MinusToken => ("__tsover_sub", false),
        SyntaxKind::AsteriskToken => ("__tsover_mul", false),
        SyntaxKind::SlashToken => ("__tsover_div", false),
        SyntaxKind::PercentToken => ("__tsover_mod", false),
        SyntaxKind::AsteriskAsteriskToken => ("__tsover_pow", false),
        SyntaxKind::PlusEqualsToken => ("__tsover_add", true),
        SyntaxKind::MinusEqualsToken => ("__tsover_sub", true),
        SyntaxKind::AsteriskEqualsToken => ("__tsover_mul", true),
        SyntaxKind::SlashEqualsToken => ("__tsover_div", true),
        SyntaxKind::PercentEqualsToken => ("__tsover_mod", true),
        SyntaxKind::AsteriskAsteriskEqualsToken => ("__tsover_pow", true),
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
