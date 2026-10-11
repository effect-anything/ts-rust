//! Plugin pipeline. This is not the `tsc` path.
//!
//! `tsc` rewrites `"use gpu"` itself, after content mapping, and does not
//! call this session. A bundler has no `tsc`, so the plugin does both
//! steps when the file needs them:
//!
//! - Sweetener source (`.sts`, `.stsx`): macro expansion,
//!   then TypeGPU on the expanded text.
//! - Any other file: TypeGPU only, and only when the text has `"use gpu"`.

use crate::{ExpandRequest, Phase, expand, sweetener_matches};

/// What the caller will do with the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformPhase {
    /// Typecheck and LSP. No `__TYPEGPU_META__`.
    #[default]
    Check,
    /// Final JavaScript input. GPU functions are wrapped for the runtime.
    Build,
}

/// Reusable plugin transform. It holds no compiler and no tsconfig.
#[derive(Clone, Debug)]
pub struct TransformSession {
    phase: TransformPhase,
}

impl Default for TransformSession {
    fn default() -> Self {
        Self::check()
    }
}

/// Virtual TypeScript for the bundler, plus the map back to the author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transformed {
    pub code: String,
    /// `.ts` or `.tsx`.
    pub extension: &'static str,
    /// Spanmap tuples:
    /// `[virtualStart, virtualLength, originalStart, originalLength, kind]`.
    pub mappings: String,
    /// Source map from `code` back to the author's file. A later pipeline
    /// composes its own map through this one.
    pub source_map: String,
    pub diagnostics: Vec<SessionDiagnostic>,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDiagnostic {
    pub message: String,
    pub start: u32,
    pub end: u32,
}

impl TransformSession {
    #[must_use]
    pub fn new() -> Self {
        Self::check()
    }

    #[must_use]
    pub fn check() -> Self {
        Self {
            phase: TransformPhase::Check,
        }
    }

    #[must_use]
    pub fn build() -> Self {
        Self {
            phase: TransformPhase::Build,
        }
    }

    /// Plugin entry. See the module docs for which passes run.
    #[must_use]
    pub fn transform(&self, path: &str, code: &str) -> Transformed {
        if sweetener_matches(path) {
            return sweetener_then_typegpu(self, path, code);
        }
        typegpu_only(self, path, code)
    }
}

fn sweetener_then_typegpu(session: &TransformSession, path: &str, code: &str) -> Transformed {
    let expanded = expand(ExpandRequest {
        filename: path,
        code,
        phase: Phase::Build,
    });
    apply_typegpu(
        session.phase,
        path,
        &expanded.code,
        &expanded,
        code != expanded.code,
    )
}

fn typegpu_only(session: &TransformSession, path: &str, code: &str) -> Transformed {
    apply_typegpu(session.phase, path, code, &crate::unchanged(code), false)
}

fn apply_typegpu(
    phase: TransformPhase,
    path: &str,
    code: &str,
    first: &crate::Expansion,
    already_changed: bool,
) -> Transformed {
    let extension = virtual_extension(path);
    if phase == TransformPhase::Build {
        // The metadata embed rewrites operators itself. A rewrite before it
        // would turn `a + b` into a call and then into a second call.
        let embedded = ts_goport::ext::typegpu::embed_metadata(path, code);
        let changed = already_changed || embedded != code;
        let mappings = mappings_json(&first.span_map_segments());
        return Transformed {
            changed,
            extension,
            source_map: source_map_json(path, &mappings),
            mappings,
            diagnostics: diagnostics_of(first),
            code: rewrite_specifiers(&embedded),
        };
    }
    let Some(rewritten) = ts_goport::ext::typegpu::rewrite_text(path, code) else {
        // No operator to rewrite. Build still wraps a marked function.
        let code = if phase == TransformPhase::Build {
            ts_goport::ext::typegpu::embed_metadata(path, code)
        } else {
            code.to_owned()
        };
        let changed = already_changed || !first.diagnostics.is_empty() || code != first.code;
        let mappings = mappings_json(&first.span_map_segments());
        return Transformed {
            changed,
            extension,
            source_map: source_map_json(path, &mappings),
            mappings,
            diagnostics: diagnostics_of(first),
            code: rewrite_specifiers(&code),
        };
    };
    let mappings = mappings_json(&compose(first, &rewritten));
    Transformed {
        changed: true,
        extension,
        source_map: source_map_json(path, &mappings),
        mappings,
        diagnostics: {
            let mut diagnostics = diagnostics_of(first);
            diagnostics.extend(rewritten.diagnostics.into_iter().filter_map(|item| {
                let start = first.original_offset(item.start)?;
                let end = first.original_offset(item.end.saturating_sub(1)).map(|end| end + 1)?;
                Some(SessionDiagnostic {
                    message: item.message,
                    start,
                    end,
                })
            }));
            diagnostics
        },
        code: rewrite_specifiers(&with_prelude(&rewritten.code)),
    }
}

fn finish(phase: TransformPhase, code: &str) -> String {
    if phase == TransformPhase::Build {
        // Operators are already rewritten. The wrapper is runtime JS, so
        // the check prelude is not part of it.
        ts_goport::ext::typegpu::embed_metadata("input.ts", code)
    } else {
        with_prelude(code)
    }
}


const TYPEGPU_PRELUDE: &str = "\ndeclare function __tsover_add(a: number, b: number): number;\ndeclare function __tsover_sub(a: number, b: number): number;\ndeclare function __tsover_mul(a: number, b: number): number;\ndeclare function __tsover_div(a: number, b: number): number;\ndeclare function __tsover_mod(a: number, b: number): number;\n";

fn with_prelude(code: &str) -> String {
    if code.contains("__tsover_") {
        format!("{TYPEGPU_PRELUDE}{code}")
    } else {
        code.to_owned()
    }
}

/// Sweetener imports name the source extension. After expansion the file is
/// TypeScript, so the specifier must name the virtual extension. A later
/// bundler then resolves `logic.ts`, not `logic.sts`.
fn rewrite_specifiers(code: &str) -> String {
    code.replace(".stsx", "").replace(".sts", "")
}

fn diagnostics_of(expansion: &crate::Expansion) -> Vec<SessionDiagnostic> {
    expansion
        .diagnostics
        .iter()
        .map(|item| SessionDiagnostic {
            message: item.message.clone(),
            start: item.start,
            end: item.end,
        })
        .collect()
}

fn compose(
    first: &crate::Expansion,
    second: &ts_goport::ext::typegpu::Rewrite,
) -> Vec<(i32, i32, i32, i32, i32)> {
    second
        .segments
        .iter()
        .filter_map(|segment| {
            let original_start = first.original_offset(segment.original_start as u32)?;
            let last = segment.original_end.saturating_sub(1).max(segment.original_start) as u32;
            let original_end = first.original_offset(last).map(|end| end + 1)?;
            Some((
                segment.virtual_start,
                segment.virtual_end,
                i32::try_from(original_start).unwrap_or(0),
                i32::try_from(original_end).unwrap_or(0),
                if segment.kind == ts_goport::spanmap::Kind::VERBATIM { 0 } else { 1 },
            ))
        })
        .collect()
}

pub fn virtual_extension(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext {
        "tsx" | "stsx" => ".tsx",
        _ => ".ts",
    }
}

/// A source map whose segments are the spanmap tuples. The next tool
/// reads `segments` and composes them; it does not replace them.
pub fn source_map_json(file: &str, mappings: &str) -> String {
    format!(
        r#"{{"version":3,"file":{},"sources":[{}],"segments":{mappings}}}"#,
        json_string(file),
        json_string(file),
    )
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub fn mappings_json(segments: &[(i32, i32, i32, i32, i32)]) -> String {
    let mut out = String::from("[");
    for (index, (v0, v1, o0, o1, kind)) in segments.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "[{v0},{},{o0},{},{kind}]",
            v1.wrapping_sub(*v0),
            o1.wrapping_sub(*o0),
        ));
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::TransformSession;

    #[test]
    fn ordinary_file_runs_typegpu_only() {
        let out = TransformSession::new().transform(
            "a.ts",
            "function f(a, b) { \"use gpu\"; return a + b }",
        );
        assert!(out.changed);
        assert!(out.code.contains("__tsover_add"), "{}", out.code);
        assert!(!out.code.contains("function f") || out.code.contains("return"), "{}", out.code);
    }

    #[test]
    fn check_rewrites_operators_without_meta() {
        let out = TransformSession::check().transform(
            "a.ts",
            "function add(a, b) { \"use gpu\"; return a + b }",
        );
        assert!(out.code.contains("__tsover_add"), "{}", out.code);
        assert!(!out.code.contains("__TYPEGPU_META__"), "{}", out.code);
    }

    #[test]
    fn check_has_operator_rewrite_and_build_has_meta() {
        let code = "function add(a, b) { \"use gpu\"; return a + b }";
        let check = TransformSession::check().transform("a.ts", code);
        assert!(check.code.contains("__tsover_add"), "{}", check.code);
        assert!(!check.code.contains("__TYPEGPU_META__"), "{}", check.code);
        let build = TransformSession::build().transform("a.ts", code);
        assert!(build.code.contains("__tsover_add"), "{}", build.code);
        assert!(build.code.contains("__TYPEGPU_META__"), "{}", build.code);
        assert!(build.code.contains("v: 2"), "{}", build.code);
    }

    fn plain_ts_is_unchanged() {
        let code = "export const n = 1\n";
        let out = TransformSession::new().transform("a.ts", code);
        assert!(!out.changed);
        assert_eq!(out.code, code);
    }

    #[test]
    fn sweetener_file_takes_the_macro_path() {
        let out = TransformSession::new().transform("a.sts", "export const n = 1\n");
        assert_eq!(out.extension, ".ts");
    }
}
