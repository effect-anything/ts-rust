//! Plugin pipeline. This is not the `tsc` path.
//!
//! `tsc` rewrites `"use gpu"` itself, after content mapping, and does not
//! call this session. A bundler has no `tsc`, so the plugin does both
//! steps when the file needs them:
//!
//! - Sweetener source: macro expansion,
//!   then [`ts_ext_typegpu::TypegpuSession`] on the expanded text.
//! - Any other file: that session only. Check leaves operators alone.
//!   Build rewrites them to `__tsover_*` and then wraps marked functions.

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
    /// Upstream's `autoNamingEnabled`. The plugin defaults it on; the plugin's
    /// test helper defaults it off, so a caller that compares against those
    /// snapshots sets it per file.
    auto_naming: bool,
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
    pub severity: crate::Severity,
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
            auto_naming: true,
        }
    }

    #[must_use]
    pub fn build() -> Self {
        Self {
            phase: TransformPhase::Build,
            auto_naming: true,
        }
    }

    /// [`Self::build`] with upstream's `autoNamingEnabled` set.
    #[must_use]
    pub fn build_with_auto_naming(auto_naming: bool) -> Self {
        Self {
            phase: TransformPhase::Build,
            auto_naming,
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
        session,
        session.phase,
        path,
        &expanded.code,
        &expanded,
        code != expanded.code,
    )
}

fn typegpu_only(session: &TransformSession, path: &str, code: &str) -> Transformed {
    apply_typegpu(
        session,
        session.phase,
        path,
        code,
        &crate::unchanged(code),
        false,
    )
}

fn apply_typegpu(
    session: &TransformSession,
    phase: TransformPhase,
    path: &str,
    code: &str,
    first: &crate::Expansion,
    already_changed: bool,
) -> Transformed {
    let extension = virtual_extension(path);
    let gpu = match phase {
        TransformPhase::Build => {
            ts_ext_typegpu::TypegpuSession::build_with_auto_naming(session.auto_naming)
        }
        TransformPhase::Check => ts_ext_typegpu::TypegpuSession::check(),
    }
    .transform(path, code);
    if phase == TransformPhase::Build {
        let mappings = mappings_json(&first.span_map_segments());
        let mut diagnostics = diagnostics_of(first);
        // Refusal offsets are bytes of this text. The payload has no
        // positions, so they are not mapped back through Sweetener.
        diagnostics.extend(gpu.diagnostics.into_iter().map(|item| SessionDiagnostic {
            message: item.message,
            start: item.start,
            end: item.end,
            severity: crate::Severity::Error,
        }));
        return Transformed {
            changed: already_changed || gpu.changed,
            extension,
            source_map: source_map_json(path, &mappings),
            mappings,
            diagnostics,
            code: rewrite_specifiers(&gpu.code),
        };
    }
    let Some(rewritten) = gpu.rewrite else {
        let changed = already_changed || !first.diagnostics.is_empty() || gpu.code != code;
        let mappings = mappings_json(&first.span_map_segments());
        return Transformed {
            changed,
            extension,
            source_map: source_map_json(path, &mappings),
            mappings,
            diagnostics: diagnostics_of(first),
            code: rewrite_specifiers(&gpu.code),
        };
    };
    let mut segments = compose(first, &rewritten);
    // Check output prepends the operator declarations. The rewrite's
    // virtual offsets do not include them, and the declarations are not
    // author text.
    let shift = gpu
        .code
        .strip_suffix(&rewritten.code)
        .map(|prefix| i32::try_from(prefix.len()).unwrap_or(0))
        .unwrap_or(0);
    for segment in &mut segments {
        segment.0 = segment.0.wrapping_add(shift);
        segment.1 = segment.1.wrapping_add(shift);
    }
    let mappings = mappings_json(&segments);
    Transformed {
        changed: true,
        extension,
        source_map: source_map_json(path, &mappings),
        mappings,
        diagnostics: {
            let mut diagnostics = diagnostics_of(first);
            diagnostics.extend(rewritten.diagnostics.into_iter().filter_map(|item| {
                let mapped = map_through(first, item.start, item.end)?;
                Some(SessionDiagnostic {
                    message: item.message,
                    start: mapped.start,
                    end: mapped.end,
                    severity: crate::Severity::Error,
                })
            }));
            diagnostics
        },
        code: rewrite_specifiers(&gpu.code),
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
            severity: item.severity,
        })
        .collect()
}

fn compose(
    first: &crate::Expansion,
    second: &ts_goport::ext::tsover::Rewrite,
) -> Vec<(i32, i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for segment in &second.segments {
        if segment.kind == ts_goport::spanmap::Kind::VERBATIM {
            split_verbatim(first, segment, &mut out);
        } else if let Some(mapped) = map_through(
            first,
            segment.original_start as u32,
            segment.original_end as u32,
        ) {
            out.push((
                segment.virtual_start,
                segment.virtual_end,
                i32::try_from(mapped.start).unwrap_or(0),
                i32::try_from(mapped.end).unwrap_or(0),
                1,
            ));
        }
    }
    out
}

fn split_verbatim(
    first: &crate::Expansion,
    segment: &ts_goport::spanmap::Segment,
    out: &mut Vec<(i32, i32, i32, i32, i32)>,
) {
    let origin = segment.original_start as u32;
    let end = segment.original_end as u32;
    let mut cursor = origin;
    for prev in &first.segments {
        if prev.virtual_end <= cursor {
            continue;
        }
        if prev.virtual_start >= end {
            break;
        }
        if prev.virtual_start > cursor {
            cursor = prev.virtual_start;
        }
        let from = cursor.max(prev.virtual_start);
        let to = end.min(prev.virtual_end);
        if to <= from {
            continue;
        }
        let shift = i32::try_from(from - origin).unwrap_or(0);
        let len = i32::try_from(to - from).unwrap_or(0);
        let (original_start, original_end, kind) = match prev.kind {
            crate::SegmentKind::Verbatim => {
                let start = prev.original_start + (from - prev.virtual_start);
                (start, start + (to - from), 0)
            }
            crate::SegmentKind::Atom => (prev.original_start, prev.original_end, 1),
        };
        out.push((
            segment.virtual_start.wrapping_add(shift),
            segment.virtual_start.wrapping_add(shift).wrapping_add(len),
            i32::try_from(original_start).unwrap_or(0),
            i32::try_from(original_end).unwrap_or(0),
            kind,
        ));
        cursor = to;
        if cursor >= end {
            break;
        }
    }
}

struct MappedRange {
    start: u32,
    end: u32,
}

/// Map a span of the first expansion's virtual text back to the author.
/// A macro atom contributes its whole author span. A gap contributes nothing.
fn map_through(first: &crate::Expansion, start: u32, end: u32) -> Option<MappedRange> {
    if end < start {
        return None;
    }
    if start == end {
        let segment = segment_at(first, start)?;
        return Some(match segment.kind {
            crate::SegmentKind::Verbatim => {
                let at = segment
                    .original_start
                    .saturating_add(start - segment.virtual_start);
                MappedRange { start: at, end: at }
            }
            crate::SegmentKind::Atom => MappedRange {
                start: segment.original_start,
                end: segment.original_end,
            },
        });
    }
    let start_at = segment_at(first, start)?;
    let end_at = segment_at(first, end - 1)?;
    if std::ptr::eq(start_at, end_at) {
        return Some(match start_at.kind {
            crate::SegmentKind::Verbatim => MappedRange {
                start: start_at
                    .original_start
                    .saturating_add(start - start_at.virtual_start),
                end: start_at
                    .original_start
                    .saturating_add(end - start_at.virtual_start),
            },
            crate::SegmentKind::Atom => MappedRange {
                start: start_at.original_start,
                end: start_at.original_end,
            },
        });
    }
    let low = match start_at.kind {
        crate::SegmentKind::Verbatim => start_at
            .original_start
            .saturating_add(start - start_at.virtual_start),
        crate::SegmentKind::Atom => start_at.original_start,
    };
    let high = match end_at.kind {
        crate::SegmentKind::Verbatim => end_at
            .original_start
            .saturating_add(end - end_at.virtual_start),
        crate::SegmentKind::Atom => end_at.original_end,
    };
    Some(MappedRange {
        start: low,
        end: high.max(low),
    })
}

fn segment_at(first: &crate::Expansion, pos: u32) -> Option<&crate::Segment> {
    first.segments.iter().find(|segment| {
        pos >= segment.virtual_start && pos < segment.virtual_end
            || (pos == segment.virtual_end
                && first
                    .segments
                    .last()
                    .is_some_and(|last| pos == last.virtual_end))
    })
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
    use super::{TransformSession, compose};

    #[test]
    fn ordinary_file_runs_typegpu_only() {
        let code = "function f(a, b) { \"use gpu\"; return a + b }";
        let out = TransformSession::new().transform("a.ts", code);
        assert!(!out.changed);
        assert_eq!(out.code, code);
        assert!(!out.code.contains("__tsover_add"), "{}", out.code);
    }

    #[test]
    fn check_keeps_the_operator_and_build_emits_the_call() {
        let code = "function add(a, b) { \"use gpu\"; return a + b }";
        let check = TransformSession::check().transform("a.ts", code);
        assert_eq!(check.code, code);
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

    #[test]
    fn check_keeps_the_author_operator_in_place() {
        let code = "function step(n: number) {\n  \"use gpu\";\n  return n + 1;\n}\n";
        let out = TransformSession::check().transform("a.ts", code);
        assert_eq!(out.code, code);
        let plus = code.find('+').expect("plus") as i32;
        let covering = tuples(&out.mappings)
            .into_iter()
            .find(|item| item[0] <= plus && plus < item[0] + item[1])
            .expect("segment");
        assert_eq!(covering[4], 0, "check does not rewrite the operator");
        assert_eq!(covering[2] + (plus - covering[0]), plus);
        let name = out.code.find("function step").expect("name") as i32;
        let name_seg = tuples(&out.mappings)
            .into_iter()
            .find(|item| item[0] <= name && name < item[0] + item[1])
            .expect("name segment");
        assert_eq!(name_seg[4], 0);
        let from = name_seg[2] as usize;
        let to = from + name_seg[3] as usize;
        let virtual_from = name_seg[0] as usize;
        let virtual_to = virtual_from + name_seg[1] as usize;
        assert_eq!(&code[from..to], &out.code[virtual_from..virtual_to]);
    }

    #[test]
    fn macro_atom_composes_onto_the_author_span() {
        use crate::{Expansion, Segment, SegmentKind};
        let head = "function step(n: number) {\n  \"use gpu\";\n  ";
        let author_macro = "unless (n < 0) { return n + 1 }";
        let virtual_macro = "if (!(n < 0)) { return n + 1 }";
        let tail = "\n  return kept;\n}\n";
        let author = format!("{head}{author_macro}{tail}");
        let virtual_text = format!("{head}{virtual_macro}{tail}");
        let head_len = head.len() as u32;
        let virtual_end = head_len + virtual_macro.len() as u32;
        let author_end = head_len + author_macro.len() as u32;
        let expansion = Expansion {
            code: virtual_text.clone(),
            segments: vec![
                Segment {
                    virtual_start: 0,
                    virtual_end: head_len,
                    original_start: 0,
                    original_end: head_len,
                    kind: SegmentKind::Verbatim,
                },
                Segment {
                    virtual_start: head_len,
                    virtual_end: virtual_end,
                    original_start: head_len,
                    original_end: author_end,
                    kind: SegmentKind::Atom,
                },
                Segment {
                    virtual_start: virtual_end,
                    virtual_end: virtual_text.len() as u32,
                    original_start: author_end,
                    original_end: author.len() as u32,
                    kind: SegmentKind::Verbatim,
                },
            ],
            diagnostics: Vec::new(),
            dependencies: Vec::new(),
        };
        let rewritten = ts_goport::ext::tsover::rewrite_text("a.ts", &virtual_text).unwrap();
        let segments = compose(&expansion, &rewritten);
        let call = rewritten.code.find("__tsover_add").unwrap() as i32;
        let covering = segments
            .iter()
            .find(|item| item.0 <= call && call < item.1)
            .unwrap();
        assert_eq!(covering.4, 1);
        assert_eq!(covering.2, head_len as i32);
        assert_eq!(covering.3, author_end as i32);
        assert_eq!(
            &author[covering.2 as usize..covering.3 as usize],
            author_macro
        );
        let kept = rewritten.code.find("return kept").unwrap() as i32;
        let kept_seg = segments
            .iter()
            .find(|item| item.0 <= kept && kept < item.1)
            .unwrap();
        assert_eq!(kept_seg.4, 0);
        assert_eq!(
            &author[kept_seg.2 as usize..kept_seg.3 as usize],
            &rewritten.code[kept_seg.0 as usize..kept_seg.1 as usize]
        );
    }

    fn tuples(mappings: &str) -> Vec<[i32; 5]> {
        let mut out = Vec::new();
        let mut number = String::new();
        let mut values = Vec::new();
        for ch in mappings.chars() {
            if ch.is_ascii_digit() || ch == '-' {
                number.push(ch);
                continue;
            }
            if !number.is_empty() {
                values.push(number.parse::<i32>().unwrap_or(0));
                number.clear();
            }
            if ch == ']' && values.len() == 5 {
                out.push([values[0], values[1], values[2], values[3], values[4]]);
                values.clear();
            }
        }
        out
    }
}
