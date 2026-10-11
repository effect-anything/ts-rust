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

use crate::ast::{
    ContentMapperSourceFileInfo, TextRange, get_source_file_of_node, is_function_like_declaration,
    is_prologue_directive, new_external_diagnostic, source_file_text,
};
use crate::astdata::SyntaxKind;
use crate::core::Node;
use crate::diagnostics::Category;
use crate::flags::ScriptKind;
use crate::frontend::parser::{
    ParsedSourceFile, SourceFileParseOptions, parse_source_file, set_source_file_diagnostics,
};
use crate::scanner_util::skip_trivia;
use crate::spanmap::{self, Feature, Kind, Segment};

/// The content-mapper identity this rewrite attaches to a file it changed.
const MAPPER: &str = crate::ext::MAPPER_NAME;
const NESTING: &str = "Nesting 'use gpu' functions is not allowed";

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

/// The calls the pinned TypeGPU plugin emits: `+ - * / %` and their
/// assignments. `**` is not in its table, so a `"use gpu"` file keeps it and
/// the tinyest AST keeps the operator for the shader printer.
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

/// [`name`] plus `**`, which `tsover-runtime` installs as `__tsover_pow`.
/// Only a `"use tsover"` file gets it.
#[must_use]
pub fn name_with_pow(kind: SyntaxKind) -> Option<(&'static str, bool)> {
    name(kind).or(match kind {
        SyntaxKind::AsteriskAsteriskToken => Some(("__tsover_pow", false)),
        SyntaxKind::AsteriskAsteriskEqualsToken => Some(("__tsover_pow", true)),
        _ => None,
    })
}

/// True when this file asked for the operator rewrite.
#[must_use]
pub fn enabled(code: &str) -> bool {
    code.contains("use tsover") || code.contains("use gpu")
}

/// Virtual text, span-map tuples, and original-file diagnostics for a
/// host outside `tsc`. `None` when `code` has no directive. A plugin calls
/// this after Sweetener expansion, or alone for an ordinary file.
#[must_use]
pub fn rewrite_text(filename: &str, code: &str) -> Option<Rewrite> {
    if !enabled(code) {
        return None;
    }
    let file_name = if filename.starts_with('/') {
        filename.to_owned()
    } else {
        format!("/{filename}")
    };
    let opts = SourceFileParseOptions {
        file_name,
        ..SourceFileParseOptions::default()
    };
    let kind = if filename.ends_with(".tsx") || filename.ends_with(".jsx") {
        ScriptKind::TSX
    } else {
        ScriptKind::TS
    };
    let leaked: &'static str = Box::leak(code.to_owned().into_boxed_str());
    let file = parse_source_file_unexpanded(&opts, leaked, kind);
    let (virtual_code, segments, nesting) = walk_file(&file, code)?;
    Some(Rewrite {
        code: virtual_code,
        segments,
        diagnostics: nesting
            .into_iter()
            .map(|(start, end)| RewriteDiagnostic {
                message: NESTING.to_owned(),
                start,
                end,
            })
            .collect(),
    })
}

/// Result of [`rewrite_text`]. Offsets are bytes of the input text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub code: String,
    pub segments: Vec<Segment>,
    pub diagnostics: Vec<RewriteDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteDiagnostic {
    pub message: String,
    pub start: u32,
    pub end: u32,
}

/// Rewrite `file` when its text contains `"use gpu"`. Otherwise return it.
#[must_use]
pub fn rewrite_after_parse(
    opts: &SourceFileParseOptions,
    file: ParsedSourceFile,
) -> ParsedSourceFile {
    if rewriting() || !enabled(&file.text) {
        return file;
    }
    let current = file.text.to_string();
    let Some((code, segments, nesting)) = walk_file(&file, &current) else {
        return file;
    };
    let _guard = RewriteGuard::enter();
    // The calls are not declared by `tsover-runtime`'s types. Ordinary
    // check and LSP only see a name after this prelude. It is synthesized,
    // so it is not in the span map and does not move the author's text.
    let code = format!("{PRELUDE}{code}");
    let shift = PRELUDE.len() as i32;
    let segments = segments
        .into_iter()
        .map(|mut segment| {
            segment.virtual_start += shift;
            segment.virtual_end += shift;
            segment
        })
        .collect::<Vec<_>>();
    let leaked: &'static str = Box::leak(code.into_boxed_str());
    let parsed = parse_source_file(opts, leaked, file.script_kind);
    if !file.content_mapper().is_empty() {
        // The mapper already set info on `file`. The rewritten parse is a
        // new file, so attaching the composed map once is the first set.
        let prior = crate::ast::source_file_span_map(file.root);
        let composed = compose_maps(prior, &segments);
        let original = crate::ast::source_file_original_text(file.root).to_string();
        if parsed.content_mapper().is_empty() {
            parsed.set_content_mapper_info(ContentMapperSourceFileInfo {
                content_mapper: file.content_mapper().to_owned(),
                transform_identity: file.content_mapper().to_owned(),
                parse_options: opts.clone(),
                virtual_file_name: opts.file_name.clone(),
                original_text: original,
                span_map: Some(std::sync::Arc::new(composed)),
                diagnostic_directives: Vec::new(),
                supplemental_source_files: Vec::new(),
                canonical_source_file: None,
            });
        }
        return parsed;
    }
    if !nesting.is_empty() {
        let mut diagnostics = parsed.diagnostics.clone();
        for (start, end) in nesting {
            diagnostics.push(new_external_diagnostic(
                parsed.root,
                TextRange::new(start as i32, end as i32),
                MAPPER,
                Category::Error,
                0,
                NESTING,
            ));
        }
        set_source_file_diagnostics(parsed.root, diagnostics);
    }
    parsed.set_content_mapper_info(ContentMapperSourceFileInfo {
        content_mapper: MAPPER.to_owned(),
        transform_identity: MAPPER.to_owned(),
        parse_options: opts.clone(),
        virtual_file_name: opts.file_name.clone(),
        original_text: current,
        span_map: Some(std::sync::Arc::new(spanmap::new(&segments))),
        diagnostic_directives: Vec::new(),
        supplemental_source_files: Vec::new(),
        canonical_source_file: None,
    });
    parsed
}

fn compose_maps(
    prior: Option<&crate::spanmap::SpanMap>,
    next: &[Segment],
) -> crate::spanmap::SpanMap {
    let Some(prior) = prior else {
        return spanmap::new(next);
    };
    let composed = next
        .iter()
        .filter_map(|segment| {
            let (original, _) = crate::spanmap::SpanMap::virtual_to_original_span(
                Some(prior),
                crate::ast::TextRange::new(segment.original_start, segment.original_end),
            );
            Some(Segment {
                virtual_start: segment.virtual_start,
                virtual_end: segment.virtual_end,
                original_start: original.pos(),
                original_end: original.end(),
                kind: segment.kind,
                features: segment.features,
            })
        })
        .collect::<Vec<_>>();
    spanmap::new(&composed)
}

fn parse_source_file_unexpanded(
    opts: &SourceFileParseOptions,
    text: &'static str,
    kind: ScriptKind,
) -> ParsedSourceFile {
    parse_without_rewrite(opts, text, kind)
}
/// Parse without the `"use gpu"` operator rewrite. The build wrapper needs
/// the original function text, not the rewritten operators.
pub fn parse_without_rewrite(
    opts: &SourceFileParseOptions,
    text: &'static str,
    kind: ScriptKind,
) -> ParsedSourceFile {
    let _guard = RewriteGuard::enter();
    parse_source_file(opts, text, kind)
}

fn walk_file(
    file: &ParsedSourceFile,
    original: &str,
) -> Option<(String, Vec<Segment>, Vec<(u32, u32)>)> {
    let mut walk = Walk::default();
    // `"use tsover"` is typenet's own directive and adds `**`. A `"use gpu"`
    // file gets the plugin's table, which has no `**`.
    walk.allow_pow = original.contains("use tsover");
    walk.visit(file.root);
    if walk.edits.is_empty() && walk.nesting.is_empty() {
        return None;
    }
    let (code, segments) = apply_edits(original, &walk.edits);
    Some((code, segments, walk.nesting))
}

#[derive(Default)]
struct Walk {
    allow_pow: bool,
    depth: u32,
    edits: Vec<Edit>,
    nesting: Vec<(u32, u32)>,
}

struct Edit {
    start: i32,
    end: i32,
    text: String,
    op_start: i32,
    op_end: i32,
}

impl Walk {
    fn visit(&mut self, node: Node) {
        if node.is_nil() {
            return;
        }
        if node.kind() == SyntaxKind::StringLiteral && is_use_tsover(node) {
            let root = get_source_file_of_node(node);
            root.for_each_child(|child| {
                if child != root {
                    self.visit_gpu(child);
                }
                false
            });
            return;
        }
        if node.kind() == SyntaxKind::StringLiteral && is_use_gpu(node) {
            if let Some(body) = gpu_body(node) {
                if self.depth > 0 {
                    self.nesting.push((node.pos() as u32, node.end() as u32));
                } else {
                    self.depth += 1;
                    note_nested(body, &mut self.nesting);
                    body.for_each_child(|child| {
                        if child != body {
                            self.visit_gpu(child);
                        }
                        false
                    });
                    self.depth -= 1;
                }
                return;
            }
        }
        node.for_each_child(|child| {
            if child != node {
                self.visit(child);
            }
            false
        });
    }

    fn visit_gpu(&mut self, node: Node) {
        if node.is_nil() || (node.kind() == SyntaxKind::StringLiteral && is_use_gpu(node)) {
            return;
        }
        if node.kind() == SyntaxKind::BinaryExpression {
            // Reading `operator_token` on another kind panics, so the kind is
            // checked first.
            let overloaded = if self.allow_pow {
                name_with_pow(node.operator_token().kind())
            } else {
                name(node.operator_token().kind())
            };
            if let Some((name, assign)) = overloaded {
                self.rewrite_binary(node, name, assign);
                return;
            }
        }
        node.for_each_child(|child| {
            if child != node {
                self.visit_gpu(child);
            }
            false
        });
    }

    fn rewrite_binary(&mut self, node: Node, name: &str, assign: bool) {
        let before = self.edits.len();
        self.visit_gpu(node.left());
        self.visit_gpu(node.right());
        let nested = self.edits.split_off(before);
        let left = rewritten_slice(node, node.left(), &nested);
        let right = rewritten_slice(node, node.right(), &nested);
        let op = node.operator_token();
        let text = if assign {
            format!("{left} = {name}({left}, {right})")
        } else {
            format!("{name}({left}, {right})")
        };
        let file_text = source_file_text(get_source_file_of_node(node));
        self.edits.push(Edit {
            start: skip_trivia(file_text.as_ref(), node.pos()),
            end: node.end(),
            text,
            op_start: skip_trivia(file_text.as_ref(), op.pos()),
            op_end: op.end(),
        });
    }
}

pub(crate) fn overload(kind: SyntaxKind) -> Option<(&'static str, bool)> {
    name(kind)
}

fn apply_edits(original: &str, edits: &[Edit]) -> (String, Vec<Segment>) {
    let mut ordered = edits.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|edit| edit.start);
    let mut code = String::new();
    let mut segments = Vec::new();
    let mut cursor = 0i32;
    for edit in ordered {
        if edit.start < cursor || edit.end < edit.start {
            continue;
        }
        push_verbatim(&mut code, &mut segments, original, cursor, edit.start);
        let v0 = i32::try_from(code.len()).unwrap_or(i32::MAX);
        code.push_str(&edit.text);
        let v1 = i32::try_from(code.len()).unwrap_or(i32::MAX);
        if v1 > v0 {
            segments.push(Segment {
                virtual_start: v0,
                virtual_end: v1,
                original_start: edit.op_start,
                original_end: edit.op_end,
                kind: Kind::ATOM,
                features: Feature::ALL,
            });
        }
        cursor = edit.end;
    }
    push_verbatim(
        &mut code,
        &mut segments,
        original,
        cursor,
        i32::try_from(original.len()).unwrap_or(i32::MAX),
    );
    (code, segments)
}

fn push_verbatim(
    code: &mut String,
    segments: &mut Vec<Segment>,
    original: &str,
    from: i32,
    to: i32,
) {
    if to <= from {
        return;
    }
    let v0 = i32::try_from(code.len()).unwrap_or(i32::MAX);
    let end = usize::try_from(to)
        .unwrap_or(original.len())
        .min(original.len());
    let start = usize::try_from(from).unwrap_or(0).min(end);
    code.push_str(&original[start..end]);
    let v1 = i32::try_from(code.len()).unwrap_or(i32::MAX);
    if let Some(prev) = segments.last_mut()
        && prev.kind == Kind::VERBATIM
        && prev.virtual_end == v0
        && prev.original_end == from
    {
        prev.virtual_end = v1;
        prev.original_end = to;
        return;
    }
    segments.push(Segment {
        virtual_start: v0,
        virtual_end: v1,
        original_start: from,
        original_end: to,
        kind: Kind::VERBATIM,
        features: Feature::ALL,
    });
}

fn node_slice(owner: Node, node: Node) -> String {
    let text = source_file_text(get_source_file_of_node(owner));
    let start = usize::try_from(skip_trivia(text.as_ref(), node.pos()).max(0)).unwrap_or(0);
    let end = usize::try_from(node.end().max(node.pos())).unwrap_or(text.len());
    text[start.min(end.min(text.len()))..end.min(text.len())].to_string()
}

fn rewritten_slice(owner: Node, node: Node, nested: &[Edit]) -> String {
    let raw = node_slice(owner, node);
    let text = source_file_text(get_source_file_of_node(owner));
    let origin = skip_trivia(text.as_ref(), node.pos());
    let local = nested
        .iter()
        .filter(|edit| edit.start >= origin && edit.end <= node.end())
        .map(|edit| Edit {
            start: edit.start - origin,
            end: edit.end - origin,
            text: edit.text.clone(),
            op_start: edit.op_start,
            op_end: edit.op_end,
        })
        .collect::<Vec<_>>();
    if local.is_empty() {
        raw
    } else {
        apply_edits(&raw, &local).0
    }
}

fn note_nested(body: Node, nesting: &mut Vec<(u32, u32)>) {
    body.for_each_child(|child| {
        if child != body && is_function_like_declaration(child) {
            let inner = child.body();
            if inner.kind() == SyntaxKind::Block {
                for statement in inner.statements().iter() {
                    if statement.kind() != SyntaxKind::ExpressionStatement {
                        break;
                    }
                    let literal = statement.expression();
                    if is_use_gpu(literal) {
                        nesting.push((literal.pos() as u32, literal.end() as u32));
                    }
                }
            }
        }
        false
    });
}
pub(crate) fn is_use_gpu(node: Node) -> bool {
    is_named_directive(node, "gpu")
}

fn is_use_tsover(node: Node) -> bool {
    is_named_directive(node, "tsover")
}

pub(crate) fn is_named_directive(node: Node, name: &str) -> bool {
    let directive = format!("use {name}");
    let quoted = format!("\"{directive}\"");
    let single = format!("'{directive}'");
    node.text() == directive || node.text() == quoted || node.text() == single
}

fn gpu_body(literal: Node) -> Option<Node> {
    let statement = literal.parent();
    if statement.kind() != SyntaxKind::ExpressionStatement {
        return None;
    }
    let body = statement.parent();
    if body.kind() != SyntaxKind::Block {
        return None;
    }
    for stmt in body.statements().iter() {
        if stmt == statement {
            return Some(body);
        }
        if stmt.kind() != SyntaxKind::ExpressionStatement
            || stmt.expression().kind() != SyntaxKind::StringLiteral
        {
            return None;
        }
    }
    None
}

std::thread_local! {
    static REWRITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn rewriting() -> bool {
    REWRITING.with(|flag| flag.get())
}

struct RewriteGuard;

impl RewriteGuard {
    fn enter() -> Self {
        REWRITING.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for RewriteGuard {
    fn drop(&mut self) {
        REWRITING.with(|flag| flag.set(false));
    }
}
