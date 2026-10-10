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
                message_text: item.message,
                start: i32::try_from(item.start).unwrap_or(0),
                length: i32::try_from(item.end.saturating_sub(item.start)).unwrap_or(0),
                code: 0,
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
}
