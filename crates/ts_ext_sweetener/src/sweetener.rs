//! Macro expansion
//!
//! Output is ordinary TypeScript. This pass does not look for `"use gpu"`.
//! A directive in the expanded text is rewritten later, on that text, by
//! the TypeScript path. Expansion owns macro errors (`SWR4001` and the
//! reader codes `SWR1002` and `SWR1003`). A panic inside the expander is
//! `SWR4990` at the invocation being expanded. TypeScript owns TypeScript
//! semantics after this print.

use crate::{ExpandRequest, Expansion};

/// Sweetener source extensions. Matched on the final extension only, so
/// `foo.sts.ts` is ordinary TypeScript.
const EXTENSIONS: &[&str] = &[".sts", ".stsx"];

/// True when the filename's final extension is a Sweetener source extension.
/// A query string (`file.sts?x=1`) is stripped before the check.
#[must_use]
pub fn matches(filename: &str) -> bool {
    let path = filename
        .split_once(['?', '#'])
        .map_or(filename, |(path, _)| path);
    let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    EXTENSIONS.iter().any(|candidate| candidate[1..] == *ext)
}

/// Expand a Sweetener source in Rust. The Node host is the oracle, not
/// this path.
#[must_use]
pub fn expand(request: ExpandRequest<'_>) -> Expansion {
    crate::sweetener_expand::expand(request)
}
