//! PORT: not in Go (lives in `ts_goport`, beside `ls`). Language features for macro expansions: "Expanded from"
//! related information on diagnostics, the expansion in hover, and the
//! `custom/textDocument/showExpansion` request. Upstream files keep one
//! call each.

use crate::ls::prelude::*;
use crate::ls::{LanguageService, lsconv};
use crate::lsp::lsproto;
use crate::spanmap::{Feature, SpanMap};

/// One macro expansion, for hover and `custom/textDocument/showExpansion`.
#[derive(Clone, Debug)]
pub struct ExpansionView {
    pub label: String,
    pub text: String,
    pub range: lsproto::Range,
}

/// Whether `loc` lies inside the output of a macro invocation (where the
/// invocation, not the top of the file, is the location).
#[must_use]
pub fn inside(span_map: &SpanMap, loc: TextRange) -> bool {
    !SpanMap::expansions_containing_virtual(Some(span_map), loc).is_empty()
}

/// The original range of the innermost invocation whose output contains `loc`.
#[must_use]
pub fn invocation(span_map: &SpanMap, loc: TextRange) -> Option<TextRange> {
    SpanMap::expansions_containing_virtual(Some(span_map), loc)
        .first()
        .map(|expansion| TextRange::new(expansion.original_start, expansion.original_end))
}

/// A compiler diagnostic inside macro output gets one related location per
/// enclosing invocation, innermost first: "Expanded from macro `name`" at
/// the code the author wrote.
#[must_use]
pub fn with_expansion_context(diag: &Diagnostic) -> Diagnostic {
    let file = diag.file();
    let Some(span_map) = source_file_span_map(file) else {
        return diag.clone();
    };
    if !diag.source().is_empty() {
        return diag.clone();
    }
    let expansions = SpanMap::expansions_containing_virtual(Some(span_map), diag.loc());
    if expansions.is_empty() {
        return diag.clone();
    }
    let mut out = diag.clone();
    let source = source_file_content_mapper(file).to_string();
    for expansion in expansions.iter().take(8) {
        out.add_related_info(Some(new_external_diagnostic(
            file,
            TextRange::new(expansion.original_start, expansion.original_end),
            if source.is_empty() {
                "content mapper"
            } else {
                &source
            },
            crate::diagnostics::Category::Message,
            0,
            &format!("Expanded from {}", expansion.label),
        )));
    }
    out
}

/// The hover, plus "Expanded from" and the expanded code when the position
/// is inside a macro invocation.
pub fn provide_hover(
    ls: &LanguageService,
    ctx: &Context,
    params: &lsproto::HoverParams,
) -> Result<lsproto::HoverResponse, GoError> {
    let mut response = ls.provide_hover(ctx, params)?;
    let Some(expansion) = expansion_at(ls, &params.text_document.uri, params.position) else {
        return Ok(response);
    };
    let caps = lsproto::get_client_capabilities(ctx);
    let markdown = lsproto::preferred_markup_kind(&caps.text_document.hover.content_format)
        == lsproto::MarkupKind::MARKDOWN;
    let note = if markdown {
        format!(
            "Expanded from {}\n```ts\n{}\n```",
            expansion.label, expansion.text
        )
    } else {
        format!("Expanded from {}\n{}", expansion.label, expansion.text)
    };
    match response.hover.as_mut() {
        Some(hover) => {
            if let Some(markup) = hover.contents.markup_content.as_mut() {
                markup
                    .value
                    .push_str(if markdown { "\n\n---\n\n" } else { "\n\n" });
                markup.value.push_str(&note);
            }
        }
        None => {
            let kind = if markdown {
                lsproto::MarkupKind::MARKDOWN
            } else {
                lsproto::MarkupKind::PLAIN_TEXT
            };
            response.hover = Some(markup_hover(kind, note, expansion.range));
        }
    }
    Ok(response)
}

fn markup_hover(kind: lsproto::MarkupKind, value: String, range: lsproto::Range) -> lsproto::Hover {
    lsproto::Hover {
        contents: lsproto::MarkupContentOrStringOrMarkedStringWithLanguageOrMarkedStrings {
            markup_content: Some(lsproto::MarkupContent { kind, value }),
            ..Default::default()
        },
        range: Some(range),
        ..Default::default()
    }
}

/// The innermost macro expansion whose invocation contains `position`: its
/// label, the virtual code it produced, and the invocation's range in the
/// author's file.
#[must_use]
pub fn expansion_at(
    ls: &LanguageService,
    uri: &lsproto::DocumentUri,
    position: lsproto::Position,
) -> Option<ExpansionView> {
    let (_, file) = ls.get_program_and_file(uri);
    for projection in
        lsconv::from_lsp_position_for_source_file(&ls.converters, file, position, Feature::ALL)
    {
        let script = projection.script;
        let span_map = source_file_span_map(script)?;
        let at = TextRange::new(projection.position, projection.position);
        let Some(expansion) = SpanMap::expansions_containing_virtual(Some(span_map), at)
            .into_iter()
            .next()
        else {
            continue;
        };
        let text = source_file_text(script);
        let start = usize::try_from(expansion.virtual_start).ok()?;
        let end = usize::try_from(expansion.virtual_end).ok()?.min(text.len());
        let mut code = text.get(start..end)?.trim().to_string();
        const LIMIT: usize = 4000;
        if code.len() > LIMIT {
            let mut cut = LIMIT;
            while !code.is_char_boundary(cut) {
                cut -= 1;
            }
            code.truncate(cut);
            code.push_str("\n…");
        }
        // The expansion's virtual text maps back over its invocation.
        let (range, _) = ls.converters.to_lsp_range(
            &script as &dyn lsconv::Script,
            TextRange::new(expansion.virtual_start, expansion.virtual_end),
        );
        return Some(ExpansionView {
            label: expansion.label.clone(),
            text: code,
            range,
        });
    }
    None
}

/// `custom/textDocument/showExpansion`: the innermost macro expansion at the
/// position, as a hover whose range is the invocation and whose contents
/// are the macro and its output.
pub fn handle_show_expansion(
    _server: &std::rc::Rc<crate::lsp::server::Server>,
    _ctx: &Context,
    ls: &LanguageService,
    params: &lsproto::TextDocumentPositionParams,
) -> Result<lsproto::CustomTextDocumentShowExpansionResponse, GoError> {
    let Some(expansion) = expansion_at(ls, &params.text_document.uri, params.position) else {
        return Ok(lsproto::HoverOrNull::default());
    };
    let value = format!(
        "Expanded from {}\n```ts\n{}\n```",
        expansion.label, expansion.text
    );
    Ok(lsproto::HoverOrNull {
        hover: Some(markup_hover(
            lsproto::MarkupKind::MARKDOWN,
            value,
            expansion.range,
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contentmapper::transform::parse_result;
    use crate::ext::spanmap_ext::Expansion;
    use crate::ls::diagnostics::is_synthesized_content_mapped_diagnostic;
    use crate::spanmap;
    use std::sync::Arc;

    #[test]
    fn diagnostics_in_macro_output_point_at_the_invocation() {
        // `const a = twice(value);` expanded to `const a = [value, value];`:
        // `[` and `, value]` are synthesized, `value` is copied.
        let original = "const a = twice(value);";
        let text = "const a = [value, value];";
        let map = spanmap::new(&[
            spanmap::Segment {
                virtual_end: 10,
                original_end: 10,
                kind: spanmap::Kind::VERBATIM,
                features: spanmap::Feature::ALL,
                ..Default::default()
            },
            spanmap::Segment {
                virtual_start: 11,
                virtual_end: 16,
                original_start: 16,
                original_end: 21,
                kind: spanmap::Kind::VERBATIM,
                features: spanmap::Feature::ALL,
            },
        ])
        .with_expansions(vec![Expansion {
            virtual_start: 10,
            virtual_end: 24,
            original_start: 10,
            original_end: 22,
            label: "macro `twice`".to_string(),
        }]);
        let result = crate::contentmapper::Result {
            text: text.to_string(),
            virtual_extension: ".ts".to_string(),
            mappings: Some(Arc::new(map)),
            ..Default::default()
        };
        let options = crate::frontend::parser::SourceFileParseOptions {
            file_name: "/a.sts".to_string(),
            path: crate::frontend::tspath::Path("/a.sts".to_string()),
            ..Default::default()
        };
        let files = parse_result(
            &options,
            original,
            &crate::contentmapper::Mapper::default(),
            "",
            result,
        )
        .unwrap();
        let file = files.canonical.unwrap().root;
        // `, value]` (16..24) is synthesized: it belongs to the invocation,
        // not the top of the file.
        let synthesized = new_diagnostic(
            file,
            TextRange::new(16, 24),
            diag::Virtual_code_produced_by_the_content_mapper_0_has_problems_with_no_corresponding_location_in_this_file,
            vec!["m".to_string()],
        );
        assert!(!is_synthesized_content_mapped_diagnostic(&synthesized));
        let with_context = with_expansion_context(&synthesized);
        let related = with_context.related_information();
        assert_eq!(related.len(), 1);
        assert_eq!((related[0].loc().pos(), related[0].loc().end()), (10, 22));
        // Outside every expansion nothing is added.
        let plain = new_diagnostic(
            file,
            TextRange::new(6, 7),
            diag::Virtual_code_produced_by_the_content_mapper_0_has_problems_with_no_corresponding_location_in_this_file,
            vec!["m".to_string()],
        );
        assert!(
            with_expansion_context(&plain)
                .related_information()
                .is_empty()
        );
        let map = source_file_span_map(file).unwrap();
        assert_eq!(
            invocation(map, TextRange::new(16, 24)),
            Some(TextRange::new(10, 22))
        );
    }
}
