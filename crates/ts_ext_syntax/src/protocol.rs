//! Content-mapper answers, built from the two transforms.
//!
//! The host owns loading, configuration, and parse. This module only turns
//! one `{fileName, content}` into the `transform` result: virtual text,
//! virtual extension, span-map tuples, and original-file diagnostics.

/// One `transform` response. `mappings` is spanmap JSON:
/// `[virtualStart, virtualLength, originalStart, originalLength, kind]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransformResponse {
    pub text: String,
    pub extension: &'static str,
    pub mappings: String,
    pub diagnostics: Vec<ProtocolDiagnostic>,
    /// Macro invocations and the virtual text each produced, for language
    /// features ("expanded from", show expansion).
    pub expansions: Vec<ExpansionRegion>,
}

/// One macro invocation (`original_*`) and the virtual range it produced.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExpansionRegion {
    pub virtual_start: i32,
    pub virtual_length: i32,
    pub original_start: i32,
    pub original_length: i32,
    pub label: String,
}

/// Each traced invocation and the output that came from inside it: every
/// segment whose original range lies within the invocation. Nested
/// invocations are listed too; their region is the part of the output that
/// copies their own text.
#[must_use]
pub fn expansion_regions(expansion: &crate::Expansion) -> Vec<ExpansionRegion> {
    // Segments that produced output, by original start: an invocation's are
    // one contiguous run, found by binary search instead of a full scan.
    let mut produced: Vec<(u32, u32, u32, u32)> = expansion
        .segments
        .iter()
        .filter(|segment| segment.virtual_end > segment.virtual_start)
        .map(|segment| {
            (
                segment.original_start,
                segment.original_end,
                segment.virtual_start,
                segment.virtual_end,
            )
        })
        .collect();
    produced.sort_unstable();
    let mut out: Vec<ExpansionRegion> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for trace in &expansion.traces {
        let mut low = u32::MAX;
        let mut high = 0;
        let first = produced.partition_point(|segment| segment.0 < trace.start);
        for &(_, original_end, virtual_start, virtual_end) in produced[first..]
            .iter()
            .take_while(|segment| segment.0 <= trace.end)
        {
            if original_end <= trace.end {
                low = low.min(virtual_start);
                high = high.max(virtual_end);
            }
        }
        if low >= high {
            continue;
        }
        let mut label = format!("macro `{}`", trace.macro_name);
        if let Some(module) = &trace.module {
            label.push_str(&format!(" from \"{module}\""));
        }
        let region = ExpansionRegion {
            virtual_start: i32::try_from(low).unwrap_or(0),
            virtual_length: i32::try_from(high - low).unwrap_or(0),
            original_start: i32::try_from(trace.start).unwrap_or(0),
            original_length: i32::try_from(trace.end.saturating_sub(trace.start)).unwrap_or(0),
            label,
        };
        if seen.insert(region.clone()) {
            out.push(region);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolDiagnostic {
    pub message_text: String,
    pub start: i32,
    pub length: i32,
    pub code: i32,
    /// TypeScript's `DiagnosticCategory`: 0 warning, 1 error.
    pub category: i32,
    /// `relatedInformation`: message, start, length in the original file.
    pub related: Vec<(String, i32, i32)>,
}

/// The number in an `SWR4001` message. A `TS` prefix is still read so a
/// TypeScript diagnostic keeps its code. Other messages stay `0`.
fn diagnostic_code(message: &str) -> i32 {
    let rest = message
        .strip_prefix("SWR")
        .or_else(|| message.strip_prefix("TS"));
    let Some(rest) = rest else {
        return 0;
    };
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

/// Run the transform the file asks for.
///
/// Content mapper path: Sweetener only. `tsc` runs TypeGPU itself after
/// this, so a mapper must not also rewrite `"use gpu"`.
#[must_use]
pub fn transform_file(file_name: &str, content: &str) -> TransformResponse {
    // Mapper path is Sweetener only. TypeGPU runs inside tsc afterwards.
    let expanded = if crate::sweetener_matches(file_name) {
        crate::expand(crate::ExpandRequest {
            filename: file_name,
            code: content,
            phase: crate::Phase::Check,
        })
    } else {
        crate::unchanged(content)
    };
    let mappings = crate::session::mappings_json(&expanded.span_map_segments());
    let expansions = expansion_regions(&expanded);
    TransformResponse {
        expansions,
        text: expanded.code,
        extension: crate::session::virtual_extension(file_name),
        mappings,
        diagnostics: expanded
            .diagnostics
            .into_iter()
            .map(|item| ProtocolDiagnostic {
                message_text: item.message.clone(),
                start: i32::try_from(item.start).unwrap_or(0),
                length: i32::try_from(item.end.saturating_sub(item.start)).unwrap_or(0),
                code: diagnostic_code(&item.message),
                category: match item.severity {
                    crate::Severity::Warning => 0,
                    crate::Severity::Error => 1,
                },
                related: item
                    .related
                    .iter()
                    .map(|related| {
                        (
                            related.message.clone(),
                            i32::try_from(related.start).unwrap_or(0),
                            i32::try_from(related.end.saturating_sub(related.start)).unwrap_or(0),
                        )
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ExpansionRegion, expansion_regions, transform_file};

    /// The scan `expansion_regions` replaced: every segment per invocation.
    fn regions_by_scan(expansion: &crate::Expansion) -> Vec<ExpansionRegion> {
        let mut out: Vec<ExpansionRegion> = Vec::new();
        for trace in &expansion.traces {
            let (mut low, mut high) = (u32::MAX, 0);
            for segment in &expansion.segments {
                if segment.original_start >= trace.start
                    && segment.original_end <= trace.end
                    && segment.virtual_end > segment.virtual_start
                {
                    low = low.min(segment.virtual_start);
                    high = high.max(segment.virtual_end);
                }
            }
            if low >= high {
                continue;
            }
            let mut label = format!("macro `{}`", trace.macro_name);
            if let Some(module) = &trace.module {
                label.push_str(&format!(" from \"{module}\""));
            }
            let region = ExpansionRegion {
                virtual_start: i32::try_from(low).unwrap(),
                virtual_length: i32::try_from(high - low).unwrap(),
                original_start: i32::try_from(trace.start).unwrap(),
                original_length: i32::try_from(trace.end - trace.start).unwrap(),
                label,
            };
            if !out.contains(&region) {
                out.push(region);
            }
        }
        out
    }

    #[test]
    fn indexed_regions_equal_the_full_scan_on_every_case() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../ts_ext_sweetener/cases");
        let mut compared = 0;
        for dir in std::fs::read_dir(root).unwrap() {
            let dir = dir.unwrap().path();
            for name in ["input.sts", "input.stsx"] {
                let path = dir.join(name);
                let Ok(code) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let expansion = crate::expand(crate::ExpandRequest {
                    filename: &path.display().to_string(),
                    code: &code,
                    phase: crate::Phase::Check,
                });
                assert_eq!(
                    expansion_regions(&expansion),
                    regions_by_scan(&expansion),
                    "{}",
                    path.display()
                );
                compared += usize::from(!expansion.traces.is_empty());
            }
        }
        assert!(compared > 50, "{compared}");
    }

    #[test]
    fn expansions_cover_the_output_of_each_invocation() {
        let code = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\nconst a = twice(value);\n";
        let response = transform_file("a.sts", code);
        assert_eq!(response.expansions.len(), 1, "{:?}", response.expansions);
        let region = &response.expansions[0];
        assert_eq!(region.label, "macro `twice`");
        let start = region.virtual_start as usize;
        let text = &response.text[start..start + region.virtual_length as usize];
        assert_eq!(text, "[value, value]");
        let original = region.original_start as usize;
        assert_eq!(
            &code[original..original + region.original_length as usize],
            "twice(value)"
        );
    }

    #[test]
    fn mapper_does_not_rewrite_typegpu() {
        let code = "function f(a, b) { \"use gpu\"; return a + b }";
        let response = transform_file("a.ts", code);
        assert_eq!(response.text, code);
    }

    #[test]
    fn ordinary_ts_is_unchanged() {
        let code = "function f(a, b) { return a + b }";
        let response = transform_file("a.ts", code);
        assert_eq!(response.text, code);
        assert_eq!(response.extension, ".ts");
    }

    #[test]
    fn unread_item_is_a_warning_with_a_related_location() {
        let code = "const x = foo 1;\n";
        let response = transform_file("a.sts", code);
        assert_eq!(response.diagnostics.len(), 1);
        let item = &response.diagnostics[0];
        assert_eq!(item.category, 0);
        assert_eq!(item.code, 4025);
        assert_eq!(
            item.related,
            vec![(
                "The reader expected ';' or automatic terminator here.".to_owned(),
                14,
                1
            )]
        );
    }
}
