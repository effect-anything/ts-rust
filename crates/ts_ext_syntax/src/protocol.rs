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
    TransformResponse {
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
    use super::transform_file;

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
