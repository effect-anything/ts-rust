//! Port of Go `ls/diagnostics.go`.

use crate::ls::prelude::*;

use crate::spanmap::{Fidelity, SpanMap};

// Go: ls/diagnostics.go:18 getAllDiagnostics
// getAllDiagnostics collects all diagnostics for a file: syntactic, semantic,
// suggestion, and (when declarations are emitted) declaration diagnostics.
// PORT: Go `*compiler.Program` methods are the `ls_program` functions
// (plan contract C3).
pub fn get_all_diagnostics(
    ctx: &Context,
    program: &compiler::NewProgram,
    file: Node,
) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut files = vec![file];
    files.extend_from_slice(source_file_supplemental_source_files(file));
    for source_file in files {
        diags.extend(ls_program::get_syntactic_diagnostics(
            program,
            ctx,
            source_file,
        ));
        diags.extend(ls_program::get_semantic_diagnostics(
            program,
            ctx,
            source_file,
        ));
        diags.extend(ls_program::get_suggestion_diagnostics(
            program,
            ctx,
            source_file,
        ));
        if program.options().get_emit_declarations() {
            diags.extend(ls_program::get_declaration_diagnostics(
                program,
                ctx,
                source_file,
            ));
        }
    }
    diags
}

impl LanguageService {
    // Go: ls/diagnostics.go:32 ProvideDiagnostics
    // PORT: Go passes the URI by value; here by reference.
    pub fn provide_diagnostics(
        &self,
        ctx: &Context,
        uri: &lsproto::DocumentUri,
    ) -> Result<lsproto::DocumentDiagnosticResponse, GoError> {
        let (program, file) = self.get_program_and_file(uri);

        if self.user_preferences().enable_validation.is_false() {
            return Ok(
                lsproto::RelatedFullDocumentDiagnosticReportOrUnchangedDocumentDiagnosticReport {
                    full_document_diagnostic_report: Some(
                        lsproto::RelatedFullDocumentDiagnosticReport {
                            items: Vec::new(),
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                },
            );
        }

        let diagnostics = get_all_diagnostics(ctx, program, file);

        Ok(
            lsproto::RelatedFullDocumentDiagnosticReportOrUnchangedDocumentDiagnosticReport {
                full_document_diagnostic_report: Some(
                    lsproto::RelatedFullDocumentDiagnosticReport {
                        items: self.to_lsp_diagnostics(ctx, &[diagnostics.as_slice()]),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
        )
    }

    // Go: ls/diagnostics.go:53 toLSPDiagnostics
    // PORT: the Go variadic `...[]*ast.Diagnostic` is a slice of slices.
    pub fn to_lsp_diagnostics(
        &self,
        ctx: &Context,
        diagnostics: &[&[Diagnostic]],
    ) -> Vec<lsproto::Diagnostic> {
        let report_style_checks_as_warnings = self
            .user_preferences()
            .report_style_checks_as_warnings
            .is_true();
        let mut size = 0;
        for diag_slice in diagnostics {
            size += diag_slice.len();
        }
        let mut lsp_diagnostics: Vec<lsproto::Diagnostic> = Vec::with_capacity(size);
        // Compiler diagnostics located entirely in a content-mapped file's synthesized code have no location
        // in the original file. Collect them per file and surface them through a single aggregate at the top
        // of the file (with the real messages as related information) rather than dropping them or scattering
        // them at position 0.
        // PORT: Go `collections.OrderedMap`; `IndexMap` keeps the same
        // insertion order.
        let mut synthesized_by_file: IndexMap<Node, Vec<Diagnostic>> = IndexMap::new();
        for diag_slice in diagnostics {
            for diag in diag_slice.iter() {
                let diag = &with_expansion_context(diag);
                if is_synthesized_content_mapped_diagnostic(diag) {
                    synthesized_by_file
                        .entry(diag.file())
                        .or_default()
                        .push(diag.clone());
                    continue;
                }
                lsp_diagnostics.push(lsconv::diagnostic_to_lsp_pull(
                    ctx,
                    &self.converters,
                    diag,
                    report_style_checks_as_warnings,
                ));
            }
        }
        for (&file, diags) in &synthesized_by_file {
            let aggregate = aggregate_synthesized_diagnostics(file, diags);
            lsp_diagnostics.push(lsconv::diagnostic_to_lsp_pull(
                ctx,
                &self.converters,
                &aggregate,
                report_style_checks_as_warnings,
            ));
        }
        lsp_diagnostics
    }
}

/// PORT: ts-rust extension. A compiler diagnostic inside macro output gets
/// one related location per enclosing invocation, innermost first:
/// "Expanded from macro `name`" at the code the author wrote.
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

// Go: ls/diagnostics.go:84 isSynthesizedContentMappedDiagnostic
// isSynthesizedContentMappedDiagnostic reports whether diag is a compiler diagnostic on a content-mapped
// file whose location lies entirely in synthesized virtual code with no counterpart in the original
// file, and so has no meaningful position to report against the original file.
pub fn is_synthesized_content_mapped_diagnostic(diag: &Diagnostic) -> bool {
    let file = diag.file();
    // PORT: Go `file == nil || file.SpanMap() == nil`; the span map of a nil
    // file is `None`.
    let Some(span_map) = source_file_span_map(file) else {
        return false;
    };
    if !diag.source().is_empty() {
        return false;
    }
    let (_, fidelity) = SpanMap::virtual_to_original_span(Some(span_map), diag.loc());
    // PORT: ts-rust extension. Inside a macro expansion the invocation is
    // the location.
    fidelity == Fidelity::NONE
        && SpanMap::expansions_containing_virtual(Some(span_map), diag.loc()).is_empty()
}

// Go: ls/diagnostics.go:97 aggregateSynthesizedDiagnostics
// aggregateSynthesizedDiagnostics builds a single diagnostic at the top of a content-mapped file standing
// in for compiler diagnostics located in synthesized code with no original location. The originals are
// attached as related information so their messages are surfaced rather than silently dropped. (A later
// change will point the related locations at a read-only view of the file's virtual TypeScript.)
pub fn aggregate_synthesized_diagnostics(file: Node, diags: &[Diagnostic]) -> Diagnostic {
    let mut aggregate = new_diagnostic(
        file,
        TextRange::new(0, 0),
        diag::Virtual_code_produced_by_the_content_mapper_0_has_problems_with_no_corresponding_location_in_this_file,
        vec![source_file_content_mapper(file).to_string()],
    );
    aggregate.set_related_info(diags.to_vec());
    aggregate.set_category(worst_category(diags));
    aggregate
}

// Go: ls/diagnostics.go:109 worstCategory
pub fn worst_category(diags: &[Diagnostic]) -> crate::diagnostics::Category {
    let mut worst = diags[0].category();
    for diag in diags {
        match diag.category() {
            crate::diagnostics::Category::Error => return crate::diagnostics::Category::Error,
            crate::diagnostics::Category::Warning => {
                worst = crate::diagnostics::Category::Warning;
            }
            _ => {}
        }
    }
    worst
}

#[cfg(test)]
mod expansion_tests {
    use super::*;
    use crate::contentmapper::transform::parse_result;
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
        .with_expansions(vec![spanmap::Expansion {
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
        // `, value]` (16..24) is synthesized: it used to be reported at the
        // top of the file; it belongs to the invocation.
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
    }
}
