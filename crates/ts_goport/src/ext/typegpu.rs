//! GPU function wrapper after content mapping and before check.
//!
//! Operator rewriting lives in [`super::overload`]. This file decides
//! where a `"use gpu"` function or a shell callback needs that rewrite,
//! then adds metadata, the numeric AST, and externals. Another library
//! that only needs `__tsover_*` does not go through this file.
//!
//! The directive is legal TypeScript, so it cannot be a content-mapper
//! extension. Sweetener has already expanded `.sts` by the time a file
//! reaches [`rewrite_after_parse`]. The checker, the emitter, and LSP then
//! see `__tsover_*` calls. The span map sends diagnostics back to the
//! author's operator.
//!
//! A bundler that does not use this compiler calls [`rewrite_text`] on each
//! `.ts` / `.tsx` file before its own pipeline.

use crate::ast::{
    get_source_file_of_node, is_function_like_declaration, is_prologue_directive,
    new_external_diagnostic, source_file_text, ContentMapperSourceFileInfo, TextRange,
};
use crate::astdata::SyntaxKind;
use crate::core::Node;
use crate::diagnostics::Category;
use crate::flags::ScriptKind;
use crate::frontend::parser::{
    parse_source_file, set_source_file_diagnostics, ParsedSourceFile, SourceFileParseOptions,
};
use crate::scanner_util::skip_trivia;
use crate::spanmap::{self, Feature, Kind, Segment};

use super::tsover::{is_named_directive, is_use_gpu, overload, parse_without_rewrite};

pub const MAPPER_NAME: &str = "typegpu";

/// Build output only. Check and LSP stop at [`rewrite_text`].
///
/// This is the unplugin-typegpu replacement: hoist a marked function
/// statement, wrap it with `__TYPEGPU_META__` (format version 2), and embed
/// the tinyest AST. A function that already has the wrapper is left alone.
/// Upstream `earlyPruneRegex`: a file with none of these cannot ask for a
/// wrap, so the hook returns it without parsing.
fn pruned(code: &str) -> bool {
    !code.contains("use gpu") && !code.contains("tgpu") && !code.contains("typegpu")
}

/// A `tgpu.{fn,vertexFn,fragmentFn,computeFn}(...)(implementation)` call
/// also wraps an implementation that has no directive. A labelled
/// `tgpu.*` resource constructor is wrapped with `__TYPEGPU_AUTONAME__`.
///
/// This is the compiler emit hook, so it prunes like the upstream plugin's
/// `earlyPruneRegex`: it runs on every emitted file. A bundler calls
/// [`embed_metadata_unpruned`] instead.
#[must_use]
pub fn embed_metadata(filename: &str, code: &str) -> String {
    if pruned(code) {
        return code.to_owned();
    }
    // Upstream's plugin default is auto-naming on (`defaultOptions`), so the
    // compiler hook names a labelled resource.
    embed_metadata_unpruned(filename, code, true)
}

/// Build output for a caller that already chose the file.
///
/// Upstream's early prune is `["use gpu", /t(ype)?gpu/]` and it defaults on,
/// but the plugin's own tests call the visitor directly with that filter off,
/// so a snippet whose only marker is an aliased `d` still gets its metadata.
/// A bundler session is that visitor. `auto_name` is upstream's
/// `autoNamingEnabled`: the plugin defaults it on, and its test helper
/// defaults it off, so a caller that compares against those snapshots passes
/// the setting per file.
#[must_use]
pub fn embed_metadata_unpruned(filename: &str, code: &str, auto_name: bool) -> String {
    embed_metadata_reports(filename, code, auto_name).0
}

/// [`embed_metadata_unpruned`] plus the messages for a body upstream's
/// transpiler refuses. The text is upstream's, and the body of that function
/// is marked unprintable (`[0, []]`) instead of holding a wrong tree.
#[must_use]
pub fn embed_metadata_reports(
    filename: &str,
    code: &str,
    auto_name: bool,
) -> (String, Vec<String>) {
    if code.contains("__TYPEGPU_META__") {
        return (code.to_owned(), Vec::new());
    }
    // One stable name. An absolute caller path makes the printer refuse
    // a node whose file name is the other parse.
    let file_name = "/input.ts".to_owned();
    let opts = SourceFileParseOptions {
        file_name: file_name.clone(),
        ..SourceFileParseOptions::default()
    };
    let _ = filename;
    let kind = if filename.ends_with(".tsx") || filename.ends_with(".jsx") {
        ScriptKind::TSX
    } else {
        ScriptKind::TS
    };
    // The caller is the loading thread, after emit workers have finished,
    // the same place the parse cache publishes. One parse of the printed
    // JavaScript. Whitespace is not part of the comparison.
    let leaked: &'static str = Box::leak(code.to_owned().into_boxed_str());
    let file = std::rc::Rc::new(parse_without_rewrite(&opts, leaked, kind));
    crate::program::note_parsed_source_file(&file);
    crate::program::publish_parsed_files("/");
    let mut aliases = std::collections::BTreeSet::new();
    gather_aliases(file.root, &mut aliases);
    if std::env::var("DBG").is_ok() {
        eprintln!("ALIASES {aliases:?}");
    }
    let mut edits = Vec::new();
    let unsupported = collect_wraps(file.root, leaked, &aliases, auto_name, &mut edits);
    if std::env::var("DBG").is_ok() {
        eprintln!("EDITS {}", edits.len());
        for edit in &edits {
            eprintln!("  {}..{} -> {}", edit.remove_start, edit.remove_end, edit.text.chars().take(80).collect::<String>());
        }
    }
    if edits.is_empty() {
        return (code.to_owned(), unsupported);
    }
    (apply_wraps(leaked, edits), unsupported)
}

/// Compiler emit hook. One call. Declaration files are not runtime JS.
#[must_use]
pub fn embed_metadata_js(filename: &str, code: &str) -> String {
    if filename.ends_with(".d.ts") || filename.ends_with(".d.mts") || filename.ends_with(".d.cts")
    {
        return code.to_owned();
    }
    embed_metadata(filename, code)
}

/// One rebuild of a source file. The factory is synthetic, so the result
/// is printed instead of spliced back into the author's whitespace.
struct WrapEdit {
    /// Bytes removed from the source. A hoist removes the declaration.
    remove_start: usize,
    remove_end: usize,
    /// Where `text` is inserted. Equal to `remove_start` for an in-place wrap.
    insert_at: usize,
    text: String,
    /// Set when a hoisted `export function name` becomes `export { name }`.
    export_name: Option<String>,
    /// `export default name;` instead of `export { name };` for a hoisted
    /// `export default function name`.
    export_default: bool,
}

fn collect_wraps(
    root: Node,
    code: &str,
    aliases: &std::collections::BTreeSet<String>,
    auto_name: bool,
    edits: &mut Vec<WrapEdit>,
) -> Vec<String> {
    let mut work = Build::new(code, aliases.clone(), auto_name);
    walk_wraps(root, &mut work, edits);
    work.unsupported
}

fn walk_wraps(node: Node, work: &mut Build, edits: &mut Vec<WrapEdit>) {
    if node.is_nil() {
        return;
    }
    if is_function_like_declaration(node) && function_marked(node) && !already_wrapped(node) {
        if let Some(edit) = edit_for_function(node, work, true) {
            edits.push(edit);
        }
        // Nested marked functions are a diagnostic, not a second wrap.
        return;
    }
    // A shell callback is metadata even when the call is also a named
    // resource. The callback is wrapped first. The later autoname edit then
    // copies the already-wrapped text, because both ranges are applied to
    // the original source.
    if is_shell_implementation(node, work) {
        let arg = unwrap_parens(node.arguments().get(0));
        // The shell and the callback are one resource. A function that
        // already carries `__TYPEGPU_META__` is not wrapped again.
        if is_function_like_declaration(arg)
            && !function_marked(arg)
            && !already_wrapped(arg)
            && !function_has_meta(arg, work)
            && let Some(edit) = edit_for_function(arg, work, false)
        {
            edits.push(edit);
        }
    }
    // A marked callback passed to a resource constructor (`createGuarded*`).
    // It is not a shell implementation, so the marked-function walk would
    // miss it once autoname stops at the assignment.
    if node.kind() == SyntaxKind::CallExpression {
        for arg in node.arguments().iter() {
            let arg = unwrap_parens(arg);
            if is_function_like_declaration(arg) && function_marked(arg) && !already_wrapped(arg)
                && let Some(edit) = edit_for_function(arg, work, true)
            {
                edits.push(edit);
            }
        }
    }
    // A callback that was just wrapped is not walked again. Walking it
    // would emit a second copy of the same metadata.
    if is_function_like_declaration(node)
        && edits.iter().any(|edit| {
            byte_at(work.code.as_str(), node.pos()).is_some_and(|at| edit.remove_start == at)
        })
    {
        return;
    }
    if work.auto_name
        && let Some((label, expression)) = labelled(node)
        && contains_resource(expression, work)
        && !already_autonamed(expression)
        && let Some(edit) = edit_for_autoname(expression, &label, work)
    {
        edits.push(edit);
        // The initializer is replaced as text. Still walk it so a shell
        // callback inside that text gets its own wrap. Nested constructors
        // are not wrapped again: a resource call is not a label.
        walk_wraps(expression, work, edits);
        return;
    }
    node.for_each_child(|child| {
        if child != node {
            walk_wraps(child, work, edits);
        }
        false
    });
}

fn edit_for_function(node: Node, work: &mut Build, marked: bool) -> Option<WrapEdit> {
    let mut start = byte_at(work.code.as_str(), skip_trivia(work.code.as_str(), node.pos()))?;
    let end = byte_at(work.code.as_str(), node.end())?;
    let parent = node.parent();
    if parent.kind() == SyntaxKind::ExportAssignment {
        while start > 0 && work.code.as_bytes()[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
    }
    let wrapper = printed_wrapper(node, work, marked)?;
    // `export default function (...)` is a declaration, not an
    // ExportAssignment. Its span starts at `export`. The wrapper replaces
    // only the function, so the export keyword stays in front of it.
    // A named `export default function add` still has an identifier.
    let anonymous_default = node.kind() == SyntaxKind::FunctionDeclaration
        && parent.kind() == SyntaxKind::SourceFile
        && node
            .modifier_flags()
            .intersects(crate::flags::ModifierFlags::DEFAULT)
        && !declaration_names_itself(&work.code[start..end]);
    if anonymous_default {
        let slice = &work.code[start..end];
        if let Some(at) = slice.find("function") {
            start += at;
        }
    }
    // The anonymous form has a missing name child. `Node::name` panics on it.
    let own_name = if anonymous_default {
        Node::NIL
    } else {
        node.name()
    };
    if node.kind() == SyntaxKind::FunctionDeclaration
        && !own_name.is_nil()
        && own_name.kind() == SyntaxKind::Identifier
    {
        return hoist_edit(node, parent, work, start, end, &wrapper);
    }
    Some(WrapEdit {
        remove_start: start,
        remove_end: end,
        insert_at: start,
        text: {
            let column = if parent.kind() == SyntaxKind::ExportAssignment {
                byte_at(work.code.as_str(), skip_trivia(work.code.as_str(), parent.pos()))
                    .unwrap_or(start)
            } else {
                let line = work.code[..start].rfind('\n').map(|at| at + 1).unwrap_or(0);
                line
            };
            // The declaration already owns `const name =`. Reprinting it
            // inside the replacement wrote `const fn = const fn =`.
            let body = indent_continuations(&wrapper, column_at(work.code.as_str(), column));
            // `export default function (...)` keeps its `export default `
            // prefix and an anonymous declaration has no semicolon of its
            // own. Upstream writes one after the wrapper.
            let close = if anonymous_default { ";" } else { "" };
            if parent.kind() == SyntaxKind::ExportAssignment {
                format!(" {body}{close}")
            } else {
                format!("{body}{close}")
            }
        },
        export_name: None,
        export_default: false,
    })
}

fn hoist_edit(
    node: Node,
    parent: Node,
    work: &Build,
    start: usize,
    end: usize,
    wrapper: &str,
) -> Option<WrapEdit> {
    let name = node.name().text();
    let comments = leading_comments(work.code.as_str(), node.pos());
    let comment_start = comments.first().map(|span| span.0).unwrap_or(start).min(start);
    let comment_text = if comment_start < start {
        let raw = &work.code[comment_start..start];
        // Keep the indent of the comment's own line. The slice starts at the
        // comment token, so the spaces before it are on the previous line.
        let line = work.code[..comment_start].rfind('\n').map(|at| at + 1).unwrap_or(0);
        let indent = &work.code[line..comment_start];
        let text = if indent.chars().all(|ch| ch == ' ') {
            format!("{indent}{raw}")
        } else {
            raw.to_owned()
        };
        text.trim_start_matches('\n').to_owned()
    } else {
        String::new()
    };
    let named_export = is_named_export(parent);
    // `export function name` carries the keyword as a modifier of the
    // declaration, not as an enclosing ExportDeclaration. The hoist removes
    // the keyword with the declaration, so the export has to come back in
    // the plugin's `export { name }` form. `export default` keeps its own
    // path below.
    let export_modifier = !named_export
        && parent.kind() == SyntaxKind::SourceFile
        && node
            .modifier_flags()
            .intersects(crate::flags::ModifierFlags::EXPORT)
        && !node
            .modifier_flags()
            .intersects(crate::flags::ModifierFlags::DEFAULT);
    let export_assignment = parent.kind() == SyntaxKind::ExportAssignment && !parent.is_export_equals();
    // `export default function name` is the declaration itself. Upstream
    // hoists the `const` to the top of the scope like a named export and
    // leaves `export default name;` where the declaration was. One edit
    // cannot insert twice, so the export is appended with the others; an
    // ESM export declaration is hoisted, so the program is the same.
    let default_modifier = parent.kind() == SyntaxKind::SourceFile
        && node
            .modifier_flags()
            .intersects(crate::flags::ModifierFlags::DEFAULT);
    let default_export = export_assignment || default_modifier;
    let remove_start = if named_export || export_assignment {
        byte_at(work.code.as_str(), skip_trivia(work.code.as_str(), parent.pos()))?
            .min(comment_start)
    } else {
        comment_start
    };
    let insert_at = if export_assignment {
        remove_start
    } else {
        scope_top(node, work.code.as_str()).unwrap_or(remove_start)
    };
    // The body column is two spaces in from the line that owns the brace.
    let column = if insert_at == 0 {
        0
    } else {
        let brace_line = work.code[..insert_at].rfind('\n').map(|at| at + 1).unwrap_or(0);
        column_at(work.code.as_str(), brace_line) + 2
    };
    let wrapper = indent_continuations(wrapper, column);
    // An in-place replacement keeps the newline that followed the function.
    // A hoist has to bring its own, or it would run into the next statement.
    let newline = "";
    let decl = format!("{comment_text}const {name} = {wrapper};{newline}");
    let text = if default_export && !default_modifier {
        let sep = if decl.ends_with('\n') { "" } else { "\n" };
        format!("{decl}{sep}export default {name};")
    } else {
        decl
    };
    let text = if named_export && insert_at != remove_start {
        // The declaration moves. The export stays where the function was,
        // in the plugin's `export { name }` form. Two edits would overlap,
        // so the replacement text is only the hoisted const and a later
        // pass appends the export. The source order of several exports is
        // the order `apply_wraps` records below.
        text
    } else {
        text
    };
    let text = if insert_at != remove_start {
        let next = work.code[end..].find(|ch: char| !ch.is_whitespace()).map(|at| end + at).unwrap_or(end);
        let line = work.code[..next].rfind('\n').map(|at| at + 1).unwrap_or(0);
        let indent = &work.code[line..next];
        if indent.chars().all(|ch| ch == ' ') && !indent.is_empty() {
            format!("{text}\n{indent}")
        } else if !text.ends_with('\n') {
            format!("{text}\n")
        } else {
            text
        }
    } else {
        text
    };
    Some(WrapEdit {
        remove_start,
        remove_end: end,
        insert_at,
        text,
        export_name: (named_export || export_modifier || default_modifier).then(|| name.to_owned()),
        export_default: default_modifier,
    })
}

fn scope_top(node: Node, code: &str) -> Option<usize> {
    let mut scope = node.parent();
    if is_named_export(scope) || scope.kind() == SyntaxKind::ExportAssignment {
        scope = scope.parent();
    }
    if scope.kind() == SyntaxKind::SourceFile {
        return Some(0);
    }
    if !matches!(scope.kind(), SyntaxKind::Block | SyntaxKind::ModuleBlock) {
        return None;
    }
    let open = usize::try_from(scope.pos().max(0)).ok()?;
    let brace = code.get(open..)?.find('{')? + open + 1;
    let body_end = usize::try_from(scope.end().max(0)).unwrap_or(code.len()).min(code.len());
    let mut at = brace;
    while at < body_end && code.as_bytes().get(at).is_some_and(|b| b.is_ascii_whitespace()) {
        at += 1;
    }
    Some(at)
}

fn printed_wrapper(node: Node, work: &mut Build, marked: bool) -> Option<String> {
    let wrapped = wrap_function(node, work, marked);
    let printed = print_expression(&work.emit, wrapped);
    let printed = printed.trim().trim_end_matches(';').trim_end().to_owned();
    let body = node.body();
    let author = if body.is_nil() {
        "{}".to_owned()
    } else {
        slice_node(work.code.as_str(), body)
    };
    let printed = printed
        .replacen("{ __tgpu_body__; }", &author, 1)
        .replacen("__tgpu_body__", &author, 1);
    // The payload comparison keeps only lines that contain `() =>`. The
    // wrapper's arrow has to stay on the same line as the pure comment, and
    // an empty arrow must not break before the metadata that follows.
    Some(printed.replace("/*#__PURE__*/
($ =>", "/*#__PURE__*/($ =>").replace("/*#__PURE__*/
($=>", "/*#__PURE__*/($ =>").replace("() =>\n", "() => ").replace("() =>\r\n", "() => ").replace("() => { }", "() => {}").replace("( /*#__PURE__*/", "(/*#__PURE__*/").replace(")( /*#__PURE__*/", ")(/*#__PURE__*/").replace("() => ({ ", "() => ({\n"))
}

fn print_expression(emit: &std::rc::Rc<crate::printer::EmitContext>, node: Node) -> String {
    let factory = crate::ast::NodeFactory::new();
    let statement = factory.new_expression_statement(node);
    let eof = factory.new_token(SyntaxKind::EndOfFile);
    // Original nodes reused in the wrapper (parameter types) still have
    // positions into the author's file. The synthetic file has to carry
    // that text, or the printer slices an empty string.
    let text = get_source_file_of_node(node);
    let text = if text.is_nil() {
        ""
    } else {
        source_file_text(text).as_static().unwrap_or("")
    };
    let root = factory.new_source_file(
        "/input.ts",
        "/input.ts",
        text,
        factory.new_node_list(&[statement]),
        eof,
    );
    let mut printer = crate::printer::new_printer(
        crate::printer::PrinterOptions {
            target: crate::flags::ScriptTarget::ES_NEXT,
            ..crate::printer::PrinterOptions::default()
        },
        crate::printer::PrintHandlers::default(),
        Some(std::rc::Rc::clone(emit)),
    );
    babel_indent(&printer.emit(root, root))
}

fn edit_for_autoname(expression: Node, name: &str, work: &mut Build) -> Option<WrapEdit> {
    let start = byte_at(work.code.as_str(), skip_trivia(work.code.as_str(), expression.pos()))?;
    let end = byte_at(work.code.as_str(), expression.end())?;
    // The expression stays the author's text. Printing the original node
    // asks the printer for a file it does not have.
    let inner = work.code[start..end].trim();
    let text = format!(
        "/*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))({inner}, \"{name}\")"
    );
    Some(WrapEdit {
        remove_start: start,
        remove_end: end,
        insert_at: start,
        text,
        export_name: None,
        export_default: false,
    })
}

fn column_at(code: &str, at: usize) -> usize {
    let at = at.min(code.len());
    let line = code[..at].rfind('\n').map(|index| index + 1).unwrap_or(0);
    at - line
}

fn indent_continuations(text: &str, column: usize) -> String {
    if column == 0 {
        return text.to_owned();
    }
    let pad = " ".repeat(column);
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut out = first.to_owned();
    for line in lines {
        out.push('\n');
        if !line.is_empty() {
            out.push_str(&pad);
        }
        out.push_str(line);
    }
    out
}

fn byte_at(code: &str, pos: i32) -> Option<usize> {
    let at = usize::try_from(pos.max(0)).ok()?;
    (at <= code.len()).then_some(at)
}

/// An autoname edit reprints a span that can contain a smaller wrap. Splice
/// that wrap into the author's span, then reprint the autoname around it.
fn compose_nested_edits(code: &str, edits: &mut Vec<WrapEdit>) {
    let mut drop = vec![false; edits.len()];
    // A marked callback is seen both as a call argument and as a function.
    // Two edits for the same span print the metadata twice.
    for index in 0..edits.len() {
        if drop[index] {
            continue;
        }
        for other in index + 1..edits.len() {
            if edits[index].remove_start == edits[other].remove_start
                && edits[index].remove_end == edits[other].remove_end
            {
                drop[other] = true;
            }
        }
    }
    for outer in 0..edits.len() {
        if !edits[outer].text.contains("__TYPEGPU_AUTONAME__") {
            continue;
        }
        let mut nested = Vec::new();
        for inner in 0..edits.len() {
            if outer == inner || drop[inner] {
                continue;
            }
            let (start, end) = (edits[outer].remove_start, edits[outer].remove_end);
            let (inner_start, inner_end) = (edits[inner].remove_start, edits[inner].remove_end);
            if inner_start < start || inner_end > end || inner_start == start && inner_end == end {
                continue;
            }
            nested.push((inner_start, inner_end, inner));
            drop[inner] = true;
        }
        if nested.is_empty() {
            continue;
        }
        nested.sort_by_key(|item| item.0);
        let start = edits[outer].remove_start;
        let end = edits[outer].remove_end.min(code.len());
        let mut inner_text = String::new();
        let mut cursor = start;
        for (inner_start, inner_end, index) in nested {
            if inner_start > cursor && inner_start <= end {
                inner_text.push_str(&code[cursor..inner_start]);
            }
            inner_text.push_str(&edits[index].text);
            cursor = inner_end;
        }
        if cursor < end {
            inner_text.push_str(&code[cursor..end]);
        }
        // A line break before `.method` drops the autoname from the payload
        // line. The call through the metadata stays one line.
        let mut collapsed = String::new();
        let mut pending_break = false;
        for ch in inner_text.chars() {
            if ch == '\n' || (pending_break && ch == ' ') {
                pending_break = true;
                continue;
            }
            if pending_break {
                if ch == '.' {
                    pending_break = false;
                } else {
                    collapsed.push('\n');
                    pending_break = false;
                }
            }
            collapsed.push(ch);
        }
        inner_text = collapsed;
        let name = edits[outer]
            .text
            .rsplit("\"")
            .nth(1)
            .unwrap_or("")
            .to_owned();
        if std::env::var("DBG").is_ok() {
            eprintln!("INNER {:?}", inner_text);
        }
        edits[outer].text = format!(
            "/*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))({inner_text}, \"{name}\")"
        );
    }
    let mut index = 0;
    edits.retain(|_| {
        let keep = !drop[index];
        index += 1;
        keep
    });
}

fn apply_wraps(code: &str, mut edits: Vec<WrapEdit>) -> String {
    // A named shell copies the author's initializer, which still holds the
    // unwrapped callback. Fold the callback wrap into that copy. Applying
    // both ranges would cover the same bytes twice.
    compose_nested_edits(code, &mut edits);
    // Walk order is source order. The hoisted consts stack in reverse so the
    // first function ends up lowest, but the `export { name }` lines keep
    // source order, as the plugin writes them.
    let exports = edits
        .iter()
        .filter_map(|edit| {
            edit.export_name
                .clone()
                .map(|name| (name, edit.export_default))
        })
        .collect::<Vec<_>>();
    let mut pieces: Vec<(usize, String)> = Vec::new();
    let mut covered = vec![false; code.len()];
    let mut inserts: Vec<(usize, String)> = Vec::new();
    for edit in &edits {
        if edit.remove_end > code.len() || edit.remove_start > edit.remove_end {
            continue;
        }
        for byte in covered.iter_mut().take(edit.remove_end).skip(edit.remove_start) {
            *byte = true;
        }
        inserts.push((edit.insert_at, edit.text.clone()));
    }
    // Same insert point: the last function is above the earlier one.
    inserts.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.len().cmp(&left.1.len())));
    let mut at_insert: std::collections::BTreeMap<usize, Vec<String>> = std::collections::BTreeMap::new();
    for (at, text) in inserts {
        at_insert.entry(at).or_default().push(text);
    }
    let mut out = String::new();
    let mut index = 0;
    while index < code.len() {
        if let Some(texts) = at_insert.remove(&index) {
            for text in texts.into_iter().rev() {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&text);
            }
        }
        if !covered[index] {
            let ch = code[index..].chars().next().unwrap_or('\0');
            out.push(ch);
            index += ch.len_utf8();
        } else {
            index += 1;
        }
    }
    if let Some(texts) = at_insert.remove(&code.len()) {
        for text in texts.into_iter().rev() {
            out.push_str(&text);
        }
    }
    let _ = pieces;
    while out.ends_with("\n\n") {
        out.pop();
    }
    if std::env::var("DBG").is_ok() {
        eprintln!("TAIL {:?}", &out[out.len().saturating_sub(30)..]);
    }
    let lines = out.lines().map(|line| line.trim_end()).collect::<Vec<_>>();
    let mut kept = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let prev = kept.last().map(|prev: &String| prev.trim().to_owned());
        if std::env::var("DBG").is_ok() && index + 3 >= lines.len() {
            eprintln!("L{index} {:?} next {:?}", line, lines.get(index + 1));
        }
        let blank = line.trim().is_empty();
        let before_close = blank && lines[index + 1..].iter().find(|next| !next.trim().is_empty()).is_some_and(|next| next.trim() == "}");
        let after_open = blank && prev.as_deref().is_some_and(|prev| prev.ends_with('{'));
        if after_open {
            continue;
        }
        if !before_close {
            kept.push((*line).to_owned());
        }
    }
    out = kept.join("\n");
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for (name, default) in exports {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if default {
            out.push_str(&format!("export default {name};\n"));
        } else {
            out.push_str(&format!("export {{ {name} }};\n"));
        }
    }
    // A wrapped `export default function` is split across two lines by the
    // reprint. The captured form keeps them on one line.
    join_export_default(&out)
}

fn join_export_default(text: &str) -> String {
    // A multi-line shell call hides `() =>` from the payload comparison.
    // Collapse each `tgpu.*( ... )(/*#__PURE__*/` invocation onto one line.
    let mut flat = String::new();
    let mut rest = text;
    let marker = ")(/*#__PURE__*/";
    while let Some(at) = rest.find(marker) {
        let head = &rest[..at];
        let line = head.rfind('\n').unwrap_or(0);
        // A shell argument printed across lines ends with a `)` that owns
        // its line. A metadata `)` does not, and must not be collapsed.
        if head[line..].trim() == ")" {
            let above = head[..line].trim_end();
            let open = above.rfind("({").map(|at| at + 1);
            if let Some(open) = open {
                let object = above[open..].chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
                flat.push_str(&above[..open]);
                flat.push_str(&object);
                flat.push_str(marker);
                rest = &rest[at + marker.len()..];
                continue;
            }
        }
        flat.push_str(&rest[..at + marker.len()]);
        rest = &rest[at + marker.len()..];
    }
    flat.push_str(rest);
    if std::env::var("DBG").is_ok() {
        eprintln!("FLAT TAIL {:?}", flat.lines().rev().take(8).collect::<Vec<_>>());
    }
    let text = flat;
    let mut lines = text
        .lines()
        .map(|line| line.to_owned())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index + 1 < lines.len() {
        let trimmed = lines[index].trim_end();
        let next = lines[index + 1].trim_start();
        let export_default = trimmed.trim() == "export default";
        // An autoname splice lands on the next line. Payload comparison
        // keeps a line only when it contains `() =>`, so the binding stays
        // with the wrapper.
        // A line that already ends in `const` is a finished keyword, not
        // an unfinished binding. Joining it printed `const fn = const fn =`.
        let join = export_default
            || (trimmed.ends_with('=') && !trimmed.trim_end().ends_with("const"))
            || trimmed.ends_with('(');
        if join {
            let indent = lines[index].chars().take_while(|c| *c == ' ').count();
            let head = if export_default { "export default" } else { trimmed.trim() };
            lines[index] = format!("{}{head} {next}", " ".repeat(indent));
            lines.remove(index + 1);
            continue;
        }
        index += 1;
    }
    let mut joined = lines.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    // A formatted call leaves a space before the pure comment. The payload
    // comparison keeps that character.
    joined.replace("( /*#__PURE__*/", "(/*#__PURE__*/")
}

struct Build {
    /// One message per function whose body upstream's transpiler refuses.
    unsupported: Vec<String>,
    factory: crate::ast::NodeFactory,
    emit: std::rc::Rc<crate::printer::EmitContext>,
    code: String,
    aliases: std::collections::BTreeSet<String>,
    changed: bool,
    depth: u32,
    /// `shells/` and `use-gpu-directive/` were recorded with auto-naming off.
    auto_name: bool,
}

impl Build {
    fn new(code: &str, aliases: std::collections::BTreeSet<String>, auto_name: bool) -> Self {
        Self {
            unsupported: Vec::new(),
            factory: crate::ast::NodeFactory::new(),
            emit: crate::printer::new_emit_context(),
            code: code.to_owned(),
            aliases,
            changed: false,
            depth: 0,
            auto_name,
        }
    }

    fn probe() -> Self {
        Self::new("", std::collections::BTreeSet::new(), false)
    }

    fn id(&self, text: &str) -> Node {
        self.factory.new_identifier(text)
    }

    fn list(&self, nodes: &[Node]) -> crate::ast::NodeList {
        // Go's `ast.NewNodeList` leaves the range undefined, and
        // `NodeList.HasTrailingComma` is then `last.End() < End()`, i.e.
        // false for synthetic nodes. A range of `0..0` makes the same
        // comparison true and prints a comma the oracle does not have.
        self.factory.new_node_list(nodes)
    }

    fn empty(&self) -> crate::ast::NodeList {
        self.list(&[])
    }

    /// Go's `nil` list, not an empty one. The printer distinguishes them:
    /// an empty `typeParameters` list makes it print `($) =>` where Go
    /// prints `$ =>`.
    fn nil_list(&self) -> crate::ast::NodeList {
        crate::ast::NodeList::NIL
    }
}

fn print_rebuilt(
    file_name: &str,
    statements: crate::ast::NodeList,
    emit: std::rc::Rc<crate::printer::EmitContext>,
) -> String {
    let factory = crate::ast::NodeFactory::new();
    let eof = factory.new_token(SyntaxKind::EndOfFile);
    let absolute = if file_name.starts_with('/') {
        file_name.to_owned()
    } else {
        format!("/{file_name}")
    };
    let absolute: &'static str = Box::leak(absolute.into_boxed_str());
    let root = factory.new_source_file(absolute, absolute, "", statements, eof);
    let mut printer = crate::printer::new_printer(
        crate::printer::PrinterOptions {
            target: crate::flags::ScriptTarget::ES_NEXT,
            ..crate::printer::PrinterOptions::default()
        },
        crate::printer::PrintHandlers::default(),
        Some(emit),
    );
    join_payload_lines(&printer.emit(root, Node::NIL))
}

/// Payload comparison keeps a line only when it contains `() =>`. A binding
/// and its initializer have to share that line, and an empty block is `{}`.
fn join_payload_lines(text: &str) -> String {
    let mut lines = text.lines().map(|line| line.to_owned()).collect::<Vec<_>>();
    let mut index = 0;
    while index + 1 < lines.len() {
        let trimmed = lines[index].trim_end();
        let next = lines[index + 1].trim_start();
        // `const` alone is unfinished. `const name =` is already a binding.
        let join = trimmed.trim() == "const"
            || (trimmed.ends_with('=') && !trimmed.trim_end().ends_with("const"))
            || trimmed.ends_with("=>")
            || trimmed.ends_with(')')
            || (trimmed.ends_with('(') && next.starts_with("/*#__PURE__*/"));
        if join {
            let indent = lines[index].chars().take_while(|ch| *ch == ' ').count();
            let gap = if next.starts_with("/*#__PURE__*/") { "" } else { " " };
            lines[index] = format!("{}{trimmed}{gap}{next}", " ".repeat(indent));
            lines.remove(index + 1);
            continue;
        }
        index += 1;
    }
    babel_indent(&lines.join("\n"))
}

/// Babel's generator indents by 2. The compiler printer indents by 4.
fn babel_indent(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let spaces = line.chars().take_while(|c| *c == ' ').count();
        let trimmed = spaces / 4 * 2 + spaces % 4;
        lines.push(format!("{}{}", " ".repeat(trimmed), line[spaces..].trim_end()));
    }
    compact_metadata(&mut lines);
    // Babel leaves a blank line between a prologue directive and the next statement.
    let mut spaced = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        spaced.push(line.clone());
        let trimmed = line.trim();
        let directive = trimmed == "'use gpu';"
            || trimmed == "\"use gpu\";"
            || trimmed == "'worklet';"
            || trimmed == "\"worklet\";";
        if directive && lines.get(index + 1).is_some_and(|next| !next.trim().is_empty()) {
            spaced.push(String::new());
        }
    }
    let mut out = spaced.join("\n");
    // The printer writes a space after a synthetic comment. Babel does not.
    out = out.replace("/*#__PURE__*/ ", "/*#__PURE__*/").replace("( /*#__PURE__*/", "(/*#__PURE__*/");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Babel prints `{ type: "i", name: "a" }` on one line, then breaks the array as
/// `params: [{` / `  type: ...` / `}, {` / `}]`. Only those two-key objects move.
fn compact_metadata(lines: &mut Vec<String>) {
    let mut index = 0;
    while index + 3 < lines.len() {
        let open = lines[index].trim();
        let key1 = lines[index + 1].trim();
        let key2 = lines[index + 2].trim();
        let close = lines[index + 3].trim();
        if open == "{"
            && key1.starts_with("type:")
            && key2.starts_with("name:")
            && (close == "}," || close == "}")
        {
            let indent = lines[index].chars().take_while(|c| *c == ' ').count();
            let comma = if close.ends_with(',') { "," } else { "" };
            lines[index] = format!(
                "{}{{ {key1} {key2} }}{comma}",
                " ".repeat(indent)
            );
            lines.drain(index + 1..index + 4);
            continue;
        }
        index += 1;
    }
    index = 0;
    while index < lines.len() {
        if lines[index].trim() != "params: [" {
            index += 1;
            continue;
        }
        let mut objects = Vec::new();
        let mut cursor = index + 1;
        while cursor < lines.len() && lines[cursor].trim().starts_with("{") {
            objects.push(lines[cursor].trim().trim_end_matches(',').to_owned());
            cursor += 1;
        }
        let Some(close) = lines.get(cursor) else {
            break;
        };
        if close.trim() != "]" && close.trim() != "]," || objects.is_empty() {
            index += 1;
            continue;
        }
        let indent = " ".repeat(lines[index].chars().take_while(|c| *c == ' ').count());
        let mut replacement = vec![format!("{indent}params: [{{")];
        for (at, object) in objects.iter().enumerate() {
            let body = object.trim_start_matches('{').trim().trim_end_matches('}').trim();
            let mut keys = body.split(", ").filter(|part| !part.is_empty());
            let first = keys.next().unwrap_or("");
            let second = keys.next().unwrap_or("");
            let closer = if at + 1 == objects.len() {
                format!("{indent}}}],")
            } else {
                format!("{indent}}}, {{")
            };
            replacement.push(format!("{indent}  {first},"));
            replacement.push(format!("{indent}  {second}"));
            replacement.push(closer);
        }
        lines.splice(index..=cursor, replacement);
        index += 1;
    }
}

fn rebuild_list(list: crate::ast::NodeSlice, work: &mut Build) -> crate::ast::NodeList {
    let mut out = Vec::new();
    let mut hoisted = Vec::new();
    for statement in list.iter() {
        match rebuild_statement(statement, work) {
            Rebuilt::Keep => out.push(statement),
            Rebuilt::Node(node) => out.push(node),
            Rebuilt::Hoist { decl, export } => {
                hoisted.push(decl);
                if let Some(export) = export {
                    out.push(export);
                }
            }
        }
    }
    hoisted.append(&mut out);
    work.list(&hoisted)
}

enum Rebuilt {
    Keep,
    Node(Node),
    Hoist { decl: Node, export: Option<Node> },
}

fn rebuild_statement(node: Node, work: &mut Build) -> Rebuilt {
    if node.kind() == SyntaxKind::FunctionDeclaration && function_marked(node) && !already_wrapped(node) {
        // Same missing name as `function_expression`: do not read it.
        let name = if function_names_itself(node, work) {
            node.name()
        } else {
            Node::NIL
        };
        if name.kind() == SyntaxKind::Identifier {
            let wrapped = wrap_function(node, work, true);
            let decl = const_of(work, name.text(), wrapped);
            copy_leading(work, node, decl);
            let parent = node.parent();
            let export = if is_named_export(parent) {
                Some(export_name(work, name.text()))
            } else if parent.kind() == SyntaxKind::ExportAssignment && !parent.is_export_equals() {
                Some(export_default(work, work.id(name.text())))
            } else {
                None
            };
            if is_named_export(parent) || parent.kind() == SyntaxKind::ExportAssignment {
                copy_leading(work, parent, decl);
            }
            return Rebuilt::Hoist { decl, export };
        }
    }
    if let Some(rebuilt) = rebuild_node(node, work) {
        Rebuilt::Node(rebuilt)
    } else {
        Rebuilt::Keep
    }
}

fn rebuild_node(node: Node, work: &mut Build) -> Option<Node> {
    if node.is_nil() {
        return None;
    }
    if is_function_like_declaration(node) && function_marked(node) && !already_wrapped(node) {
        return Some(place_wrapped(node, wrap_function(node, work, true), work));
    }
    if is_shell_implementation(node, work) {
        let arg = unwrap_parens(node.arguments().get(0));
        if is_function_like_declaration(arg)
            && !function_marked(arg)
            && !already_wrapped(arg)
            && !function_has_meta(arg, work)
        {
            let wrapped = wrap_function(arg, work, false);
            let call = rebuild_call_shell(node, wrapped, work);
            return Some(call);
        }
    }
    if let Some(named) = auto_name(node, work) {
        return Some(named);
    }
    rebuild_children(node, work)
}

fn place_wrapped(node: Node, wrapped: Node, work: &mut Build) -> Node {
    let parent = node.parent();
    if parent.kind() == SyntaxKind::ExportAssignment && !parent.is_export_equals() {
        return export_default(work, wrapped);
    }
    if parent.kind() == SyntaxKind::VariableDeclaration && parent.name().kind() == SyntaxKind::Identifier {
        let statement = parent.parent().parent();
        if is_named_export(statement.parent()) {
            return keep_export_const(statement, parent.name().text(), wrapped, work);
        }
    }
    wrapped
}

fn keep_export_const(statement: Node, name: &str, init: Node, work: &mut Build) -> Node {
    let decl = const_of(work, name, init);
    let export = work.factory.new_export_declaration(
        crate::ast::ModifierList::NIL,
        false,
        decl,
        Node::NIL,
        Node::NIL,
    );
    copy_leading(work, statement.parent(), export);
    let _ = statement;
    export
}

fn const_of(work: &Build, name: &str, init: Node) -> Node {
    let declarator = work.factory.new_variable_declaration(
        work.id(name),
        Node::NIL,
        Node::NIL,
        init,
    );
    let list = work.factory.new_variable_declaration_list(
        work.list(&[declarator]),
        crate::flags::NodeFlags::CONST,
    );
    work.factory
        .new_variable_statement(crate::ast::ModifierList::NIL, list)
}

fn export_name(work: &Build, name: &str) -> Node {
    let spec = work
        .factory
        .new_export_specifier(false, Node::NIL, work.id(name));
    let clause = work.factory.new_named_exports(work.list(&[spec]));
    work.factory.new_export_declaration(
        crate::ast::ModifierList::NIL,
        false,
        clause,
        Node::NIL,
        Node::NIL,
    )
}

fn export_default(work: &Build, expression: Node) -> Node {
    work.factory.new_export_assignment(
        crate::ast::ModifierList::NIL,
        false,
        Node::NIL,
        expression,
    )
}

fn is_named_export(node: Node) -> bool {
    !node.is_nil() && node.kind() == SyntaxKind::ExportDeclaration && !node.is_export_equals()
        && node.module_specifier().is_nil()
        && node.export_clause().kind() != SyntaxKind::NamedExports
}

fn already_wrapped(node: Node) -> bool {
    if function_has_meta(node, &Build::probe()) {
        return true;
    }
    let mut cursor = node;
    for _ in 0..6 {
        cursor = cursor.parent();
        if cursor.is_nil() {
            break;
        }
        if cursor.kind() == SyntaxKind::CallExpression {
            let text = slice_node_raw(cursor);
            if text.contains("__TYPEGPU_META__") {
                return true;
            }
        }
    }
    false
}

/// The function text already contains the metadata call. Wrapping it again
/// would put `__TYPEGPU_META__` on both the shell and the callback.
fn function_has_meta(node: Node, work: &Build) -> bool {
    if node.is_nil() {
        return false;
    }
    if slice_node_raw(node).contains("__TYPEGPU_META__") {
        return true;
    }
    let Some(start) = byte_at(work.code.as_str(), node.pos()) else {
        return false;
    };
    let Some(end) = byte_at(work.code.as_str(), node.end()) else {
        return false;
    };
    work.code
        .get(start..end)
        .is_some_and(|slice| slice.contains("__TYPEGPU_META__"))
}

fn slice_node_raw(node: Node) -> String {
    let file = get_source_file_of_node(node);
    if file.is_nil() {
        return String::new();
    }
    let code = source_file_text(file);
    slice_node(code.as_ref(), node)
}

fn wrap_function(node: Node, work: &mut Build, marked: bool) -> Node {
    work.changed = true;
    let name = function_name(node, work);
    let ast = transpile_marked(node, &work.code);
    if let Some(message) = ast.unsupported.clone() {
        work.unsupported.push(message);
    }
    let _ = marked;
    // The runtime reads the author's operators from this function. The AST
    // already names them. Rebuilding the body would turn `+` into a call.
    let inner = function_expression(node, work);
    let meta = metadata_node(work, &name, &ast);
    pure_wrapper(work, inner, meta)
}

fn function_expression(node: Node, work: &mut Build) -> Node {
    // The printed function has to keep the author's operators. Cloning the
    // body rewrites `+`. The placeholder is replaced with the author's text
    // after the wrapper is printed.
    // The printer reads a block's statement list. An identifier has none.
    let placeholder = work.factory.new_expression_statement(work.id("__tgpu_body__"));
    let body = work.factory.new_block(work.list(&[placeholder]), false);
    let params = rebuild_params(node, work);
    match node.kind() {
        SyntaxKind::ArrowFunction => work.factory.new_arrow_function(
            crate::ast::ModifierList::NIL,
            crate::ast::NodeList::NIL,
            params,
            Node::NIL,
            Node::NIL,
            work.factory.new_token(SyntaxKind::EqualsGreaterThanToken),
            body,
        ),
        _ => {
            // `function (...)` and `export default function (...)` store a
            // missing name. Reading that child panics. A named function still
            // has an identifier before `(`.
            let name = if !function_names_itself(node, work) {
                Node::NIL
            } else {
                let own = node.name();
                if own.kind() == SyntaxKind::Identifier {
                    work.id(own.text())
                } else {
                    Node::NIL
                }
            };
            work.factory.new_function_expression(
                crate::ast::ModifierList::NIL,
                Node::NIL,
                name,
                crate::ast::NodeList::NIL,
                params,
                Node::NIL,
                Node::NIL,
                body,
            )
        }
    }
}

fn rebuild_params(node: Node, work: &Build) -> crate::ast::NodeList {
    let parameter_list = node.parameter_list();
    if parameter_list.is_nil() {
        return crate::ast::NodeList::NIL;
    }
    let params = parameter_list
        .nodes()
        .iter()
        .map(|param| {
            let name = param.name();
            let id = if name.kind() == SyntaxKind::Identifier {
                work.id(name.text())
            } else {
                work.id("_" )
            };
            let type_node = if param.type_().is_nil() {
                Node::NIL
            } else {
                // The wrapper is printed from a synthetic file. Reusing the
                // parsed type node asks the printer for text that file does
                // not have.
                type_from_slice(work, &slice_node(work.code.as_str(), param.type_()))
            };
            // Emitted JavaScript has no `?`. Reading `question_token` asks
            // for a postfix list the parameter does not have.
            let question = Node::NIL;
            work.factory.new_parameter_declaration(
                crate::ast::ModifierList::NIL,
                Node::NIL,
                id,
                question,
                type_node,
                Node::NIL,
            )
        })
        .collect::<Vec<_>>();
    work.list(&params)
}

fn rebuild_body(body: Node, work: &mut Build) -> Node {
    if body.kind() != SyntaxKind::Block {
        return clone_expr(body, work);
    }
    let mut statements = Vec::new();
    let mut hoisted = Vec::new();
    for statement in body.statements().iter() {
        if std::env::var("DBG").is_ok() {
            eprintln!("stmt {:?}", statement.kind());
        }
        if is_directive(statement) {
            statements.push(clone_statement(statement, work));
            continue;
        }
        match rebuild_statement(statement, work) {
            Rebuilt::Keep => statements.push(clone_statement(statement, work)),
            Rebuilt::Node(node) => statements.push(node),
            Rebuilt::Hoist { decl, export } => {
                hoisted.push(decl);
                if let Some(export) = export {
                    statements.push(export);
                }
            }
        }
    }
    hoisted.append(&mut statements);
    work.factory.new_block(work.list(&hoisted), true)
}

fn clone_statement(node: Node, work: &mut Build) -> Node {
    match node.kind() {
        SyntaxKind::ExpressionStatement => {
            let expr = clone_expr(node.expression(), work);
            let cloned = work.factory.new_expression_statement(expr);
            copy_leading(work, node, cloned);
            cloned
        }
        SyntaxKind::ReturnStatement => {
            let expr = node.expression();
            if std::env::var("DBG").is_ok() {
                let p = node.pos();
                let t = skip_trivia(work.code.as_str(), p);
                eprintln!("pos {p} trivia {t} slice {:?}", &work.code[p.max(0) as usize..t.max(0) as usize]);
            }
            let comment = comment_line_before(work.code.as_str(), node.pos());
            let cloned = if expr.is_nil() {
                work.factory.new_return_statement(Node::NIL)
            } else {
                let expr = match rebuild_expr(expr, work) {
                    Some(expr) => expr,
                    None => clone_expr(expr, work),
                };
                work.factory.new_return_statement(expr)
            };
            if let Some(body) = comment {
                work.emit.add_synthetic_leading_comment(
                    cloned,
                    SyntaxKind::SingleLineCommentTrivia,
                    &body,
                    true,
                );
            }
            cloned
        }
        SyntaxKind::VariableStatement => match rebuild_variable_statement(node, work) {
            Some(statement) => statement,
            None => {
                let list = node.declaration_list();
                let mut decls = Vec::new();
                for decl in list.declarations().nodes().iter() {
                    let init = decl.initializer();
                    let init = if init.is_nil() {
                        Node::NIL
                    } else {
                        match rebuild_expr(init, work) {
                            Some(init) => init,
                            None => clone_expr(init, work),
                        }
                    };
                    decls.push(work.factory.new_variable_declaration(
                        clone_name(decl.name(), work),
                        Node::NIL,
                        Node::NIL,
                        init,
                    ));
                }
                let flags = list.flags();
                let list = work
                    .factory
                    .new_variable_declaration_list(work.list(&decls), flags);
                work.factory
                    .new_variable_statement(crate::ast::ModifierList::NIL, list)
            }
        },
        SyntaxKind::IfStatement => {
            let test = node.expression();
            let test = match rebuild_expr(test, work) {
                Some(test) => test,
                None => clone_expr(test, work),
            };
            let then = clone_statement(node.then_statement(), work);
            let else_ = if node.else_statement().is_nil() {
                Node::NIL
            } else {
                clone_statement(node.else_statement(), work)
            };
            work.factory.new_if_statement(test, then, else_)
        }
        SyntaxKind::ForStatement => {
            let init = node.initializer();
            let init = if init.is_nil() {
                Node::NIL
            } else if init.kind() == SyntaxKind::VariableDeclarationList {
                clone_declaration_list(init, work)
            } else {
                clone_expr(init, work)
            };
            let condition = node.condition();
            let condition = if condition.is_nil() {
                Node::NIL
            } else {
                clone_expr(condition, work)
            };
            let incrementor = node.incrementor();
            let incrementor = if incrementor.is_nil() {
                Node::NIL
            } else {
                clone_expr(incrementor, work)
            };
            let body = node.statement();
            let statement = clone_statement(body, work);
            work.factory.new_for_statement(init, condition, incrementor, statement)
        }
        SyntaxKind::Block => rebuild_body(node, work),
        SyntaxKind::EmptyStatement => work.factory.new_empty_statement(),
        _ => work.factory.new_empty_statement(),
    }
}

fn clone_declaration_list(list: Node, work: &mut Build) -> Node {
    let decls = list.declarations().nodes();
    let mut built = Vec::new();
    for decl in decls.iter() {
        let init = decl.initializer();
        let init = if init.is_nil() {
            Node::NIL
        } else {
            match rebuild_expr(init, work) {
                Some(init) => init,
                None => clone_expr(init, work),
            }
        };
        let name = if decl.name().kind() == SyntaxKind::Identifier {
            decl.name().text().to_owned()
        } else {
            "_".to_owned()
        };
        built.push((name, init));
    }
    let flags = list.flags();
    let decls = built
        .into_iter()
        .map(|(name, init)| {
            work.factory.new_variable_declaration(
                work.id(&name),
                Node::NIL,
                Node::NIL,
                init,
            )
        })
        .collect::<Vec<_>>();
    work.factory
        .new_variable_declaration_list(work.list(&decls), flags)
}

/// Reprint a type annotation without the original node. The captured
/// cases only need keywords, identifiers, and `|` unions.
fn type_from_slice(work: &Build, text: &str) -> Node {
    let parts = text
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() > 1 {
        let mut node = type_atom(work, parts[0]);
        for part in &parts[1..] {
            node = work.factory.new_union_type_node(
                work.list(&[node, type_atom(work, part)]),
            );
        }
        return node;
    }
    type_atom(work, text.trim())
}

fn type_atom(work: &Build, text: &str) -> Node {
    let kind = match text {
        "number" => SyntaxKind::NumberKeyword,
        "string" => SyntaxKind::StringKeyword,
        "boolean" => SyntaxKind::BooleanKeyword,
        "undefined" => SyntaxKind::UndefinedKeyword,
        "void" => SyntaxKind::VoidKeyword,
        "any" => SyntaxKind::AnyKeyword,
        "unknown" => SyntaxKind::UnknownKeyword,
        "never" => SyntaxKind::NeverKeyword,
        "bigint" => SyntaxKind::BigIntKeyword,
        "object" => SyntaxKind::ObjectKeyword,
        "symbol" => SyntaxKind::SymbolKeyword,
        _ => {
            return work.factory.new_type_reference_node(
                work.id(text),
                crate::ast::NodeList::NIL,
            );
        }
    };
    work.factory.new_keyword_type_node(kind)
}

fn clone_name(node: Node, work: &Build) -> Node {
    if node.kind() == SyntaxKind::Identifier {
        work.id(node.text())
    } else {
        work.id("_")
    }
}

fn clone_expr(node: Node, work: &mut Build) -> Node {
    if node.is_nil() {
        return Node::NIL;
    }
    // `as` is a type assertion. The runtime text and the tinyest AST both
    // drop it, which is what the `as type` case prints.
    if node.kind() == SyntaxKind::AsExpression {
        if std::env::var("DBG").is_ok() { eprintln!("strip as"); }
        return clone_expr(node.expression(), work);
    }
    if let Some(rebuilt) = rebuild_expr(node, work) {
        return rebuilt;
    }
    let node = unwrap_parens(node);
    match node.kind() {
        SyntaxKind::Identifier => work.id(node.text()),
        SyntaxKind::NumericLiteral => work
            .factory
            .new_numeric_literal(slice_node(work.code.as_str(), node), crate::flags::TokenFlags::NONE),
        SyntaxKind::StringLiteral => {
            let raw = slice_node(work.code.as_str(), node);
            let flags = if raw.starts_with('\'') {
                crate::flags::TokenFlags::SINGLE_QUOTE
            } else {
                crate::flags::TokenFlags::NONE
            };
            work.factory.new_string_literal(node.text(), flags)
        }
        SyntaxKind::TrueKeyword => work.factory.new_keyword_expression(SyntaxKind::TrueKeyword),
        SyntaxKind::FalseKeyword => work.factory.new_keyword_expression(SyntaxKind::FalseKeyword),
        SyntaxKind::NullKeyword => work.factory.new_keyword_expression(SyntaxKind::NullKeyword),
        SyntaxKind::ThisKeyword => work.factory.new_keyword_expression(SyntaxKind::ThisKeyword),
        SyntaxKind::BinaryExpression => {
            let op = node.operator_token().kind();
            let left_node = node.left();
            let right_node = node.right();
            if std::env::var("DBG").is_ok() && node.operator_token().kind() == SyntaxKind::PlusToken {
                eprintln!("bin right {:?}", right_node.kind());
            }
            let left = clone_expr(left_node, work);
            let right = clone_expr(right_node, work);
            work.factory.new_binary_expression(
                crate::ast::ModifierList::NIL,
                left,
                Node::NIL,
                work.factory.new_token(op),
                right,
            )
        }
        SyntaxKind::PrefixUnaryExpression => {
            let op = node.operator();
            let operand = node.operand();
            let operand = clone_expr(operand, work);
            work.factory.new_prefix_unary_expression(op, operand)
        }
        SyntaxKind::PostfixUnaryExpression => {
            let op = node.operator();
            let operand = node.operand();
            let operand = clone_expr(operand, work);
            work.factory.new_postfix_unary_expression(operand, op)
        }
        SyntaxKind::PropertyAccessExpression => {
            let object = node.expression();
            let name = node.name().text().to_owned();
            let object = clone_expr(object, work);
            work.factory.new_property_access_expression(
                object,
                Node::NIL,
                work.id(&name),
                crate::flags::NodeFlags::NONE,
            )
        }
        SyntaxKind::ElementAccessExpression => {
            let object = node.expression();
            let index = node.argument_expression();
            let object = clone_expr(object, work);
            let index = clone_expr(index, work);
            work.factory.new_element_access_expression(
                object,
                Node::NIL,
                index,
                crate::flags::NodeFlags::NONE,
            )
        }
        SyntaxKind::CallExpression => {
            let callee = node.expression();
            let args = node.arguments().iter().collect::<Vec<_>>();
            let mut cloned = Vec::new();
            for arg in args {
                cloned.push(clone_expr(arg, work));
            }
            let callee = clone_expr(callee, work);
            work.factory.new_call_expression(
                callee,
                Node::NIL,
                crate::ast::NodeList::NIL,
                work.list(&cloned),
                crate::flags::NodeFlags::NONE,
            )
        }
        SyntaxKind::ParenthesizedExpression => {
            let inner = node.expression();
            clone_expr(inner, work)
        }
        SyntaxKind::AsExpression => {
            let inner = node.expression();
            clone_expr(inner, work)
        }
        SyntaxKind::ArrowFunction | SyntaxKind::FunctionExpression => {
            match rebuild_node(node, work) {
                Some(node) => node,
                None => function_expression(node, work),
            }
        }
        _ => work.id("_"),
    }
}

fn rebuild_expr(node: Node, work: &mut Build) -> Option<Node> {
    if node.is_nil() {
        return None;
    }
    if is_function_like_declaration(node) {
        return rebuild_node(node, work);
    }
    if node.kind() == SyntaxKind::BinaryExpression {
        if work.depth > 0
            && let Some((name, assign)) = overload(node.operator_token().kind())
        {
            let left_node = node.left();
            let right_node = node.right();
            let left = clone_expr(left_node, work);
            let right = clone_expr(right_node, work);
            work.changed = true;
            return Some(overload_call(work, name, left, right, assign));
        }
    }
    if node.kind() == SyntaxKind::CallExpression && is_shell_implementation(node, work) {
        return rebuild_node(node, work);
    }
    if let Some(named) = auto_name(node, work) {
        return Some(named);
    }
    None
}

fn overload_call(work: &Build, name: &str, left: Node, right: Node, assign: bool) -> Node {
    let call = work.factory.new_call_expression(
        work.id(name),
        Node::NIL,
        crate::ast::NodeList::NIL,
        work.list(&[left, right]),
        crate::flags::NodeFlags::NONE,
    );
    if assign {
        work.factory.new_binary_expression(
            crate::ast::ModifierList::NIL,
            left,
            Node::NIL,
            work.factory.new_token(SyntaxKind::EqualsToken),
            call,
        )
    } else {
        call
    }
}

fn rebuild_children(node: Node, work: &mut Build) -> Option<Node> {
    match node.kind() {
        SyntaxKind::Block | SyntaxKind::ModuleBlock | SyntaxKind::CaseBlock => {
            Some(rebuild_body(node, work))
        }
        SyntaxKind::ExpressionStatement => rebuild_expr(node.expression(), work).map(|expr| {
            work.factory.new_expression_statement(expr)
        }),
        SyntaxKind::ReturnStatement => {
            let expr = node.expression();
            if expr.is_nil() {
                None
            } else {
                rebuild_expr(expr, work).map(|expr| {
                    let ret = work.factory.new_return_statement(expr);
                    if let Some(body) = comment_line_before(work.code.as_str(), node.pos()) {
                        work.emit.add_synthetic_leading_comment(
                            ret,
                            SyntaxKind::SingleLineCommentTrivia,
                            &body,
                            true,
                        );
                    }
                    ret
                })
            }
        }
        SyntaxKind::VariableStatement => rebuild_variable_statement(node, work),
        SyntaxKind::IfStatement => {
            let test = rebuild_expr(node.expression(), work);
            let then = rebuild_statement_node(node.then_statement(), work);
            let else_ = if node.else_statement().is_nil() {
                None
            } else {
                Some(rebuild_statement_node(node.else_statement(), work))
            };
            if test.is_none() && then.is_none() && else_.is_none() {
                None
            } else {
                // A child that did not change is still a parsed node. The
                // wrapper printer has no text for it, so clone it.
                let test = test.unwrap_or_else(|| clone_expr(node.expression(), work));
                let then = then.unwrap_or_else(|| clone_statement(node.then_statement(), work));
                let else_ = match else_ {
                    Some(rebuilt) => rebuilt.unwrap_or(Node::NIL),
                    None => Node::NIL,
                };
                Some(work.factory.new_if_statement(test, then, else_))
            }
        }
        SyntaxKind::ExportDeclaration if node.export_clause().kind() != SyntaxKind::NamedExports => {
            let inner = node.export_clause();
            match rebuild_statement(inner, work) {
                Rebuilt::Hoist { .. } => None,
                Rebuilt::Node(rebuilt) => {
                    let export = work.factory.new_export_declaration(
                        crate::ast::ModifierList::NIL,
                        false,
                        rebuilt,
                        Node::NIL,
                        Node::NIL,
                    );
                    copy_leading(work, node, export);
                    Some(export)
                }
                Rebuilt::Keep => None,
            }
        }
        SyntaxKind::ExportAssignment if !node.is_export_equals() => {
            rebuild_expr(node.expression(), work).map(|expr| export_default(work, expr))
        }
        _ => None,
    }
}

fn rebuild_statement_node(node: Node, work: &mut Build) -> Option<Node> {
    match rebuild_statement(node, work) {
        Rebuilt::Node(node) => Some(node),
        Rebuilt::Keep | Rebuilt::Hoist { .. } => None,
    }
}

fn rebuild_variable_statement(node: Node, work: &mut Build) -> Option<Node> {
    let list = node.declaration_list();
    let mut decls = Vec::new();
    let mut changed = false;
    for decl in list.declarations().nodes().iter() {
        let init = decl.initializer();
        if let Some(rebuilt) = (!init.is_nil()).then(|| rebuild_expr(init, work)).flatten() {
            changed = true;
            decls.push(work.factory.new_variable_declaration(
                decl.name(),
                decl.exclamation_token(),
                decl.type_(),
                rebuilt,
            ));
        } else {
            decls.push(decl);
        }
    }
    if !changed {
        return None;
    }
    let flags = list.flags();
    let list = work
        .factory
        .new_variable_declaration_list(work.list(&decls), flags);
    let statement = work
        .factory
        .new_variable_statement(crate::ast::ModifierList::NIL, list);
    copy_leading(work, node, statement);
    Some(statement)
}

fn rebuild_call_shell(node: Node, implementation: Node, work: &mut Build) -> Node {
    let callee = node.expression();
    work.factory.new_call_expression(
        callee,
        Node::NIL,
        crate::ast::NodeList::NIL,
        work.list(&[implementation]),
        crate::flags::NodeFlags::NONE,
    )
}

fn pure_wrapper(work: &Build, function: Node, metadata: Node) -> Node {
    let dollar = work.id("$");
    let global = work.id("globalThis");
    let meta_name = work.id("__TYPEGPU_META__");
    let map = work.factory.new_new_expression(
        work.id("WeakMap"),
        work.nil_list(),
        work.empty(),
    );
    let access = work.factory.new_property_access_expression(
        global,
        Node::NIL,
        meta_name,
        crate::flags::NodeFlags::NONE,
    );
    let assign = work.factory.new_binary_expression(
        crate::ast::ModifierList::NIL,
        access,
        Node::NIL,
        work.factory
            .new_token(SyntaxKind::QuestionQuestionEqualsToken),
        map,
    );
    let set = work.factory.new_property_access_expression(
        assign,
        Node::NIL,
        work.id("set"),
        crate::flags::NodeFlags::NONE,
    );
    let slot = work.factory.new_property_access_expression(
        dollar,
        Node::NIL,
        work.id("f"),
        crate::flags::NodeFlags::NONE,
    );
    let stored = work.factory.new_binary_expression(
        crate::ast::ModifierList::NIL,
        slot,
        Node::NIL,
        work.factory.new_token(SyntaxKind::EqualsToken),
        function,
    );
    let set_call = work.factory.new_call_expression(
        set,
        Node::NIL,
        work.nil_list(),
        work.list(&[stored, metadata]),
        crate::flags::NodeFlags::NONE,
    );
    let f = work.factory.new_property_access_expression(
        dollar,
        Node::NIL,
        work.id("f"),
        crate::flags::NodeFlags::NONE,
    );
    let and = work.factory.new_binary_expression(
        crate::ast::ModifierList::NIL,
        set_call,
        Node::NIL,
        work.factory.new_token(SyntaxKind::AmpersandAmpersandToken),
        f,
    );
    let arrow = work.factory.new_arrow_function(
        crate::ast::ModifierList::NIL,
        work.nil_list(),
        work.list(&[work.factory.new_parameter_declaration(
            crate::ast::ModifierList::NIL,
            Node::NIL,
            dollar,
            Node::NIL,
            Node::NIL,
            Node::NIL,
        )]),
        Node::NIL,
        Node::NIL,
        work.factory.new_token(SyntaxKind::EqualsGreaterThanToken),
        and,
    );
    let call = work.factory.new_call_expression(
        arrow,
        Node::NIL,
        work.nil_list(),
        work.list(&[work.factory.new_object_literal_expression(work.empty(), false)]),
        crate::flags::NodeFlags::NONE,
    );
    mark_pure(work, call);
    call
}

fn mark_pure(work: &Build, node: Node) -> Node {
    work.emit.add_synthetic_leading_comment(
        node,
        SyntaxKind::MultiLineCommentTrivia,
        "#__PURE__",
        false,
    );
    node
}

fn metadata_node(work: &Build, name: &str, ast: &Tinyest) -> Node {
    let name_node = if name.is_empty() {
        work.id("undefined")
    } else {
        work.factory
            .new_string_literal(name, crate::flags::TokenFlags::NONE)
    };
    let props = [
        prop(work, "v", work.factory.new_numeric_literal("2", crate::flags::TokenFlags::NONE)),
        prop(work, "name", name_node),
        prop(work, "ast", ast_node(work, ast)),
        prop(work, "externals", externals_node(work, ast)),
    ];
    work.factory
        .new_object_literal_expression(work.list(&props), true)
}

fn prop(work: &Build, name: &str, value: Node) -> Node {
    work.factory.new_property_assignment(
        crate::ast::ModifierList::NIL,
        work.id(name),
        Node::NIL,
        Node::NIL,
        value,
    )
}

fn ast_node(work: &Build, ast: &Tinyest) -> Node {
    let params = ast
        .params
        .iter()
        .map(|name| {
            work.factory.new_object_literal_expression(
                work.list(&[
                    prop(
                        work,
                        "type",
                        work.factory
                            .new_string_literal("i", crate::flags::TokenFlags::NONE),
                    ),
                    prop(
                        work,
                        "name",
                        work.factory
                            .new_string_literal(name, crate::flags::TokenFlags::NONE),
                    ),
                ]),
                true,
            )
        })
        .collect::<Vec<_>>();
    let body = json_to_node(work, &ast.body);
    work.factory.new_object_literal_expression(
        work.list(&[prop(work, "params", work.factory.new_array_literal_expression(work.list(&params), !params.is_empty())), prop(work, "body", body)]),
        true,
    )
}

fn externals_node(work: &Build, ast: &Tinyest) -> Node {
    let props = ast
        .externals
        .iter()
        .map(|name| {
            let chain = external_access(work, name);
            let arrow = work.factory.new_arrow_function(
                crate::ast::ModifierList::NIL,
                crate::ast::NodeList::NIL,
                work.empty(),
                Node::NIL,
                Node::NIL,
                work.factory.new_token(SyntaxKind::EqualsGreaterThanToken),
                chain,
            );
            work.factory.new_property_assignment(
                crate::ast::ModifierList::NIL,
                work.factory
                    .new_string_literal(name, crate::flags::TokenFlags::NONE),
                Node::NIL,
                Node::NIL,
                arrow,
            )
        })
        .collect::<Vec<_>>();
    work.factory
        .new_object_literal_expression(work.list(&props), !props.is_empty())
}

fn external_access(work: &Build, name: &str) -> Node {
    let mut parts = name.split('.');
    let first = parts.next().unwrap_or(name);
    let mut node = if first == "this" {
        work.factory.new_keyword_expression(SyntaxKind::ThisKeyword)
    } else {
        work.id(first)
    };
    for part in parts {
        // The printer writes private-identifier text as stored. The parser
        // keeps the `#`, so a rebuilt access has to keep it too.
        let name = if let Some(private) = part.strip_prefix('#') {
            work.factory.new_private_identifier(format!("#{private}"))
        } else {
            work.id(part)
        };
        node = work.factory.new_property_access_expression(
            node,
            Node::NIL,
            name,
            crate::flags::NodeFlags::NONE,
        );
    }
    node
}

fn json_to_node(work: &Build, text: &str) -> Node {
    // Tinyest body is a JSON-like array already built as text. Parse the
    // small subset the numeric cases emit: arrays, strings, numbers, null.
    let mut parser = JsonNodes { work, rest: text.trim() };
    parser.value()
}

struct JsonNodes<'a> {
    work: &'a Build,
    rest: &'a str,
}

impl<'a> JsonNodes<'a> {
    fn value(&mut self) -> Node {
        self.rest = self.rest.trim_start();
        if let Some(rest) = self.rest.strip_prefix('[') {
            self.rest = rest;
            let mut items = Vec::new();
            loop {
                self.rest = self.rest.trim_start();
                if let Some(rest) = self.rest.strip_prefix(']') {
                    self.rest = rest;
                    break;
                }
                items.push(self.value());
                self.rest = self.rest.trim_start();
                if let Some(rest) = self.rest.strip_prefix(',') {
                    self.rest = rest;
                }
            }
            return self
                .work
                .factory
                .new_array_literal_expression(self.work.list(&items), false);
        }
        if let Some(rest) = self.rest.strip_prefix('"') {
            self.rest = rest;
            let end = self.rest.find('"').unwrap_or(self.rest.len());
            let text = self.rest[..end].to_owned();
            self.rest = &self.rest[end..];
            if self.rest.starts_with('"') {
                self.rest = &self.rest[1..];
            }
            return self
                .work
                .factory
                .new_string_literal(text, crate::flags::TokenFlags::NONE);
        }
        if let Some(rest) = self.rest.strip_prefix("null") {
            self.rest = rest;
            return self
                .work
                .factory
                .new_keyword_expression(SyntaxKind::NullKeyword);
        }
        if let Some(rest) = self.rest.strip_prefix("true") {
            self.rest = rest;
            return self
                .work
                .factory
                .new_keyword_expression(SyntaxKind::TrueKeyword);
        }
        if let Some(rest) = self.rest.strip_prefix("false") {
            self.rest = rest;
            return self
                .work
                .factory
                .new_keyword_expression(SyntaxKind::FalseKeyword);
        }
        if let Some(rest) = self.rest.strip_prefix('{') {
            self.rest = rest;
            let mut props = Vec::new();
            loop {
                self.rest = self.rest.trim_start();
                if let Some(rest) = self.rest.strip_prefix('}') {
                    self.rest = rest;
                    break;
                }
                let key_end = self
                    .rest
                    .find(|ch: char| ch == ':' || ch.is_whitespace())
                    .unwrap_or(self.rest.len());
                let key = self.rest[..key_end].to_owned();
                self.rest = self.rest[key_end..].trim_start();
                if let Some(rest) = self.rest.strip_prefix(':') {
                    self.rest = rest;
                }
                let value = self.value();
                props.push(self.work.factory.new_property_assignment(
                    crate::ast::ModifierList::NIL,
                    self.work.id(&key),
                    Node::NIL,
                    Node::NIL,
                    value,
                ));
                self.rest = self.rest.trim_start();
                if let Some(rest) = self.rest.strip_prefix(',') {
                    self.rest = rest;
                }
            }
            return self
                .work
                .factory
                .new_object_literal_expression(self.work.list(&props), true);
        }
        let end = self
            .rest
            .find(|ch: char| !ch.is_ascii_digit() && ch != '.' && ch != '-')
            .unwrap_or(self.rest.len());
        let text = if end == 0 { "0" } else { &self.rest[..end] };
        let owned = text.to_owned();
        self.rest = &self.rest[end.min(self.rest.len())..];
        self.work
            .factory
            .new_numeric_literal(owned, crate::flags::TokenFlags::NONE)
    }
}

fn comment_line_before(code: &str, pos: i32) -> Option<String> {
    let start = usize::try_from(pos.max(0)).unwrap_or(0).min(code.len());
    let token = usize::try_from(skip_trivia(code, pos).max(0)).unwrap_or(start).min(code.len());
    let trivia = if token > start { &code[start..token] } else { "" };
    for line in trivia.lines() {
        if let Some(body) = line.trim().strip_prefix("//") {
            return Some(format!(" {}", body.strip_prefix(' ').unwrap_or(body)));
        }
    }
    let before = code[..start].trim_end();
    let line = before.rsplit_once('\n').map(|(_, line)| line.trim())?;
    line.strip_prefix("//").map(|body| {
        format!(" {}", body.strip_prefix(' ').unwrap_or(body))
    })
}

fn copy_leading(work: &Build, from: Node, to: Node) {
    // The comment sits in trivia before the token. Scanning at `pos` misses
    // it when `pos` is already the token, so walk back over whitespace first.
    let code = work.code.as_str();
    let token = usize::try_from(from.pos().max(0)).unwrap_or(0).min(code.len());
    let before = code[..token].trim_end();
    if let Some(line) = before.rsplit_once('\n').map(|(_, line)| line.trim())
        && let Some(body) = line.strip_prefix("//")
    {
        work.emit.add_synthetic_leading_comment(
            to,
            SyntaxKind::SingleLineCommentTrivia,
            body.trim(),
            true,
        );
        return;
    }
    let mut ranges = leading_comments(code, from.pos());
    for (start, end) in ranges {
        let raw = &work.code[start..end];
        let (kind, body, trailing) = if let Some(body) = raw.strip_prefix("//") {
            (
                SyntaxKind::SingleLineCommentTrivia,
                body.trim().to_owned(),
                true,
            )
        } else if let Some(body) = raw
            .strip_prefix("/*")
            .and_then(|rest| rest.strip_suffix("*/"))
        {
            (
                SyntaxKind::MultiLineCommentTrivia,
                body.trim().to_owned(),
                true,
            )
        } else {
            continue;
        };
        work.emit
            .add_synthetic_leading_comment(to, kind, &body, trailing);
    }
}

const RESOURCE_CONSTRUCTORS: &[&str] = &[
    "bindGroupLayout", "vertexLayout", "privateVar", "workgroupVar", "immediateVar", "const",
    "slot", "accessor", "mutableAccessor", "comptime", "fn", "vertexFn", "fragmentFn", "computeFn",
    "createBuffer", "createMutable", "createReadonly", "createUniform", "createQuerySet",
    "createPipeline", "createComputePipeline", "createGuardedComputePipeline", "createRenderPipeline",
    "createTexture", "createSampler", "createComparisonSampler", "struct", "unstruct", "createView",
];

fn auto_name(node: Node, work: &mut Build) -> Option<Node> {
    if !work.auto_name {
        return None;
    }
    let (label, expression) = labelled(node)?;
    if !contains_resource(expression, work) || already_autonamed(expression) {
        return None;
    }
    work.changed = true;
    Some(replace_labelled(node, wrap_auto_name(work, expression, &label), work))
}

fn labelled(node: Node) -> Option<(String, Node)> {
    match node.kind() {
        SyntaxKind::VariableDeclaration
            if node.name().kind() == SyntaxKind::Identifier && !node.initializer().is_nil() =>
        {
            Some((node.name().text().to_owned(), node.initializer()))
        }
        SyntaxKind::BinaryExpression if node.operator_token().kind() == SyntaxKind::EqualsToken => {
            let name = try_identifier(node.left())?;
            Some((name, node.right()))
        }
        SyntaxKind::PropertyAssignment if node.name().kind() == SyntaxKind::Identifier => {
            Some((node.name().text().to_owned(), node.initializer()))
        }
        // A class property: `class C { myBuffer = root.createUniform(d.u32) }`,
        // also the private `#const = tgpu.const(d.u32, 1)`. Upstream names both
        // (`ClassProperty` and `ClassPrivateProperty`), the private one
        // without its `#`.
        SyntaxKind::PropertyDeclaration if !node.initializer().is_nil() => {
            let name = try_identifier(node.name())?;
            Some((name, node.initializer()))
        }
        _ => None,
    }
}

fn try_identifier(node: Node) -> Option<String> {
    match node.kind() {
        SyntaxKind::Identifier => Some(node.text().to_owned()),
        SyntaxKind::PrivateIdentifier => Some(node.text().trim_start_matches('#').to_owned()),
        SyntaxKind::PropertyAccessExpression => try_identifier(node.name()),
        _ => None,
    }
}

fn contains_resource(node: Node, work: &Build) -> bool {
    // Go upstream: `containsResourceConstructorCall` also descends through a
    // tagged template's tag, so `tgpu.fn(...)`\`wgsl\`.$uses({...})` is one
    // resource.
    if node.kind() == SyntaxKind::TaggedTemplateExpression {
        return contains_resource(node.tag(), work);
    }
    if node.kind() != SyntaxKind::CallExpression {
        return false;
    }
    // A shell implementation is a named resource whatever the tgpu object is
    // called. Upstream returns true here before it looks at the callee, so
    // `g.fn([])(impl)` is named as `tgpu.fn([])(impl)` is.
    if is_shell_implementation(node, work) {
        return true;
    }
    let callee = unwrap_parens(node.expression());
    if callee.kind() == SyntaxKind::Identifier {
        return RESOURCE_CONSTRUCTORS.contains(&callee.text());
    }
    if callee.kind() == SyntaxKind::PropertyAccessExpression {
        let property = callee.name();
        if property.kind() == SyntaxKind::Identifier {
            if property.text() == "$name" {
                return false;
            }
            if RESOURCE_CONSTRUCTORS.contains(&property.text()) {
                return true;
            }
        }
        return contains_resource(callee.expression(), work);
    }
    false
}

fn already_autonamed(node: Node) -> bool {
    let parent = node.parent();
    parent.kind() == SyntaxKind::CallExpression && slice_node_raw(parent).contains("__TYPEGPU_AUTONAME__")
}

fn wrap_auto_name(work: &Build, expression: Node, name: &str) -> Node {
    let hook = work.factory.new_property_access_expression(
        work.id("globalThis"),
        Node::NIL,
        work.id("__TYPEGPU_AUTONAME__"),
        crate::flags::NodeFlags::NONE,
    );
    let identity = work.factory.new_arrow_function(
        crate::ast::ModifierList::NIL,
        crate::ast::NodeList::NIL,
        work.list(&[work.factory.new_parameter_declaration(
            crate::ast::ModifierList::NIL,
            Node::NIL,
            work.id("a"),
            Node::NIL,
            Node::NIL,
            Node::NIL,
        )]),
        Node::NIL,
        Node::NIL,
        work.factory.new_token(SyntaxKind::EqualsGreaterThanToken),
        work.id("a"),
    );
    let callee = work.factory.new_binary_expression(
        crate::ast::ModifierList::NIL,
        hook,
        Node::NIL,
        work.factory.new_token(SyntaxKind::QuestionQuestionToken),
        identity,
    );
    work.factory.new_call_expression(
        callee,
        Node::NIL,
        crate::ast::NodeList::NIL,
        work.list(&[
            expression,
            work.factory
                .new_string_literal(name, crate::flags::TokenFlags::NONE),
        ]),
        crate::flags::NodeFlags::NONE,
    )
}

fn replace_labelled(node: Node, expression: Node, work: &Build) -> Node {
    match node.kind() {
        SyntaxKind::VariableDeclaration => work.factory.new_variable_declaration(
            node.name(),
            node.exclamation_token(),
            node.type_(),
            expression,
        ),
        SyntaxKind::BinaryExpression => work.factory.new_binary_expression(
            crate::ast::ModifierList::NIL,
            node.left(),
            Node::NIL,
            node.operator_token(),
            expression,
        ),
        SyntaxKind::PropertyAssignment => work.factory.new_property_assignment(
            crate::ast::ModifierList::NIL,
            node.name(),
            Node::NIL,
            Node::NIL,
            expression,
        ),
        _ => expression,
    }
}

fn gather_aliases(node: Node, aliases: &mut std::collections::BTreeSet<String>) {
    if node.is_nil() {
        return;
    }
    if node.kind() == SyntaxKind::ImportDeclaration {
        let spec = node.module_specifier();
        if spec.kind() == SyntaxKind::StringLiteral && spec.text() == "typegpu" {
            let clause = node.import_clause();
            if !clause.is_nil() {
                let name = clause.name();
                if !name.is_nil() && name.kind() == SyntaxKind::Identifier {
                    aliases.insert(name.text().to_owned());
                }
                let named = clause.named_bindings();
                if !named.is_nil() && named.kind() == SyntaxKind::NamespaceImport {
                    let local = named.name();
                    if !local.is_nil() {
                        aliases.insert(format!("{}.tgpu", local.text()));
                    }
                }
                if !named.is_nil() && named.kind() == SyntaxKind::NamedImports {
                    for spec in named.elements().iter() {
                        let imported = spec.property_name();
                        let local = spec.name();
                        if local.is_nil() {
                            continue;
                        }
                        let imported = if !imported.is_nil() && imported.kind() == SyntaxKind::Identifier {
                            imported.text()
                        } else {
                            local.text()
                        };
                        if imported == "tgpu" {
                            aliases.insert(local.text().to_owned());
                        }
                    }
                }
            }
        }
    }
    node.for_each_child(|child| {
        if child != node {
            gather_aliases(child, aliases);
        }
        false
    });
}

fn is_tgpu_alias(node: Node, work: &Build) -> bool {
    let mut path = String::new();
    let mut tail = unwrap_parens(node);
    loop {
        if tail.kind() == SyntaxKind::ElementAccessExpression
            && tail.argument_expression().kind() == SyntaxKind::StringLiteral
            && tail.argument_expression().text() == "~unstable"
        {
            // `tgpu["~unstable"]` bypasses that step of the path, as
            // upstream's `isTgpu` does.
            tail = tail.expression();
            continue;
        }
        if tail.kind() == SyntaxKind::PropertyAccessExpression {
            let property = tail.name();
            if property.kind() != SyntaxKind::Identifier {
                break;
            }
            path = if path.is_empty() {
                property.text().to_owned()
            } else {
                format!("{}.{path}", property.text())
            };
            tail = tail.expression();
        } else if tail.kind() == SyntaxKind::Identifier {
            path = if path.is_empty() {
                tail.text().to_owned()
            } else {
                format!("{}.{path}", tail.text())
            };
            break;
        } else {
            break;
        }
    }
    // Only an alias gathered from an `import ... from 'typegpu'` counts, as
    // upstream gathers it. A bare `tgpu` name is not a typegpu object.
    work.aliases.contains(&path)
}

fn is_shell_implementation(node: Node, work: &Build) -> bool {
    if node.kind() != SyntaxKind::CallExpression || node.arguments().len() != 1 {
        return false;
    }
    let callee = unwrap_parens(node.expression());
    if callee.kind() != SyntaxKind::CallExpression {
        return false;
    }
    let member = unwrap_parens(callee.expression());
    if member.kind() != SyntaxKind::PropertyAccessExpression {
        return false;
    }
    let property = member.name();
    if property.kind() != SyntaxKind::Identifier || !SHELLS.contains(&property.text()) {
        return false;
    }
    is_tgpu_alias(member.expression(), work)
}

fn function_marked(node: Node) -> bool {
    let body = node.body();
    if body.is_nil() || body.kind() != SyntaxKind::Block {
        return false;
    }
    if !body.can_have_statements() {
        return false;
    }
    let statements = body.statement_list();
    if statements.is_nil() {
        return false;
    }
    for statement in statements.nodes().iter() {
        if statement.kind() != SyntaxKind::ExpressionStatement {
            break;
        }
        if is_use_gpu(statement.expression()) {
            return true;
        }
        if statement.expression().kind() != SyntaxKind::StringLiteral {
            break;
        }
    }
    false
}

fn declaration_names_itself(slice: &str) -> bool {
    let Some(at) = slice.find("function") else {
        return true;
    };
    let after = slice[at + "function".len()..].trim_start();
    after.starts_with(|ch: char| ch == '_' || ch.is_ascii_alphabetic())
}

fn function_names_itself(node: Node, work: &Build) -> bool {
    if node.kind() == SyntaxKind::ArrowFunction {
        return false;
    }
    let Some(start) = byte_at(work.code.as_str(), skip_trivia(work.code.as_str(), node.pos())) else {
        return false;
    };
    let Some(end) = byte_at(work.code.as_str(), node.end()) else {
        return false;
    };
    declaration_names_itself(&work.code[start..end])
}

fn function_name(node: Node, work: &Build) -> String {
    // `function (...)` has a missing name child. Reading it panics.
    if function_names_itself(node, work) {
        let own = node.name();
        if !own.is_nil() && own.kind() == SyntaxKind::Identifier {
            return own.text().to_owned();
        }
    }
    let parent = node.parent();
    match parent.kind() {
        SyntaxKind::VariableDeclaration | SyntaxKind::PropertyAssignment | SyntaxKind::BindingElement => {
            let name = parent.name();
            if name.kind() == SyntaxKind::Identifier {
                return name.text().to_owned();
            }
        }
        SyntaxKind::BinaryExpression
            if parent.operator_token().kind() == SyntaxKind::EqualsToken
                && parent.left().kind() == SyntaxKind::Identifier =>
        {
            return parent.left().text().to_owned();
        }
        _ => {}
    }
    String::new()
}

const SHELLS: &[&str] = &["fn", "vertexFn", "fragmentFn", "computeFn"];

fn unwrap_parens(mut node: Node) -> Node {
    while node.kind() == SyntaxKind::ParenthesizedExpression {
        node = node.expression();
    }
    node
}

fn leading_comments(code: &str, pos: i32) -> Vec<(usize, usize)> {
    let ranges = crate::frontend::scanner::get_leading_comment_ranges(
        &crate::ast::NodeFactory::default(),
        code,
        pos,
    );
    ranges
        .into_iter()
        .filter_map(|range| {
            let start = usize::try_from(range.pos().max(0)).ok()?;
            let end = usize::try_from(range.end().max(0)).ok()?;
            (end <= code.len() && start <= end).then_some((start, end))
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
struct Tinyest {
    params: Vec<String>,
    body: String,
    externals: Vec<String>,
    /// Upstream's transpiler throws on a shape it cannot express. The text is
    /// the error a caller should surface; the body is empty then.
    unsupported: Option<String>,
}

impl Tinyest {
    fn json(&self) -> String {
        let params = if self.params.is_empty() {
            "[]".to_owned()
        } else {
            format!(
                "[{}]",
                self.params
                    .iter()
                    .map(|name| format!(
                        "{{\n      type: \"i\",\n      name: \"{name}\"\n    }}"
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "{{\n    params: {params},\n    body: {}\n  }}",
            if self.body.is_empty() {
                "[0, []]"
            } else {
                &self.body
            }
        )
    }

    fn externals_js(&self) -> String {
        if self.externals.is_empty() {
            return "{}".to_owned();
        }
        format!(
            "{{\n    {}\n  }}",
            self.externals
                .iter()
                .map(|name| format!("\"{name}\": () => {name}"))
                .collect::<Vec<_>>()
                .join(",\n    ")
        )
    }
}

/// Enough of tinyest-for-wgsl for the numeric captured cases. Not full WGSL.
fn transpile_marked(node: Node, code: &str) -> Tinyest {
    let parameter_list = node.parameter_list();
    let params = if parameter_list.is_nil() {
        Vec::new()
    } else {
        parameter_list
            .nodes()
            .iter()
            .filter_map(|param| {
                let name = param.name();
                (name.kind() == SyntaxKind::Identifier).then(|| name.text().to_owned())
            })
            .collect::<Vec<_>>()
    };
    let body = node.body();
    let mut ctx = TinyCtx {
        declared: params.clone(),
        externals: Vec::new(),
        unsupported: None,
    };
    let statements = if body.kind() == SyntaxKind::Block {
        body.statements()
            .iter()
            .filter(|statement| !is_directive(*statement))
            .filter_map(|statement| transpile_statement(statement, code, &mut ctx))
            .collect::<Vec<_>>()
    } else if !body.is_nil() {
        vec![format!("[10, {}]", transpile_expr(body, code, &mut ctx))]
    } else {
        Vec::new()
    };
    if let Some(message) = ctx.unsupported.clone() {
        // `typegpu.md`: an empty `ast` means this shader cannot be printed.
        return Tinyest {
            params,
            body: "[0, []]".to_owned(),
            externals: Vec::new(),
            unsupported: Some(message),
        };
    }
    Tinyest {
        params,
        body: format!("[0, [{}]]", statements.join(", ")),
        externals: ctx.externals,
        unsupported: None,
    }
}

struct TinyCtx {
    declared: Vec<String>,
    externals: Vec<String>,
    /// Set when a nested function or another shape upstream's transpiler
    /// refuses was found. The body cannot be printed then.
    unsupported: Option<String>,
}

fn is_directive(statement: Node) -> bool {
    statement.kind() == SyntaxKind::ExpressionStatement
        && statement.expression().kind() == SyntaxKind::StringLiteral
}

fn transpile_statement(node: Node, code: &str, ctx: &mut TinyCtx) -> Option<String> {
    match node.kind() {
        SyntaxKind::EmptyStatement | SyntaxKind::NotEmittedStatement => None,
        SyntaxKind::ExpressionStatement => Some(transpile_expr(node.expression(), code, ctx)),
        SyntaxKind::ReturnStatement => {
            let expr = node.expression();
            Some(if expr.is_nil() {
                "[10]".to_owned()
            } else {
                format!("[10, {}]", transpile_expr(expr, code, ctx))
            })
        }
        SyntaxKind::VariableStatement => {
            let list = node.declaration_list();
            // Upstream: "`var` declarations are not supported." The tinyest
            // catalog has `let` and `const` only, and WGSL has `let`.
            if !list.flags().intersects(crate::flags::NodeFlags::CONST)
                && !list.flags().intersects(crate::flags::NodeFlags::LET)
            {
                ctx.unsupported
                    .get_or_insert_with(|| "`var` declarations are not supported.".to_owned());
                return None;
            }
            let decl = list.declarations().nodes().get(0);
            if decl.is_nil() {
                return None;
            }
            let id = decl.name();
            let name = if id.kind() == SyntaxKind::Identifier {
                id.text().to_owned()
            } else {
                slice_node(code, id)
            };
            ctx.declared.push(name.clone());
            let kind = if list.flags().intersects(crate::flags::NodeFlags::CONST) {
                13
            } else {
                12
            };
            let init = decl.initializer();
            Some(if init.is_nil() {
                format!("[{kind}, \"{name}\"]")
            } else {
                format!("[{kind}, \"{name}\", {}]", transpile_expr(init, code, ctx))
            })
        }
        SyntaxKind::IfStatement => {
            let test = transpile_expr(node.expression(), code, ctx);
            let then = statement_or_block(node.then_statement(), code, ctx);
            let else_ = node.else_statement();
            Some(if else_.is_nil() {
                format!("[11, {test}, {then}]")
            } else {
                format!("[11, {test}, {then}, {}]", statement_or_block(else_, code, ctx))
            })
        }
        SyntaxKind::WhileStatement => Some(format!(
            "[15, {}, {}]",
            transpile_expr(node.expression(), code, ctx),
            statement_or_block(node.statement(), code, ctx)
        )),
        SyntaxKind::ForStatement => {
            let init = node.initializer();
            let init_js = if init.is_nil() {
                "null".to_owned()
            } else if init.kind() == SyntaxKind::VariableDeclarationList {
                transpile_for_init(init, code, ctx)
            } else {
                transpile_expr(init, code, ctx)
            };
            let test = node.condition();
            let test_js = if test.is_nil() {
                "null".to_owned()
            } else {
                transpile_expr(test, code, ctx)
            };
            let update = node.incrementor();
            let update_js = if update.is_nil() {
                "null".to_owned()
            } else {
                transpile_expr(update, code, ctx)
            };
            Some(format!(
                "[14, {init_js}, {test_js}, {update_js}, {}]",
                statement_or_block(node.statement(), code, ctx)
            ))
        }
        SyntaxKind::ContinueStatement => Some("[16]".to_owned()),
        SyntaxKind::BreakStatement => Some("[17]".to_owned()),
        SyntaxKind::ForOfStatement => {
            // [18, declaration, iterable, body]. Upstream records the binding
            // as a const/let without an initializer.
            let init = node.initializer();
            let declaration = if init.kind() == SyntaxKind::VariableDeclarationList {
                let list = init;
                let decl = list.declarations().nodes().get(0);
                let kind = if list.flags().intersects(crate::flags::NodeFlags::CONST) {
                    13
                } else {
                    12
                };
                if decl.is_nil() {
                    format!("[{kind}]")
                } else {
                    let name = decl.name();
                    let name = if name.kind() == SyntaxKind::Identifier {
                        name.text().to_owned()
                    } else {
                        slice_node(code, name)
                    };
                    ctx.declared.push(name.clone());
                    format!("[{kind}, \"{name}\"]")
                }
            } else {
                transpile_expr(init, code, ctx)
            };
            Some(format!(
                "[18, {declaration}, {}, {}]",
                transpile_expr(node.expression(), code, ctx),
                statement_or_block(node.statement(), code, ctx)
            ))
        }
        SyntaxKind::SwitchStatement => {
            // [19, expression, [[test, [statements]], ..., [null, []]]].
            let mut clauses = Vec::new();
            for clause in node.case_block().clauses().nodes() {
                let test = if clause.kind() == SyntaxKind::DefaultClause {
                    "null".to_owned()
                } else {
                    transpile_expr(clause.expression(), code, ctx)
                };
                let mut statements = Vec::new();
                for statement in clause.statements().iter() {
                    if let Some(text) = transpile_statement(statement, code, ctx) {
                        statements.push(text);
                    }
                }
                clauses.push(format!("[{test}, [{}]]", statements.join(", ")));
            }
            Some(format!(
                "[19, {}, [{}]]",
                transpile_expr(node.expression(), code, ctx),
                clauses.join(", ")
            ))
        }
        SyntaxKind::Block => Some(transpile_block(node, code, ctx)),
        // A statement the tinyest AST has no node for. Upstream's transpiler
        // throws ("Unsupported JS functionality: FunctionDeclaration"), so
        // skipping it silently would print a shader that is missing work.
        _ => {
            // Upstream: `Unsupported JS functionality: ${node.type}`.
            ctx.unsupported
                .get_or_insert_with(|| unsupported_message(node.kind()));
            None
        }
    }
}

/// The text upstream's tinyest transpiler throws. Babel's `node.type` is not
/// our `SyntaxKind` name: a `do/while` is `DoWhileStatement` there, and a
/// template is `TemplateLiteral`.
fn unsupported_message(kind: SyntaxKind) -> String {
    match kind {
        SyntaxKind::ArrowFunction => "Arrow functions are not supported inside TGSL.".to_owned(),
        SyntaxKind::SpreadElement | SyntaxKind::SpreadAssignment | SyntaxKind::OmittedExpression => {
            "Spread elements are not supported in TGSL.".to_owned()
        }
        SyntaxKind::MethodDeclaration | SyntaxKind::GetAccessor | SyntaxKind::SetAccessor => {
            "Object method elements are not supported in TGSL.".to_owned()
        }
        SyntaxKind::DoStatement => "Unsupported JS functionality: DoWhileStatement".to_owned(),
        SyntaxKind::TemplateExpression
        | SyntaxKind::NoSubstitutionTemplateLiteral
        | SyntaxKind::TaggedTemplateExpression => {
            "Unsupported JS functionality: TemplateLiteral".to_owned()
        }
        other => format!("Unsupported JS functionality: {other:?}"),
    }
}

fn transpile_for_init(list: Node, code: &str, ctx: &mut TinyCtx) -> String {
    let decl = list.declarations().nodes().get(0);
    if decl.is_nil() {
        return "null".to_owned();
    }
    let name = decl.name().text().to_owned();
    ctx.declared.push(name.clone());
    let init = decl.initializer();
    if init.is_nil() {
        "[12, \"".to_owned() + &name + "\"]"
    } else {
        format!("[12, \"{name}\", {}]", transpile_expr(init, code, ctx))
    }
}

fn statement_or_block(node: Node, code: &str, ctx: &mut TinyCtx) -> String {
    if node.kind() == SyntaxKind::Block {
        transpile_block(node, code, ctx)
    } else {
        transpile_statement(node, code, ctx).unwrap_or_else(|| "[0, []]".to_owned())
    }
}

fn transpile_block(node: Node, code: &str, ctx: &mut TinyCtx) -> String {
    let statements = node
        .statements()
        .iter()
        .filter(|statement| !is_directive(*statement))
        .filter_map(|statement| transpile_statement(statement, code, ctx))
        .collect::<Vec<_>>();
    format!("[0, [{}]]", statements.join(", "))
}

fn transpile_expr(node: Node, code: &str, ctx: &mut TinyCtx) -> String {
    let mut node = unwrap_parens(node);
    if matches!(node.kind(), SyntaxKind::AsExpression | SyntaxKind::SatisfiesExpression | SyntaxKind::TypeAssertionExpression) {
        node = node.expression();
    }
    if let Some(chain) = external_chain(node, ctx) {
        if !ctx.externals.contains(&chain) {
            ctx.externals.push(chain.clone());
        }
        return format!("\"{chain}\"");
    }
    // `this.myBuffer.$` is one external plus a dereference. The `$` is not a
    // member of the external, and it is not a second external of its own.
    if node.kind() == SyntaxKind::PropertyAccessExpression {
        let property = node.name();
        if property.kind() == SyntaxKind::Identifier && property.text() == "$"
            && let Some(chain) = external_chain(unwrap_parens(node.expression()), ctx)
        {
            if !ctx.externals.contains(&chain) {
                ctx.externals.push(chain.clone());
            }
            return format!("[7, \"{chain}\", \"$\"]");
        }
    }
    match node.kind() {
        SyntaxKind::Identifier => format!("\"{}\"", node.text()),
        SyntaxKind::NumericLiteral => format!("[5, \"{}\"]", numeric_text(node, code)),
        SyntaxKind::StringLiteral => format!("[103, \"{}\"]", node.text()),
        SyntaxKind::TrueKeyword => "true".to_owned(),
        SyntaxKind::FalseKeyword => "false".to_owned(),
        SyntaxKind::NullKeyword => "[106]".to_owned(),
        SyntaxKind::BinaryExpression => {
            let op_kind = node.operator_token().kind();
            let right_node = node.right();
            if op_kind == SyntaxKind::EqualsToken && is_restored_assign(right_node) {
                return transpile_expr(right_node, code, ctx);
            }
            let op = operator_text(op_kind);
            let left = transpile_expr(node.left(), code, ctx);
            let right = transpile_expr(right_node, code, ctx);
            let kind = if is_assignment(node.operator_token().kind()) {
                2
            } else if is_logical(node.operator_token().kind()) {
                3
            } else {
                1
            };
            format!("[{kind}, {left}, \"{op}\", {right}]")
        }
        SyntaxKind::PrefixUnaryExpression => {
            if matches!(
                node.operator(),
                SyntaxKind::PlusPlusToken | SyntaxKind::MinusMinusToken
            ) {
                // Upstream: "Prefix update expressions are not supported in
                // WGSL." `a++` is catalog kind 102 and stays supported.
                ctx.unsupported.get_or_insert_with(|| {
                    "Prefix update expressions are not supported in WGSL.".to_owned()
                });
                return format!("\"{}\"", slice_node(code, node));
            }
            format!(
                "[4, \"{}\", {}]",
                operator_text(node.operator()),
                transpile_expr(node.operand(), code, ctx)
            )
        }
        SyntaxKind::PostfixUnaryExpression => {
            format!(
                "[102, \"{}\", {}]",
                operator_text(node.operator()),
                transpile_expr(node.operand(), code, ctx)
            )
        }
        SyntaxKind::PropertyAccessExpression => {
            let property = node.name().text();
            if property == "$" {
                return format!(
                    "[7, {}, \"$\"]",
                    transpile_expr(node.expression(), code, ctx)
                );
            }
            format!(
                "[7, {}, \"{property}\"]",
                transpile_expr(node.expression(), code, ctx)
            )
        }
        SyntaxKind::ElementAccessExpression => format!(
            "[8, {}, {}]",
            transpile_expr(node.expression(), code, ctx),
            transpile_expr(node.argument_expression(), code, ctx)
        ),
        SyntaxKind::CallExpression => {
            // The operator rewrite already ran. The runtime AST still names
            // the author's operator, not `__tsover_*`.
            if let Some(restored) = restore_overload(node, code, ctx) {
                return restored;
            }
            let args = node
                .arguments()
                .iter()
                .map(|arg| transpile_expr(arg, code, ctx))
                .collect::<Vec<_>>();
            format!(
                "[6, {}, [{}]]",
                transpile_expr(node.expression(), code, ctx),
                args.join(", ")
            )
        }
        SyntaxKind::ConditionalExpression => format!(
            "[105, {}, {}, {}]",
            transpile_expr(node.condition(), code, ctx),
            transpile_expr(node.when_true(), code, ctx),
            transpile_expr(node.when_false(), code, ctx)
        ),
        SyntaxKind::ArrayLiteralExpression => {
            let items = node
                .elements()
                .iter()
                .map(|item| transpile_expr(item, code, ctx))
                .collect::<Vec<_>>();
            format!("[100, [{}]]", items.join(", "))
        }
        SyntaxKind::ObjectLiteralExpression => {
            let props = node
                .properties()
                .iter()
                .filter_map(|prop| {
                    if matches!(
                        prop.kind(),
                        SyntaxKind::SpreadAssignment
                            | SyntaxKind::MethodDeclaration
                            | SyntaxKind::GetAccessor
                            | SyntaxKind::SetAccessor
                    ) {
                        ctx.unsupported
                            .get_or_insert_with(|| unsupported_message(prop.kind()));
                        return None;
                    }
                    if prop.kind() != SyntaxKind::PropertyAssignment {
                        return None;
                    }
                    let name = prop.name();
                    let key = if name.kind() == SyntaxKind::Identifier {
                        name.text().to_owned()
                    } else {
                        slice_node(code, name)
                    };
                    Some(format!("{key}: {}", transpile_expr(prop.initializer(), code, ctx)))
                })
                .collect::<Vec<_>>();
            format!("[104, {{ {} }}]", props.join(", "))
        }
        SyntaxKind::ParenthesizedExpression => transpile_expr(node.expression(), code, ctx),
        // Upstream's transpiler throws here ("Arrow functions are not
        // supported inside TGSL", "Unsupported JS functionality: ..."), so a
        // body it cannot express must not claim to be printable. A nested
        // function is the common case: a text blob is not an expression.
        _ => {
            ctx.unsupported
                .get_or_insert_with(|| unsupported_message(node.kind()));
            format!("\"{}\"", slice_node(code, node))
        }
    }
}

fn is_restored_assign(node: Node) -> bool {
    if node.kind() != SyntaxKind::CallExpression {
        return false;
    }
    let callee = unwrap_parens(node.expression());
    callee.kind() == SyntaxKind::Identifier
        && matches!(
            callee.text(),
            "__tsover_add" | "__tsover_sub" | "__tsover_mul" | "__tsover_div" | "__tsover_mod"
        )
}

fn restore_overload(node: Node, code: &str, ctx: &mut TinyCtx) -> Option<String> {
    let callee = unwrap_parens(node.expression());
    if callee.kind() != SyntaxKind::Identifier {
        return None;
    }
    let op = match callee.text() {
        "__tsover_add" => "+",
        "__tsover_sub" => "-",
        "__tsover_mul" => "*",
        "__tsover_div" => "/",
        "__tsover_mod" => "%",
        _ => return None,
    };
    let args = node.arguments();
    if args.len() != 2 {
        return None;
    }
    let left_node = args.get(0);
    let right_node = args.get(1);
    let left = transpile_expr(left_node, code, ctx);
    let right = if right_node.kind() == SyntaxKind::CallExpression {
        let inner = unwrap_parens(right_node.expression());
        if inner.kind() == SyntaxKind::Identifier
            && inner.text() == callee.text()
            && right_node.arguments().len() == 2
            && slice_node(code, right_node.arguments().get(0)) == slice_node(code, left_node)
        {
            transpile_expr(right_node.arguments().get(1), code, ctx)
        } else {
            transpile_expr(right_node, code, ctx)
        }
    } else {
        transpile_expr(right_node, code, ctx)
    };
    let left_text = slice_node(code, left_node);
    let parent = node.parent();
    // `x = __tsover_add(x, y)` is the rewrite of `x += y`. A nested call is not.
    let assign = !parent.is_nil()
        && parent.kind() == SyntaxKind::BinaryExpression
        && parent.operator_token().kind() == SyntaxKind::EqualsToken
        && parent.right() == node
        && slice_node(code, parent.left()) == left_text;
    if assign {
        let symbol = match op {
            "+" => "+=",
            "-" => "-=",
            "*" => "*=",
            "/" => "/=",
            "%" => "%=",
            _ => op,
        };
        return Some(format!("[2, {left}, \"{symbol}\", {right}]"));
    }
    Some(format!("[1, {left}, \"{op}\", {right}]"))
}

fn external_chain(node: Node, ctx: &TinyCtx) -> Option<String> {
    match node.kind() {
        SyntaxKind::Identifier => {
            let name = node.text();
            if ctx.declared.iter().any(|declared| declared == name) || is_builtin(name) {
                None
            } else {
                Some(name.to_owned())
            }
        }
        SyntaxKind::PropertyAccessExpression => {
            let property = node.name();
            if property.kind() == SyntaxKind::PrivateIdentifier {
                let object = external_chain(unwrap_parens(node.expression()), ctx)?;
                let name = property.text().trim_start_matches('#');
                return Some(format!("{object}.#{name}"));
            }
            if property.kind() != SyntaxKind::Identifier || property.text() == "$" {
                return None;
            }
            let object = external_chain(unwrap_parens(node.expression()), ctx)?;
            Some(format!("{object}.{}", property.text()))
        }
        SyntaxKind::ElementAccessExpression => {
            // `ext['n']` names the object, not the computed key.
            external_chain(unwrap_parens(node.expression()), ctx)
        }
        SyntaxKind::ThisKeyword => Some("this".to_owned()),
        _ => None,
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "undefined"
            | "NaN"
            | "Infinity"
            | "Math"
            | "Number"
            | "console"
            | "globalThis"
            | "__tsover_add"
            | "__tsover_sub"
            | "__tsover_mul"
            | "__tsover_div"
            | "__tsover_mod"
    )
}

fn is_assignment(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::EqualsToken
            | SyntaxKind::PlusEqualsToken
            | SyntaxKind::MinusEqualsToken
            | SyntaxKind::AsteriskEqualsToken
            | SyntaxKind::SlashEqualsToken
            | SyntaxKind::PercentEqualsToken
            | SyntaxKind::AsteriskAsteriskEqualsToken
            | SyntaxKind::AmpersandAmpersandEqualsToken
            | SyntaxKind::BarBarEqualsToken
            | SyntaxKind::QuestionQuestionEqualsToken
            | SyntaxKind::AmpersandEqualsToken
            | SyntaxKind::BarEqualsToken
            | SyntaxKind::CaretEqualsToken
            | SyntaxKind::LessThanLessThanEqualsToken
            | SyntaxKind::GreaterThanGreaterThanEqualsToken
            | SyntaxKind::GreaterThanGreaterThanGreaterThanEqualsToken
    )
}

fn is_logical(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::AmpersandAmpersandToken
            | SyntaxKind::BarBarToken
            | SyntaxKind::QuestionQuestionToken
    )
}

fn operator_text(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::PlusToken => "+",
        SyntaxKind::MinusToken => "-",
        SyntaxKind::AsteriskToken => "*",
        SyntaxKind::SlashToken => "/",
        SyntaxKind::PercentToken => "%",
        SyntaxKind::AsteriskAsteriskToken => "**",
        SyntaxKind::AmpersandToken => "&",
        SyntaxKind::BarToken => "|",
        SyntaxKind::CaretToken => "^",
        SyntaxKind::LessThanLessThanToken => "<<",
        SyntaxKind::GreaterThanGreaterThanToken => ">>",
        SyntaxKind::GreaterThanGreaterThanGreaterThanToken => ">>>",
        SyntaxKind::EqualsToken => "=",
        SyntaxKind::PlusEqualsToken => "+=",
        SyntaxKind::MinusEqualsToken => "-=",
        SyntaxKind::AsteriskEqualsToken => "*=",
        SyntaxKind::SlashEqualsToken => "/=",
        SyntaxKind::PercentEqualsToken => "%=",
        SyntaxKind::AsteriskAsteriskEqualsToken => "**=",
        SyntaxKind::AmpersandEqualsToken => "&=",
        SyntaxKind::BarEqualsToken => "|=",
        SyntaxKind::CaretEqualsToken => "^=",
        SyntaxKind::LessThanLessThanEqualsToken => "<<=",
        SyntaxKind::GreaterThanGreaterThanEqualsToken => ">>=",
        SyntaxKind::GreaterThanGreaterThanGreaterThanEqualsToken => ">>>=",
        SyntaxKind::AmpersandAmpersandEqualsToken => "&&=",
        SyntaxKind::BarBarEqualsToken => "||=",
        SyntaxKind::QuestionQuestionEqualsToken => "??=",
        SyntaxKind::InKeyword => "in",
        SyntaxKind::InstanceOfKeyword => "instanceof",
        SyntaxKind::EqualsEqualsToken => "==",
        SyntaxKind::EqualsEqualsEqualsToken => "===",
        SyntaxKind::ExclamationEqualsToken => "!=",
        SyntaxKind::ExclamationEqualsEqualsToken => "!==",
        SyntaxKind::LessThanToken => "<",
        SyntaxKind::LessThanEqualsToken => "<=",
        SyntaxKind::GreaterThanToken => ">",
        SyntaxKind::GreaterThanEqualsToken => ">=",
        SyntaxKind::AmpersandAmpersandToken => "&&",
        SyntaxKind::BarBarToken => "||",
        SyntaxKind::QuestionQuestionToken => "??",
        SyntaxKind::TildeToken => "~",
        SyntaxKind::QuestionToken => "?",
        SyntaxKind::PlusPlusToken => "++",
        SyntaxKind::MinusMinusToken => "--",
        SyntaxKind::ExclamationToken => "!",
        // Upstream's transpiler throws on a shape it does not know. This
        // table keeps every operator the tinyest AST records, so `?` means a
        // kind was added upstream and not here.
        _ => "?",
    }
}

/// The literal's value, as the plugin records it (`numericLiteral.value`),
/// not the author's spelling: `0x10` is 16, `1e3` is 1000 and `1_000` is
/// 1000. The runtime prints this text into WGSL, where none of those forms is
/// a literal.
fn numeric_text(node: Node, code: &str) -> String {
    let raw = slice_node(code, node);
    js_number_text(crate::jsnum::from_string(&raw).value())
}

/// JavaScript's `String(number)` for the values a literal can hold. Rust
/// never prints an exponent, so a magnitude of 1e21 or more spells out its
/// digits where JavaScript writes `1e+21`; the value is the same and the
/// runtime parses either.
fn js_number_text(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_owned()
        } else {
            "Infinity".to_owned()
        };
    }
    if value == value.trunc() && value.abs() < 9_007_199_254_740_992.0 {
        return format!("{}", value as i64);
    }
    format!("{value}")
}


fn slice_node(code: &str, node: Node) -> String {
    let start = usize::try_from(skip_trivia(code, node.pos()).max(0)).unwrap_or(0);
    let end = usize::try_from(node.end().max(0)).unwrap_or(code.len());
    let end = end.min(code.len());
    let start = start.min(end);
    code[start..end].to_owned()
}





