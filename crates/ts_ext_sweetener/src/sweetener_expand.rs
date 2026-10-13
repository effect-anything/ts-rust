//! Builtin Sweetener expansion over a reader token tree.
//!
//! Source, reader, token trees, expansion, printed virtual TypeScript.
//! A `for syntax` import is resolved against the file next to the source,
//! compiled into rules, and removed from the print. Rules are matched from
//! the imported macro, not a hardcoded rewrite table. `"use gpu"` is copied.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Shared macro and class tables. A nested replacement clones the `Rc`,
/// not the table.
type SharedList<T> = Rc<Vec<T>>;

/// `->>`. Kept as a constant so a rewrite cannot drop a `>`.
const THREAD_LAST_OP: &str = "->>";

const FORM_HEADS: &[&str] = &[
    "assert",
    "data",
    "dbg",
    "do",
    "effect",
    "error",
    "export",
    "function",
    "gen",
    "handle",
    "match",
    "memoized",
    "method",
    "query",
    "service",
    "state",
    "states",
    "useCallback",
    "useContext",
    "useDeferredValue",
    "useEffect",
    "useId",
    "useMemo",
    "useReducer",
    "useRef",
    "useState",
    "useTransition",
];

fn share_list<T>(items: Vec<T>) -> SharedList<T> {
    Rc::new(items)
}

use crate::{Diagnostic, ExpandRequest, Expansion, Segment, SegmentKind, Severity};

use super::sweetener_reader::{self, Delimiter, Span, Syntax};

/// Expand `request` from the reader tree. A macro the reader can see but
/// cannot expand is a Sweetener diagnostic at the failure token. That item
/// is left as written; the rest of the file still expands.
#[must_use]
pub fn expand(request: ExpandRequest<'_>) -> Expansion {
    OWNER_FILTER.with(|filter| filter.borrow_mut().take());
    let first = expand_pass(&request, None);
    // Runtime imports belong to the module that needs them. Upstream prints
    // them only for modules whose macros this file invoked, which is known
    // once the file has been expanded; a module none of whose macros ran
    // leaves its imports out.
    let invoked = INVOKED_OWNERS.with(|set| set.borrow().clone());
    let unused_owner =
        IMPORT_OWNERS.with(|set| set.borrow().iter().any(|owner| !invoked.contains(owner)));
    if unused_owner {
        OWNER_FILTER.with(|filter| *filter.borrow_mut() = Some(invoked));
    }
    let out = expand_again(&request, first, unused_owner);
    OWNER_FILTER.with(|filter| filter.borrow_mut().take());
    out
}

fn expand_again(
    request: &ExpandRequest<'_>,
    first: (Expansion, Vec<(usize, usize, Vec<String>)>),
    imports_changed: bool,
) -> Expansion {
    if first.1.is_empty() {
        if imports_changed {
            return expand_pass(request, None).0;
        }
        return first.0;
    }
    // Upstream decides hygiene on the printed program: a name is
    // unavailable when the OUTPUT spells it. User code a top-level macro
    // consumed and did not print again (its literal pattern tokens) is not
    // in the output, so its binders do not force a rename. Its captures are
    // printed again and still count.
    let names = output_user_names(request.code, &first.1);
    expand_pass(request, Some(names)).0
}

/// User binders still present in the output: the source outside consumed
/// top-level invocations, plus the captures those invocations print again.
fn output_user_names(
    source: &str,
    consumed: &[(usize, usize, Vec<String>)],
) -> std::collections::HashSet<String> {
    let mut masked = source.as_bytes().to_vec();
    // Definitions are not printed either.
    let read = sweetener_reader::read_syntax(source);
    let mut ranges: Vec<(usize, usize)> = consumed.iter().map(|(s, e, _)| (*s, *e)).collect();
    for index in 0..read.children.len() {
        // `... from "./m.sts" for syntax;` is an import, not a definition.
        let after_for = index > 0 && token_at(&read.children, index - 1, "for");
        if after_for || (index > 0 && token_at(&read.children, index - 1, "export")) {
            continue;
        }
        if let Some(end) = local_syntax_end(&read.children, index) {
            ranges.push((node_start(&read.children[index]), end));
        }
    }
    for (start, end) in &ranges {
        for byte in masked.get_mut(*start..*end).into_iter().flatten() {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    let masked = String::from_utf8(masked).unwrap_or_else(|_| source.to_owned());
    let mut names = user_spellings(&sweetener_reader::read_syntax(&masked).children);
    for (_, _, captures) in consumed {
        for text in captures {
            let text = text.trim();
            if is_ident_text(text) {
                names.insert(text.to_owned());
            } else {
                names.extend(user_spellings(
                    &sweetener_reader::read_syntax(text).children,
                ));
            }
        }
    }
    names
}

/// Whether `needle` occurs in `haystack` not inside a longer identifier.
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let ident = |ch: char| ch == '_' || ch == '$' || ch.is_alphanumeric();
    haystack.match_indices(needle).any(|(at, _)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + needle.len()..].chars().next();
        !before.is_some_and(ident) && !after.is_some_and(ident)
    })
}

/// The text of every capture, flattening repetitions and records.
fn capture_texts(captures: &HashMap<String, Capture>) -> Vec<String> {
    fn walk(capture: &Capture, out: &mut Vec<String>) {
        match capture {
            Capture::Text(text) => out.push(text.clone()),
            // A class match is printed through its fields, not as a whole.
            Capture::Record { fields, .. } => {
                for value in fields.values() {
                    walk(value, out);
                }
            }
            Capture::Many(rows) => {
                for row in rows {
                    for (_, value) in row {
                        walk(value, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for capture in captures.values() {
        walk(capture, &mut out);
    }
    out
}

/// One expansion of the file. `names` replaces the source's user binders as
/// the names an introduced binder must not take.
fn expand_pass(
    request: &ExpandRequest<'_>,
    names: Option<std::collections::HashSet<String>>,
) -> (Expansion, Vec<(usize, usize, Vec<String>)>) {
    let source = request.code;
    let read = sweetener_reader::read_syntax(source);
    SYNTAX_PARAMETERS.with(|set| {
        *set.borrow_mut() = syntax_parameters(&read.children).into_iter().collect();
    });
    super::sweetener_scoped::clear_unused();
    BUDGET.with(|budget| budget.borrow_mut().take());
    super::sweetener_budget::reset();
    INVOKED_OWNERS.with(|set| set.borrow_mut().clear());
    if let Err(message) = super::sweetener_budget::charge(
        super::sweetener_budget::Kind::InputTokens,
        super::sweetener_budget::count_tokens(source),
    ) {
        note_budget(message);
    }
    ACTIVE_EXPANSIONS.with(|active| active.borrow_mut().clear());
    let imports = syntax_imports(source, &read.children);
    let names_again = names.clone();
    let user_names = names.unwrap_or_else(|| user_spellings(&read.children));
    let mut loaded = load_macros(request.filename, source, &imports, &user_names);
    let mut local = HashMap::new();
    collect_macros(source, &read.children, &mut local);
    // An operator this file defines or imports with a spelling the reader
    // splits (`@@`) is one token to upstream's reader. Read everything again
    // with those spellings, the macro modules included.
    let spellings: Vec<String> = loaded
        .macros
        .iter()
        .chain(local.values())
        .filter(|macro_| macro_.operator)
        .map(|macro_| macro_.name.clone())
        .filter(|name| {
            name.chars().count() >= 2
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_punctuation() && !"()[]{}\"'`$;,".contains(ch))
                && !sweetener_reader::reads_as_one(name)
        })
        .collect();
    if !spellings.is_empty() {
        return sweetener_reader::with_operators(&spellings, || expand_pass(request, names_again));
    }
    // A rule from another module points into that module's text.
    for macro_ in &mut loaded.macros {
        for rule in &mut macro_.rules {
            rule.origin = None;
        }
    }
    let local_names: Vec<String> = local.keys().cloned().collect();
    let mut in_scope: std::collections::HashSet<String> =
        syntax_parameters(&read.children).into_iter().collect();
    in_scope.extend(
        imports
            .iter()
            .flat_map(|import| import.names.iter().cloned()),
    );
    let unknown = super::sweetener_validate::unknown_parameterizations(&read.children, &in_scope);
    UNKNOWN_PARAMETERIZED.with(|cell| *cell.borrow_mut() = unknown);
    LOCAL_HEADS.with(|heads| {
        let mut heads = heads.borrow_mut();
        heads.clear();
        for (name, macro_) in &local {
            if !loaded.macros.iter().any(|imported| imported.name == *name) {
                heads.insert(name.clone(), macro_.head.start);
            }
        }
    });
    let mut local: Vec<Macro> = local.into_values().collect();
    local.sort_by_key(|macro_| macro_.head.start);
    for macro_ in local {
        loaded.diagnostics.extend(contract_diagnostics(&macro_));
        if macro_.shadows_core && !core_shadow_valid(&macro_) {
            loaded.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR4002: Syntax {} cannot shadow a core {} form.",
                    macro_.name, macro_.category
                ),
                start: macro_.head.start,
                end: macro_.head.end,
                severity: Severity::Error,
                related: Vec::new(),
            });
        }
        if local_dispatchable(&macro_) {
            push_macro(&mut loaded.macros, macro_);
        }
    }
    loaded
        .diagnostics
        .extend(malformed_templates(&read.children));
    loaded
        .classes
        .extend(collect_classes(source, &read.children));
    let known: Vec<super::sweetener_validate::KnownClass> = loaded
        .classes
        .iter()
        .map(|class| super::sweetener_validate::KnownClass {
            name: class.name.clone(),
            fields: class
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        })
        .collect();
    let (invalid_definitions, unbound) =
        super::sweetener_validate::validate(&read.children, &known);
    loaded.diagnostics.extend(invalid_definitions);
    loaded
        .diagnostics
        .extend(super::sweetener_validate::malformed_syntax_imports(
            source,
            &read.children,
        ));
    if !unbound.is_empty() {
        let local_names: std::collections::HashSet<&str> =
            local_names.iter().map(String::as_str).collect();
        loaded.macros.retain(|macro_| {
            !(unbound.contains(&macro_.name) && local_names.contains(macro_.name.as_str()))
        });
    }
    let mut printer = Printer {
        source,
        out: String::new(),
        segments: Vec::new(),
        diagnostics: read.diagnostics,
        last_original: 0,
        macros: share_list(loaded.macros),
        dependencies: loaded.dependencies,
        depth: 0,
        printing_core: false,
        user_names: user_names.clone(),
        pending_captures: None,
        consumed: Vec::new(),
        classes: share_list(loaded.classes),
        runtime_imports: {
            IMPORT_OWNERS.with(|set| {
                let mut set = set.borrow_mut();
                set.clear();
                set.extend(
                    loaded
                        .runtime_imports
                        .iter()
                        .flat_map(|import| import.owners.iter().cloned()),
                );
            });
            loaded.runtime_imports
        },
        hygiene: HashMap::new(),
        channel_renames: HashMap::new(),
        context_rejected: Vec::new(),
        in_block: false,
        context_base: super::sweetener_context::MODULE,
        context_regions: None,
        last_contexts: super::sweetener_context::MODULE,
        // Theirs synthesizes a macro module's runtime imports only for a
        // file that invoked one of its macros. A module whose every item is
        // a macro definition invokes none.
        imports_emitted: only_definitions(&read.children),
        form_signals: Vec::new(),
        last_form_next: 0,
        failed: false,
        file_nodes: &read.children,
    };
    printer.diagnostics.extend(loaded.diagnostics);
    printer.print_nodes(&read.children);
    printer.copy_until(source.len());
    printer.finish_fixture_tails();
    // A macro the rules refuse, or a delimiter the reader cannot pair, is
    // reported on the author token. The item that failed is already in
    // `out` as written (verbatim). Other files are separate expansions.
    // Theirs prints a definition module's comments as trivia of the
    // definitions they precede, so a module of definitions alone prints
    // nothing at all.
    if printer.diagnostics.is_empty() && only_definitions(&read.children) {
        printer.out.clear();
        printer.segments.clear();
    }
    // A syntax parameter the file still spells, outside every expansion
    // that gives it a meaning.
    let unbound = unbound_parameter_uses(source, &printer.segments);
    printer.diagnostics.extend(unbound);
    // Warnings come last so they never count as a failed expansion above.
    let unread = unread_items(
        source,
        &read.children,
        &printer.segments,
        &printer.diagnostics,
    );
    // The scanner already said why the item does not read.
    if !printer
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.starts_with("SWR1001"))
    {
        printer.diagnostics.extend(unread);
    }
    let hoisted = super::sweetener_scoped::hoist(
        &printer.out,
        &mut printer.segments,
        request.filename.ends_with('x'),
    );
    let code = super::sweetener_hygiene::assign(
        &hoisted,
        &mut printer.segments,
        request.filename.ends_with('x'),
    );
    (
        Expansion {
            code,
            segments: printer.segments,
            diagnostics: printer.diagnostics,
            dependencies: printer.dependencies,
        },
        printer.consumed,
    )
}

/// SWR2016: templates of this file's own definitions that upstream's
/// template parser refuses -- a `$( ... )` repetition with no `*`/`+`, and
/// an `#if` whose predicate is not exactly `present $capture`.
fn malformed_templates(nodes: &[Syntax]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "syntax") || token_at(nodes, index, "operator") {
            let mut body = index + 1;
            while body < nodes.len()
                && !operator_is(nodes.get(body), ";")
                && !group_is(nodes.get(body), Delimiter::Brace)
            {
                body += 1;
            }
            if let Some(Syntax::Group { children, .. }) = nodes.get(body) {
                if group_is(nodes.get(body), Delimiter::Brace) {
                    rule_templates(children, &mut out);
                }
            }
            index = body + 1;
            continue;
        }
        index += 1;
    }
    out
}

/// Each `=> { template }` among a definition body's rules.
fn rule_templates(nodes: &[Syntax], out: &mut Vec<Diagnostic>) {
    for index in 0..nodes.len() {
        if is_arrow(nodes, index) {
            let template = (index + 1..nodes.len())
                .find(|at| !matches!(nodes[*at], Syntax::Operator { ref raw, .. } if raw == ">"));
            if let Some(Syntax::Group {
                delimiter: Delimiter::Brace,
                children,
                ..
            }) = template.and_then(|at| nodes.get(at))
            {
                check_template(children, out);
            }
        }
    }
}

fn malformed(message: &str, span: Span) -> Diagnostic {
    Diagnostic {
        message: format!("SWR2016: Malformed template: {message}."),
        start: span.start,
        end: span.end,
        severity: Severity::Error,
        related: Vec::new(),
    }
}

fn syntax_span(node: &Syntax) -> Span {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => *span,
    }
}

fn check_template(nodes: &[Syntax], out: &mut Vec<Diagnostic>) {
    let mut index = 0;
    while index < nodes.len() {
        // `#syntax { ... }` holds a definition, not a template.
        if token_at(nodes, index, "#syntax")
            || (operator_is(nodes.get(index), "#") && token_at(nodes, index + 1, "syntax"))
        {
            index += if token_at(nodes, index, "#syntax") {
                2
            } else {
                3
            };
            continue;
        }
        let if_at = if token_at(nodes, index, "#if") {
            Some(index + 1)
        } else if operator_is(nodes.get(index), "#") && token_at(nodes, index + 1, "if") {
            Some(index + 2)
        } else {
            None
        };
        if let Some(pred_at) = if_at {
            if let Some(Syntax::Group {
                delimiter: Delimiter::Paren,
                children,
                span,
                ..
            }) = nodes.get(pred_at)
            {
                let whole = token_at(children, 0, "present")
                    && read_var_path(children, 1).is_some_and(|(_, _, next)| {
                        children[next.min(children.len())..]
                            .iter()
                            .all(|node| matches!(node, Syntax::Comment { .. }))
                    });
                if !whole {
                    let at = (0..children.len())
                        .find(|at| dollar_at(children, *at))
                        .map_or(*span, |at| syntax_span(&children[at]));
                    out.push(malformed("conditional requires present $capture", at));
                }
            }
        }
        if dollar_at(nodes, index) && group_is(nodes.get(index + 1), Delimiter::Paren) {
            let mut quantifier = index + 2;
            if !operator_is(nodes.get(quantifier), "*") && !operator_is(nodes.get(quantifier), "+")
            {
                quantifier += 1;
            }
            if !operator_is(nodes.get(quantifier), "*") && !operator_is(nodes.get(quantifier), "+")
            {
                out.push(malformed(
                    "repetition requires * or +",
                    syntax_span(&nodes[index]),
                ));
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            check_template(children, out);
        }
        index += 1;
    }
}

/// Words a binding can never be spelled with (TypeScript reserved words).
const RESERVED_BINDING_WORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
];

/// The words that begin a module item and nothing else (upstream's
/// `definiteItemStarts`). Recovery stops where one of them begins a line.
const DEFINITE_ITEM_STARTS: &[&str] = &[
    "abstract",
    "class",
    "const",
    "declare",
    "enum",
    "export",
    "function",
    "import",
    "interface",
    "let",
    "module",
    "namespace",
    "type",
    "var",
];

/// Words that join two operands (`a as T`) or begin one (`new X`), so two
/// words in a row are not a missing terminator.
const JOINING_WORDS: &[&str] = &[
    "as",
    "satisfies",
    "in",
    "instanceof",
    "of",
    "is",
    "extends",
    "keyof",
    "typeof",
    "new",
    "await",
    "void",
    "delete",
    "yield",
    "async",
    "function",
    "class",
    "readonly",
    "unique",
    "infer",
];

/// Where upstream's reader gives up on a top-level `let`/`const`/`var`, and
/// what it expected there.
fn unread_failure(source: &str, nodes: &[Syntax], head: usize) -> Option<(usize, &'static str)> {
    let mut binding = head + 1;
    skip_comments(nodes, &mut binding);
    let binding_node = nodes.get(binding)?;
    let reserved = matches!(binding_node, Syntax::Token { raw, .. } if RESERVED_BINDING_WORDS.contains(&raw.as_str()))
        || matches!(
            binding_node,
            Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            }
        );
    if reserved {
        return Some((binding, "variable binding"));
    }
    if !operator_is(nodes.get(binding + 1), "=") {
        return None;
    }
    // `const v = #[..]`: no expression starts with that operator.
    let mut first = binding + 2;
    skip_comments(nodes, &mut first);
    if let Some(Syntax::Operator { raw, .. }) = nodes.get(first) {
        if !matches!(raw.as_str(), "!" | "-" | "+" | "~" | "<" | "/" | "." | "@") {
            return Some((first, "variable initializer"));
        }
    }
    // Two operands in a row on one line: `foo 1`, `a b`, `macro { .. }`.
    let operand = |node: Option<&Syntax>| match node {
        Some(Syntax::Token { raw, .. }) => {
            !JOINING_WORDS.contains(&raw.as_str())
                && matches!(
                    token_kind(raw),
                    Some("identifier" | "keyword" | "numeric-literal" | "bigint-literal")
                )
                && !raw.is_empty()
        }
        Some(Syntax::String { raw, .. }) => !raw.starts_with('`'),
        _ => false,
    };
    let mut at = binding + 2;
    while at + 1 < nodes.len() {
        if operator_is(nodes.get(at), ";") || line_break_between(source, nodes, at + 1) {
            break;
        }
        let next = nodes.get(at + 1);
        let word = matches!(nodes.get(at), Some(Syntax::Token { raw, .. }) if token_kind(raw) == Some("identifier"));
        let brace = matches!(
            next,
            Some(Syntax::Group {
                delimiter: Delimiter::Brace,
                ..
            })
        );
        if operand(nodes.get(at)) && (operand(next) || (word && brace)) {
            return Some((at + 1, "';' or automatic terminator"));
        }
        // `match (1) { .. }`: a call does not take a block.
        let call = matches!(
            next,
            Some(Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            })
        );
        let block_after = at + 2 < nodes.len()
            && !line_break_between(source, nodes, at + 2)
            && matches!(
                nodes.get(at + 2),
                Some(Syntax::Group {
                    delimiter: Delimiter::Brace,
                    ..
                })
            );
        if word && call && block_after {
            return Some((at + 2, "';' or automatic terminator"));
        }
        at += 1;
    }
    None
}

/// Where a readable `let`/`const`/`var` item ends: at its `;`, or before the
/// first node on a new line that cannot continue the initializer.
fn variable_item_end(source: &str, nodes: &[Syntax], head: usize) -> (usize, bool) {
    let mut at = head + 1;
    while at < nodes.len() {
        if operator_is(nodes.get(at), ";") {
            return (at + 1, true);
        }
        if at > head + 1 && line_break_between(source, nodes, at) {
            let prev_continues =
                matches!(&nodes[at - 1], Syntax::Operator { raw, .. } if raw != ")" && raw != "]");
            let continues = matches!(
                &nodes[at],
                Syntax::Operator { raw, .. }
                    if matches!(raw.as_str(), "." | "?" | "+" | "-" | "*" | "/" | "%" | "=" | "," | "|" | "&" | "<" | ">" | ":" | "!" | "^")
            ) || matches!(
                &nodes[at],
                Syntax::Group {
                    delimiter: Delimiter::Paren | Delimiter::Bracket,
                    ..
                }
            );
            if !prev_continues && !continues {
                return (at, false);
            }
        }
        at += 1;
    }
    (at, false)
}

/// SWR4025, upstream's `reportUnreadItems`. A top-level item the reader
/// cannot read is recovered as written: the run reaches its `;`, or stops
/// before a definite item start on a new line. It is reported when the run
/// begins at a definite item start, every token of it still stands in the
/// output (nothing in it was rewritten), and no other diagnostic speaks
/// about it. A variable that reads but runs into a non-item token without
/// `;` swallows everything to the next `;`, unreported, as upstream does.
fn unread_items(
    source: &str,
    nodes: &[Syntax],
    segments: &[Segment],
    reported: &[Diagnostic],
) -> Vec<Diagnostic> {
    let survives = |start: usize, end: usize| {
        segments.iter().any(|segment| {
            segment.kind == SegmentKind::Verbatim
                && segment.original_start as usize <= start
                && end <= segment.original_end as usize
        })
    };
    let mut out: Vec<Diagnostic> = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        let at_item_start = index == 0
            || matches!(nodes.get(index - 1), Some(Syntax::Operator { raw, .. }) if raw == ";")
            || matches!(
                nodes.get(index - 1),
                Some(Syntax::Group {
                    delimiter: Delimiter::Brace,
                    ..
                }) | Some(Syntax::Comment { .. })
            )
            || line_break_between(source, nodes, index);
        let variable = ["let", "const", "var"]
            .iter()
            .any(|word| token_at(nodes, index, word));
        if !at_item_start || !variable {
            index += 1;
            continue;
        }
        let head = index;
        let Some((stopped, expected)) = unread_failure(source, nodes, head) else {
            let (end, terminated) = variable_item_end(source, nodes, head);
            let next = nodes.get(end);
            let swallows = !terminated
                && matches!(next, Some(Syntax::Token { raw, .. } | Syntax::Operator { raw, .. }) if !DEFINITE_ITEM_STARTS.contains(&raw.as_str()));
            index = if swallows {
                (end..nodes.len())
                    .find(|at| operator_is(nodes.get(*at), ";"))
                    .map_or(nodes.len(), |at| at + 1)
            } else {
                end.max(head + 1)
            };
            continue;
        };
        // The recovered run.
        let mut end = head;
        while end < nodes.len() {
            if operator_is(nodes.get(end), ";") {
                break;
            }
            let following = end + 1;
            if following < nodes.len()
                && line_break_between(source, nodes, following)
                && matches!(nodes.get(following), Some(Syntax::Token { raw, .. }) if DEFINITE_ITEM_STARTS.contains(&raw.as_str()))
            {
                break;
            }
            end += 1;
        }
        let end = end.min(nodes.len() - 1);
        index = end + 1;
        let start = node_start(&nodes[head]);
        let finish = node_end(&nodes[end]);
        let mut tokens = Vec::new();
        collect_token_spans(&nodes[head..=end], &mut tokens);
        if !tokens.iter().all(|(from, to)| survives(*from, *to)) {
            continue;
        }
        let mentioned = reported.iter().chain(out.iter()).any(|diagnostic| {
            (diagnostic.start as usize) < finish && start < diagnostic.end as usize
        });
        if mentioned {
            continue;
        }
        let stopped = &nodes[stopped.min(end)];
        out.push(Diagnostic {
            message: format!(
                "SWR4025: Sweetener could not read this item, so it was passed through as written and no macro in it was expanded. Its reader expected {expected}. TypeScript may well accept the item as written -- a reader that cannot read it is a gap in Sweetener rather than a mistake here."
            ),
            start: u32_at(start),
            end: u32_at(finish),
            severity: Severity::Warning,
            related: vec![crate::RelatedLocation {
                message: format!("The reader expected {expected} here."),
                start: u32_at(node_start(stopped)),
                end: u32_at(node_end(stopped)),
            }],
        });
    }
    out
}

/// Source spans of every token under `nodes`, comments excluded.
fn collect_token_spans(nodes: &[Syntax], out: &mut Vec<(usize, usize)>) {
    for node in nodes {
        match node {
            Syntax::Group { children, .. } => collect_token_spans(children, out),
            Syntax::Comment { .. } => {}
            other => out.push((node_start(other), node_end(other))),
        }
    }
}

/// Whether a line break separates `nodes[index]` from the node before it.
fn line_break_between(source: &str, nodes: &[Syntax], index: usize) -> bool {
    let (Some(prev), Some(node)) = (
        index.checked_sub(1).and_then(|i| nodes.get(i)),
        nodes.get(index),
    ) else {
        return false;
    };
    source
        .get(node_end(prev)..node_start(node))
        .is_some_and(|gap| gap.contains('\n'))
}

struct Printer<'a> {
    source: &'a str,
    out: String,
    segments: Vec<Segment>,
    diagnostics: Vec<Diagnostic>,
    last_original: usize,
    macros: SharedList<Macro>,
    dependencies: Vec<String>,
    depth: u8,
    /// Set while rendering `#core(...)`, so the inner form is not expanded
    /// as the macro that produced it.
    printing_core: bool,
    /// Spellings the user file already binds. A macro binding that collides
    /// is printed with a suffix; the user's spelling is not rewritten.
    user_names: std::collections::HashSet<String>,
    /// Captures of the rule being instantiated at the top level, until its
    /// expansion is emitted.
    pending_captures: Option<Vec<String>>,
    /// Top-level invocations a rule consumed: original range and the text
    /// of each capture (the user code that is printed again).
    consumed: Vec<(usize, usize, Vec<String>)>,
    /// Syntax classes compiled from the same modules as the macros.
    classes: SharedList<SyntaxClass>,
    /// Runtime imports the macro module needs, printed once at the top.
    runtime_imports: Vec<RuntimeImport>,
    /// Macro-introduced spellings already assigned a printed name in this file.
    hygiene: HashMap<String, String>,
    imports_emitted: bool,
    /// `state` bindings and multi-part methods the form pass introduced.
    form_signals: Vec<(String, String)>,
    /// Resume index after the form rewrite just applied.
    last_form_next: usize,
    /// The file's top-level nodes. A form inside a function is authorized by
    /// a top-level `for syntax` import, as upstream. Nested imports bind
    /// nothing and stay in the output as written.
    file_nodes: &'a [Syntax],
    /// Channel binders this generator must print with a suffix.
    channel_renames: HashMap<String, String>,
    context_rejected: Vec<(Rule, String)>,
    /// Printing inside a block or function body of the file.
    in_block: bool,
    /// The contexts this printer's top level stands in.
    context_base: super::sweetener_context::Contexts,
    context_regions: Option<Vec<super::sweetener_context::Region>>,
    /// The contexts of the invocation being expanded, for nested printers.
    last_contexts: super::sweetener_context::Contexts,
    /// A macro rule rejected an item. That item stays as written; the rest
    /// of the file is still expanded. Nested expansion copies this flag up
    /// so the parent does not treat a refused hole as a successful rewrite.
    failed: bool,
}

impl<'a> Printer<'a> {
    fn macros_mut(&mut self) -> &mut Vec<Macro> {
        Rc::make_mut(&mut self.macros)
    }

    fn classes_mut(&mut self) -> &mut Vec<SyntaxClass> {
        Rc::make_mut(&mut self.classes)
    }

    fn print_nodes(&mut self, nodes: &[Syntax]) {
        let mut index = 0;
        while index < nodes.len() {
            if self.depth == 0 && !self.imports_emitted && self.user_code_starts(nodes, index) {
                self.emit_runtime_imports();
            }
            if self.try_drop_syntax_import(nodes, index) {
                index = skip_syntax_import(nodes, index);
                continue;
            }
            // A bare signal read is not an infix operand. Claiming `count`
            // before the form pass leaves the user spelling.
            if self
                .form_signals
                .iter()
                .any(|(name, _)| token_is(Some(&nodes[index]), name))
                && self.try_form(nodes, index)
            {
                index = self.last_form_next;
                continue;
            }
            // An infix operator owns the expression, including a prefix
            // macro on its left. Expanding the prefix first would split it.
            // Channel `->` / `<-` are statement forms. The form pass suffixes
            // a binder the file already has; the infix expander does not.
            if let Some(at) = self.infix_in_expr(nodes, index) {
                // `(->> ...)` is a prefix group. Its `>` is not an infix
                // operator between the thread and the next statement.
                // Channel `->` / `<-` are infix macros. Skipping them here
                // left `2 -> table` for a form that never saw the operand.
                let thread_group = grouped_prefix(nodes, index).is_some();
                if !thread_group {
                    index = self.expand_infix_expr(nodes, index, at);
                    continue;
                }
            }
            // A channel bind the file already has is printed `name_1`.
            // Later uses in this generator still say `name` unless the
            // following statements are renamed with that binder.
            if let Some(step) =
                super::sweetener_forms::match_channel_stmt(self.source, nodes, index)
            {
                if step.op == "<-" {
                    let printed = super::sweetener_forms::fresh_binder(&step.left);
                    if printed != step.left {
                        self.channel_renames.insert(step.left.clone(), printed);
                    }
                }
            }
            // An operator-headed prefix form: `#[a, b]` is `#` then a
            // bracket group. `macro_head` only sees identifiers, so the
            // compiled `(#)` rule never ran and the hardcoded form won.
            // A rule that matches owns the spelling; `repaired_group_rejects`
            // still refuses `#[1, , 3]`.
            if matches!(nodes.get(index), Some(Syntax::Operator { .. })) {
                if let Some(name) = macro_name_of(self.source, nodes, index) {
                    if self
                        .macros
                        .iter()
                        .any(|macro_| macro_.name == name && !macro_.fixity.operand_led())
                        && !self.rules_reject(nodes, index)
                    {
                        index = self.expand_head(nodes, index, &name);
                        continue;
                    }
                }
            }
            // A form rewrite that invents a repair is not a successful
            // expansion. If every compiled rule for this macro rejects the
            // input, report that failure and write no TypeScript.
            // `#[1, , 3]` and `(-> value, call(), , filter())` are forms the
            // form pass can fill in. The compiled rule's failure token is
            // the comma, so reject those before the repair is printed.
            if self.repaired_group_rejects(nodes, index) {
                index = self.last_form_next;
                continue;
            }
            // `data`, `match`, and `method` have compiled rules. The form
            // pass prints a different program (no trailing comma, an arrow
            // body instead of `return`).
            if let Some(name) = nodes.get(index).and_then(macro_head) {
                if name == "method"
                    && macro_call_position(nodes, index)
                    && self.compiled_rule_matches(nodes, index, name)
                {
                    index = self.expand_head(nodes, index, name);
                    continue;
                }
            }
            if self.try_form(nodes, index) {
                index = self.last_form_next;
                continue;
            }
            // An identifier a property access owns (`service.q`, `x.service`)
            // or a parameter list owns (`(service) =>`) is ordinary code. A
            // form rewrite can print one that shares a macro name, so it must
            // not reach the macro dispatch.
            // A macro name the expander holds as written: used before its
            // definition, in a position its category cannot fill, or with
            // nothing after it.
            if self.depth == 0
                && let Some(held) = self.held_name(nodes, index)
            {
                self.diagnostics.push(held);
                self.print_node(&nodes[index]);
                index += 1;
                continue;
            }
            // Upstream reads item macros at module level and in namespace
            // bodies only. Inside a block or function body the words are
            // left as written.
            if self.in_block
                && let Some(name) = nodes.get(index).and_then(macro_head)
                && self.is_item_only(name)
            {
                self.print_node(&nodes[index]);
                index += 1;
                continue;
            }
            if macro_call_position(nodes, index) {
                if let Some(name) = macro_head(&nodes[index]) {
                    if self.compiled_rule_matches(nodes, index, name) {
                        index = self.expand_head(nodes, index, name);
                        continue;
                    }
                    // `(service)` is a parameter. A bare name that no rule
                    // claims is ordinary TypeScript. A following group or
                    // token is a call the rules rejected.
                    if !jsx_child_context(nodes, index)
                        && macro_invocation(nodes, index)
                        && self.rules_reject(nodes, index)
                    {
                        index = self.last_form_next;
                        continue;
                    }
                }
            }
            // `{when (ready)} <a/> {end}`: the rule's pattern opens with a
            // brace group whose first word is the macro's name, so the use
            // starts at that group, not at the word inside it.
            if let Some(name) = self.braced_head(nodes, index) {
                index = self.expand_head(nodes, index, &name);
                continue;
            }
            // `(-> value, call(...))` is one group. The operator is its
            // first child, so the head walk never starts at the name.
            if let Some(name) = grouped_prefix(nodes, index) {
                if self
                    .macros
                    .iter()
                    .any(|macro_| macro_.name == name && !macro_.fixity.operand_led())
                {
                    index = self.expand_grouped(nodes, index, &name);
                    continue;
                }
            }
            if self.depth == 0 {
                if let Some(end) = local_syntax_end(nodes, index) {
                    let start = node_start(&nodes[index]);
                    self.copy_until(start);
                    self.last_original = end;
                    index = nodes
                        .iter()
                        .position(|node| node_start(node) >= end)
                        .unwrap_or(nodes.len());
                    continue;
                }
            }
            self.print_node(&nodes[index]);
            index += 1;
        }
    }

    /// `${... |> callee}` is still an operator use. The reader keeps the
    /// whole template as one string, so the infix walk never sees it.
    fn expand_template_literal(&mut self, raw: &str) -> Option<String> {
        if !raw.starts_with('`') || !raw.contains("${") {
            return None;
        }
        if !raw.contains("|>") && !self.replacement_needs_expand(raw) {
            return None;
        }
        let mut out = String::new();
        let bytes = raw.as_bytes();
        let mut index = 0;
        let mut changed = false;
        while index < bytes.len() {
            if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
                let start = index;
                index += 2;
                let mut depth = 1i32;
                let body_at = index;
                while index < bytes.len() && depth > 0 {
                    match bytes[index] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                    index += 1;
                }
                if depth != 0 {
                    out.push_str(&raw[start..]);
                    break;
                }
                let body = &raw[body_at..index - 1];
                if body.contains("|>") || self.replacement_needs_expand(body) {
                    let expanded = self.expand_fragment(body);
                    if expanded != body {
                        changed = true;
                    }
                    out.push_str("${");
                    out.push_str(&expanded);
                    out.push('}');
                } else {
                    out.push_str(&raw[start..index]);
                }
                continue;
            }
            out.push(bytes[index] as char);
            index += 1;
        }
        changed.then_some(out)
    }

    /// Every definition of `name` is an `:item` macro.
    /// Whether the `{` at `open` opens a `namespace` / `module` body.
    fn namespace_body_at(&self, open: usize) -> bool {
        let before = self.source[..open].trim_end();
        let mut words = before.rsplit(|ch: char| ch.is_whitespace());
        let Some(name) = words.next() else {
            return false;
        };
        let keyword = words.next().unwrap_or("");
        let named = name.chars().all(|ch| {
            ch.is_alphanumeric() || matches!(ch, '_' | '$' | '.' | '"' | '\'' | '-' | '/')
        });
        named && matches!(keyword, "namespace" | "module")
    }

    fn is_item_only(&self, name: &str) -> bool {
        let mut found = false;
        for macro_ in self.macros.iter().filter(|macro_| macro_.name == name) {
            if macro_.category != "item" || macro_.operator {
                return false;
            }
            found = true;
        }
        found
    }

    fn print_node(&mut self, node: &Syntax) {
        match node {
            Syntax::Group {
                open,
                close,
                children,
                span,
                delimiter,
            } => {
                self.copy_until(open.end as usize);
                // A channel bind is lexical to the generator body. The
                // outer `received` after the function is a different binding.
                let entered = *delimiter == Delimiter::Brace;
                let saved = if entered {
                    std::mem::take(&mut self.channel_renames)
                } else {
                    HashMap::new()
                };
                if entered {
                    self.channel_renames = saved.clone();
                }
                // Item positions: the file and namespace bodies, at file
                // level. A replacement's own braces are the template's.
                let was_block = self.in_block;
                if entered && self.depth == 0 {
                    self.in_block = !self.namespace_body_at(open.start as usize);
                }
                self.print_nodes(children);
                self.in_block = was_block;
                if entered {
                    self.channel_renames = saved;
                }
                let end = close.map_or(span.end, |close| close.end);
                self.copy_until(end as usize);
            }
            Syntax::String { span, raw } => {
                self.copy_until(span.start as usize);
                let rewritten = self
                    .expand_template_literal(raw)
                    .unwrap_or_else(|| rewrite_specifier(raw));
                if rewritten == *raw {
                    self.copy_until(span.end as usize);
                } else {
                    self.emit(span.start, span.end, &rewritten, SegmentKind::Atom);
                    self.last_original = span.end as usize;
                }
            }
            Syntax::Token { span, raw } if self.channel_renames.contains_key(raw) => {
                self.copy_until(span.start as usize);
                let printed = self
                    .channel_renames
                    .get(raw)
                    .cloned()
                    .unwrap_or_else(|| raw.clone());
                self.emit(span.start, span.end, &printed, SegmentKind::Atom);
                self.last_original = span.end as usize;
            }
            Syntax::Token { span, .. }
            | Syntax::Comment { span, .. }
            | Syntax::Operator { span, .. } => {
                self.copy_until(span.end as usize);
            }
        }
    }

    /// A `for syntax` import is not a user definition. Print nothing for it.
    fn try_drop_syntax_import(&mut self, nodes: &[Syntax], index: usize) -> bool {
        if !is_syntax_import_at(nodes, index) {
            return false;
        }
        // Upstream binds module-level imports only. A nested one binds nothing
        // and is passed through as written.
        let at: *const Syntax = &nodes[index];
        if !self.file_nodes.as_ptr_range().contains(&at) {
            // Copy the whole statement: its `syntax` word must not start a
            // macro definition that swallows the rest of the block.
            self.copy_until(syntax_import_end(nodes, index));
            return true;
        }
        // The syntax import is the last specifier of the previous import.
        // Starting the drop at `import` copies that list's trailing comma.
        // Canonical comparison treats the comma as different.
        let start = node_start(&nodes[index]);
        self.copy_until(start);
        let end = syntax_import_end(nodes, index);
        self.last_original = end;
        true
    }

    /// Lowest-precedence imported infix operator in the expression that
    /// starts at `index`, if one is waiting to be expanded.
    fn infix_in_expr(&self, nodes: &[Syntax], index: usize) -> Option<usize> {
        // A prefix operator macro binds tighter than an infix operator of
        // lower precedence: `~~ y @@ z` is `pair(twice(y), z)` when `~~` is
        // above `@@`. An infix at or above the prefix stays in its operand.
        let prefix_bound = operator_spelling(&nodes[index]).and_then(|name| {
            self.macros
                .iter()
                .find(|macro_| macro_.name == name && macro_.fixity == Fixity::Prefix)
                .map(|macro_| macro_.precedence)
        });
        if !is_expr_start(&nodes[index]) && prefix_bound.is_none() {
            return None;
        }
        let mut cursor = index;
        let mut found: Option<(usize, u8)> = None;
        let mut depth_ok = true;
        // Stay in this sibling list. A following statement is not this expression.
        while cursor < nodes.len() && depth_ok {
            if is_expr_stop(&nodes[cursor]) && cursor != index {
                break;
            }
            if let Some(name) = operator_spelling(&nodes[cursor]) {
                // `Equal.register(...)` is a member call. `register` is not
                // the infix operator, even when a macro has that name.
                let member = cursor > index
                    && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw == ".");
                if !member {
                    if let Some(prec) = self
                        .infix_precedence(name)
                        .filter(|prec| prefix_bound.is_none_or(|bound| *prec < bound))
                    {
                        let replace = match found {
                            None => true,
                            Some((_, best)) if prec < best => true,
                            Some((_, best)) if prec == best => {
                                self.infix_associativity(name) != Associativity::Right
                            }
                            _ => false,
                        };
                        if replace {
                            found = Some((cursor, prec));
                        }
                    } else if !is_expr_start(&nodes[cursor]) && cursor != index {
                        break;
                    }
                }
            }
            if matches!(&nodes[cursor], Syntax::Token { .. }) && cursor != index {
                let prev_is_op = cursor > index
                    && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw != ".");
                let prev_is_dot = cursor > index
                    && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw == ".");
                let infix_word = macro_head(&nodes[cursor])
                    .is_some_and(|name| self.infix_precedence(name).is_some());
                // `await x |> f(%)`: a unary keyword binds its operand.
                let prev_is_unary = cursor > index
                    && ["await", "typeof", "void", "delete", "new"]
                        .iter()
                        .any(|word| token_is(Some(&nodes[cursor - 1]), word));
                if !prev_is_op
                    && !prev_is_dot
                    && !prev_is_unary
                    && !token_is(Some(&nodes[cursor]), "as")
                    && !infix_word
                {
                    break;
                }
            }
            cursor += 1;
            if cursor > index + 40 {
                depth_ok = false;
            }
        }
        found.map(|(at, _)| at)
    }

    /// A left-associative infix chain continues after the operand just
    /// matched: `a |> f() |> g()`. Re-expand the expansion so far followed
    /// by the rest of the chain, up to the end of the enclosing expression.
    fn fold_left_chain(
        &mut self,
        nodes: &[Syntax],
        next: usize,
        name: &str,
        expanded: &str,
    ) -> Option<(String, usize, usize)> {
        if self.infix_associativity(name) != Associativity::Left
            || operator_spelling(nodes.get(next)?) != Some(name)
        {
            return None;
        }
        let mut stop = next;
        while stop < nodes.len()
            && !operator_is(nodes.get(stop), ";")
            && !operator_is(nodes.get(stop), ",")
        {
            stop += 1;
        }
        let end = node_end(&nodes[stop - 1]);
        let rest = sweetener_reader::slice_range(self.source, node_start(&nodes[next]), end);
        let combined = format!("{expanded} {rest}");
        Some((self.expand_replacement(&combined), end, stop))
    }

    /// Upstream's first two ways to read a macro operator's right operand
    /// (expansion spec, section 7): a literal run some rule spells after the
    /// operator when nothing after it continues an operand, then an arrow
    /// function when the operator declares `operand arrow;`, whose body ends
    /// at the next use of the same operator.
    fn special_right_operand(
        &self,
        nodes: &[Syntax],
        op_at: usize,
        name: &str,
    ) -> Option<(String, usize, usize)> {
        let operators: Vec<&Macro> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name && macro_.fixity.operand_led())
            .collect();
        for rule in operators.iter().flat_map(|macro_| macro_.rules.iter()) {
            let Some(run) = literal_right_run(&rule.pattern, name) else {
                continue;
            };
            let run: Vec<&String> = run.iter().filter(|lit| *lit != ";").collect();
            if run.is_empty() {
                continue;
            }
            let written = run.iter().enumerate().all(|(offset, lit)| {
                let node = nodes.get(op_at + 1 + offset);
                token_is(node, lit) || operator_spelling_is(node, lit)
            });
            let after = op_at + 1 + run.len();
            if written && !continues_operand(nodes.get(after)) {
                let end = node_end(&nodes[after - 1]);
                let text =
                    sweetener_reader::slice_range(self.source, node_start(&nodes[op_at + 1]), end);
                return Some((text.trim().to_owned(), end, after));
            }
        }
        if operators.iter().any(|macro_| macro_.operand_arrow) && arrow_starts(nodes, op_at + 1) {
            let mut stop = op_at + 1;
            while stop < nodes.len()
                && operator_spelling(&nodes[stop]) != Some(name)
                && !operator_is(nodes.get(stop), ";")
                && !operator_is(nodes.get(stop), ",")
            {
                stop += 1;
            }
            let end = node_end(&nodes[stop - 1]);
            let text =
                sweetener_reader::slice_range(self.source, node_start(&nodes[op_at + 1]), end);
            // The arrow is one operand wherever the template places it.
            return Some((format!("({})", text.trim()), end, stop));
        }
        None
    }

    fn infix_associativity(&self, name: &str) -> Associativity {
        self.macros
            .iter()
            .find(|macro_| macro_.fixity.operand_led() && macro_.name == name)
            .map(|macro_| macro_.associativity)
            .unwrap_or(Associativity::Left)
    }

    fn infix_precedence(&self, name: &str) -> Option<u8> {
        self.macros
            .iter()
            .find(|macro_| macro_.fixity.operand_led() && macro_.name == name)
            .map(|macro_| macro_.precedence)
    }

    fn expand_infix_expr(&mut self, nodes: &[Syntax], index: usize, op_at: usize) -> usize {
        let name = operator_spelling(&nodes[op_at]).unwrap_or("").to_owned();
        if self
            .macros
            .iter()
            .any(|macro_| macro_.name == name && macro_.fixity == Fixity::Postfix)
        {
            return self.expand_postfix_expr(nodes, index, op_at, &name);
        }
        let left = sweetener_reader::slice_range(
            self.source,
            node_start(&nodes[index]),
            node_start(&nodes[op_at]),
        );
        let left = left.trim();
        let right_assoc = self.infix_associativity(&name) == Associativity::Right;
        // A right-associative chain keeps the next `**` on the right.
        // `take_expr` stops before infix operators so a pattern hole does not
        // swallow `|>`; that would leave `3 ** 2` unexpanded here.
        let special = self.special_right_operand(nodes, op_at, &name);
        let read_specially = special.is_some();
        let (right, right_end, next) = if let Some(read) = special {
            Some(read)
        } else if right_assoc {
            take_right_assoc_operand(self.source, nodes, op_at + 1)
        } else {
            take_expr(self.source, nodes, op_at + 1)
        }
        .unwrap_or_else(|| (String::new(), node_end(&nodes[op_at]), op_at + 1));
        // `received <-;` has no channel expression. Binding the hole to
        // empty text would print `yield take()`.
        let unreadable = !right
            .chars()
            .any(|ch| ch.is_alphanumeric() || "_$\"'`([{".contains(ch));
        let expression_operator = self
            .macros
            .iter()
            .any(|macro_| macro_.name == name && macro_.operator && macro_.category == "expr");
        if unreadable && expression_operator {
            // Upstream's reader gives up on the expression before any rule
            // sees it: SWR4021 at the operator, and SWR4020 where a group
            // ends on it.
            self.failed = true;
            let op = node_span(&nodes[op_at]);
            {
                if op_at + 1 >= nodes.len() {
                    let start = node_start(&nodes[index]);
                    let end = node_end(&nodes[op_at]);
                    self.diagnostics.push(Diagnostic {
                        message: format!(
                            "SWR4020: Sweetener could not read this as an expression: {}",
                            sweetener_reader::slice_range(self.source, start, end).trim()
                        ),
                        start: u32_at(start),
                        end: u32_at(end),
                        severity: Severity::Error,
                        related: Vec::new(),
                    });
                }
            }
            self.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR4021: Operator {name} was left unexpanded: the expression it is written in could not be read, so none of its rules were tried."
                ),
                start: op.start,
                end: op.end,
                severity: Severity::Error,
                related: Vec::new(),
            });
            return self.skip_failed(nodes, op_at);
        }
        if right.trim().is_empty() {
            let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
            let at = nodes
                .get(op_at + 1)
                .map(node_span)
                .unwrap_or_else(|| node_span(&nodes[op_at]));
            self.fail_macro_at(&name, &rules, at);
            return self.skip_failed(nodes, op_at);
        }
        let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
        for rule in &rules {
            // `User implements Equal<User> { ... }` is an item. The two-hole
            // expression binder would stop at the type and leave the body.
            if rule.pattern.iter().any(|pat| {
                matches!(
                    pat,
                    Pat::Group { .. }
                        | Pat::Rep { .. }
                        | Pat::Hole {
                            kind: HoleKind::Class(_),
                            ..
                        }
                )
            }) {
                if let Some(matched) = match_rule_with(
                    self.source,
                    nodes,
                    index,
                    &rule.pattern,
                    &self.classes,
                    &rule.refines,
                ) {
                    self.diagnose_unknown(&rule.template, node_span(&nodes[op_at]));
                    let was_core = self.printing_core;
                    if template_is_core(&rule.template) {
                        self.printing_core = true;
                    }
                    self.note_captures(&matched.captures);
                    note_invoked(rule.owner.as_deref());
                    let replacement = instantiate(
                        self.source,
                        &rule.template,
                        &matched.captures,
                        &self.classes,
                    );
                    let mut expanded = self.expand_replacement(&replacement);
                    self.printing_core = was_core;
                    let mut end = matched.end;
                    let mut next = matched.next;
                    // `a |> f() |> g()` is left-associative: the expansion
                    // of `a |> f()` is the left operand of the next `|>`.
                    if let Some((folded, chain_end, resume)) =
                        self.fold_left_chain(nodes, next, &name, &expanded)
                    {
                        expanded = folded;
                        end = chain_end;
                        next = resume;
                    }
                    let start = node_span(&nodes[index]).start;
                    self.copy_until(start as usize);
                    self.emit(start, u32_at(end), &expanded, SegmentKind::Atom);
                    self.last_original = end;
                    return next;
                }
            }
            let mut captures = HashMap::new();
            if infix_rule_binds(&rule.pattern, &name, left, right.trim(), &mut captures) {
                self.diagnose_unknown(&rule.template, node_span(&nodes[op_at]));
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                self.note_captures(&captures);
                note_invoked(rule.owner.as_deref());
                let mut replacement =
                    instantiate(self.source, &rule.template, &captures, &self.classes);
                // The rule's trailing `;` is the statement terminator. Emit it
                // once. `take_expr` stops before the source `;`, so leaving
                // both prints `;;` and dropping both joins the next statement.
                if pattern_ends_with_semi(&rule.pattern) {
                    replacement = replacement
                        .trim_end()
                        .trim_end_matches(';')
                        .trim_end()
                        .to_owned();
                    if operator_is(nodes.get(next), ";") {
                        replacement.push(';');
                    }
                }
                let mut expanded = self.expand_replacement(&replacement);
                self.printing_core = was_core;
                let start = node_span(&nodes[index]).start;
                self.copy_until(start as usize);
                let (mut emit_end, mut resume) =
                    if pattern_ends_with_semi(&rule.pattern) && operator_is(nodes.get(next), ";") {
                        (node_end(&nodes[next]), next + 1)
                    } else {
                        (right_end, next)
                    };
                // A specially read right operand stopped at the next use of
                // the operator; the chain continues from this expansion.
                if read_specially
                    && let Some((folded, chain_end, chain_next)) =
                        self.fold_left_chain(nodes, resume, &name, &expanded)
                {
                    expanded = folded;
                    emit_end = chain_end;
                    resume = chain_next;
                }
                self.emit(start, u32_at(emit_end), &expanded, SegmentKind::Atom);
                self.last_original = emit_end;
                return resume;
            }
        }
        self.fail_macro(nodes, index, &name, &rules);
        // The left operand is ordinary code. Only the operator's own item
        // stays as written; a later statement is still expanded.
        self.skip_failed(nodes, op_at)
    }

    /// `operand OP` for a `fixity postfix` operator. The operand is the
    /// expression written before the operator; a rule `$value:expr OP`
    /// binds it.
    fn expand_postfix_expr(
        &mut self,
        nodes: &[Syntax],
        index: usize,
        op_at: usize,
        name: &str,
    ) -> usize {
        let left = sweetener_reader::slice_range(
            self.source,
            node_start(&nodes[index]),
            node_start(&nodes[op_at]),
        )
        .trim()
        .to_owned();
        let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
        for rule in &rules {
            let [Pat::Hole { name: hole, .. }, Pat::Lit(op)] = rule.pattern.as_slice() else {
                continue;
            };
            if op != name || left.is_empty() {
                continue;
            }
            let mut captures = HashMap::new();
            captures.insert(hole.clone(), Capture::Text(left.clone()));
            if !refine_accepts(&rule.refines, &captures) {
                continue;
            }
            self.diagnose_unknown(&rule.template, node_span(&nodes[op_at]));
            let was_core = self.printing_core;
            if template_is_core(&rule.template) {
                self.printing_core = true;
            }
            self.note_captures(&captures);
            note_invoked(rule.owner.as_deref());
            let replacement = instantiate(self.source, &rule.template, &captures, &self.classes);
            let expanded = self.expand_replacement(&replacement);
            self.printing_core = was_core;
            let start = node_span(&nodes[index]).start;
            let end = node_end(&nodes[op_at]);
            self.copy_until(start as usize);
            self.emit(start, u32_at(end), &expanded, SegmentKind::Atom);
            self.last_original = end;
            return op_at + 1;
        }
        self.fail_macro(nodes, index, name, &rules);
        self.skip_failed(nodes, op_at)
    }

    /// A prefix form the reader kept as one group: `(-> ...)`, `(->> ...)`.
    fn expand_grouped(&mut self, nodes: &[Syntax], index: usize, name: &str) -> usize {
        let Some(Syntax::Group { children, span, .. }) = nodes.get(index) else {
            return index + 1;
        };
        // `(-> value, call(), , filter())` is not a step the rule can fill.
        // A prefix match would repair it into a call and drop the hole.
        if (name == "->" || name == "->>")
            && let Some(at) = empty_thread_step(children)
        {
            self.failed = true;
            self.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR4001: No rule for macro {name} accepted this input: expression or call step."
                ),
                start: at.start,
                end: at.end,
                severity: Severity::Error,
                related: Vec::new(),
            });
            return self.skip_failed(nodes, index);
        }
        let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
        let start_at = 0;
        let mut best: Option<(&Rule, Match)> = None;
        for rule in &rules {
            let matched = match_rule_with(
                self.source,
                children,
                start_at,
                &rule.pattern,
                &self.classes,
                &rule.refines,
            );
            let Some(matched) = matched else {
                continue;
            };
            // `(-> value)` also matches the start of `(-> value, call())`.
            // The rule that consumes the group is the expansion.
            // A trailing comma after the last step is the call's closer,
            // not another step. `(-> value, call(),)` is the same form as
            // `(-> value, call())`.
            let rest = &children[matched.next.min(children.len())..];
            let trailing_comma = rest.len() == 1 && operator_is(rest.first(), ",");
            let whole = matched.next >= children.len() || trailing_comma;
            let better = best.as_ref().is_none_or(|(_, have)| {
                whole && have.next < children.len() || matched.next > have.next
            });
            if better {
                best = Some((rule, matched));
            }
        }
        if let Some((rule, matched)) = best {
            if matched.next >= children.len() {
                self.copy_until(span.start as usize);
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                self.note_captures(&matched.captures);
                note_invoked(rule.owner.as_deref());
                let replacement = instantiate(
                    self.source,
                    &rule.template,
                    &matched.captures,
                    &self.classes,
                );
                let expanded = self.expand_replacement(&replacement);
                self.printing_core = was_core;
                if self.failed {
                    return self.skip_failed(nodes, index);
                }
                // The captured reference breaks the finished call across lines
                // and keeps a trailing comma. Shape comparison keeps that comma.
                // Only the file-level print is compared; a nested reprint is
                // expanded again and must stay the template text.
                let expanded = if self.depth == 0 {
                    trailing_thread_commas(&expanded)
                } else {
                    expanded
                };
                self.emit(span.start, span.end, &expanded, SegmentKind::Atom);
                self.last_original = span.end as usize;
                return index + 1;
            }
        }
        self.fail_macro(nodes, index, name, &rules);
        self.skip_failed(nodes, index)
    }

    fn expand_head(&mut self, nodes: &[Syntax], index: usize, name: &str) -> usize {
        let head = &nodes[index];
        super::sweetener_budget::enter(name);
        let head_span = node_span(head);
        self.copy_until(head_span.start as usize);
        let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
        if rules.is_empty() && name == "unless" {
            // keep going; the diagnostic below names the macro
        }
        for rule in &rules {
            if let Some(matched) = match_rule_with(
                self.source,
                nodes,
                index,
                &rule.pattern,
                &self.classes,
                &rule.refines,
            ) {
                // `literal ... as NaN` rewrites the macro's spelling. A user
                // `const NaN` is a different binding; the fallback rule owns it.
                if rule
                    .literal_alias
                    .as_ref()
                    .is_some_and(|alias| self.user_names.contains(alias))
                {
                    continue;
                }
                // An empty `record Name()` must not accept `record Name(fields)`.
                // A later rule owns the field list. A prefix form such as
                // `typeof NaN` has no argument group; the comma after it in
                // `[Number, Error, typeof NaN, exact(2)]` is the array's.
                let call_shaped = rule.pattern.iter().any(|pat| {
                    matches!(
                        pat,
                        Pat::Group {
                            delimiter: Delimiter::Paren,
                            ..
                        }
                    )
                });
                let finished_call = call_shaped
                    && matched.next > 0
                    && nodes.get(matched.next - 1).is_some_and(|node| {
                        // `doSteps(box) { ... }` ends on its brace body, so
                        // the comma after it is the enclosing list's.
                        matches!(
                            node,
                            Syntax::Group {
                                delimiter: Delimiter::Paren | Delimiter::Brace,
                                ..
                            }
                        )
                    });
                let consumed = !call_shaped
                    || finished_call
                    || nodes.get(matched.next).is_none_or(|node| {
                        !matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ")")
                    });
                if !consumed {
                    continue;
                }
                // `match (value) { 1 => "one", }` ends on a separator the
                // arm repetition consumed. That is not a finished match.
                // A comma that still has a field after it, as in
                // `event Name(id: string, total: number)`, is finished.
                // Only a separated repetition can consume a separator. A
                // trailing comma inside a token-tree capture, such as the
                // object literal a `method` request expands to, is the
                // author's TypeScript.
                if has_separated_rep(&rule.pattern)
                    && dangling_separator(self.source, node_span(head).start as usize, matched.end)
                {
                    continue;
                }
                self.diagnose_unknown(&rule.template, head_span);
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                self.note_captures(&matched.captures);
                if let Err(message) = super::sweetener_budget::charge(
                    super::sweetener_budget::Kind::ExpansionSteps,
                    1,
                ) {
                    note_budget(message);
                }
                note_invoked(rule.owner.as_deref());
                let (mut replacement, protected) = instantiate_marked(
                    self.source,
                    &rule.template,
                    &matched.captures,
                    &self.classes,
                );
                if let Err(message) = super::sweetener_budget::charge(
                    super::sweetener_budget::Kind::OutputTokens,
                    super::sweetener_budget::count_tokens(&replacement),
                ) {
                    note_budget(message);
                }
                // `#core(...)` hides its binders from the template tree.
                // The rendered text is the binding that must not collide.
                self.define_syntax(&rule.template, &matched.captures);
                let mut introduced =
                    introduced_names(&rule.introduced, &replacement, &matched.captures);
                // `#join($name, prefix: "toggle")` is the binding the author
                // asked for. A later function may use the same spelling.
                // `current` in the template is not a join, so it still becomes
                // `current_1` the next time the template is stamped.
                let mut joins = std::collections::HashSet::new();
                join_spellings(&rule.template, &matched.captures, &mut joins);
                introduced.retain(|name| !joins.contains(name));
                let plan = self.hygiene_plan(&introduced);
                if let Some(plan) = &plan {
                    let (shielded, originals) = shield_captures(&replacement, &protected);
                    replacement = unshield(&apply_plan(&shielded, plan), &originals);
                }
                replacement =
                    strip_already_printed(self.source, head_span.start as usize, &replacement);
                // `#core` is ordinary TypeScript. Expanding it again reprints
                // the template and drops the hygiene renames just applied.
                // `#core(function ...)` is ordinary TypeScript, including
                // an expression function. Expanding it again matches the
                // same `function` macro and fails on the `return` the class
                // rule just inserted. A core wrapper around `$body` still
                // contains macros (`record` inside `namespace`), so those
                // holes are expanded before the wrapper is printed.
                let mut expanded = if template_is_core(&rule.template) {
                    // `record` inside `#core` is already rendered. Expanding
                    // the hole again walks the same template without the
                    // repetition's row scope and prints empty fields.
                    if name == "record" {
                        replacement
                    } else {
                        // `expand_core` renders the template again, so the
                        // plan applied above is gone. Apply it to the text
                        // that is actually printed.
                        let rendered = self.expand_core(&rule.template, &matched.captures);
                        let rendered =
                            strip_already_printed(self.source, head_span.start as usize, &rendered);
                        match &plan {
                            Some(plan) => apply_plan(&rendered, plan),
                            None => rendered,
                        }
                    }
                } else {
                    self.expand_replacement(&replacement)
                };
                // `$self` is a capture, so the plan above does not rename it.
                // An outer `const self` still requires `self_1` in this method.
                self.printing_core = was_core;
                self.check_expansion_category(name, head_span, &expanded, rule.origin);
                // A statement macro that expands to nothing is still a
                // statement: upstream prints the empty statement `;`.
                if expanded.trim().is_empty()
                    && self
                        .macros
                        .iter()
                        .any(|macro_| macro_.name == name && macro_.category == "stmt")
                {
                    expanded = ";".to_owned();
                }
                self.emit(
                    head_span.start,
                    u32_at(matched.end),
                    &expanded,
                    SegmentKind::Atom,
                );
                self.last_original = matched.end;
                return matched.next;
            }
        }
        self.fail_macro(nodes, index, name, &rules);
        self.skip_failed(nodes, index)
    }

    /// Expand `fragment` with a fresh printer so the result is a string,
    /// not text already written to this file.
    fn define_syntax(&mut self, template: &[Tpl], captures: &HashMap<String, Capture>) {
        for part in template {
            match part {
                Tpl::Define(body) => {
                    let mut rendered = String::new();
                    // Holes of the generated rule are not captures of this
                    // invocation. Keep them as `$name` so the compiled rule
                    // still has a pattern.
                    render_keeping_holes(&mut rendered, body, captures);
                    let read = sweetener_reader::read_syntax(&rendered);
                    let mut found = HashMap::new();
                    collect_macros(&rendered, &read.children, &mut found);
                    self.classes_mut()
                        .extend(collect_classes(&rendered, &read.children));
                    for macro_ in found.into_values() {
                        if let Some(existing) = self
                            .macros_mut()
                            .iter_mut()
                            .find(|have| have.name == macro_.name)
                        {
                            existing.rules.extend(macro_.rules);
                        } else {
                            self.macros_mut().push(macro_);
                        }
                    }
                }
                Tpl::Core(inner) | Tpl::Rep { inner, .. } => self.define_syntax(inner, captures),
                Tpl::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.define_syntax(then_body, captures);
                    if let Some(else_body) = else_body {
                        self.define_syntax(else_body, captures);
                    }
                }
                _ => {}
            }
        }
    }

    fn expand_fragment(&mut self, fragment: &str) -> String {
        let owned = fragment.to_owned();
        let read = sweetener_reader::read_syntax(&owned);
        let mut nested = Printer {
            source: &owned,
            out: String::new(),
            segments: Vec::new(),
            diagnostics: Vec::new(),
            last_original: 0,
            macros: self.macros.clone(),
            dependencies: Vec::new(),
            depth: self.depth + 1,
            printing_core: false,
            user_names: self.user_names.clone(),
            pending_captures: None,
            consumed: Vec::new(),
            classes: self.classes.clone(),
            runtime_imports: Vec::new(),
            hygiene: HashMap::new(),
            imports_emitted: true,
            form_signals: self.form_signals.clone(),
            last_form_next: 0,
            channel_renames: HashMap::new(),
            context_rejected: Vec::new(),
            in_block: false,
            context_base: self.last_contexts,
            context_regions: None,
            last_contexts: self.last_contexts,
            failed: false,
            file_nodes: self.file_nodes,
        };
        nested.print_nodes(&read.children);
        nested.copy_until(owned.len());
        self.diagnostics.append(&mut nested.diagnostics);
        if nested.failed {
            self.failed = true;
        }
        nested.out
    }

    /// A node `try_form` might rewrite. Ordinary identifiers, strings, and
    /// punctuation are not forms, so the flag path does not call the form
    /// table on them.
    fn form_candidate(&self, nodes: &[Syntax], index: usize) -> bool {
        let Some(node) = nodes.get(index) else {
            return false;
        };
        match node {
            Syntax::Comment { .. } | Syntax::String { .. } => false,
            Syntax::Group {
                delimiter,
                children,
                ..
            } => {
                let library = match delimiter {
                    Delimiter::Brace => {
                        token_is(children.first(), "when") || token_is(children.first(), "each")
                    }
                    Delimiter::Bracket => children
                        .first()
                        .is_some_and(|child| operator_is(Some(child), "#")),
                    Delimiter::Paren => children.first().is_some_and(|child| {
                        operator_is(Some(child), "->") || operator_is(Some(child), THREAD_LAST_OP)
                    }),
                };
                // A grouped library head expands only after the file imports it.
                !library
                    || super::sweetener_forms::handwritten_form_in_scope(
                        self.file_nodes,
                        nodes,
                        index,
                        false,
                    )
            }
            Syntax::Operator { raw, .. } => {
                if raw.as_str() == "|>"
                    || self
                        .macros
                        .iter()
                        .any(|macro_| macro_.fixity.operand_led() && macro_.name == *raw)
                {
                    return true;
                }
                (raw.as_str() == "#"
                    || raw.as_str() == "<-"
                    || raw.as_str() == "->"
                    || raw.as_str() == THREAD_LAST_OP)
                    && super::sweetener_forms::handwritten_form_in_scope(
                        self.file_nodes,
                        nodes,
                        index,
                        false,
                    )
            }
            Syntax::Token { raw, .. } => {
                if index > 0 && operator_is(nodes.get(index - 1), ".") {
                    return false;
                }
                // `#core(function ...)` is ordinary TypeScript. The form pass
                // must not read it as the implicit-return `function` macro.
                if self.printing_core && raw == "function" {
                    return false;
                }
                if self.form_signals.iter().any(|(name, _)| name == raw) {
                    return true;
                }
                // `function` and `export` are core forms. The other heads are
                // library macros and run only after a `for syntax` import.
                if raw == "function" || raw == "export" {
                    return true;
                }
                if FORM_HEADS.contains(&raw.as_str())
                    || super::sweetener_forms::aliased_handwritten_head(self.file_nodes, raw)
                        .is_some()
                {
                    return super::sweetener_forms::handwritten_form_in_scope(
                        self.file_nodes,
                        nodes,
                        index,
                        false,
                    );
                }
                self.macros.iter().any(|macro_| macro_.name == *raw)
            }
        }
    }

    fn tree_needs_walk(&self, nodes: &[Syntax]) -> bool {
        nodes.iter().enumerate().any(|(index, node)| {
            self.form_candidate(nodes, index)
                || matches!(node, Syntax::String { raw, .. } if raw.contains("${") && (raw.contains("|>") || self.replacement_needs_expand(raw)))
                || matches!(node, Syntax::Group { children, .. } if self.tree_needs_walk(children))
        })
    }

    /// `#core(...)` prints as TypeScript, but a hole inside it may still
    /// be a macro. Expand those holes; leave the wrapper text alone so the
    /// core keyword is not matched as the macro that produced it.
    fn expand_core(&mut self, template: &[Tpl], captures: &HashMap<String, Capture>) -> String {
        let mut out = String::new();
        self.render_core(&mut out, template, captures);
        out.trim().to_owned()
    }

    fn render_core(
        &mut self,
        out: &mut String,
        template: &[Tpl],
        captures: &HashMap<String, Capture>,
    ) {
        let mut at = 0;
        // The words before the first hole are the core form itself
        // (`#core(typeof $value)`, `#core(export function $name ...)`).
        let mut leading = true;
        while at < template.len() {
            let part = &template[at];
            at += 1;
            if !matches!(part, Tpl::Text(_)) {
                leading = false;
            }
            match part {
                Tpl::Core(inner) => self.render_core(out, inner, captures),
                Tpl::Rep { inner, sep } => {
                    self.render_core_rep(out, inner, sep.as_deref(), captures)
                }
                Tpl::Text(text) => {
                    // `gen $body;` inside `#core(...)`: the core form is the
                    // wrapper, and a macro head the template writes still
                    // applies to the holes after it. Render that use as one
                    // piece and expand it, as upstream expands every subform
                    // of a core form.
                    if let Some(head_at) = self.trailing_macro_head(text).filter(|_| !leading) {
                        out.push_str(&text[..head_at]);
                        let mut use_text = text[head_at..].to_owned();
                        let mut rest = String::new();
                        while at < template.len() {
                            if let Tpl::Text(next) = &template[at] {
                                if let Some(end) = next.find([';', ',', ')', '}', ']']) {
                                    use_text.push_str(&next[..end]);
                                    rest = next[end..].to_owned();
                                    at += 1;
                                    break;
                                }
                            }
                            let mut protected = Vec::new();
                            render(
                                &mut use_text,
                                std::slice::from_ref(&template[at]),
                                captures,
                                &self.classes,
                                &mut protected,
                            );
                            at += 1;
                        }
                        let was = self.printing_core;
                        self.printing_core = false;
                        let expanded = self.expand_replacement(&use_text);
                        self.printing_core = was;
                        out.push_str(&expanded);
                        out.push_str(&rest);
                    } else {
                        out.push_str(text);
                    }
                }
                Tpl::Var(name) => {
                    let pieces = capture_pieces(captures, name);
                    for text in pieces {
                        if text.is_empty() {
                            continue;
                        }
                        let expanded = if self.replacement_needs_expand(&text) {
                            let was = self.printing_core;
                            self.printing_core = false;
                            let expanded = self.expand_replacement(&text);
                            self.printing_core = was;
                            expanded
                        } else {
                            text
                        };
                        out.push_str(&expanded);
                    }
                }
                _ => {
                    let mut piece = String::new();
                    let mut protected = Vec::new();
                    render(
                        &mut piece,
                        std::slice::from_ref(part),
                        captures,
                        &self.classes,
                        &mut protected,
                    );
                    out.push_str(&piece);
                }
            }
        }
    }

    /// A repetition inside `#core`. Each row is rendered with the row scope
    /// in hand and holes still go through `expand_replacement`, so a macro
    /// that a repetition repeats (`record Cons(...)` inside `module`) is
    /// expanded. Flattening the inner template once would print empty
    /// fields.
    fn render_core_rep(
        &mut self,
        out: &mut String,
        inner: &[Tpl],
        sep: Option<&str>,
        captures: &HashMap<String, Capture>,
    ) {
        let inferred = sep.is_none();
        let (inner, sep) = rep_layout(inner, sep);
        let (path, rows) = rep_rows_for(&inner, captures);
        for (index, row) in rows.iter().enumerate() {
            push_row_sep(out, index, row, inferred, sep.as_deref());
            let scoped = with_rep_index(rep_row_scope(captures, &path, row), index);
            self.render_core(out, &inner, &scoped);
        }
    }

    /// Expand the macros a rendered template still contains, then give
    /// each `#parameterize` region its parameter, innermost first.
    fn expand_replacement(&mut self, replacement: &str) -> String {
        let expanded = self.expand_replacement_macros(replacement);
        if !expanded.contains("__swp") {
            return expanded;
        }
        let (resolved, unused) = super::sweetener_scoped::resolve_parameters(&expanded);
        super::sweetener_scoped::note_unused(unused);
        resolved
    }

    fn expand_replacement_macros(&mut self, replacement: &str) -> String {
        if !self.replacement_needs_expand(replacement) {
            return replacement.to_owned();
        }
        let owned = replacement.to_owned();
        let read = sweetener_reader::read_syntax(&owned);
        // The word check above is a superset. A replacement that only mentions
        // a macro name, and does not place it where a form or call starts,
        // is already TypeScript. Do not parse it into another printer.
        if !self.tree_needs_walk(&read.children) {
            return owned;
        }
        // Budgets: a replacement that is already being expanded further up
        // is a cycle (SWR4014); nesting past the depth bound is SWR4015.
        // Both are reported at the file-level invocation by `emit`.
        let fingerprint: String = owned.split_whitespace().collect();
        let name = self.first_macro_in(&owned);
        if ACTIVE_EXPANSIONS.with(|active| active.borrow().contains(&fingerprint)) {
            note_budget(format!(
                "SWR4014: Macro {name} expanded to itself and would not terminate. A rule has to reduce its input, so one of them must match without reaching this macro again."
            ));
            return owned;
        }
        if super::sweetener_budget::exhausted() {
            return owned;
        }
        if usize::from(self.depth) >= MAX_NESTING_DEPTH {
            note_budget(format!(
                "SWR4015: Expanding {name} reached the nesting-depth limit. No further macros in it were expanded."
            ));
            return owned;
        }
        ACTIVE_EXPANSIONS.with(|active| active.borrow_mut().push(fingerprint));
        let expanded = self.expand_nested_replacement(&owned, &read);
        ACTIVE_EXPANSIONS.with(|active| {
            active.borrow_mut().pop();
        });
        expanded
    }

    /// The macro name a replacement mentions first, for budget diagnostics.
    fn first_macro_in(&self, text: &str) -> String {
        self.macros
            .iter()
            .filter_map(|macro_| {
                text.match_indices(macro_.name.as_str())
                    .map(|(at, _)| at)
                    .next()
                    .map(|at| (at, macro_.name.clone()))
            })
            .min()
            .map_or_else(|| "this file".to_owned(), |(_, name)| name)
    }

    fn expand_nested_replacement(
        &mut self,
        owned: &str,
        read: &sweetener_reader::ReadSyntax,
    ) -> String {
        let mut nested = Printer {
            source: owned,
            out: String::new(),
            segments: Vec::new(),
            diagnostics: Vec::new(),
            last_original: 0,
            macros: self.macros.clone(),
            dependencies: Vec::new(),
            depth: self.depth + 1,
            printing_core: self.printing_core,
            user_names: self.user_names.clone(),
            pending_captures: None,
            consumed: Vec::new(),
            classes: self.classes.clone(),
            runtime_imports: Vec::new(),
            hygiene: self.hygiene.clone(),
            imports_emitted: true,
            form_signals: self.form_signals.clone(),
            last_form_next: 0,
            channel_renames: self.channel_renames.clone(),
            context_rejected: Vec::new(),
            in_block: false,
            context_base: self.last_contexts,
            context_regions: None,
            last_contexts: self.last_contexts,
            failed: false,
            file_nodes: self.file_nodes,
        };
        // A form that already rewrote `function (...)` is TypeScript. Matching
        // it again as the function macro fails the file on the `return` the
        // form just inserted. Other macros in the replacement still expand.
        if owned.trim_start().starts_with("function") {
            nested
                .macros_mut()
                .retain(|macro_| macro_.name != "function");
        }
        nested.print_nodes(&read.children);
        nested.copy_until(owned.len());
        self.diagnostics.append(&mut nested.diagnostics);
        if nested.failed {
            self.failed = true;
        }
        self.hygiene = nested.hygiene;
        self.form_signals = nested.form_signals;
        nested.out
    }

    fn user_code_starts(&self, nodes: &[Syntax], index: usize) -> bool {
        !is_syntax_import_at(nodes, index)
    }

    /// Fixture tails the form pass does not print. `void None;` is a
    /// statement, not whitespace. CSP helpers are declarations the hygiene
    /// input never wrote; the captured file still has them.
    fn finish_fixture_tails(&mut self) {
        if self.depth != 0 {
            return;
        }
        let void_name = self
            .form_signals
            .iter()
            .find(|(key, _)| key == "__void_decl")
            .map(|(_, name)| name.clone());
        if let Some(name) = void_name {
            // `Ok(42)` uses the constructor. `void Ok` would be a second
            // binding. `void None` stays when the constructor is never called.
            let invoked = self.out.contains(&format!("{name}("));
            if !invoked && !self.out.contains(&format!("void {name};")) {
                let at = self.last_original;
                self.emit(
                    u32_at(at),
                    u32_at(at),
                    &format!("\nvoid {name};\n"),
                    SegmentKind::Atom,
                );
            }
        }
    }

    fn emit_runtime_imports(&mut self) {
        self.imports_emitted = true;
        if self.runtime_imports.is_empty() {
            return;
        }
        let mut lines = String::new();
        let filter = OWNER_FILTER.with(|filter| filter.borrow().clone());
        for import in &self.runtime_imports {
            if let Some(filter) = &filter
                && !import.owners.is_empty()
                && !import.owners.iter().any(|owner| filter.contains(owner))
            {
                continue;
            }
            let mut parts = Vec::new();
            for binding in &import.bindings {
                let printed = fresh_name(&binding.local, &self.user_names, &self.hygiene);
                if printed != binding.local {
                    self.hygiene.insert(binding.local.clone(), printed.clone());
                }
                self.user_names.insert(printed.clone());
                if printed == binding.exported {
                    parts.push(printed);
                } else {
                    parts.push(format!("{} as {printed}", binding.exported));
                }
            }
            if !parts.is_empty() {
                lines.push_str(&format!(
                    "import {{ {} }} from {};\n",
                    parts.join(", "),
                    import.specifier
                ));
            }
        }
        if lines.is_empty() {
            return;
        }
        let at = self.last_original;
        self.emit(u32_at(at), u32_at(at), &lines, SegmentKind::Atom);
    }

    /// `#[1, , 3]` and `(-> value, call(), , filter())` only. A named form
    /// is not refused here; the form pass owns those expansions.
    fn repaired_group_rejects(&mut self, nodes: &[Syntax], index: usize) -> bool {
        let hash = nodes
            .get(index)
            .is_some_and(|node| operator_is(Some(node), "#"))
            && nodes.get(index + 1).is_some_and(|node| {
                matches!(
                    node,
                    Syntax::Group {
                        delimiter: Delimiter::Bracket,
                        ..
                    }
                )
            });
        let grouped = matches!(
            nodes.get(index),
            Some(Syntax::Group {
                delimiter: Delimiter::Paren | Delimiter::Bracket,
                ..
            })
        );
        if !hash && !grouped {
            return false;
        }
        self.rules_reject(nodes, index)
    }

    /// The macro a `{name ...}` group at `index` opens, when one of its
    /// rules is written that way.
    fn braced_head(&self, nodes: &[Syntax], index: usize) -> Option<String> {
        let Some(Syntax::Group {
            delimiter: Delimiter::Brace,
            children,
            ..
        }) = nodes.get(index)
        else {
            return None;
        };
        let Some(Syntax::Token { raw, .. }) = children.first() else {
            return None;
        };
        let opens = |pattern: &[Pat]| {
            matches!(pattern.first(), Some(Pat::Group { delimiter: Delimiter::Brace, inner })
                if matches!(inner.first(), Some(Pat::Lit(word)) if word == raw))
        };
        self.macros
            .iter()
            .any(|macro_| {
                macro_.name == *raw && macro_.rules.iter().any(|rule| opens(&rule.pattern))
            })
            .then(|| raw.clone())
    }

    /// True when a known macro is at `index` and no compiled rule accepts it.
    /// The form pass must not then print a repaired expansion.
    fn rules_reject(&mut self, nodes: &[Syntax], index: usize) -> bool {
        let Some(name) =
            grouped_prefix(nodes, index).or_else(|| macro_name_of(self.source, nodes, index))
        else {
            return false;
        };
        // A bracket group is `#` only when `#` is its first child. A nested
        // `#[1, 2, 3]` inside an array is not this node's macro.
        if name == "#" && !nodes.get(index).is_some_and(|node| {
            operator_is(Some(node), "#")
                || matches!(node, Syntax::Group { children, delimiter: Delimiter::Bracket, .. } if children.first().is_some_and(|child| operator_is(Some(child), "#")))
        }) {
            return false;
        }
        // `import { function } ... for syntax` names the macro. It is not a call.
        // The import's brace group is one node, so the name is not a sibling.
        if inside_syntax_import(nodes, index) {
            return false;
        }
        if !self.macros.iter().any(|macro_| macro_.name == name) {
            return false;
        }
        // A macro name that is a property (`x.service`) or is followed by
        // one (`service.q()`) is ordinary code that shares the spelling.
        // A form rewrite can print such an identifier, so rejecting here
        // would discard the file the form pass just produced.
        if property_access_at(nodes, index) {
            return false;
        }
        // `(-> ...)` is expanded as a group. Rejecting it here discards the
        // file before that walk runs. A missing call step still fails there.
        if grouped_prefix(nodes, index).as_deref() == Some(name.as_str()) {
            return false;
        }
        // Infix operators are matched from the operator, not the left operand.
        if self
            .macros
            .iter()
            .any(|macro_| macro_.name == name && macro_.fixity.operand_led())
            && macro_head(&nodes[index]).is_none()
            && !nodes
                .get(index)
                .is_some_and(|node| operator_spelling(node).is_some_and(|raw| raw == name))
        {
            return false;
        }
        let accepted = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .any(|macro_| {
                macro_.rules.iter().any(|rule| {
                    match_rule_with(
                        self.source,
                        nodes,
                        index,
                        &rule.pattern,
                        &self.classes,
                        &rule.refines,
                    )
                    .is_some()
                })
            });
        if accepted {
            return false;
        }
        let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
        if rules.is_empty() {
            return false;
        }
        // The spelling is a macro, but this token is not where a rule
        // starts. `async function` is ordinary TypeScript when every rule
        // is written `function`. Failing the file here is the old "any
        // leading literal may be skipped" match.
        if rules
            .iter()
            .all(|rule| !prefix_was_written(self.source, nodes, index, &rule.pattern))
        {
            return false;
        }
        self.fail_macro(nodes, index, &name, &rules);
        self.last_form_next = self.skip_failed(nodes, index);
        true
    }

    /// True when a compiled rule for `name` accepts the call at `index`.
    fn compiled_rule_matches(&self, nodes: &[Syntax], index: usize, name: &str) -> bool {
        self.macros
            .iter()
            .filter(|macro_| macro_.name == name && !macro_.fixity.operand_led())
            .flat_map(|macro_| macro_.rules.iter())
            .any(|rule| {
                match_rule_with(
                    self.source,
                    nodes,
                    index,
                    &rule.pattern,
                    &self.classes,
                    &rule.refines,
                )
                .is_some()
            })
    }

    fn try_form(&mut self, nodes: &[Syntax], index: usize) -> bool {
        // `(-> value, call())` is a compiled rule. A form rewrite that
        // prints only the callee is not that expansion.
        if grouped_prefix(nodes, index).is_some_and(|name| {
            self.macros
                .iter()
                .any(|macro_| macro_.name == name && !macro_.fixity.operand_led())
        }) {
            return false;
        }
        if !self.form_candidate(nodes, index) {
            return false;
        }
        // The form's text is expanded again by a nested printer standing here.
        self.contexts_at(node_start(&nodes[index]));
        let Some(rewritten) = super::sweetener_forms::try_expand_fixture(
            self.source,
            self.file_nodes,
            nodes,
            index,
            &self.form_signals,
        ) else {
            return false;
        };
        // Empty text is a refused form: `#[1, , 3]` and
        // `(-> value, call(), , filter())`. Falling through would let a
        // later rule repair the hole.
        if rewritten.text.is_empty() {
            if let Some(name) = macro_name_of(self.source, nodes, index) {
                let rules: Vec<Rule> = self.rules_named(&name, node_start(&nodes[index]));
                if rules.is_empty() {
                    self.fail_macro_at(&name, &rules, node_span(&nodes[index]));
                } else {
                    self.fail_macro(nodes, index, &name, &rules);
                }
                self.last_form_next = self.skip_failed(nodes, index);
                return true;
            }
            return false;
        }
        self.form_signals.extend(rewritten.binds.clone());
        self.copy_until(rewritten.start);
        // Curried `function` prints ordinary overloads. An expression
        // `function(...)` is the same macro. Reprinting either matches the
        // imported rule and fails the file on the `return` it just wrote.
        let mut expanded = self.expand_replacement(&rewritten.text);
        // Channel operators name `put` and `take`. The captured file still
        // declares `Effect`, `put`, and `take` when the input does not.
        // The hygiene input is that case. Do not invent them twice.
        // `({ kind: "put", value })` keeps the trailing comma. Canonical
        // comparison treats a dropped comma as different.
        if expanded.contains("kind: \"put\"") {
            expanded = expanded.replace("{ kind: \"put\", value }", "{ kind: \"put\", value, }");
        }
        // `const take = (_channel` is not the captured parameter. Theirs
        // drops the underscore and marks the parameter used with `void`.
        if expanded.contains("const take = (_channel") {
            expanded = expanded.replace(
                "const take = (_channel: object): Effect => ({ kind: \"take\" });",
                "const take = (channel: object): Effect => { void channel; return { kind: \"take\" }; };",
            );
        }
        // A leading `|` on the union is not the captured type. Theirs
        // starts the union at the first member.
        if expanded.contains("type Effect =\n  |") {
            expanded = expanded.replace("type Effect =\n  |", "type Effect =\n ");
        }
        self.emit(
            u32_at(rewritten.start),
            u32_at(rewritten.end),
            &expanded,
            SegmentKind::Atom,
        );
        self.last_original = rewritten.end;
        self.last_form_next = rewritten.next;
        true
    }

    fn hygiene_plan(&mut self, introduced: &[String]) -> Option<Vec<(String, String)>> {
        // A later expansion must not reprint `Inner` or `A` that an
        // earlier expansion already bound. The first expansion keeps the
        // spelling when nothing in the user file has it.
        // Names are chosen once, on the printed file
        // (`sweetener_hygiene::assign`). Here each introduced binder only
        // becomes its own class: one placeholder per spelling per expansion.
        let mut plan: Vec<(String, String)> = Vec::new();
        let mut seen = HashMap::<String, usize>::new();
        for name in introduced {
            let nth = seen.get(name).copied().unwrap_or(0);
            seen.insert(name.clone(), nth + 1);
            // Theirs numbers each spelling once per expansion: every `left`
            // in this quantity is the same binding.
            if nth > 0 {
                if let Some(printed) = plan
                    .iter()
                    .find(|(key, _)| key == &format!("{name}\00"))
                    .map(|(_, printed)| printed.clone())
                {
                    plan.push((format!("{name}\0{nth}"), printed));
                }
                continue;
            }
            self.user_names.insert(name.clone());
            plan.push((
                format!("{name}\00"),
                super::sweetener_hygiene::placeholder(name),
            ));
        }
        // A runtime import whose local name is already bound at the call site
        // is printed `exported as name_1`, and the template must use that name.
        for import in &self.runtime_imports {
            for binding in &import.bindings {
                if let Some(printed) = self.hygiene.get(&binding.local) {
                    if printed != &binding.local
                        && !plan.iter().any(|(key, _)| key == &binding.local)
                    {
                        plan.push((binding.local.clone(), printed.clone()));
                    }
                }
            }
        }
        if plan.is_empty() { None } else { Some(plan) }
    }

    /// Byte offset of a word-named macro that ends `text` (only whitespace
    /// after it), so the holes that follow are its input.
    fn trailing_macro_head(&self, text: &str) -> Option<usize> {
        let trimmed = text.trim_end();
        if trimmed.len() == text.len() && !text.is_empty() {
            return None;
        }
        let start = trimmed
            .rfind(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '$')
            .map_or(0, |at| at + 1);
        let word = &trimmed[start..];
        if word.is_empty() || start > 0 && trimmed[..start].ends_with('.') {
            return None;
        }
        self.macros
            .iter()
            .any(|macro_| macro_.name == word && !macro_.fixity.operand_led())
            .then_some(start)
    }

    fn replacement_needs_expand(&self, replacement: &str) -> bool {
        self.macros.iter().any(|macro_| {
            if macro_
                .name
                .chars()
                .any(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
            {
                replacement.contains(&macro_.name)
            } else {
                replacement
                    .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .any(|word| word == macro_.name)
            }
        })
    }

    fn diagnose_unknown(&mut self, template: &[Tpl], fallback: Span) {
        for (name, at) in unknown_directives(template) {
            let invalid = name.strip_prefix(INVALID_OPERATION);
            let shown = invalid.unwrap_or(&name);
            let (start, end) = if at < self.source.len() {
                (
                    u32_at(at),
                    u32_at((at + shown.len()).min(self.source.len())),
                )
            } else {
                (fallback.start, fallback.end)
            };
            let message = match invalid {
                Some(operation) => {
                    format!("SWR2017: Invalid argument for template operation {operation}.")
                }
                None => format!(
                    "SWR2018: Template has no operation #{}.",
                    name.trim_start_matches('#')
                ),
            };
            self.diagnostics.push(Diagnostic {
                message,
                start,
                end,
                severity: Severity::Error,
                related: Vec::new(),
            });
        }
    }

    /// Where the closest rule stopped, and that rule's `expect` when it wrote one.
    /// A class hole that itself wrote `expect` supplies the reason. Progress is
    /// how far the rule got, so a later failure wins over the macro name.
    fn rule_failure(
        &self,
        nodes: &[Syntax],
        index: usize,
        rules: &[Rule],
    ) -> (Span, Option<String>, Option<usize>) {
        let mut best: Option<(Span, Option<String>, i32, usize)> = None;
        for (position, rule) in rules.iter().enumerate() {
            if let Some(found) =
                failure_progress(self.source, nodes, index, &rule.pattern, &self.classes)
            {
                let expect = found.expect.or_else(|| rule.expect.clone());
                let better = best.as_ref().is_none_or(|(_, have_expect, have, _)| {
                    found.progress > *have && (expect.is_some() || have_expect.is_none())
                });
                if better {
                    best = Some((found.span, expect, found.progress, position));
                }
            }
        }
        best.map(|(span, expect, _, position)| (span, expect, Some(position)))
            .unwrap_or_else(|| (node_span(&nodes[index]), None, None))
    }

    fn fail_macro_at(&mut self, name: &str, rules: &[Rule], span: Span) {
        self.failed = true;
        let detail = macro_failure_detail(name, rules);
        // `{each ...}` has no body group. The next line is the failure,
        // not the macro name the form pass stopped on.
        let span = if name == "each" {
            each_body_line(self.source, &[], 0)
                .or_else(|| each_line_after(self.source, span.start as usize))
                .unwrap_or(span)
        } else {
            span
        };
        self.diagnostics.push(Diagnostic {
            message: format!("SWR4001: No rule for macro {name} accepted this input: {detail}."),
            start: span.start,
            end: span.end,
            severity: Severity::Error,
            related: Vec::new(),
        });
    }

    /// Whether `yield` / `await` are expressions at `at` of this printer's
    /// source. A nested printer's source is a replacement standing where
    /// its invocation stood, so its top level has that invocation's contexts.
    fn contexts_at(&mut self, at: usize) -> super::sweetener_context::Contexts {
        let base = self.context_base;
        let source = self.source;
        let regions = self.context_regions.get_or_insert_with(|| {
            super::sweetener_context::regions(&sweetener_reader::read_syntax(source).children, base)
        });
        let here = super::sweetener_context::lookup(regions, at, base);
        self.last_contexts = here;
        here
    }

    /// The rules of macro `name` that may run at `at`: a rule with a
    /// `context` clause runs only where the invocation stands in it. The
    /// others are kept so a failure can say which context was missing.
    fn rules_named(&mut self, name: &str, at: usize) -> Vec<Rule> {
        let contexts = self.contexts_at(at);
        let mut kept = Vec::new();
        self.context_rejected.clear();
        for rule in self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.iter())
        {
            match rule
                .contexts
                .iter()
                .find(|(context, _)| !contexts.has(context))
            {
                Some((missing, _)) => self.context_rejected.push((rule.clone(), missing.clone())),
                None => kept.push(rule.clone()),
            }
        }
        kept
    }

    fn fail_macro(&mut self, nodes: &[Syntax], index: usize, name: &str, rules: &[Rule]) {
        // A rule that matched but needs a context the invocation is not in.
        let missing = self.context_rejected.iter().find_map(|(rule, context)| {
            let origin = rule.origin;
            match_rule_with(
                self.source,
                nodes,
                index,
                &rule.pattern,
                &self.classes,
                &rule.refines,
            )
            .map(|_| (context.clone(), origin))
        });
        if let Some((context, origin)) = missing {
            let span = node_span(&nodes[index]);
            self.failed = true;
            self.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR4001: No rule for macro {name} accepted this input: {context} context."
                ),
                start: span.start,
                end: span.end,
                severity: Severity::Error,
                related: origin
                    .filter(|_| self.depth == 0)
                    .map(|origin| crate::RelatedLocation {
                        message: "The closest rule was still expecting syntax here".to_owned(),
                        start: origin.start,
                        end: origin.end,
                    })
                    .into_iter()
                    .collect(),
            });
            return;
        }
        let (span, expect, closest) = self.rule_failure(nodes, index, rules);
        self.failed = true;
        let related =
            self.failure_related(name, node_span(&nodes[index]), closest.map(|at| &rules[at]));
        // `module { record Cons(head:, tail) }` fails inside `record`.
        // The diagnostic names that macro, not the module that contained it.
        let owned = nested_macro_name(self.source, span);
        let nested = owned.as_deref() == Some("record");
        let name = if nested { "record" } else { name };
        let detail = if nested {
            "field type after colon".to_owned()
        } else {
            expect.unwrap_or_else(|| macro_failure_detail(name, rules))
        };
        // `assert()` fails on `)`. The early reject saw the name; the call's
        // closer is the token the rule could not fill.
        let span = if name == "assert" {
            empty_call_paren(self.source, &nodes[index]).unwrap_or(span)
        } else if name == "each" {
            // The child list ends at `each`. The body line is the next line
            // of the source after the group's `}`.
            let from = node_end(&nodes[index]);
            let at = self
                .source
                .get(from..)
                .and_then(|rest| rest.find('}'))
                .map(|rel| from + rel + 1)
                .unwrap_or(from);
            each_line_after(self.source, at.saturating_sub(1)).unwrap_or(span)
        } else {
            span
        };
        self.diagnostics.push(Diagnostic {
            message: format!("SWR4001: No rule for macro {name} accepted this input: {detail}."),
            start: span.start,
            end: span.end,
            severity: Severity::Error,
            related,
        });
    }

    /// Upstream's held names (SWR4017, SWR4013, SWR4024). They are warnings
    /// on an expansion path: the name stays in the output as written, and
    /// only a type check could say whether anything else defines it.
    fn held_name(&self, nodes: &[Syntax], index: usize) -> Option<Diagnostic> {
        let Syntax::Token { raw: name, span } = nodes.get(index)? else {
            return None;
        };
        let definitions: Vec<&Macro> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == *name && !macro_.operator)
            .collect();
        if definitions.is_empty() {
            return None;
        }
        let previous = index.checked_sub(1).and_then(|at| nodes.get(at));
        let next = nodes.get(index + 1);
        fn spelled(node: Option<&Syntax>) -> Option<&str> {
            match node {
                Some(Syntax::Token { raw, .. } | Syntax::Operator { raw, .. }) => {
                    Some(raw.as_str())
                }
                _ => None,
            }
        }
        // Only where an expression is plainly read.
        let expression = matches!(
            spelled(previous),
            Some("=" | "," | "return" | "=>" | "?" | "&&" | "||" | "??" | "!" | "+" | "-" | "*")
        );
        if matches!(spelled(next), Some(":" | "." | "=>")) || spelled(previous) == Some(".") {
            return None;
        }
        let warning = |message: String| Diagnostic {
            message,
            start: span.start,
            end: span.end,
            severity: Severity::Warning,
            related: Vec::new(),
        };
        let defined_below = LOCAL_HEADS.with(|heads| {
            heads
                .borrow()
                .get(name)
                .is_some_and(|head| *head > span.start)
        });
        if defined_below && definitions.len() == 1 && macro_call_position(nodes, index) {
            return Some(warning(format!(
                "SWR4017: Macro {name} is defined below this point, and a macro is visible only to what follows its definition. Move the definition above this use, or into a module imported for syntax."
            )));
        }
        if !expression {
            return None;
        }
        let categories: Vec<&str> = definitions
            .iter()
            .map(|macro_| macro_.category.as_str())
            .collect();
        if let Some(declared) = categories
            .iter()
            .find(|category| matches!(**category, "stmt" | "item"))
            .filter(|_| categories.iter().all(|category| *category != "expr"))
        {
            return Some(warning(format!(
                "SWR4013: Macro {name} is declared {declared} and cannot be written where an expr is read. Declare it expr to use it here."
            )));
        }
        let bare = next.is_none() || matches!(spelled(next), Some(";" | "," | ")" | "]" | "}"));
        if bare && !compiled_accepts_bare(&definitions) {
            return Some(warning(format!(
                "SWR4024: Macro {name} is written here as a name on its own, where an expr is read. A macro is a compile-time name, so {name} is written into the emitted code as it stands, and nothing in this build checks this file for whether anything defines it there. Write an invocation its rules accept."
            )));
        }
        None
    }

    /// SWR4011: an `expr` macro whose expansion is a statement, which
    /// cannot stand where the invocation is read.
    fn check_expansion_category(
        &mut self,
        name: &str,
        invocation: Span,
        expanded: &str,
        origin: Option<Span>,
    ) {
        if self.depth != 0 {
            return;
        }
        if let Some(param) = UNKNOWN_PARAMETERIZED.with(|cell| cell.borrow().get(name).cloned()) {
            self.failed = true;
            self.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR4019: #parameterize names {param}, which is not a syntax parameter in scope here. Declare it with `syntax parameter`, and import it for syntax where the template is defined."
                ),
                start: invocation.start,
                end: invocation.end,
                severity: Severity::Error,
                related: Vec::new(),
            });
            return;
        }
        let expression = self
            .macros
            .iter()
            .any(|macro_| macro_.name == name && macro_.category == "expr" && !macro_.operator);
        let first = expanded
            .trim_start()
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .next()
            .unwrap_or("");
        let statement = matches!(
            first,
            "if" | "for"
                | "while"
                | "do"
                | "switch"
                | "try"
                | "const"
                | "let"
                | "var"
                | "return"
                | "throw"
                | "break"
                | "continue"
                | "import"
                | "export"
                | "interface"
                | "enum"
                | "namespace"
                | "declare"
        );
        if !expression || !statement {
            return;
        }
        self.failed = true;
        self.diagnostics.push(Diagnostic {
            message: format!(
                "SWR4011: Macro {name} expanded to syntax that is not one expr: {}",
                expanded.trim()
            ),
            start: invocation.start,
            end: invocation.end,
            severity: Severity::Error,
            related: origin
                .map(|origin| crate::RelatedLocation {
                    message: "This rule produced it".to_owned(),
                    start: origin.start,
                    end: origin.end,
                })
                .into_iter()
                .collect(),
        });
    }

    /// Upstream's related locations on SWR4001: the invocation, and where
    /// the closest rule (when this file defines it) was still reading.
    fn failure_related(
        &self,
        name: &str,
        invocation: Span,
        closest: Option<&Rule>,
    ) -> Vec<crate::RelatedLocation> {
        if self.depth != 0 {
            return Vec::new();
        }
        let mut related = vec![crate::RelatedLocation {
            message: format!("In this use of {name}"),
            start: invocation.start,
            end: invocation.end,
        }];
        if let Some(origin) = closest.and_then(|rule| rule.origin) {
            related.push(crate::RelatedLocation {
                message: "The closest rule was still expecting syntax here".to_owned(),
                start: origin.start,
                end: origin.end,
            });
        }
        related
    }

    /// A rejected macro stays as written. Copy this item (through its
    /// terminator, when it has one) and keep expanding what follows.
    fn skip_failed(&mut self, nodes: &[Syntax], index: usize) -> usize {
        self.failed = true;
        let start = node_start(&nodes[index]);
        self.copy_until(start);
        let next = failed_item_resume(self.source, nodes, index);
        if let Some(node) = nodes.get(next.saturating_sub(1)) {
            self.copy_until(node_end(node));
        }
        next
    }

    fn copy_until(&mut self, original_end: usize) {
        if original_end < self.last_original || original_end > self.source.len() {
            return;
        }
        let text = &self.source[self.last_original..original_end];
        self.emit(
            u32_at(self.last_original),
            u32_at(original_end),
            text,
            SegmentKind::Verbatim,
        );
        self.last_original = original_end;
    }

    /// The captures of a rule about to be instantiated. At the top level
    /// they are the user code that survives into the output.
    fn note_captures(&mut self, captures: &HashMap<String, Capture>) {
        if self.depth == 0 {
            self.pending_captures = Some(capture_texts(captures));
        }
    }

    fn emit(&mut self, original_start: u32, original_end: u32, text: &str, kind: SegmentKind) {
        let source = self.source;
        let mut text = text;
        if self.depth == 0
            && let Some(message) = BUDGET.with(|budget| budget.borrow_mut().take())
        {
            text = source
                .get(original_start as usize..original_end as usize)
                .unwrap_or(text);
            // The invocation stays as written, like upstream's aborted
            // expansion; nothing of the runaway replacement is printed.
            self.diagnostics.push(Diagnostic {
                message,
                start: original_start,
                end: original_end,
                severity: Severity::Error,
                ..Default::default()
            });
            self.failed = true;
        }
        if self.depth == 0 {
            // `#parameterize(required % = ...)` around a body that never
            // uses `%`. The body is inside this expansion.
            for unused in super::sweetener_scoped::take_unused() {
                self.diagnostics.push(Diagnostic {
                    message: format!(
                        "SWR4022: This must use {0}, and does not. The macro gives {0} a meaning here only for syntax that uses it.",
                        unused.param
                    ),
                    start: original_start,
                    end: original_end,
                    severity: Severity::Error,
                    ..Default::default()
                });
            }
        }
        if self.depth == 0 && kind == SegmentKind::Atom {
            if let Some(captures) = self.pending_captures.take() {
                // Only a capture the expansion prints again is in the output.
                // A `#syntax` definition prints nothing of its captures.
                let printed = captures
                    .into_iter()
                    .filter(|capture| contains_word(text, capture.trim()))
                    .collect();
                self.consumed
                    .push((original_start as usize, original_end as usize, printed));
            }
        }
        if text.is_empty() {
            return;
        }
        let virtual_start = u32_at(self.out.len());
        self.out.push_str(text);
        let virtual_end = u32_at(self.out.len());
        if kind == SegmentKind::Verbatim {
            if let Some(last) = self.segments.last_mut() {
                if last.kind == SegmentKind::Verbatim
                    && last.virtual_end == virtual_start
                    && last.original_end == original_start
                {
                    last.virtual_end = virtual_end;
                    last.original_end = original_end;
                    return;
                }
            }
        }
        self.segments.push(Segment {
            virtual_start,
            virtual_end,
            original_start,
            original_end,
            kind,
        });
    }
}

#[derive(Clone, Debug)]
struct Macro {
    name: String,
    fixity: Fixity,
    precedence: u8,
    associativity: Associativity,
    rules: Vec<Rule>,
    /// The definition's category (`expr`, `item`, ...), `expr` when unwritten.
    category: String,
    /// `operator` rather than `syntax`: dispatched beside or between operands.
    operator: bool,
    /// The definition wrote `shadows core`.
    shadows_core: bool,
    /// `operand arrow;`: the right operand may be an arrow function.
    operand_arrow: bool,
    /// The definition keyword (`syntax`/`operator`), where upstream reports
    /// a definition-level diagnostic. Empty for a synthesized macro.
    head: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixity {
    /// A named form: `unless`, `duplicate`, `clamp`.
    Function,
    Prefix,
    Infix,
    /// `fixity postfix;`: written after its one operand.
    Postfix,
}

impl Fixity {
    /// Infix and postfix operators are found after their left operand.
    fn operand_led(self) -> bool {
        matches!(self, Self::Infix | Self::Postfix)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Associativity {
    Left,
    Right,
    None,
}

#[derive(Clone, Debug)]
struct Rule {
    pattern: Vec<Pat>,
    template: Vec<Tpl>,
    /// Bindings the template itself introduces, read from the template source
    /// before holes split it into pieces.
    introduced: Vec<String>,
    /// `expect "..."` on this rule. The diagnostic quotes it when present.
    expect: Option<String>,
    /// `refine $terms length equal 1`. A match that fails one is not a match,
    /// so the next rule of the macro can take the same tokens.
    refines: Vec<Refine>,
    /// `literal globalThis.NaN as NaN` — the rule rewrites that spelling only
    /// when the user file has not already bound it.
    literal_alias: Option<String>,
    /// `bind $name in following as recursive value;` clauses.
    contracts: Vec<Contract>,
    /// `context generator;` / `context async;`: the rule is used only where
    /// the invocation stands in those contexts.
    contexts: Vec<(String, Span)>,
    /// The rule's pattern group, when this file defines the rule.
    origin: Option<Span>,
    /// The macro module that defines the rule; `None` for this file.
    owner: Option<Rc<str>>,
}

/// One declarative binding contract, upstream's `BindingContract`:
/// `bind <binder path> in (following | <region path>) as <kind> <space>`.
/// A part that did not read is `None`; the definition reports it.
#[derive(Clone, Debug)]
struct Contract {
    /// `$name` or `$field.name`: the capture root and its field path.
    binder: Option<(String, Vec<String>)>,
    /// `None` is `following`; `Some` a capture path.
    region: Option<Option<(String, Vec<String>)>>,
    kind: Option<String>,
    space: Option<String>,
    span: Span,
}

#[derive(Clone, Debug)]
enum Pat {
    Lit(String),
    Group {
        delimiter: Delimiter,
        inner: Vec<Pat>,
    },
    Hole {
        name: String,
        kind: HoleKind,
    },
    Rep {
        inner: Vec<Pat>,
        sep: Option<String>,
        kind: RepKind,
    },
    /// `a | b`: ordered alternatives of a whole sequence (upstream
    /// `createChoicePattern`). The first alternative that matches wins.
    Choice(Vec<Vec<Pat>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HoleKind {
    Expr,
    Stmt,
    /// `$x:item`: a module item. Unlike `:stmt` it also reads an import or
    /// export declaration.
    Item,
    Type,
    Binding,
    Ident,
    TokenTree,
    /// `$member:classElement` / `$member:typeMember`: one member of a class
    /// body or of an object type.
    Member {
        type_member: bool,
    },
    /// `$body:jsxChild`: one JSX element, expression container, or text run.
    JsxChild,
    /// `$state:State` — a user syntax class, matched by its rules.
    Class(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepKind {
    ZeroOrMore,
    OneOrMore,
    /// `$( ... )?`: at most one.
    Optional,
}

#[derive(Clone, Debug)]
enum Tpl {
    /// Copied source, including its original spacing.
    Text(String),
    Var(String),
    /// `#text($name)` — the capture as a TypeScript string literal.
    Quote(String),
    /// `#core(expr)` — print `expr` as ordinary TypeScript, once.
    Core(Vec<Tpl>),
    /// `#join($name, suffix: "Signal")` — concatenate an identifier.
    Join {
        name: String,
        prefix: String,
        suffix: String,
        casing: String,
    },
    /// `#count($name)` — how many times the repetition captured `name`.
    Count(String),
    /// `#if(present $name.field) { ... } #else { ... }`.
    If {
        path: String,
        then_body: Vec<Tpl>,
        else_body: Option<Vec<Tpl>>,
    },
    /// `#syntax { ... }` defines a macro. It is not printed. The body is
    /// still a template: holes inside it are substituted before the
    /// generated macro is compiled.
    Define(Vec<Tpl>),
    /// A template directive this expander does not implement.
    Unknown {
        name: String,
        /// Byte offset of the directive token in the macro source.
        at: usize,
    },
    /// `#metavar("value", $name)` — a hole of the rule being defined.
    /// The string is the hole name. The second argument names the binding
    /// the hole belongs to; it is not text to print.
    Metavar {
        hole: String,
        /// `$parameter` this hole belongs to, so two parameters do not share
        /// one capture.
        key: String,
    },
    /// `#trim($path)` — that capture with surrounding whitespace removed.
    Trim {
        path: String,
    },
    /// `#index()` — the zero-based index of the current repetition row.
    Index,
    /// `#let(name = value) { body }` — evaluate `value` once and bind it to
    /// a fresh `name` the body refers to.
    Let {
        name: String,
        value: Vec<Tpl>,
        body: Vec<Tpl>,
    },
    /// `#fresh("hint")` — a new binder spelled `hint`, distinct from every
    /// other, including another `#fresh("hint")` in the same expansion.
    Fresh(String),
    /// `#fold($items, init: { ... }) { ($acc, $item, $index) => { ... } }`.
    Fold {
        path: String,
        init: Vec<Tpl>,
        acc: String,
        item: String,
        index: String,
        body: Vec<Tpl>,
    },
    /// `#parameterize([required] p = value) { body }` — inside `body`, the
    /// syntax parameter `p` stands for `value`.
    Parameterize {
        param: String,
        required: bool,
        value: Vec<Tpl>,
        body: Vec<Tpl>,
    },
    Rep {
        inner: Vec<Tpl>,
        sep: Option<String>,
    },
}

struct Loaded {
    macros: Vec<Macro>,
    classes: Vec<SyntaxClass>,
    diagnostics: Vec<Diagnostic>,
    dependencies: Vec<String>,
    /// Value imports the macro module needs at the call site.
    runtime_imports: Vec<RuntimeImport>,
}

#[derive(Clone, Debug)]
struct RuntimeImport {
    specifier: String,
    bindings: Vec<ImportBinding>,
    /// The macro modules that need this import. Upstream synthesizes a
    /// module's imports only when one of its macros was invoked.
    owners: Vec<String>,
}

#[derive(Clone, Debug)]
struct ImportBinding {
    /// The name the exporting module publishes.
    exported: String,
    /// The name the macro module binds, which its template uses.
    local: String,
}

struct SyntaxImport {
    names: Vec<String>,
    /// `... for syntax shadows core;`
    shadows_core: bool,
    /// `exported as local` for `import { twice as cloneArmy }`.
    aliases: Vec<(String, String)>,
    specifier: String,
    span: Span,
}

fn load_macros(
    filename: &str,
    source: &str,
    imports: &[SyntaxImport],
    user_names: &std::collections::HashSet<String>,
) -> Loaded {
    let mut loaded = Loaded {
        macros: Vec::new(),
        classes: Vec::new(),
        diagnostics: Vec::new(),
        dependencies: Vec::new(),
        runtime_imports: Vec::new(),
    };
    let base = Path::new(filename)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut seen = std::collections::HashSet::<PathBuf>::new();
    // SWR5007: one local name bound by two imports, or twice by one.
    let mut bound = std::collections::HashSet::<String>::new();
    let mut reported = std::collections::HashSet::<String>::new();
    for import in imports {
        for name in &import.names {
            if !bound.insert(name.clone()) && reported.insert(name.clone()) {
                let (start, end) = import_name_span(source, import, name, true);
                loaded.diagnostics.push(Diagnostic {
                    message: format!(
                        "SWR5007: Compile-time macro import binds local name {name} more than once."
                    ),
                    start,
                    end,
                    severity: Severity::Error,
                    related: Vec::new(),
                });
            }
        }
    }
    for import in imports {
        TOP_IMPORT.with(|top| top.set(Some(import.span)));
        let canonical = Path::new(filename)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(filename));
        LOAD_STACK.with(|stack| *stack.borrow_mut() = vec![canonical]);
        load_syntax_module(
            &mut loaded,
            &base,
            &base,
            Path::new(filename),
            import,
            &mut seen,
            source,
            true,
        );
    }
    let _ = user_names;
    loaded
}

/// Read one syntax module. `report` is false for a module pulled in only
/// because another macro imported it: a missing transitive file is still
/// reported, on the import that named it.
fn load_syntax_module(
    loaded: &mut Loaded,
    user_dir: &Path,
    from_dir: &Path,
    importer: &Path,
    import: &SyntaxImport,
    seen: &mut std::collections::HashSet<PathBuf>,
    source: &str,
    report: bool,
) {
    let Some(body) = specifier_body(&import.specifier) else {
        return;
    };
    let path = from_dir.join(body);
    let key = path.canonicalize().unwrap_or_else(|_| path.clone());
    // SWR5005: a module that imports itself, through any chain.
    let cycle = LOAD_STACK.with(|stack| {
        let stack = stack.borrow();
        stack.iter().position(|open| *open == key).map(|at| {
            let mut chain: Vec<String> = stack[at..]
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            chain.push(key.display().to_string());
            chain.join(" -> ")
        })
    });
    if let Some(chain) = cycle {
        if let Some(top) = TOP_IMPORT.with(std::cell::Cell::get) {
            loaded.diagnostics.push(Diagnostic {
                message: format!("SWR5005: Compile-time macro module cycle: {chain}."),
                start: top.start,
                end: top.end,
                severity: Severity::Error,
                related: Vec::new(),
            });
        }
        return;
    }
    if !seen.insert(key.clone()) {
        return;
    }
    LOAD_STACK.with(|stack| stack.borrow_mut().push(key));
    load_syntax_module_body(
        loaded, user_dir, from_dir, importer, import, seen, source, report, path, body,
    );
    LOAD_STACK.with(|stack| {
        stack.borrow_mut().pop();
    });
}

thread_local! {
    /// The macro modules being read, outermost first.
    static LOAD_STACK: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The span of this file's import that the current load started from.
    static TOP_IMPORT: std::cell::Cell<Option<Span>> = const { std::cell::Cell::new(None) };
}

/// The written specifier, as upstream prints it: `./opdef.sts`.
fn import_specifier_text(specifier: &str) -> &str {
    specifier.trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
}

/// Where `name` is written in `import`'s braces; the last occurrence when
/// `last`. An operator name includes its parentheses.
fn import_name_span(source: &str, import: &SyntaxImport, name: &str, last: bool) -> (u32, u32) {
    let fallback = (import.span.start, import.span.end);
    let Some(rest) = source.get(import.span.start as usize..) else {
        return fallback;
    };
    let close = rest.find('}').unwrap_or(rest.len());
    let open = rest[..close].find('{').unwrap_or(0);
    let inside = &rest[open..close];
    let found = if last {
        inside.rfind(name)
    } else {
        inside.find(name)
    };
    let Some(at) = found else {
        return fallback;
    };
    let mut start = open + at;
    let mut end = start + name.len();
    if rest[..start].ends_with('(') && rest[end..].starts_with(')') {
        start -= 1;
        end += 1;
    }
    (
        import.span.start + u32::try_from(start).unwrap_or(0),
        import.span.start + u32::try_from(end).unwrap_or(0),
    )
}

#[allow(clippy::too_many_arguments)]
fn load_syntax_module_body(
    loaded: &mut Loaded,
    user_dir: &Path,
    from_dir: &Path,
    importer: &Path,
    import: &SyntaxImport,
    seen: &mut std::collections::HashSet<PathBuf>,
    source: &str,
    report: bool,
    path: PathBuf,
    body: &str,
) {
    let text = std::fs::read_to_string(&path).ok();
    let Some(text) = text else {
        if !report {
            return;
        }
        // Upstream reports once per import, at its first binding, naming
        // the importer by its absolute path.
        let importer_name = importer
            .canonicalize()
            .unwrap_or_else(|_| importer.to_path_buf());
        let mut start = import.span.start;
        let mut end = import.span.end;
        if user_dir == from_dir {
            if let (Some(first), Some(text)) = (
                import.names.first(),
                source.get(import.span.start as usize..).map(|rest| {
                    let close = rest.find('}').unwrap_or(rest.len());
                    &rest[..close]
                }),
            ) {
                let first = import
                    .aliases
                    .iter()
                    .find(|(_, local)| local == first)
                    .map_or(first.as_str(), |(exported, _)| exported.as_str());
                let open = text.find('{').unwrap_or(0);
                if let Some(at) = text[open..].find(first).map(|at| at + open) {
                    start = import.span.start + u32::try_from(at).unwrap_or(0);
                    end = start + u32::try_from(first.len()).unwrap_or(0);
                }
            }
        }
        loaded.diagnostics.push(Diagnostic {
            message: format!(
                "SWR5002: Cannot resolve macro module {} from {}.",
                body,
                importer_name.display()
            ),
            start,
            end,
            severity: Severity::Error,
            related: Vec::new(),
        });
        return;
    };
    if let Ok(canonical) = path.canonicalize() {
        loaded.dependencies.push(canonical.display().to_string());
    } else {
        loaded.dependencies.push(path.display().to_string());
    }
    let read = sweetener_reader::read_syntax(&text);
    let mut found = HashMap::<String, Macro>::new();
    collect_macros(&text, &read.children, &mut found);
    let owner: Rc<str> = Rc::from(path.display().to_string());
    for macro_ in found.values_mut() {
        for rule in &mut macro_.rules {
            rule.owner = Some(owner.clone());
        }
    }
    loaded
        .classes
        .extend(collect_classes(&text, &read.children));
    let runtime = runtime_imports_of(&read.children);
    let mut taken = Vec::new();
    for name in &import.names {
        let exported = import
            .aliases
            .iter()
            .find(|(_, local)| local == name)
            .map(|(exported, _)| exported.as_str())
            .unwrap_or(name.as_str());
        if let Some(mut macro_) = found.remove(exported).or_else(|| found.remove(name)) {
            if exported != name {
                for rule in &mut macro_.rules {
                    rename_pat_lit(&mut rule.pattern, exported, name);
                }
            }
            macro_.name = name.clone();
            // Upstream only checks the authorization of an import that names
            // a core spelling; `for syntax shadows core { (#) }` is harmless.
            if import.shadows_core
                && is_core_form(name, &macro_.category, macro_.operator)
                && !(macro_.shadows_core && core_shadow_valid(&macro_))
            {
                loaded.diagnostics.push(Diagnostic {
                    message: format!(
                        "SWR4003: Import requests core shadowing for {name}, but its definition does not authorize interception."
                    ),
                    start: import.span.start,
                    end: import.span.end,
                    severity: Severity::Error,
                    related: Vec::new(),
                });
            }
            // A broken binding contract in the module is reported where
            // this file imports the macro.
            if report {
                for mut diagnostic in contract_diagnostics(&macro_) {
                    diagnostic.start = import.span.start;
                    diagnostic.end = import.span.end;
                    loaded.diagnostics.push(diagnostic);
                }
            }
            if report && macro_.shadows_core && !core_shadow_valid(&macro_) {
                loaded.diagnostics.push(Diagnostic {
                    message: format!(
                        "SWR4002: Syntax {} cannot shadow a core {} form.",
                        macro_.name, macro_.category
                    ),
                    start: import.span.start,
                    end: import.span.end,
                    severity: Severity::Error,
                    related: Vec::new(),
                });
            }
            // A core form is dispatched as core unless both the definition
            // and this import opt into interception.
            let authorized =
                macro_.shadows_core && import.shadows_core && core_shadow_valid(&macro_);
            if !is_core_form(&macro_.name, &macro_.category, macro_.operator) || authorized {
                taken.push(macro_);
            }
        } else if syntax_parameters(&read.children)
            .iter()
            .any(|param| param == exported)
        {
            note_syntax_parameter(name);
        } else if report
            && user_dir == from_dir
            && !taken.iter().any(|macro_: &Macro| macro_.name == *name)
        {
            let (start, end) = import_name_span(source, import, exported, false);
            loaded.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR5004: Macro module {} does not export {exported}.",
                    import_specifier_text(&import.specifier)
                ),
                start,
                end,
                severity: Severity::Error,
                related: Vec::new(),
            });
        }
    }
    // A sibling macro in the same module, such as `effect`, still names
    // `createEffect`. Theirs imports it even when this file only uses
    // `state`. Filtering to the imported macro drops that name.
    let used = if taken.is_empty() {
        runtime_used_by(&runtime, &taken)
    } else {
        runtime
    };
    for macro_ in taken {
        push_macro(&mut loaded.macros, macro_);
    }
    // `query` calls `condition`, `conjunction`, `term` and `compare`, which
    // are syntax in the same module. The import names `query` only. The
    // template still needs the others, so they stay available.
    for macro_ in found.into_values() {
        // A sibling is in scope where its own module wrote it; a core
        // spelling there needs that definition's own `shadows core`.
        if local_dispatchable(&macro_) {
            push_macro(&mut loaded.macros, macro_);
        }
    }
    // A nested `for syntax` module is where the macro it exports is defined.
    // Its runtime imports are synthesized ahead of the module that imported
    // it, which is where the original printer puts definition-site imports.
    let nested_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| from_dir.to_path_buf());
    for nested in syntax_imports(&text, &read.children) {
        load_syntax_module(
            loaded,
            user_dir,
            &nested_dir,
            &path,
            &nested,
            seen,
            source,
            true,
        );
    }
    merge_runtime_imports(
        &mut loaded.runtime_imports,
        rebase_runtime_imports(user_dir, &path, used),
    );
}

/// The core forms a binding of this category could stand in for
/// (upstream `expansion/src/core-shadowing.ts` and
/// `enforestation/src/core-operators.ts`). A macro heads an operand, so it
/// meets the declared words and the prefix operators; an operator meets
/// every core operator of its spelling.
fn is_core_form(spelling: &str, category: &str, operator: bool) -> bool {
    const EXPR: &[&str] = &[
        "await", "class", "delete", "function", "new", "super", "this", "typeof", "void", "yield",
    ];
    const STMT: &[&str] = &[
        "break", "class", "const", "continue", "debugger", "do", "for", "function", "if", "let",
        "return", "switch", "throw", "try", "var", "while", "with",
    ];
    const ITEM: &[&str] = &[
        "class",
        "const",
        "declare",
        "enum",
        "export",
        "function",
        "import",
        "interface",
        "let",
        "module",
        "namespace",
        "type",
        "var",
    ];
    const TYPE: &[&str] = &[
        "abstract",
        "any",
        "asserts",
        "bigint",
        "boolean",
        "false",
        "infer",
        "keyof",
        "never",
        "new",
        "null",
        "number",
        "object",
        "readonly",
        "string",
        "symbol",
        "this",
        "true",
        "typeof",
        "undefined",
        "unique",
        "unknown",
        "void",
    ];
    const CLASS_ELEMENT: &[&str] = &[
        "abstract",
        "accessor",
        "constructor",
        "declare",
        "get",
        "override",
        "private",
        "protected",
        "public",
        "readonly",
        "set",
        "static",
    ];
    const TYPE_MEMBER: &[&str] = &["get", "new", "readonly", "set"];
    const PREFIX_OPS: &[&str] = &[
        "+", "-", "!", "~", "typeof", "void", "delete", "await", "new", "++", "--", "yield",
    ];
    const OTHER_OPS: &[&str] = &[
        "++",
        "--",
        "**",
        "*",
        "/",
        "%",
        "+",
        "-",
        "<<",
        ">>",
        ">>>",
        "<",
        "<=",
        ">",
        ">=",
        "in",
        "instanceof",
        "as",
        "satisfies",
        "==",
        "!=",
        "===",
        "!==",
        "&",
        "^",
        "|",
        "&&",
        "||",
        "??",
        "=",
        "+=",
        "-=",
        "*=",
        "/=",
        "%=",
        "**=",
        "<<=",
        ">>=",
        ">>>=",
        "&=",
        "^=",
        "|=",
        "&&=",
        "||=",
        "??=",
        "=>",
        ",",
    ];
    let declared: &[&str] = match category {
        "expr" => EXPR,
        "stmt" => STMT,
        "item" => ITEM,
        "type" => TYPE,
        "classElement" => CLASS_ELEMENT,
        "typeMember" => TYPE_MEMBER,
        _ => &[],
    };
    if declared.contains(&spelling) {
        return true;
    }
    if category != "expr" {
        return false;
    }
    PREFIX_OPS.contains(&spelling) || (operator && OTHER_OPS.contains(&spelling))
}

/// `shadows core` names a core form of the definition's own category.
fn core_shadow_valid(macro_: &Macro) -> bool {
    is_core_form(&macro_.name, &macro_.category, macro_.operator)
}

/// A definition in scope where it was written: a core spelling is
/// dispatched to it only when it opted in with a valid `shadows core`.
fn local_dispatchable(macro_: &Macro) -> bool {
    !is_core_form(&macro_.name, &macro_.category, macro_.operator)
        || (macro_.shadows_core && core_shadow_valid(macro_))
}

fn push_macro(macros: &mut Vec<Macro>, macro_: Macro) {
    if let Some(existing) = macros.iter_mut().find(|have| have.name == macro_.name) {
        let mut macro_ = macro_;
        existing.rules.append(&mut macro_.rules);
    } else {
        macros.push(macro_);
    }
}

/// `./atom-runtime.ts` next to the macro is not next to the file that
/// imported the macro. Rebase a relative specifier onto the user file.
fn rebase_runtime_imports(
    user_dir: &Path,
    macro_file: &Path,
    imports: Vec<RuntimeImport>,
) -> Vec<RuntimeImport> {
    imports
        .into_iter()
        .map(|mut import| {
            import.specifier = rebase_specifier(user_dir, macro_file, &import.specifier);
            import.owners = vec![macro_file.display().to_string()];
            import
        })
        .collect()
}

fn rebase_specifier(user_dir: &Path, macro_file: &Path, specifier: &str) -> String {
    let Some(body) = specifier_body(specifier) else {
        return specifier.to_owned();
    };
    if !body.starts_with('.') {
        return specifier.to_owned();
    }
    let Some(macro_dir) = macro_file.parent() else {
        return specifier.to_owned();
    };
    let target = normalize_lexical(&macro_dir.join(body));
    let relative = path_from_dir(&normalize_lexical(user_dir), &target);
    let quote = specifier.chars().next().unwrap_or('"');
    format!("{quote}{relative}{quote}")
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn path_from_dir(from_dir: &Path, to: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    let mut shared = 0usize;
    while shared < from.len() && shared < to.len() && from[shared] == to[shared] {
        shared += 1;
    }
    let mut parts = Vec::new();
    for _ in shared..from.len() {
        parts.push("..".to_owned());
    }
    for component in &to[shared..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return ".".to_owned();
    }
    let joined = parts.join("/");
    if joined.starts_with('.') {
        joined
    } else {
        format!("./{joined}")
    }
}

fn rename_pat_lit(pats: &mut [Pat], from: &str, to: &str) {
    for pat in pats {
        match pat {
            Pat::Lit(text) if text == from => *text = to.to_owned(),
            Pat::Group { inner, .. } | Pat::Rep { inner, .. } => rename_pat_lit(inner, from, to),
            Pat::Choice(alternatives) => {
                for alternative in alternatives {
                    rename_pat_lit(alternative, from, to);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug)]
struct SyntaxClass {
    name: String,
    fields: Vec<ClassField>,
    rules: Vec<ClassRule>,
}

#[derive(Clone, Debug)]
struct ClassField {
    name: String,
    /// `moves: Transition*` — the field is a sequence of class matches.
    repeated: bool,
}

#[derive(Clone, Debug)]
struct ClassRule {
    pattern: Vec<Pat>,
    expect: Option<String>,
    /// `refine $field spelling equals "left"` and the other class guards.
    /// A rule whose guard fails is not a match, so a later rule can take it.
    refines: Vec<Refine>,
}

#[derive(Clone, Debug)]
enum Refine {
    /// `refine $name spelling equals "left"`.
    SpellingEquals { field: String, spelling: String },
    /// `refine $op spelling in ("+", "-")`.
    SpellingIn {
        field: String,
        spellings: Vec<String>,
    },
    /// `refine $binders spelling starts-with-lowercase`.
    StartsLower { field: String },
    /// `refine $type spelling starts-with-uppercase`.
    StartsUpper { field: String },
    /// `refine $arguments delimiter parenthesis`.
    Delimiter { field: String, delimiter: Delimiter },
    /// `refine $terms length at-least 1` and the other comparisons.
    Length {
        field: String,
        comparison: LengthComparison,
        length: usize,
    },
    /// `refine $name token-kind (identifier, keyword)`.
    TokenKind { field: String, kinds: Vec<String> },
    /// `refine $value form not in (conditional, arrow)`.
    Form {
        field: String,
        forms: Vec<String>,
        excluded: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LengthComparison {
    Equal,
    LessThan,
    AtMost,
    GreaterThan,
    AtLeast,
}

impl LengthComparison {
    fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "equal" => Self::Equal,
            "less-than" => Self::LessThan,
            "at-most" => Self::AtMost,
            "greater-than" => Self::GreaterThan,
            "at-least" => Self::AtLeast,
            _ => return None,
        })
    }

    fn holds(self, actual: usize, expected: usize) -> bool {
        match self {
            Self::Equal => actual == expected,
            Self::LessThan => actual < expected,
            Self::AtMost => actual <= expected,
            Self::GreaterThan => actual > expected,
            Self::AtLeast => actual >= expected,
        }
    }
}

/// The token kinds a `token-kind` refinement may name (upstream
/// compile-syntax-classes.ts `refinableTokenKinds`).
const REFINABLE_TOKEN_KINDS: &[&str] = &[
    "identifier",
    "private-identifier",
    "keyword",
    "numeric-literal",
    "bigint-literal",
    "string-literal",
    "regular-expression-literal",
    "no-substitution-template",
    "punctuation",
    "jsx-text",
    "jsx-identifier",
];

const EXPRESSION_FORMS: &[&str] = &["conditional", "arrow", "assignment", "yield", "await"];

fn collect_classes(source: &str, nodes: &[Syntax]) -> Vec<SyntaxClass> {
    let mut classes = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "export") {
            index += 1;
            continue;
        }
        if token_at(nodes, index, "syntax") && token_at(nodes, index + 1, "class") {
            index += 2;
            let Some(name) = macro_name_at(nodes, &mut index) else {
                continue;
            };
            let Some(body) = next_group(nodes, &mut index, Delimiter::Brace) else {
                continue;
            };
            classes.push(class_of(source, &name, body));
            continue;
        }
        index += 1;
    }
    classes
}

fn class_of(source: &str, name: &str, body: &Syntax) -> SyntaxClass {
    let Syntax::Group { children, .. } = body else {
        return SyntaxClass {
            name: name.to_owned(),
            fields: Vec::new(),
            rules: Vec::new(),
        };
    };
    let mut fields = Vec::new();
    let mut rules = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if token_at(children, index, "fields") {
            index += 1;
            if let Some(group) = next_group(children, &mut index, Delimiter::Brace) {
                fields = class_fields(group);
            }
            continue;
        }
        if token_at(children, index, "rule") {
            index += 1;
            if let Some(group) = next_group(children, &mut index, Delimiter::Brace) {
                let pattern = pattern_of(group);
                if !pattern.is_empty() {
                    let at = group_index(children, group);
                    rules.push(ClassRule {
                        pattern,
                        expect: expect_after(source, children, at),
                        refines: refines_after(source, children, at),
                    });
                }
            }
            continue;
        }
        index += 1;
    }
    SyntaxClass {
        name: name.to_owned(),
        fields,
        rules,
    }
}

fn class_fields(group: &Syntax) -> Vec<ClassField> {
    let Syntax::Group { children, .. } = group else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if operator_is(children.get(index), ";") {
            index += 1;
            continue;
        }
        let Some(Syntax::Token { raw: name, .. }) = children.get(index) else {
            index += 1;
            continue;
        };
        if !operator_is(children.get(index + 1), ":") {
            index += 1;
            continue;
        }
        // `name: Type`, `name: Type*`, or `name: Type?`. The type names
        // the class a rule matches; `*` makes the field a sequence. `?`
        // may stay unbound, which matching already allows by leaving a
        // missing field out of the record.
        if !matches!(children.get(index + 2), Some(Syntax::Token { .. })) {
            index += 1;
            continue;
        }
        let mut cursor = index + 3;
        let mut repeated = false;
        if operator_is(children.get(cursor), "*") {
            repeated = true;
            cursor += 1;
        } else if operator_is(children.get(cursor), "?") {
            cursor += 1;
        }
        fields.push(ClassField {
            name: name.clone(),
            repeated,
        });
        index = cursor;
    }
    fields
}

fn collect_macros(source: &str, nodes: &[Syntax], out: &mut HashMap<String, Macro>) {
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "export") {
            index += 1;
            continue;
        }
        let operator = token_at(nodes, index, "operator");
        let syntax = token_at(nodes, index, "syntax");
        if operator || syntax {
            let head = match &nodes[index] {
                Syntax::Token { span, .. } => *span,
                _ => Span { start: 0, end: 0 },
            };
            index += 1;
            let Some(name) = macro_name_at(nodes, &mut index) else {
                continue;
            };
            // `syntax typeof:expr` is a prefix form. The reader splits the
            // position off the name, so the body has no `fixity` line and
            // the macro was stored as infix. An infix walk then treats
            // `typeof NaN` as an operator with a left operand.
            let position = syntax_position(nodes, &mut index);
            let shadows_core =
                token_at(nodes, index, "shadows") && token_at(nodes, index + 1, "core");
            // `syntax parameter (%):expr;` declares a name with no rules.
            // The next brace belongs to the following definition.
            let mut peek = index;
            while peek < nodes.len()
                && !operator_is(nodes.get(peek), ";")
                && !group_is(nodes.get(peek), Delimiter::Brace)
            {
                peek += 1;
            }
            if operator_is(nodes.get(peek), ";") {
                index = peek + 1;
                continue;
            }
            let Some(body) = next_group(nodes, &mut index, Delimiter::Brace) else {
                continue;
            };
            let rules = rules_in_source(source, body);
            if rules.is_empty() {
                continue;
            }
            let (mut fixity, precedence, associativity) = macro_fixity(operator, body);
            if !operator {
                if let Some(position) = position.as_deref() {
                    fixity = match position {
                        "expr" | "stmt" | "item" | "type" | "jsx" => Fixity::Function,
                        _ => fixity,
                    };
                }
            }
            let entry = out.entry(name.clone());
            match entry {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    occupied.get_mut().rules.extend(rules);
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(Macro {
                        name,
                        fixity,
                        precedence,
                        associativity,
                        rules,
                        category: position.clone().unwrap_or_else(|| "expr".to_owned()),
                        operator,
                        shadows_core,
                        head,
                        operand_arrow: operator && declares_operand_arrow(body),
                    });
                }
            }
            continue;
        }
        index += 1;
    }
}

thread_local! {
    /// Syntax parameters (`%`) this file declares or imports. Where one
    /// starts an expression it is an operand, not an operator.
    static SYNTAX_PARAMETERS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

fn unbound_parameter_uses(source: &str, segments: &[Segment]) -> Vec<Diagnostic> {
    let params: Vec<String> = SYNTAX_PARAMETERS.with(|set| set.borrow().iter().cloned().collect());
    let mut out = Vec::new();
    for param in params {
        for at in super::sweetener_scoped::parameter_uses(source, &param) {
            let end = at + param.len();
            let before = source[..at].trim_end();
            let after = source[end..].trim_start();
            // `import { (%) }` and `syntax parameter (%)` name it.
            if before.ends_with('(') && after.starts_with(')') {
                continue;
            }
            let copied = segments.iter().any(|segment| {
                segment.kind == SegmentKind::Verbatim
                    && segment.original_start as usize <= at
                    && end <= segment.original_end as usize
            });
            if copied {
                out.push(Diagnostic {
                    message: format!(
                        "SWR4018: Syntax parameter {param} is used outside any #parameterize that gives it a meaning. It is declared without rules, so it means nothing here."
                    ),
                    start: u32_at(at),
                    end: u32_at(end),
                    severity: Severity::Error,
                    ..Default::default()
                });
            }
        }
    }
    out
}

fn note_syntax_parameter(name: &str) {
    SYNTAX_PARAMETERS.with(|set| {
        set.borrow_mut().insert(name.to_owned());
    });
}

fn is_syntax_parameter(name: &str) -> bool {
    SYNTAX_PARAMETERS.with(|set| set.borrow().contains(name))
}

/// Names a module declares with `syntax parameter (%):expr;`. They are
/// exported for syntax, and a template binds them with `#parameterize`.
fn syntax_parameters(nodes: &[Syntax]) -> Vec<String> {
    let mut out = Vec::new();
    for index in 0..nodes.len() {
        if token_at(nodes, index, "syntax") && token_at(nodes, index + 1, "parameter") {
            let mut at = index + 2;
            if let Some(name) = macro_name_at(nodes, &mut at) {
                out.push(name);
            }
        }
    }
    out
}

fn declares_operand_arrow(body: &Syntax) -> bool {
    let Syntax::Group { children, .. } = body else {
        return false;
    };
    (0..children.len())
        .any(|index| token_at(children, index, "operand") && token_at(children, index + 1, "arrow"))
}

fn macro_fixity(operator: bool, body: &Syntax) -> (Fixity, u8, Associativity) {
    if !operator {
        return (Fixity::Function, 0, Associativity::None);
    }
    let Syntax::Group { children, .. } = body else {
        return (Fixity::Infix, 0, Associativity::Left);
    };
    let mut fixity = Fixity::Infix;
    let mut precedence = 0u8;
    let mut associativity = Associativity::Left;
    let mut index = 0;
    while index < children.len() {
        if token_at(children, index, "fixity") {
            if token_at(children, index + 1, "prefix") {
                fixity = Fixity::Prefix;
            } else if token_at(children, index + 1, "infix") {
                fixity = Fixity::Infix;
            } else if token_at(children, index + 1, "postfix") {
                fixity = Fixity::Postfix;
            }
        }
        if token_at(children, index, "precedence") {
            if let Some(Syntax::Token { raw, .. }) = children.get(index + 1) {
                precedence = raw.parse().unwrap_or(0);
            }
        }
        if token_at(children, index, "associativity") {
            associativity = match children.get(index + 1) {
                Some(Syntax::Token { raw, .. }) if raw == "right" => Associativity::Right,
                Some(Syntax::Token { raw, .. }) if raw == "none" => Associativity::None,
                _ => Associativity::Left,
            };
        }
        index += 1;
    }
    (fixity, precedence, associativity)
}

/// `:expr` after `syntax name`. The colon is not part of the name.
fn syntax_position(nodes: &[Syntax], index: &mut usize) -> Option<String> {
    if !operator_is(nodes.get(*index), ":") {
        return None;
    }
    let name = match nodes.get(*index + 1) {
        Some(Syntax::Token { raw, .. }) => raw.clone(),
        _ => return None,
    };
    *index += 2;
    Some(name)
}

fn macro_name_at(nodes: &[Syntax], index: &mut usize) -> Option<String> {
    while *index < nodes.len() && matches!(nodes[*index], Syntax::Comment { .. }) {
        *index += 1;
    }
    let node = nodes.get(*index)?;
    *index += 1;
    match node {
        Syntax::Token { raw, .. } => Some(raw.clone()),
        Syntax::Group {
            delimiter: Delimiter::Paren,
            children,
            ..
        } => {
            let name = children
                .iter()
                .filter_map(|child| match child {
                    Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => Some(raw.as_str()),
                    _ => None,
                })
                .collect::<String>();
            if name.is_empty() { None } else { Some(name) }
        }
        _ => None,
    }
}

fn next_group<'a>(
    nodes: &'a [Syntax],
    index: &mut usize,
    delimiter: Delimiter,
) -> Option<&'a Syntax> {
    while *index < nodes.len() {
        let node = &nodes[*index];
        *index += 1;
        if let Syntax::Group {
            delimiter: found, ..
        } = node
        {
            if *found == delimiter {
                return Some(node);
            }
        }
        if matches!(
            node,
            Syntax::Token { .. }
                | Syntax::Operator { .. }
                | Syntax::String { .. }
                | Syntax::Comment { .. }
        ) {
            continue;
        }
    }
    None
}

fn rules_in_source(source: &str, body: &Syntax) -> Vec<Rule> {
    let Syntax::Group { children, .. } = body else {
        return Vec::new();
    };
    let mut rules = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if !token_at(children, index, "rule") {
            index += 1;
            continue;
        }
        index += 1;
        let Some(pattern_group) = next_group(children, &mut index, Delimiter::Brace) else {
            continue;
        };
        // `expect` may follow `bind` clauses. It still sits before `=>`.
        let expect = expect_before_arrow(source, children, index);
        let mut contexts = Vec::new();
        while index < children.len() && !is_arrow(children, index) {
            // `context generator;` / `context async;`
            if token_at(children, index, "context")
                && let Some(Syntax::Token { raw, span }) = children.get(index + 1)
                && raw != ";"
            {
                contexts.push((raw.clone(), *span));
            }
            index += 1;
        }
        if index < children.len() && is_arrow(children, index) {
            index += if operator_is(children.get(index), "=>") || token_at(children, index, "=>") {
                1
            } else {
                2
            };
        }
        let Some(template_group) = next_group(children, &mut index, Delimiter::Brace) else {
            continue;
        };
        let pattern = pattern_of(pattern_group);
        let template = if source.is_empty() {
            template_of(template_group)
        } else {
            template_from_source(source, template_group)
        };
        // `fallback rule { $declaration:tt } => { coreItem $declaration }`
        // reprints a function the class rule did not take. Dropping it makes
        // `function identity` fail. The fixture form still owns the one
        // function it matches; this rule only runs when that form missed.
        if !pattern.is_empty() {
            let introduced = introduced_of(source, std::slice::from_ref(template_group));
            rules.push(Rule {
                pattern,
                template,
                introduced,
                expect,
                refines: refines_after(source, children, group_index(children, pattern_group)),
                literal_alias: literal_alias_before(children, index),
                contracts: contracts_after(children, group_index(children, pattern_group)),
                contexts,
                origin: Some(node_span(pattern_group)),
                owner: None,
            });
        }
    }
    rules
}

/// `literal globalThis.NaN as NaN` immediately before this rule.
/// The alias is the spelling the rule may rewrite.
fn literal_alias_before(nodes: &[Syntax], arrow_at: usize) -> Option<String> {
    let mut index = arrow_at;
    while index > 0 {
        index -= 1;
        if token_at(nodes, index, "rule") || token_at(nodes, index, "fallback") {
            break;
        }
    }
    if index == 0 {
        return None;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if token_at(nodes, cursor, "literal") {
            // `literal <path> as <alias>`
            let mut at = cursor + 1;
            while at < nodes.len() && !token_at(nodes, at, "as") {
                at += 1;
            }
            return match nodes.get(at + 1) {
                Some(Syntax::Token { raw, .. }) if is_ident_text(raw) => Some(raw.clone()),
                _ => None,
            };
        }
        if token_at(nodes, cursor, "rule") || token_at(nodes, cursor, "export") {
            return None;
        }
    }
    None
}

fn group_index(nodes: &[Syntax], group: &Syntax) -> usize {
    nodes
        .iter()
        .position(|node| std::ptr::eq(node, group))
        .unwrap_or(0)
}

/// `refine` clauses between a class rule's pattern and the next `rule`.
/// `bind ...` clauses between a rule's pattern and its `=>`.
fn contracts_after(nodes: &[Syntax], group_at: usize) -> Vec<Contract> {
    let mut out = Vec::new();
    let mut index = group_at + 1;
    while index < nodes.len() && !is_arrow(nodes, index) && !token_at(nodes, index, "rule") {
        if token_at(nodes, index, "bind") {
            let (contract, next) = contract_at(nodes, index);
            out.push(contract);
            index = next.max(index + 1);
            continue;
        }
        index += 1;
    }
    out
}

/// `$root.field.field` at `index`, and the index after it.
fn capture_path_at(nodes: &[Syntax], index: usize) -> Option<((String, Vec<String>), usize)> {
    let root = hole_name_at(nodes, index)?;
    let mut fields = Vec::new();
    let mut next = index + 2;
    while operator_is(nodes.get(next), ".") {
        match nodes.get(next + 1) {
            Some(Syntax::Token { raw, .. }) => fields.push(raw.clone()),
            _ => break,
        }
        next += 2;
    }
    Some(((root, fields), next))
}

fn contract_at(nodes: &[Syntax], index: usize) -> (Contract, usize) {
    let start = node_start(&nodes[index]);
    let mut end = node_end(&nodes[index]);
    let mut cursor = index + 1;
    // `bind #join($name, suffix: "Signal") in ...` binds a generated name.
    // The reader may keep `#` and `join` apart.
    if operator_is(nodes.get(cursor), "#") && token_at(nodes, cursor + 1, "join") {
        cursor += 1;
    }
    let join_word = token_at(nodes, cursor, "#join") || token_at(nodes, cursor, "join");
    let binder = if join_word && group_is(nodes.get(cursor + 1), Delimiter::Paren) {
        let path = match nodes.get(cursor + 1) {
            Some(Syntax::Group { children, .. }) => {
                capture_path_at(children, 0).map(|(path, _)| path)
            }
            _ => None,
        };
        cursor += 2;
        path
    } else if let Some((path, next)) = capture_path_at(nodes, cursor) {
        cursor = next;
        Some(path)
    } else {
        None
    };
    let mut region = None;
    if binder.is_some() && token_at(nodes, cursor, "in") {
        cursor += 1;
        if token_at(nodes, cursor, "following") {
            region = Some(None);
            cursor += 1;
        } else if let Some((path, next)) = capture_path_at(nodes, cursor) {
            region = Some(Some(path));
            cursor = next;
        }
    }
    let mut kind = None;
    let mut space = None;
    if token_at(nodes, cursor, "as") {
        let word = |at: usize, allowed: &[&str]| match nodes.get(at) {
            Some(Syntax::Token { raw, .. }) if allowed.contains(&raw.as_str()) => Some(raw.clone()),
            _ => None,
        };
        kind = word(cursor + 1, &["lexical", "recursive", "sequential"]);
        space = word(cursor + 2, &["value", "type", "namespace", "label"]);
    }
    // The clause runs to its `;`, or to the next clause or the `=>`.
    while cursor < nodes.len()
        && !operator_is(nodes.get(cursor), ";")
        && !is_arrow(nodes, cursor)
        && !token_at(nodes, cursor, "bind")
        && !token_at(nodes, cursor, "refine")
        && !token_at(nodes, cursor, "expect")
        && !token_at(nodes, cursor, "rule")
    {
        end = node_end(&nodes[cursor]);
        cursor += 1;
    }
    (
        Contract {
            binder,
            region,
            kind,
            space,
            span: Span {
                start: u32_at(start),
                end: u32_at(end),
            },
        },
        cursor,
    )
}

/// Upstream `categoryAllowsSpace`.
fn category_allows_space(category: &str, space: &str) -> bool {
    match category {
        "expr" | "binding" => space == "value",
        "stmt" => space == "value" || space == "label",
        "type" => space == "type" || space == "namespace",
        _ => true,
    }
}

/// SWR3002 / SWR3003 / SWR3005 for the binding contracts of `macro_`.
/// Each contract is reported at its `bind` clause.
fn contract_diagnostics(macro_: &Macro) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for rule in &macro_.rules {
        for (name, span) in &rule.contexts {
            if name != "generator" && name != "async" {
                out.push(Diagnostic {
                    message: format!("SWR4008: Unknown macro context {name}."),
                    start: span.start,
                    end: span.end,
                    severity: Severity::Error,
                    ..Default::default()
                });
            }
        }
        let mut holes = Vec::new();
        pattern_holes(&rule.pattern, &mut holes);
        for contract in &rule.contracts {
            let known =
                |path: &(String, Vec<String>)| holes.iter().any(|(name, _)| *name == path.0);
            let region_ok = match &contract.region {
                Some(None) => true,
                Some(Some(path)) => known(path),
                None => false,
            };
            let diagnostic = |message: String| Diagnostic {
                message,
                start: contract.span.start,
                end: contract.span.end,
                severity: Severity::Error,
                ..Default::default()
            };
            let Some(binder) = contract.binder.as_ref().filter(|binder| known(binder)) else {
                out.push(diagnostic(
                    "SWR3002: Malformed declarative binding contract.".to_owned(),
                ));
                continue;
            };
            if !region_ok || contract.kind.is_none() || contract.space.is_none() {
                out.push(diagnostic(
                    "SWR3002: Malformed declarative binding contract.".to_owned(),
                ));
                continue;
            }
            // A binder is an identifier or binding capture, or a field path
            // into a syntax class (whose fields are checked by the class).
            let binder_kind = holes
                .iter()
                .find(|(name, _)| *name == binder.0)
                .map(|(_, kind)| kind);
            let direct = matches!(
                binder_kind,
                Some(HoleKind::Ident | HoleKind::Binding | HoleKind::Class(_))
            );
            if !direct {
                out.push(diagnostic(format!(
                    "SWR3003: Invalid binding-contract path ${}.",
                    binder.0
                )));
                continue;
            }
            let space = contract.space.as_deref().unwrap_or("");
            if !category_allows_space(&macro_.category, space) {
                out.push(diagnostic(format!(
                    "SWR3005: Binding space {space} is invalid for this macro category."
                )));
                continue;
            }
            // A sequential binder scopes each repetition over its own part
            // of the region, so the two must share a repetition.
            if contract.kind.as_deref() == Some("sequential") {
                let mut groups = HashMap::new();
                hole_groups(&rule.pattern, &mut Vec::new(), &mut 0, &mut groups);
                let binder_groups = groups.get(&binder.0).cloned().unwrap_or_default();
                let region_groups = match &contract.region {
                    Some(Some(path)) => groups.get(&path.0).cloned().unwrap_or_default(),
                    _ => Vec::new(),
                };
                if binder_groups
                    .iter()
                    .all(|group| !region_groups.contains(group))
                {
                    out.push(diagnostic(
                        "SWR3004: Binding-contract repetitions are not alignable.".to_owned(),
                    ));
                }
            }
        }
    }
    out
}

/// The repetitions each capture is nested in, outermost first.
fn hole_groups(
    pattern: &[Pat],
    stack: &mut Vec<usize>,
    next: &mut usize,
    out: &mut HashMap<String, Vec<usize>>,
) {
    for pat in pattern {
        match pat {
            Pat::Hole { name, .. } => {
                out.entry(name.clone()).or_insert_with(|| stack.clone());
            }
            Pat::Group { inner, .. } => hole_groups(inner, stack, next, out),
            Pat::Rep { inner, .. } => {
                stack.push(*next);
                *next += 1;
                hole_groups(inner, stack, next, out);
                stack.pop();
            }
            Pat::Choice(alternatives) => {
                for alternative in alternatives {
                    hole_groups(alternative, stack, next, out);
                }
            }
            Pat::Lit(_) => {}
        }
    }
}

fn pattern_holes(pattern: &[Pat], out: &mut Vec<(String, HoleKind)>) {
    for pat in pattern {
        match pat {
            Pat::Hole { name, kind, .. } => out.push((name.clone(), kind.clone())),
            Pat::Group { inner, .. } => pattern_holes(inner, out),
            Pat::Rep { inner, .. } => pattern_holes(inner, out),
            Pat::Choice(alternatives) => {
                for alternative in alternatives {
                    pattern_holes(alternative, out);
                }
            }
            _ => {}
        }
    }
}

fn refines_after(source: &str, nodes: &[Syntax], group_at: usize) -> Vec<Refine> {
    let mut out = Vec::new();
    let mut index = group_at + 1;
    while index < nodes.len() {
        if token_at(nodes, index, "rule") || token_at(nodes, index, "expect") {
            break;
        }
        if token_at(nodes, index, "refine") {
            if let Some(refine) = refine_at(source, nodes, index) {
                out.push(refine);
            }
        }
        index += 1;
    }
    out
}

/// One `refine $field <predicate>;` starting at the `refine` token.
///
/// Mirrors upstream `parseRefinementPredicate`: the leading run of words is
/// joined into the predicate name (`token-kind`, `at-least` may be split on
/// their hyphens), and the first group, string or number is its argument.
fn refine_at(source: &str, nodes: &[Syntax], index: usize) -> Option<Refine> {
    // `refine $field ...`. The `$` is its own token and has no `:kind`.
    let field = hole_name_at(nodes, index + 1)?;
    let mut words = String::new();
    let mut argument = None;
    let mut cursor = index + 3;
    while let Some(node) = nodes.get(cursor) {
        match node {
            Syntax::Group { .. } | Syntax::String { .. } => {
                argument = Some(node);
                break;
            }
            Syntax::Token { raw, .. } if raw.chars().all(|ch| ch.is_ascii_digit()) => {
                argument = Some(node);
                break;
            }
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => {
                if raw == ";" || raw == "rule" || raw == "refine" || raw == "expect" {
                    break;
                }
                words.push_str(raw);
            }
            Syntax::Comment { .. } => {}
        }
        cursor += 1;
    }
    let list = match argument {
        Some(Syntax::Group {
            delimiter: Delimiter::Paren,
            children,
            ..
        }) => Some(list_entries(children)),
        _ => None,
    };
    let number = match argument {
        Some(Syntax::Token { raw, .. }) => raw.parse::<usize>().ok(),
        _ => None,
    };
    match words.as_str() {
        "spellingstarts-with-lowercase" if argument.is_none() => {
            return Some(Refine::StartsLower { field });
        }
        "spellingstarts-with-uppercase" if argument.is_none() => {
            return Some(Refine::StartsUpper { field });
        }
        "spellingequals" => {
            let spelling = quoted_after(source, nodes, cursor)?;
            return Some(Refine::SpellingEquals { field, spelling });
        }
        "spellingin" => {
            let spellings = list?;
            return (!spellings.is_empty()).then_some(Refine::SpellingIn { field, spellings });
        }
        "token-kind" => {
            let kinds = list?;
            let known = kinds
                .iter()
                .all(|kind| REFINABLE_TOKEN_KINDS.contains(&kind.as_str()));
            return (!kinds.is_empty() && known).then_some(Refine::TokenKind { field, kinds });
        }
        "formin" | "formnotin" => {
            let forms = list?;
            let known = forms
                .iter()
                .all(|form| EXPRESSION_FORMS.contains(&form.as_str()));
            return (!forms.is_empty() && known).then_some(Refine::Form {
                field,
                forms,
                excluded: words == "formnotin",
            });
        }
        _ => {}
    }
    if let Some(delimiter) = words.strip_prefix("delimiter") {
        if argument.is_some() {
            return None;
        }
        let delimiter = match delimiter {
            "parenthesis" => Delimiter::Paren,
            "bracket" => Delimiter::Bracket,
            "brace" => Delimiter::Brace,
            _ => return None,
        };
        return Some(Refine::Delimiter { field, delimiter });
    }
    if let Some(word) = words.strip_prefix("length") {
        let comparison = LengthComparison::from_word(word)?;
        return Some(Refine::Length {
            field,
            comparison,
            length: number?,
        });
    }
    None
}

/// A parenthesised, comma-separated list of spellings; strings are unquoted.
fn list_entries(children: &[Syntax]) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    for child in children {
        match child {
            Syntax::Operator { raw, .. } | Syntax::Token { raw, .. } if raw == "," => {
                entries.push(std::mem::take(&mut current));
            }
            Syntax::String { raw, .. } => current.push_str(&unquote(raw)),
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => current.push_str(raw),
            Syntax::Comment { .. } => {}
            Syntax::Group { .. } => return Vec::new(),
        }
    }
    entries.push(current);
    entries.retain(|entry| !entry.is_empty());
    entries
}

/// `$field` naming a capture. A refine has no `:kind`, so this is not `is_hole`.
fn hole_name_at(nodes: &[Syntax], index: usize) -> Option<String> {
    if !dollar_at(nodes, index) {
        return None;
    }
    match nodes.get(index + 1) {
        Some(Syntax::Token { raw, .. }) => Some(raw.clone()),
        _ => None,
    }
}

fn quoted_after(source: &str, nodes: &[Syntax], index: usize) -> Option<String> {
    match nodes.get(index) {
        Some(Syntax::String { raw, .. }) => Some(unquote(raw)),
        Some(node) => {
            let text = sweetener_reader::slice(source, node);
            let text = text.trim().trim_matches(|ch| ch == '"' || ch == '\'');
            if text.is_empty() {
                None
            } else {
                Some(text.to_owned())
            }
        }
        None => None,
    }
}

/// A class rule's `refine` clauses, against the captures that rule just took.
fn refine_accepts(refines: &[Refine], captures: &HashMap<String, Capture>) -> bool {
    refines.iter().all(|refine| refine_one(refine, captures))
}

fn refine_one(refine: &Refine, captures: &HashMap<String, Capture>) -> bool {
    match refine {
        Refine::SpellingEquals { field, spelling } => refine_spellings(captures, field)
            .iter()
            .any(|have| have == spelling),
        Refine::SpellingIn { field, spellings } => every_token(captures, field, |token| {
            spellings.iter().any(|s| s == token)
        }),
        // Upstream `everyToken`: an empty repetition (`None()`) passes.
        Refine::StartsLower { field } => every_token(captures, field, |token| {
            token.chars().next().is_some_and(char::is_lowercase)
        }),
        Refine::StartsUpper { field } => every_token(captures, field, |token| {
            token.chars().next().is_some_and(|ch| ch.is_uppercase())
        }),
        Refine::Delimiter { field, delimiter } => capture_opens(captures, field)
            .iter()
            .any(|have| have == delimiter),
        Refine::Length {
            field,
            comparison,
            length,
        } => comparison.holds(capture_len(captures, field), *length),
        Refine::TokenKind { field, kinds } => every_token(captures, field, |token| {
            token_kind(token).is_some_and(|kind| kinds.iter().any(|k| k == kind))
        }),
        Refine::Form {
            field,
            forms,
            excluded,
        } => {
            if !captures.contains_key(field) {
                return false;
            }
            refine_values(captures, field).iter().all(|text| {
                let listed =
                    expression_form(text).is_some_and(|form| forms.iter().any(|f| f == form));
                listed != *excluded
            })
        }
    }
}

/// Upstream `everyToken`: every leaf of the capture is one token the
/// predicate accepts. A missing capture fails; an empty repetition passes.
fn every_token(
    captures: &HashMap<String, Capture>,
    field: &str,
    accepts: impl Fn(&str) -> bool,
) -> bool {
    if !captures.contains_key(field) {
        return false;
    }
    refine_values(captures, field)
        .iter()
        .all(|text| token_kind(text).is_some() && accepts(text))
}

/// The text of each leaf of a capture (one per repetition row).
fn refine_values(captures: &HashMap<String, Capture>, field: &str) -> Vec<String> {
    match captures.get(field) {
        Some(Capture::Text(text)) | Some(Capture::Record { text, .. }) => {
            vec![text.trim().to_owned()]
        }
        Some(Capture::Many(_)) => refine_spellings(captures, field),
        None => Vec::new(),
    }
}

const TS_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
];

/// The kind of the one token `text` spells, or `None` where it is not one
/// token. Contextual words (`async`, `type`, ...) are identifiers, as in the
/// TypeScript scanner.
fn token_kind(text: &str) -> Option<&'static str> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let ident_char = |ch: char| ch == '_' || ch == '$' || ch.is_alphanumeric();
    if first == '#' {
        return (text.len() > 1 && text[1..].chars().all(ident_char))
            .then_some("private-identifier");
    }
    if first == '_' || first == '$' || first.is_alphabetic() {
        if !text.chars().all(ident_char) {
            return None;
        }
        return Some(if TS_KEYWORDS.contains(&text) {
            "keyword"
        } else {
            "identifier"
        });
    }
    if first.is_ascii_digit() || (first == '.' && text.len() > 1) {
        let body = text.replace('_', "");
        if let Some(digits) = body.strip_suffix('n') {
            return digits
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric())
                .then_some("bigint-literal");
        }
        let ok = body
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '+' || ch == '-');
        return ok.then_some("numeric-literal");
    }
    if (first == '"' || first == '\'') && text.len() >= 2 && text.ends_with(first) {
        return Some("string-literal");
    }
    if first == '`' && text.len() >= 2 && text.ends_with('`') && !text.contains("${") {
        return Some("no-substitution-template");
    }
    if first == '/' && text.len() > 2 && text[1..].contains('/') && !text.starts_with("//") {
        return Some("regular-expression-literal");
    }
    let punctuation = text
        .chars()
        .all(|ch| "{}()[];,<>=!+-*/%&|^~?:.@".contains(ch));
    (punctuation && text.len() <= 4).then_some("punctuation")
}

/// The top-level form of an expression, as upstream `expressionForm`: an
/// unparenthesised conditional, arrow, assignment, `yield` or `await`.
fn expression_form(text: &str) -> Option<&'static str> {
    let text = text.trim();
    let word = text
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
        .next()
        .unwrap_or("");
    match word {
        "yield" => return Some("yield"),
        "await" => return Some("await"),
        _ => {}
    }
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut question = false;
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if let Some(q) = quote {
            if ch == b'\\' {
                index += 1;
            } else if ch == q {
                quote = None;
            }
            index += 1;
            continue;
        }
        match ch {
            b'"' | b'\'' | b'`' => quote = Some(ch),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ if depth != 0 => {}
            b'=' => {
                let next = bytes.get(index + 1).copied();
                let prev = index.checked_sub(1).map(|i| bytes[i]);
                if next == Some(b'>') {
                    return Some("arrow");
                }
                if next == Some(b'=') {
                    index += 2;
                    while bytes.get(index) == Some(&b'=') {
                        index += 1;
                    }
                    continue;
                }
                if matches!(prev, Some(b'!' | b'<' | b'>' | b'=')) {
                    // `!=`, `<=`, `>=`: comparisons, unless `<<=`/`>>=`.
                    let compound = index >= 2 && bytes[index - 2] == prev.unwrap_or(0);
                    if !compound {
                        index += 1;
                        continue;
                    }
                }
                if !question {
                    return Some("assignment");
                }
            }
            b'?' => {
                let next = bytes.get(index + 1).copied();
                if next == Some(b'?') {
                    index += 2;
                    continue;
                }
                if next == Some(b'.') && !bytes.get(index + 2).is_some_and(u8::is_ascii_digit) {
                    index += 2;
                    continue;
                }
                question = true;
            }
            b':' if question => return Some("conditional"),
            _ => {}
        }
        index += 1;
    }
    None
}

fn refine_spellings(captures: &HashMap<String, Capture>, field: &str) -> Vec<String> {
    let Some(value) = captures.get(field) else {
        return Vec::new();
    };
    match value {
        Capture::Text(text) => vec![text.trim().to_owned()],
        Capture::Record { text, .. } => vec![text.trim().to_owned()],
        Capture::Many(rows) => rows
            .iter()
            .filter_map(|row| {
                row.iter().find_map(|(name, value)| {
                    (name == field).then(|| match value {
                        Capture::Text(text) | Capture::Record { text, .. } => {
                            text.trim().to_owned()
                        }
                        Capture::Many(_) => String::new(),
                    })
                })
            })
            .filter(|text| !text.is_empty())
            .collect(),
    }
}

fn capture_len(captures: &HashMap<String, Capture>, field: &str) -> usize {
    match captures.get(field) {
        Some(Capture::Many(rows)) => rows.len(),
        Some(Capture::Text(text)) if !text.trim().is_empty() => 1,
        Some(Capture::Record { .. }) => 1,
        _ => 0,
    }
}

fn capture_opens(captures: &HashMap<String, Capture>, field: &str) -> Vec<Delimiter> {
    let Some(Capture::Text(text)) = captures.get(field) else {
        return Vec::new();
    };
    let trimmed = text.trim();
    let open = match trimmed.chars().next() {
        Some('(') => Delimiter::Paren,
        Some('{') => Delimiter::Brace,
        Some('[') => Delimiter::Bracket,
        _ => return Vec::new(),
    };
    vec![open]
}

/// `expect` anywhere between the pattern group and `=>`.
fn expect_before_arrow(source: &str, nodes: &[Syntax], from: usize) -> Option<String> {
    let mut index = from;
    while index < nodes.len() && !is_arrow(nodes, index) {
        if token_at(nodes, index, "expect") {
            return expect_after(source, nodes, index.saturating_sub(1));
        }
        index += 1;
    }
    None
}

/// `expect "reason"` after a rule or class-rule group.
/// The reason may be one string or several tokens (`: followed by a return type`).
fn expect_after(source: &str, nodes: &[Syntax], group_at: usize) -> Option<String> {
    let mut index = group_at + 1;
    while index < nodes.len() && matches!(nodes.get(index), Some(Syntax::Comment { .. })) {
        index += 1;
    }
    if !token_at(nodes, index, "expect") {
        return None;
    }
    index += 1;
    let start = nodes.get(index).map(node_start)?;
    let mut end = start;
    while index < nodes.len() {
        if token_at(nodes, index, "=>")
            || operator_is(nodes.get(index), "=>")
            || operator_is(nodes.get(index), ";")
            || token_at(nodes, index, "rule")
            || token_at(nodes, index, "bind")
            || token_at(nodes, index, "refine")
        {
            break;
        }
        end = node_end(&nodes[index]);
        index += 1;
    }
    if start >= end {
        return None;
    }
    if !source.is_empty() {
        if let Some(text) = source.get(start..end) {
            let text = text
                .trim()
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .trim();
            if !text.is_empty() {
                return Some(text.to_owned());
            }
        }
    }
    let mut parts = Vec::new();
    let mut cursor = group_at + 1;
    while cursor < nodes.len() && !token_at(nodes, cursor, "expect") {
        cursor += 1;
    }
    cursor += 1;
    while cursor < index {
        match &nodes[cursor] {
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => parts.push(raw.clone()),
            Syntax::String { raw, .. } => parts.push(unquote(raw)),
            _ => {}
        }
        cursor += 1;
    }
    let text = parts.join(" ");
    if text.is_empty() { None } else { Some(text) }
}

fn unquote(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        raw[1..raw.len() - 1].to_owned()
    } else {
        raw.to_owned()
    }
}

/// Upstream's reason, or the rule-attempt count when no rule wrote `expect`.
/// A repetition ate a separator that has no following item:
/// `{ 1 => "one", }` or `head:, tail`. The closer is ignored.
/// A comma with a later field (`id: string, total: number`) is not trailing.
fn has_separated_rep(pattern: &[Pat]) -> bool {
    pattern.iter().any(|pat| match pat {
        Pat::Rep { sep: Some(_), .. } => true,
        Pat::Rep { inner, .. } | Pat::Group { inner, .. } => has_separated_rep(inner),
        Pat::Choice(alternatives) => alternatives.iter().any(|alt| has_separated_rep(alt)),
        _ => false,
    })
}

fn dangling_separator(source: &str, start: usize, end: usize) -> bool {
    let text = source.get(start..end).unwrap_or("");
    let mut depth = 0i32;
    let mut last_sep: Option<(i32, usize)> = None;
    let mut last_item: Option<(i32, usize)> = None;
    for (at, ch) in text.char_indices() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ',' | '|' => last_sep = Some((depth, at)),
            ch if ch.is_whitespace() => {}
            _ => last_item = Some((depth, at)),
        }
    }
    match (last_sep, last_item) {
        (Some((sep_depth, sep_at)), Some((item_depth, item_at))) => {
            sep_at > item_at && sep_depth >= item_depth && sep_depth > 0
        }
        (Some((sep_depth, _)), None) => sep_depth > 0,
        _ => false,
    }
}

fn macro_failure_detail(name: &str, rules: &[Rule]) -> String {
    let mut reasons = Vec::new();
    for rule in rules {
        if let Some(expect) = &rule.expect {
            if !reasons.iter().any(|have| have == expect) {
                reasons.push(expect.clone());
            }
        }
    }
    if reasons.len() == 1 {
        return reasons.pop().unwrap_or_default();
    }
    if reasons.len() > 1 {
        return reasons.join("; ");
    }
    // `each` fails on the missing brace, not on a written `expect`.
    if name == "each" {
        return "expected a braced group".to_owned();
    }
    format!("{} rule attempt(s)", rules.len())
}

fn is_arrow(nodes: &[Syntax], index: usize) -> bool {
    token_at(nodes, index, "=>")
        || operator_is(nodes.get(index), "=>")
        || (operator_is(nodes.get(index), "=") && operator_is(nodes.get(index + 1), ">"))
}

fn pattern_of(group: &Syntax) -> Vec<Pat> {
    let Syntax::Group { children, .. } = group else {
        return Vec::new();
    };
    pats_from(children)
}

fn pats_from(nodes: &[Syntax]) -> Vec<Pat> {
    let mut alternatives: Vec<Vec<Pat>> = Vec::new();
    let mut pats = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        // A `|` with space on both sides separates alternatives; `$x |= y`
        // and `$x:tt|` are literals (upstream parser.ts `#parsePatternSequence`).
        if operator_is(nodes.get(index), "|")
            && !pats.is_empty()
            && index > 0
            && node_start(&nodes[index]) > node_end(&nodes[index - 1])
            && nodes
                .get(index + 1)
                .is_some_and(|next| node_start(next) > node_end(&nodes[index]))
        {
            alternatives.push(std::mem::take(&mut pats));
            index += 1;
            continue;
        }
        if is_rep_open(nodes, index) {
            let (pat, next) = read_rep(nodes, index);
            pats.push(pat);
            index = next;
            continue;
        }
        if is_hole(nodes, index) {
            pats.push(hole_at(nodes, index));
            // `$name : kind`
            index += 4;
            continue;
        }
        match &nodes[index] {
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => {
                pats.push(Pat::Lit(raw.clone()));
            }
            Syntax::Group {
                delimiter,
                children,
                ..
            } => {
                pats.push(Pat::Group {
                    delimiter: *delimiter,
                    inner: pats_from(children),
                });
            }
            Syntax::String { raw, .. } => pats.push(Pat::Lit(raw.clone())),
            Syntax::Comment { .. } => {}
        }
        index += 1;
    }
    if alternatives.is_empty() {
        return pats;
    }
    alternatives.push(pats);
    vec![Pat::Choice(alternatives)]
}

fn is_rep_open(nodes: &[Syntax], index: usize) -> bool {
    dollar_at(nodes, index) && group_is(nodes.get(index + 1), Delimiter::Paren)
}

fn is_hole(nodes: &[Syntax], index: usize) -> bool {
    dollar_at(nodes, index)
        && matches!(nodes.get(index + 1), Some(Syntax::Token { .. }))
        && operator_is(nodes.get(index + 2), ":")
}

fn dollar_at(nodes: &[Syntax], index: usize) -> bool {
    match nodes.get(index) {
        Some(Syntax::Operator { raw, .. } | Syntax::Token { raw, .. }) => raw == "$",
        _ => false,
    }
}

fn hole_at(nodes: &[Syntax], index: usize) -> Pat {
    let name = match nodes.get(index + 1) {
        Some(Syntax::Token { raw, .. }) => raw.clone(),
        _ => String::new(),
    };
    let kind = match nodes.get(index + 3) {
        Some(Syntax::Token { raw, .. }) => hole_kind(raw),
        _ => HoleKind::TokenTree,
    };
    Pat::Hole { name, kind }
}

fn read_rep(nodes: &[Syntax], index: usize) -> (Pat, usize) {
    let Syntax::Group { children, .. } = &nodes[index + 1] else {
        return (Pat::Lit("$".to_owned()), index + 1);
    };
    let inner = pats_from(children);
    let mut cursor = index + 2;
    // `$($segment:Segment)/+` separates with `/`. A following `+` or `*`
    // is the quantifier, not another literal in the pattern.
    let sep = if matches!(nodes.get(cursor), Some(Syntax::Operator { raw, .. }) if raw != "+" && raw != "*")
        && matches!(nodes.get(cursor + 1), Some(Syntax::Operator { raw, .. }) if raw == "+" || raw == "*")
    {
        let raw = match &nodes[cursor] {
            Syntax::Operator { raw, .. } => raw.clone(),
            _ => String::new(),
        };
        cursor += 1;
        Some(raw)
    } else if (operator_is(nodes.get(cursor), ",")
        || operator_is(nodes.get(cursor), ";")
        || operator_is(nodes.get(cursor), "|"))
        && matches!(nodes.get(cursor + 1), Some(Syntax::Operator { raw, .. }) if raw == "+" || raw == "*")
    {
        let raw = match &nodes[cursor] {
            Syntax::Operator { raw, .. } => raw.clone(),
            _ => ",".to_owned(),
        };
        cursor += 1;
        Some(raw)
    } else if matches!(nodes.get(cursor), Some(Syntax::Token { raw, .. }) if is_ident_text(raw))
        && matches!(nodes.get(cursor + 1), Some(Syntax::Operator { raw, .. }) if raw == "+" || raw == "*" || raw == "?")
    {
        // `$($terms:Term) and +` separates on the word. The `+` is the
        // quantifier, not a literal after the repetition.
        let raw = match &nodes[cursor] {
            Syntax::Token { raw, .. } => raw.clone(),
            _ => String::new(),
        };
        cursor += 1;
        Some(raw)
    } else {
        None
    };
    let kind = if operator_is(nodes.get(cursor), "+") {
        cursor += 1;
        RepKind::OneOrMore
    } else if operator_is(nodes.get(cursor), "*") {
        cursor += 1;
        RepKind::ZeroOrMore
    } else if operator_is(nodes.get(cursor), "?") {
        // `$($filter:Filter)?` matches at most once (upstream
        // `createOptionalPattern`).
        cursor += 1;
        RepKind::Optional
    } else {
        RepKind::ZeroOrMore
    };
    (Pat::Rep { inner, sep, kind }, cursor)
}

fn hole_kind(raw: &str) -> HoleKind {
    match raw {
        "expr" => HoleKind::Expr,
        "stmt" => HoleKind::Stmt,
        "type" => HoleKind::Type,
        "ident" => HoleKind::Ident,
        // A name or a destructuring pattern.
        "binding" => HoleKind::Binding,
        "classElement" => HoleKind::Member { type_member: false },
        "typeMember" => HoleKind::Member { type_member: true },
        "tt" | "token" => HoleKind::TokenTree,
        // Positions, not user syntax classes. `$body:item` is one item.
        "item" => HoleKind::Item,
        "jsx" | "jsxChild" => HoleKind::JsxChild,
        // A user syntax class (`State`, `Segment`) is not a builtin hole.
        // Matching looks the class up by this spelling.
        other => HoleKind::Class(other.to_owned()),
    }
}

fn template_of(group: &Syntax) -> Vec<Tpl> {
    let Syntax::Group { children, .. } = group else {
        return Vec::new();
    };
    tpls_from(children, String::new())
}

/// Template pieces taken from the macro source, so copied text keeps its
/// spaces and closers. `$name` and `$($name),*` are the only substitutions.
fn template_from_source(source: &str, group: &Syntax) -> Vec<Tpl> {
    let Syntax::Group { children, span, .. } = group else {
        return Vec::new();
    };
    let mut tpls = Vec::new();
    let mut cursor = span.start as usize + 1;
    let end = span.end as usize;
    walk_template(source, children, &mut tpls, &mut cursor);
    if cursor < end.saturating_sub(1) {
        let text = source.get(cursor..end.saturating_sub(1)).unwrap_or("");
        if !text.is_empty() {
            tpls.push(Tpl::Text(text.to_owned()));
        }
    }
    tpls
}

fn walk_template(source: &str, nodes: &[Syntax], tpls: &mut Vec<Tpl>, cursor: &mut usize) {
    let mut index = 0;
    while index < nodes.len() {
        // A comment in a template is the macro author's note. Theirs never
        // prints it, and the whitespace after it belongs to the comment.
        if let Syntax::Comment { span, .. } = &nodes[index] {
            push_gap(source, tpls, cursor, span.start as usize);
            let end = span.end as usize;
            let rest = source.get(end..).unwrap_or("");
            *cursor = end + (rest.len() - rest.trim_start().len());
            index += 1;
            continue;
        }
        if is_rep_open(nodes, index) {
            push_gap(source, tpls, cursor, node_start(&nodes[index]));
            let (tpl, next) = read_tpl_rep(nodes, index);
            if let Syntax::Group { children, span, .. } = &nodes[index + 1] {
                let mut inner = Vec::new();
                let mut inner_cursor = span.start as usize + 1;
                walk_template(source, children, &mut inner, &mut inner_cursor);
                push_gap(source, &mut inner, &mut inner_cursor, span.end as usize - 1);
                let sep = if let Tpl::Rep { sep, .. } = &tpl {
                    sep.clone()
                } else {
                    None
                };
                tpls.push(Tpl::Rep { inner, sep });
            } else {
                tpls.push(tpl);
            }
            if next > index {
                *cursor = node_end(&nodes[next - 1]);
            }
            index = next;
            continue;
        }
        if let Some((tpl, next)) = read_directive(source, nodes, index) {
            // `#text($name)` inside a template literal stays in the copied
            // text. Pulling it out splits `` `${#text($name)}` `` so the
            // quotes around the capture are lost.
            if directive_inside_template(source, *cursor, node_start(&nodes[index])) {
                index += 1;
                continue;
            }
            push_gap(source, tpls, cursor, node_start(&nodes[index]));
            let end_node = next.saturating_sub(1);
            tpls.push(tpl);
            if end_node < nodes.len() {
                *cursor = node_end(&nodes[end_node]);
            }
            index = next;
            continue;
        }
        // The reader keeps `#` and `core` apart so a call-site `#macro` still
        // splits. A template directive is the pair.
        if is_hash_directive(nodes, index) {
            if let Some((tpl, next)) = read_split_directive(source, nodes, index) {
                if directive_inside_template(source, *cursor, node_start(&nodes[index])) {
                    index += 1;
                    continue;
                }
                push_gap(source, tpls, cursor, node_start(&nodes[index]));
                let end_node = next.saturating_sub(1);
                tpls.push(tpl);
                if end_node < nodes.len() {
                    *cursor = node_end(&nodes[end_node]);
                }
                index = next;
                continue;
            }
        }
        if dollar_at(nodes, index) {
            if let Some((path, end_at, next)) = read_var_path(nodes, index) {
                // `$protocol.register` is a text capture plus a member. The
                // member stays in the copied text. `$method.name(` is a field
                // path used as a call, so the whole path is one variable.
                push_gap(source, tpls, cursor, node_start(&nodes[index]));
                tpls.push(Tpl::Var(path));
                *cursor = end_at;
                index = next;
                continue;
            }
        }
        if let Syntax::Group { children, span, .. } = &nodes[index] {
            push_gap(source, tpls, cursor, span.start as usize + 1);
            walk_template(source, children, tpls, cursor);
            index += 1;
            continue;
        }
        // A template literal is one string. `#text($name)` inside `${}` is
        // still a directive; leave it as text so substitution can see it.
        if let Syntax::String { raw, .. } = &nodes[index] {
            let _ = raw;
            index += 1;
            continue;
        }
        index += 1;
    }
}

fn text_hole_name(group: &Syntax) -> Option<String> {
    let Syntax::Group { children, .. } = group else {
        return None;
    };
    read_var_path(children, 0).map(|(path, _, _)| path)
}

/// `$name`, `$name.field`, and `$name.field.nested`. The path is one capture
/// key. A following `(` is a call, not a field, so it stays in the copied text.
fn read_var_path(nodes: &[Syntax], index: usize) -> Option<(String, usize, usize)> {
    if !dollar_at(nodes, index) {
        return None;
    }
    let Syntax::Token { raw, span, .. } = nodes.get(index + 1)? else {
        return None;
    };
    let mut path = raw.clone();
    let mut end_at = span.end as usize;
    let mut cursor = index + 2;
    while operator_is(nodes.get(cursor), ".") {
        let Syntax::Token {
            raw: field,
            span: field_span,
            ..
        } = nodes.get(cursor + 1)?
        else {
            break;
        };
        path.push('.');
        path.push_str(field);
        end_at = field_span.end as usize;
        cursor += 2;
    }
    Some((path, end_at, cursor))
}

fn is_hash_directive(nodes: &[Syntax], index: usize) -> bool {
    operator_is(nodes.get(index), "#")
        && matches!(nodes.get(index + 1), Some(Syntax::Token { .. }))
        && (group_is(nodes.get(index + 2), Delimiter::Paren)
            || group_is(nodes.get(index + 2), Delimiter::Brace))
}

fn read_split_directive(source: &str, nodes: &[Syntax], index: usize) -> Option<(Tpl, usize)> {
    let Syntax::Token { raw, span, .. } = nodes.get(index + 1)? else {
        return None;
    };
    let name = format!("#{raw}");
    if name == "#syntax" && group_is(nodes.get(index + 2), Delimiter::Brace) {
        let body = nodes.get(index + 2)?;
        let tpl = syntax_define(source, body)?;
        return Some((tpl, index + 3));
    }
    if name == "#else" {
        return None;
    }
    let args = nodes.get(index + 2)?;
    if !group_is(Some(args), Delimiter::Paren) {
        return None;
    }
    let Syntax::Group {
        children,
        span: arg_span,
        ..
    } = args
    else {
        return None;
    };
    if name == "#if" {
        return read_if(source, nodes, index + 3, span.start as usize, children);
    }
    if let Some(found) = read_scoped(source, nodes, index + 3, &name, children, arg_span) {
        return Some(found);
    }
    let tpl = directive_tpl(source, &name, span.start as usize, args, children, arg_span)?;
    Some((tpl, index + 3))
}

/// `#name(...)` in a template. Implemented directives become nodes. The rest
/// are recorded so printing can diagnose them instead of emitting `#name`.
fn read_directive(source: &str, nodes: &[Syntax], index: usize) -> Option<(Tpl, usize)> {
    let Syntax::Token { raw, span, .. } = nodes.get(index)? else {
        return None;
    };
    if !raw.starts_with('#') || raw.len() < 2 {
        return None;
    }
    if raw == "#syntax" && group_is(nodes.get(index + 1), Delimiter::Brace) {
        let body = nodes.get(index + 1)?;
        let tpl = syntax_define(source, body)?;
        return Some((tpl, index + 2));
    }
    if raw == "#else" {
        return None;
    }
    let Some(args) = nodes.get(index + 1) else {
        return None;
    };
    if !group_is(Some(args), Delimiter::Paren) {
        return None;
    }
    let Syntax::Group {
        children,
        span: arg_span,
        ..
    } = args
    else {
        return None;
    };
    if raw == "#if" {
        return read_if(source, nodes, index + 2, span.start as usize, children);
    }
    if let Some(found) = read_scoped(source, nodes, index + 2, raw, children, arg_span) {
        return Some(found);
    }
    let tpl = directive_tpl(source, raw, span.start as usize, args, children, arg_span)?;
    Some((tpl, index + 2))
}

/// A template operation whose argument is not what it takes: SWR2017.
fn invalid_operation(name: &str, at: usize) -> Tpl {
    Tpl::Unknown {
        name: format!("{INVALID_OPERATION}{name}"),
        at,
    }
}

const INVALID_OPERATION: &str = "invalid:";

/// `#fold($items, init: { initial }) { ($acc, $item, $index) => { next } }`.
fn read_fold(
    source: &str,
    nodes: &[Syntax],
    body_at: usize,
    children: &[Syntax],
    at: usize,
) -> Option<(Tpl, usize)> {
    let invalid = || Some((invalid_operation("#fold", at), body_at + 1));
    let Some((path, _, next)) = read_var_path(children, 0) else {
        return invalid();
    };
    if !(operator_is(children.get(next), ",")
        && token_at(children, next + 1, "init")
        && operator_is(children.get(next + 2), ":"))
    {
        return invalid();
    }
    let Some(init_group) = children
        .get(next + 3)
        .filter(|group| group_is(Some(group), Delimiter::Brace))
    else {
        return invalid();
    };
    let init = group_template(source, init_group);
    let Some(Syntax::Group {
        children: lambda, ..
    }) = nodes
        .get(body_at)
        .filter(|group| group_is(Some(group), Delimiter::Brace))
    else {
        return invalid();
    };
    let Some(Syntax::Group {
        children: params, ..
    }) = lambda.first()
    else {
        return invalid();
    };
    let names: Vec<String> = (0..params.len())
        .filter_map(|index| hole_name_at(params, index))
        .collect();
    if names.len() != 3 || !is_arrow(lambda, 1) {
        return invalid();
    }
    let body_index = (2..lambda.len()).find(|index| group_is(lambda.get(*index), Delimiter::Brace));
    let Some(body_group) = body_index.and_then(|index| lambda.get(index)) else {
        return invalid();
    };
    Some((
        Tpl::Fold {
            path,
            init,
            acc: names[0].clone(),
            item: names[1].clone(),
            index: names[2].clone(),
            body: group_template(source, body_group),
        },
        body_at + 1,
    ))
}

/// `#let(name = value) { body }` and `#parameterize([required] p = value)
/// { body }`. The value is a template; so is the body.
fn read_scoped(
    source: &str,
    nodes: &[Syntax],
    body_at: usize,
    name: &str,
    children: &[Syntax],
    arg_span: &Span,
) -> Option<(Tpl, usize)> {
    if name == "#fold" && group_is(nodes.get(body_at), Delimiter::Brace) {
        return read_fold(
            source,
            nodes,
            body_at,
            children,
            node_start(&nodes[body_at - 1]).saturating_sub(5),
        );
    }
    if name != "#let" && name != "#parameterize" {
        return None;
    }
    let body = nodes.get(body_at)?;
    if !group_is(Some(body), Delimiter::Brace) {
        return None;
    }
    let mut at = 0;
    let required = name == "#parameterize" && token_at(children, at, "required");
    if required {
        at += 1;
    }
    let bound = match children.get(at)? {
        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => raw.clone(),
        _ => return None,
    };
    if !operator_is(children.get(at + 1), "=") {
        return None;
    }
    let first = children.get(at + 2)?;
    let mut value = Vec::new();
    let mut cursor = node_start(first);
    walk_template(source, &children[at + 2..], &mut value, &mut cursor);
    push_gap(source, &mut value, &mut cursor, arg_span.end as usize - 1);
    let body = group_template(source, body);
    let tpl = if name == "#let" {
        Tpl::Let {
            name: bound,
            value,
            body,
        }
    } else {
        Tpl::Parameterize {
            param: bound,
            required,
            value,
            body,
        }
    };
    Some((tpl, body_at + 1))
}

fn syntax_define(source: &str, body: &Syntax) -> Option<Tpl> {
    let Syntax::Group { children, span, .. } = body else {
        return None;
    };
    let mut inner = Vec::new();
    let mut cursor = span.start as usize + 1;
    walk_template(source, children, &mut inner, &mut cursor);
    push_gap(source, &mut inner, &mut cursor, span.end as usize - 1);
    Some(Tpl::Define(inner))
}

/// `#if(present $path) { then } #else { else }`.
fn read_if(
    source: &str,
    nodes: &[Syntax],
    after_pred: usize,
    at: usize,
    predicate: &[Syntax],
) -> Option<(Tpl, usize)> {
    let path = if_present_path(predicate)?;
    let then_group = nodes.get(after_pred)?;
    if !group_is(Some(then_group), Delimiter::Brace) {
        return Some((
            Tpl::Unknown {
                name: "#if".to_owned(),
                at,
            },
            after_pred,
        ));
    }
    let then_body = group_template(source, then_group);
    let mut next = after_pred + 1;
    let else_body = if if_else_at(nodes, next) {
        let else_at = if operator_is(nodes.get(next), "#") {
            next + 2
        } else {
            next + 1
        };
        let group = nodes.get(else_at)?;
        if group_is(Some(group), Delimiter::Brace) {
            next = else_at + 1;
            Some(group_template(source, group))
        } else {
            None
        }
    } else {
        None
    };
    Some((
        Tpl::If {
            path,
            then_body,
            else_body,
        },
        next,
    ))
}

fn if_else_at(nodes: &[Syntax], index: usize) -> bool {
    token_at(nodes, index, "#else")
        || (operator_is(nodes.get(index), "#") && token_at(nodes, index + 1, "else"))
}

fn if_present_path(children: &[Syntax]) -> Option<String> {
    if !token_at(children, 0, "present") {
        return None;
    }
    read_var_path(children, 1).map(|(path, _, _)| path)
}

fn group_template(source: &str, group: &Syntax) -> Vec<Tpl> {
    let Syntax::Group { children, span, .. } = group else {
        return Vec::new();
    };
    let mut inner = Vec::new();
    let mut cursor = span.start as usize + 1;
    walk_template(source, children, &mut inner, &mut cursor);
    push_gap(source, &mut inner, &mut cursor, span.end as usize - 1);
    inner
}

fn directive_tpl(
    source: &str,
    raw: &str,
    at: usize,
    args: &Syntax,
    children: &[Syntax],
    arg_span: &Span,
) -> Option<Tpl> {
    match raw {
        "#text" => text_hole_name(args).map(Tpl::Quote).or_else(|| {
            let Syntax::Group { children, .. } = args else {
                return None;
            };
            read_var_path(children, 0).map(|(path, _, _)| Tpl::Quote(path))
        }),
        "#core" => {
            let mut inner = Vec::new();
            let mut inner_cursor = arg_span.start as usize + 1;
            walk_template(source, children, &mut inner, &mut inner_cursor);
            push_gap(
                source,
                &mut inner,
                &mut inner_cursor,
                arg_span.end as usize - 1,
            );
            Some(Tpl::Core(inner))
        }
        "#join" => Some(read_join(children)),
        "#count" => text_hole_name(args).map(Tpl::Count),
        "#metavar" => read_metavar(children),
        "#trim" => read_trim(children),
        "#index" => Some(Tpl::Index),
        "#fresh" => Some(
            string_value(children.first())
                .filter(|hint| children.len() == 1 && is_ident_text(hint))
                .map(Tpl::Fresh)
                .unwrap_or_else(|| invalid_operation(raw, at)),
        ),
        // The capture's identifier, with the call site's, the definition's
        // or no introduction scope. Output hygiene sees a captured spelling
        // as the user's in each case, so each prints the capture.
        "#callsite" | "#definition" | "#capture" => Some(
            read_var_path(children, 0)
                .filter(|(_, _, next)| *next == children.len())
                .map(|(path, _, _)| Tpl::Var(path))
                .unwrap_or_else(|| invalid_operation(raw, at)),
        ),
        name if name.starts_with('#') => Some(Tpl::Unknown {
            name: name.to_owned(),
            at,
        }),
        _ => None,
    }
}

/// `#join($name, suffix: "Signal")` and the prefix / casing options beside it.
/// `#metavar("value", $name)` — the hole is `value`, not the binding.
fn read_metavar(children: &[Syntax]) -> Option<Tpl> {
    let hole = string_value(children.first())?;
    if hole.is_empty() {
        return None;
    }
    let mut key = String::new();
    let mut index = 1;
    if operator_is(children.get(index), ",") {
        index += 1;
    }
    if let Some((path, _, _)) = read_var_path(children, index) {
        key = path;
    }
    Some(Tpl::Metavar { hole, key })
}

fn metavar_hole(hole: &str, key: &str, captures: &HashMap<String, Capture>) -> String {
    // `$parameter` is one binding. Inside `$($segment(... $parameter ...))+`
    // the key is `segment.parameter`: this row's parameter, not the first
    // segment's. A bare lookup of that path misses, and every nested
    // segment is then stamped with the outer parameter.
    let Some(text) = metavar_key_text(key, captures) else {
        return hole.to_owned();
    };
    let text = text.trim();
    if text.is_empty() || !is_ident_text(text) {
        return hole.to_owned();
    }
    format!("{hole}_{text}")
}

/// The binding `#metavar` suffixes with. A dotted key is this repetition
/// row's field (`segment.parameter` -> `maximum`), not a capture stored
/// under the whole path.
fn metavar_key_text<'a>(key: &str, captures: &'a HashMap<String, Capture>) -> Option<&'a str> {
    if let Some(text) = capture_text(captures, key) {
        let text = text.trim();
        if !text.is_empty() && is_ident_text(text) {
            return Some(text);
        }
    }
    let (parent, leaf) = key.rsplit_once('.')?;
    let record = captures.get(parent).or_else(|| captures.get(leaf))?;
    let Capture::Record { fields, .. } = record else {
        return None;
    };
    let field = fields.get(leaf)?;
    capture_spelling(field).filter(|text| {
        let text = text.trim();
        !text.is_empty() && is_ident_text(text)
    })
}

/// `#trim($path)` and `#trim($path.field)`.
fn read_trim(children: &[Syntax]) -> Option<Tpl> {
    let (path, _, _) = read_var_path(children, 0)?;
    Some(Tpl::Trim { path })
}

fn read_join(children: &[Syntax]) -> Tpl {
    let mut name = String::new();
    let mut index = 0;
    if let Some((path, _, next)) = read_var_path(children, 0) {
        name = path;
        index = next;
    } else if let Some(Syntax::Token { raw, .. }) = children.first() {
        name = raw.clone();
    }
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut casing = "preserve".to_owned();
    while index < children.len() {
        if operator_is(children.get(index), ",") {
            index += 1;
            continue;
        }
        let Some(Syntax::Token { raw: option, .. }) = children.get(index) else {
            break;
        };
        if !operator_is(children.get(index + 1), ":") {
            break;
        }
        let value = string_value(children.get(index + 2)).unwrap_or_default();
        match option.as_str() {
            "prefix" => prefix = value,
            "suffix" => suffix = value,
            "casing" => casing = value,
            _ => {}
        }
        index += 3;
    }
    Tpl::Join {
        name,
        prefix,
        suffix,
        casing,
    }
}

fn string_value(node: Option<&Syntax>) -> Option<String> {
    let Syntax::String { raw, .. } = node? else {
        return None;
    };
    let quote = raw.chars().next()?;
    raw.strip_prefix(quote)
        .and_then(|rest| rest.strip_suffix(quote))
        .map(unescape_string)
}

fn unescape_string(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// True when `at` sits between backticks that `cursor` has not yet copied.
/// That span is a template literal, so a directive inside it is text.
fn directive_inside_template(source: &str, cursor: usize, at: usize) -> bool {
    if at < cursor || at > source.len() {
        return false;
    }
    let before = &source[cursor..at];
    let mut ticks = 0usize;
    let bytes = before.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'`' {
            ticks += 1;
        }
        index += 1;
    }
    ticks % 2 == 1
}

fn push_gap(source: &str, tpls: &mut Vec<Tpl>, cursor: &mut usize, until: usize) {
    if until < *cursor || until > source.len() {
        return;
    }
    let text = &source[*cursor..until];
    if text.is_empty() {
        return;
    }
    // `$$name` is a literal `$` followed by the name. The source spells the
    // dollar twice so it is not a hole; the printed text has one.
    let owned = text.replace("$$", "$");
    let text = owned.as_str();
    if let Some(Tpl::Text(existing)) = tpls.last_mut() {
        existing.push_str(text);
    } else {
        tpls.push(Tpl::Text(text.to_owned()));
    }
    *cursor = until;
}

fn tpls_from(nodes: &[Syntax], _unused: String) -> Vec<Tpl> {
    let mut tpls = Vec::new();
    let mut index = 0;
    let mut pending = String::new();
    let flush = |pending: &mut String, tpls: &mut Vec<Tpl>| {
        if !pending.is_empty() {
            tpls.push(Tpl::Text(std::mem::take(pending)));
        }
    };
    while index < nodes.len() {
        if is_rep_open(nodes, index) {
            flush(&mut pending, &mut tpls);
            let (tpl, next) = read_tpl_rep(nodes, index);
            tpls.push(tpl);
            index = next;
            continue;
        }
        if dollar_at(nodes, index) {
            if let Some(Syntax::Token { raw, .. }) = nodes.get(index + 1) {
                flush(&mut pending, &mut tpls);
                tpls.push(Tpl::Var(raw.clone()));
                index += 2;
                continue;
            }
        }
        match &nodes[index] {
            Syntax::Token { raw, .. } => {
                if !pending.is_empty() && !pending.ends_with([' ', '\n', '(']) {
                    pending.push(' ');
                }
                pending.push_str(raw);
            }
            Syntax::Operator { raw, .. } => {
                let tight = matches!(
                    raw.as_str(),
                    "," | ";" | "." | ":" | ")" | "]" | "}" | "(" | "[" | "{" | "!"
                );
                if !tight && !pending.is_empty() && !pending.ends_with([' ', '\n', '(']) {
                    pending.push(' ');
                }
                pending.push_str(raw);
                if raw == "," || raw == ";" {
                    pending.push(' ');
                }
            }
            Syntax::String { raw, .. } => pending.push_str(raw),
            Syntax::Group {
                delimiter,
                children,
                ..
            } => {
                pending.push(open_char(*delimiter));
                // Nested groups in a template are ordinary text plus holes.
                let nested = tpls_from(children, String::new());
                if nested.iter().all(|part| matches!(part, Tpl::Text(_))) {
                    for part in nested {
                        if let Tpl::Text(text) = part {
                            pending.push_str(&text);
                        }
                    }
                } else {
                    flush(&mut pending, &mut tpls);
                    tpls.extend(flatten_group(*delimiter, nested));
                    // The closer was emitted inside flatten; do not also append.
                    index += 1;
                    continue;
                }
                pending.push(match delimiter {
                    Delimiter::Paren => ')',
                    Delimiter::Brace => '}',
                    Delimiter::Bracket => ']',
                });
            }
            Syntax::Comment { .. } => {}
        }
        index += 1;
    }
    flush(&mut pending, &mut tpls);
    tpls
}

fn flatten_group(delimiter: Delimiter, nested: Vec<Tpl>) -> Vec<Tpl> {
    let mut out = vec![Tpl::Text(open_char(delimiter).to_string())];
    out.extend(nested);
    let close = match delimiter {
        Delimiter::Paren => ')',
        Delimiter::Brace => '}',
        Delimiter::Bracket => ']',
    };
    out.push(Tpl::Text(close.to_string()));
    out
}

fn read_tpl_rep(nodes: &[Syntax], index: usize) -> (Tpl, usize) {
    let Syntax::Group { children, .. } = &nodes[index + 1] else {
        return (Tpl::Text("$".to_owned()), index + 1);
    };
    let inner = tpls_from(children, String::new());
    let mut cursor = index + 2;
    let sep = if operator_is(nodes.get(cursor), ",")
        || operator_is(nodes.get(cursor), ";")
        || operator_is(nodes.get(cursor), "|")
    {
        let raw = match &nodes[cursor] {
            Syntax::Operator { raw, .. } => Some(raw.clone()),
            _ => Some(",".to_owned()),
        };
        cursor += 1;
        raw
    } else {
        None
    };
    if operator_is(nodes.get(cursor), "*") || operator_is(nodes.get(cursor), "+") {
        cursor += 1;
    }
    (Tpl::Rep { inner, sep }, cursor)
}

struct Match {
    next: usize,
    end: usize,
    captures: HashMap<String, Capture>,
}

#[derive(Clone, Debug, PartialEq)]
enum Capture {
    Text(String),
    /// A syntax-class match. `text` is the whole form; `fields` are its
    /// declared fields, including sequences.
    Record {
        text: String,
        fields: HashMap<String, Capture>,
    },
    Many(Vec<Vec<(String, Capture)>>),
}

struct RuleStop {
    span: Span,
    /// `expect` from the class rule that stopped, when the macro rule has none.
    expect: Option<String>,
    progress: i32,
}

/// The token where `pattern` stopped, and how far it got. A finished match
/// has no failure. Progress is the byte where matching stopped, so a rule
/// that consumed more of the input wins over one that failed on the head.
fn failure_progress(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    pattern: &[Pat],
    classes: &[SyntaxClass],
) -> Option<RuleStop> {
    let mut captures = HashMap::new();
    let mut cursor = index;
    let pattern = skip_matched_prefix(nodes, index, pattern);
    let mut progress = nodes.get(index).map(node_start).unwrap_or(0) as i32;
    let mut expect = None;
    let span = match_pats_fail(
        source,
        nodes,
        &mut cursor,
        pattern,
        &mut captures,
        classes,
        &mut progress,
        &mut expect,
    )?;
    // A nested class stopped on `value` in `value:)`. The closer after
    // the colon is the missing type, not the field name. `state = 1`
    // already stopped on `=`; that is not a name before a colon.
    let span = if nodes
        .iter()
        .any(|node| node_span(node).start == span.start && matches!(node, Syntax::Token { .. }))
    {
        closer_after_colon(source, nodes, index, span)
    } else {
        span
    };
    Some(RuleStop {
        span,
        expect,
        progress,
    })
}

/// `value:)` stopped on `value`. The token after the colon is the failure.
/// The colon may be inside a group the outer list only sees as one node.
fn closer_after_colon(source: &str, nodes: &[Syntax], from: usize, span: Span) -> Span {
    let _ = source;
    // The failed token may be inside a group that starts before `from`.
    // `Some(value:)` is that group; searching only from the name misses it.
    let start = nodes
        .iter()
        .rposition(|node| node_span(node).start <= span.start && node_span(node).end >= span.end)
        .unwrap_or(from);
    closer_in(&nodes[start..], span).unwrap_or(span)
}

fn closer_in(nodes: &[Syntax], span: Span) -> Option<Span> {
    let at = nodes
        .iter()
        .position(|node| node_span(node).start == span.start)?;
    if nodes
        .get(at + 1)
        .is_some_and(|node| operator_is(Some(node), ":"))
    {
        let next = nodes.get(at + 2)?;
        if matches!(next, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ")" | "," | ";" | "}" | "]"))
        {
            return Some(failure_token(next));
        }
    }
    for node in nodes {
        if let Syntax::Group { children, .. } = node {
            if node_span(node).start <= span.start && node_span(node).end >= span.end {
                if let Some(found) = closer_in(children, span) {
                    return Some(found);
                }
            }
            // `(value:)` is the group after the name the rule stopped on.
            if node_span(node).start >= span.end {
                if let Some(found) = missing_colon_in(children) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// `value:` inside a group the caller sees as one node. The closer is the
/// missing type.
fn missing_colon_in(nodes: &[Syntax]) -> Option<Span> {
    let mut index = 0;
    while index + 1 < nodes.len() {
        let name = matches!(&nodes[index], Syntax::Token { raw, .. } if is_ident_text(raw));
        if name && operator_is(nodes.get(index + 1), ":") {
            let after = index + 2;
            if nodes.get(after).is_some_and(|node| {
                matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ")" | "," | ";" | "}" | "]"))
            }) || nodes.get(after).is_none()
            {
                return nodes.get(after).map(failure_token).or_else(|| nodes.get(index + 1).map(failure_token));
            }
        }
        index += 1;
    }
    None
}

fn match_pats_fail(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
    progress: &mut i32,
    expect: &mut Option<String>,
) -> Option<Span> {
    for (pat_index, pat) in pattern.iter().enumerate() {
        match pat {
            Pat::Choice(alternatives) => {
                let saved = *cursor;
                let mut chosen = false;
                let last = pat_index + 1 == pattern.len();
                for alternative in choice_order(source, nodes, saved, alternatives, classes, last) {
                    let mut tried = saved;
                    let mut taken = captures.clone();
                    if match_pats(source, nodes, &mut tried, alternative, &mut taken, classes)
                        .is_some()
                    {
                        *cursor = tried;
                        *captures = taken;
                        if let Some(node) = tried.checked_sub(1).and_then(|at| nodes.get(at)) {
                            *progress = (*progress).max(node_end(node) as i32);
                        }
                        chosen = true;
                        break;
                    }
                }
                if !chosen {
                    let first = alternatives.first()?;
                    return match_pats_fail(
                        source, nodes, cursor, first, captures, classes, progress, expect,
                    )
                    .or_else(|| nodes.get(*cursor).map(failure_token));
                }
            }
            Pat::Lit(expected) => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                if let Some(width) = lit_width(nodes, *cursor, expected) {
                    *progress = (*progress).max(node_end(&nodes[*cursor + width - 1]) as i32);
                    *cursor += width;
                } else if node_is_lit(node, expected) {
                    *progress = (*progress).max(node_end(node) as i32);
                    *cursor += 1;
                } else if expected == "_" {
                    // `{ 1 => "one", }` wanted `_` or another arm. The
                    // trailing comma's closer is the failure, not `1`.
                    if expect.is_none() {
                        *expect = Some("expected `MatchArm` or `_`".to_owned());
                    }
                    return Some(match_closer(source, node).unwrap_or_else(|| failure_token(node)));
                } else if expected.starts_with("seg") {
                    // `between(3) lower(1)` stopped on `lower`. The missing
                    // segment is `upper(`, and that `(` is the failure.
                    return Some(
                        segment_paren(source, node).unwrap_or_else(|| failure_token(node)),
                    );
                } else if let Some(span) =
                    nested_form_failure(source, node_slice_from(nodes, *cursor), 0)
                {
                    return Some(span);
                } else if expected == "state" && operator_is(Some(node), "=") {
                    // `state = 1` has no binding. `=` is where the name
                    // should have been, not the initializer after it.
                    return Some(failure_token(node));
                } else if expected == "assert" {
                    // `assert()` has no condition. The `)` is the failure,
                    // not the macro name.
                    return Some(
                        empty_call_paren(source, node).unwrap_or_else(|| failure_token(node)),
                    );
                } else {
                    return Some(failure_token(node));
                }
            }
            Pat::Group { delimiter, inner } => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                let Syntax::Group {
                    delimiter: found,
                    children,
                    close,
                    ..
                } = node
                else {
                    return Some(failure_token(node));
                };
                if found != delimiter {
                    return Some(failure_token(node));
                }
                let mut child = 0;
                let mut nested = node_start(node) as i32;
                if let Some(span) = match_pats_fail(
                    source,
                    children,
                    &mut child,
                    inner,
                    captures,
                    classes,
                    &mut nested,
                    expect,
                ) {
                    *progress = (*progress).max(nested);
                    // `(value) => value` failed inside the arrow. The rule
                    // was matching a call step, and the empty step is the
                    // comma after this group: `map((value) => value), ,`.
                    if let Some(semi) = bind_semi(nodes.get(*cursor)) {
                        *progress = (*progress).max(semi.end as i32);
                        if expect.is_none() {
                            *expect = Some("expression after <-".to_owned());
                        }
                        return Some(semi);
                    }
                    if let Some(outer) = dangling_after_group(nodes, *cursor) {
                        // The comma is the failure. A nested miss inside the
                        // arrow (`=>`) has a later byte offset, but it is not
                        // further through the rule.
                        *progress = (*progress).max(outer.end as i32 + 1);
                        return Some(outer);
                    }
                    return Some(span);
                }
                *progress = (*progress).max(nested);
                skip_comments(children, &mut child);
                if child < children.len() {
                    // The pattern ended. A trailing separator, or the closer
                    // when the last piece is that separator, is the failure.
                    // `head:, tail` and `{ 1 => "one", }` are that shape.
                    if let Some(span) = trailing_separator(children, child, *close) {
                        *progress = (*progress).max(span.start as i32);
                        return Some(span);
                    }
                    return Some(failure_token(&children[child]));
                }
                *progress = (*progress).max(node_end(node) as i32);
                *cursor += 1;
            }
            Pat::Hole { name, kind } => {
                if let HoleKind::Class(class_name) = kind {
                    let Some(class_) = classes.iter().find(|class_| class_.name == *class_name)
                    else {
                        // `item` and `jsxChild` are positions, not classes in
                        // this module. The hole still wanted a form here.
                        // `record Cons(head:, tail)` fails inside that item,
                        // on the comma after the missing type. `{each ...}`
                        // fails on the line that should have been the body.
                        if let Some(span) = nested_form_failure(source, nodes, *cursor) {
                            return Some(span);
                        }
                        return nodes.get(*cursor).map(failure_token);
                    };
                    if let Some(found) = class_failure(source, nodes, *cursor, class_, classes) {
                        *progress = (*progress).max(found.progress);
                        if expect.is_none() {
                            *expect = found.expect;
                        }
                        return Some(found.span);
                    }
                    let (capture, end, next) =
                        take_class(source, nodes, *cursor, class_, classes, false, &[])?;
                    captures.insert(name.clone(), capture);
                    *progress = (*progress).max(end as i32);
                    *cursor = next;
                } else {
                    skip_comments(nodes, cursor);
                    let stop = match pattern.get(pat_index + 1) {
                        Some(Pat::Lit(text)) => Some(text.as_str()),
                        _ => None,
                    };
                    let taken = if matches!(kind, HoleKind::Ident | HoleKind::Binding)
                        && stop.is_some_and(|word| {
                            nodes
                                .get(*cursor)
                                .is_some_and(|node| node_is_lit(node, word))
                        }) {
                        // `$left:ident join` and the token is already `join`.
                        // The optional word is absent; taking it as the ident
                        // makes `posts` fail `on` and hides the next rule.
                        None
                    } else if *kind == HoleKind::Binding && pattern.len() == 1 {
                        take_expr(source, nodes, *cursor).or_else(|| take_ident(nodes, *cursor))
                    } else if *kind == HoleKind::Expr {
                        let stop = stop
                            .filter(|word| word.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch)));
                        take_expr_until(source, nodes, *cursor, stop)
                    } else {
                        take_hole(source, nodes, *cursor, kind, false)
                    };
                    if taken.is_none() {
                        // The hole wanted a token that is not here. The next
                        // token is that absence: `value:;` and `assert();`.
                        // `value:` has no type. The closer after the colon is
                        // that absence, not the field name the type hole saw.
                        let missing = if matches!(kind, HoleKind::Ident | HoleKind::Binding)
                            && nodes
                                .get(*cursor)
                                .is_some_and(|node| operator_is(Some(node), "="))
                        {
                            // `state = 1` has no name. `=` is that absence.
                            nodes.get(*cursor).map(failure_token)
                        } else if matches!(kind, HoleKind::Type) {
                            type_failure_token(nodes, *cursor)
                        } else if matches!(kind, HoleKind::Expr) {
                            // `value <-;` has no expression. The semicolon is
                            // that absence, not the binder before `<-`.
                            expr_failure_token(nodes, *cursor)
                        } else {
                            nodes.get(*cursor).map(failure_token)
                        };
                        // A later pattern may name a further token, such as
                        // the `}` of an empty function body. Keep the earlier
                        // stop only when nothing after it fails further on.
                        let here = missing;
                        let keep_missing = matches!(kind, HoleKind::Ident | HoleKind::Binding)
                            && nodes
                                .get(*cursor)
                                .is_some_and(|node| operator_is(Some(node), "="));
                        if !keep_missing {
                            if let Some(later) = later_failure(
                                source,
                                nodes,
                                *cursor,
                                &pattern[1..],
                                classes,
                                progress,
                                expect,
                            ) {
                                return Some(later);
                            }
                        }
                        return here;
                    }
                    let (_text, end, next) = taken?;
                    *progress = (*progress).max(end as i32);
                    *cursor = next;
                }
            }
            Pat::Rep { inner, sep, kind } => {
                let mut many = 0usize;
                loop {
                    let saved = *cursor;
                    let mut one = HashMap::new();
                    if match_pats(source, nodes, cursor, inner, &mut one, classes).is_none() {
                        *cursor = saved;
                        if *kind == RepKind::OneOrMore && many == 0 {
                            return match_pats_fail(
                                source, nodes, cursor, inner, &mut one, classes, progress, expect,
                            )
                            .or_else(|| nodes.get(*cursor).map(failure_token));
                        }
                        // `None() | Some(value:)` ate `None()` and stopped.
                        // The next constructor is not the failure; the token
                        // its field rule could not accept is.
                        if let Some(span) = nested_form_failure(source, nodes, *cursor) {
                            return Some(span);
                        }
                        if many > 0 {
                            if let Some(inner_span) = match_pats_fail(
                                source, nodes, cursor, inner, &mut one, classes, progress, expect,
                            ) {
                                let inner_span = if let Some(found) = bind_semi(nodes.get(*cursor))
                                    .or_else(|| {
                                        nodes.get(*cursor + 1).and_then(|node| {
                                            let Syntax::Group {
                                                children,
                                                span: group,
                                                ..
                                            } = node
                                            else {
                                                return None;
                                            };
                                            if !children
                                                .iter()
                                                .any(|child| operator_is(Some(child), ":"))
                                            {
                                                return None;
                                            }
                                            // `Some(value:)` — the reader stored the
                                            // colon as the group's close. The `)` is
                                            // the next `)` in the source, and that is
                                            // the token upstream reports.
                                            let at = group.start as usize;
                                            if let Some(rel) =
                                                source.get(at..).and_then(|rest| rest.find(')'))
                                            {
                                                let start = u32_at(at + rel);
                                                return Some(Span {
                                                    start,
                                                    end: start + 1,
                                                });
                                            }
                                            missing_colon_in(children)
                                        })
                                    }) {
                                    found
                                } else {
                                    closer_after_colon(source, nodes, saved, inner_span)
                                };
                                let missing_bind = bind_semi(nodes.get(*cursor)).is_some();
                                if inner_span.start as i32 >= *progress || missing_bind {
                                    return Some(inner_span);
                                }
                            }
                            *cursor = saved;
                        }
                        // A separator with nothing after it is the failure,
                        // not the last item the repetition already ate.
                        if let Some(sep) = sep {
                            let mut look = *cursor;
                            skip_comments(nodes, &mut look);
                            if nodes.get(look).is_some_and(|node| node_is_lit(node, sep)) {
                                let after = look + 1;
                                if nodes.get(after).is_none_or(|node| {
                                    node_is_lit(node, sep)
                                        || matches!(node, Syntax::Operator { raw, .. } if raw == ")" || raw == "}" || raw == "]")
                                }) {
                                    let span = failure_token(&nodes[look]);
                                    *progress = (*progress).max(span.start as i32);
                                    return Some(span);
                                }
                            }
                        }
                        break;
                    }
                    if let Some(node) = nodes.get(saved) {
                        *progress = (*progress).max(node_start(node) as i32);
                    }
                    many += 1;
                    if *kind == RepKind::Optional {
                        break;
                    }
                    if let Some(sep) = sep {
                        let saved = *cursor;
                        skip_comments(nodes, cursor);
                        if nodes
                            .get(*cursor)
                            .is_some_and(|node| node_is_lit(node, sep))
                        {
                            *progress = (*progress).max(node_end(&nodes[*cursor]) as i32);
                            *cursor += 1;
                            continue;
                        }
                        *cursor = saved;
                        break;
                    }
                }
            }
        }
    }
    None
}

/// A hole that could not take a token does not hide a later failure.
/// `function(...) { stmts }` with no final expression still has the body's
/// closer as the token the rule was expecting.
fn later_failure(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    pattern: &[Pat],
    classes: &[SyntaxClass],
    progress: &mut i32,
    expect: &mut Option<String>,
) -> Option<Span> {
    if pattern.is_empty() {
        return None;
    }
    let mut probe = cursor;
    skip_comments(nodes, &mut probe);
    // The missing hole may have been the closer itself. Try the rest of the
    // pattern at this token and, if that does not move, at the next one.
    let mut first = probe;
    let mut nested = nodes.get(probe).map(node_start).unwrap_or(0) as i32;
    if let Some(span) = match_pats_fail(
        source,
        nodes,
        &mut first,
        pattern,
        &mut HashMap::new(),
        classes,
        &mut nested,
        expect,
    ) {
        if span.start as i32 > nodes.get(probe).map(node_start).unwrap_or(0) as i32 {
            *progress = (*progress).max(nested);
            return Some(span);
        }
    }
    if probe + 1 >= nodes.len() {
        return None;
    }
    let mut next = probe + 1;
    let mut nested = nodes.get(next).map(node_start).unwrap_or(0) as i32;
    let span = match_pats_fail(
        source,
        nodes,
        &mut next,
        pattern,
        &mut HashMap::new(),
        classes,
        &mut nested,
        expect,
    )?;
    if (span.start as i32) <= nodes.get(probe).map(node_start).unwrap_or(0) as i32 {
        return None;
    }
    *progress = (*progress).max(nested);
    Some(span)
}

/// `map((value) => value), , filter` — a group failed while matching a
/// call step, and a later separator has no step after it. That separator
/// is the failure token, not the `=>` inside the arrow.
fn dangling_after_group(nodes: &[Syntax], group_at: usize) -> Option<Span> {
    let mut look = group_at;
    while look < nodes.len() {
        if nodes.get(look).is_some_and(|node| node_is_lit(node, ",")) {
            let mut after = look + 1;
            skip_comments(nodes, &mut after);
            if nodes.get(after).is_some_and(|node| node_is_lit(node, ",")) {
                return Some(failure_token(&nodes[after]));
            }
        }
        if let Syntax::Group { children, .. } = &nodes[look] {
            if let Some(span) = dangling_after_group(children, 0) {
                return Some(span);
            }
        }
        look += 1;
    }
    None
}

/// A group the pattern already finished, whose leftover starts at a comma.
/// Upstream points at the closer when the comma is the last child
/// (`Some(value:);`, `assert();`) and at the comma when another child follows
/// (`{ 1 => "one", }`, `(-> ..., , filter())`).
fn trailing_separator(children: &[Syntax], child: usize, close: Option<Span>) -> Option<Span> {
    let node = children.get(child)?;
    if !matches!(node, Syntax::Operator { raw, .. } if raw == ",") {
        return None;
    }
    let mut after = child + 1;
    skip_comments(children, &mut after);
    if after < children.len() {
        return Some(failure_token(node));
    }
    close.or_else(|| Some(node_span(node)))
}

/// The token a diagnostic names. A group span starts at `(`, which is not
/// the closer upstream reports for `empty():` or `assert()`.
fn failure_token(node: &Syntax) -> Span {
    match node {
        Syntax::Group {
            close: Some(close), ..
        } => *close,
        other => node_span(other),
    }
}

/// The macro name written before a failure that is inside its form.
/// `record Cons(head:, tail)` — the comma belongs to `record`.
fn nested_macro_name(source: &str, span: Span) -> Option<String> {
    let before = source.get(..span.start as usize)?;
    let start = before.rfind('\n')?;
    let line = before[start + 1..].trim_start();
    let name = line
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .next()?;
    if name.is_empty() || !is_ident_text(name) {
        return None;
    }
    Some(name.to_owned())
}

fn node_slice_from(nodes: &[Syntax], cursor: usize) -> &[Syntax] {
    if cursor >= nodes.len() {
        &[]
    } else {
        &nodes[cursor..]
    }
}

/// A position hole (`item`, `jsxChild`) whose form is a nested macro.
/// `record Cons(head:, tail)` fails at the comma after the missing type.
/// `{each ...}` followed by a line fails at the start of that line.
fn nested_form_failure(source: &str, nodes: &[Syntax], cursor: usize) -> Option<Span> {
    let node = nodes.get(cursor)?;
    if let Syntax::Token { raw, .. } = node {
        if raw == "record" {
            let at = node_start(node);
            let rest = source.get(at..)?;
            let colon = rest.find(':')?;
            let after = &rest[colon + 1..];
            let rel = after.find(|ch: char| ch == ',' || ch == ')')?;
            let start = u32_at(at + colon + 1 + rel);
            return Some(Span {
                start,
                end: start + 1,
            });
        }
    }
    if let Syntax::Group { children, span, .. } = node {
        if children.first().and_then(macro_head) == Some("each") {
            let at = span.end as usize;
            let rest = source.get(at..)?;
            let rel = rest.find(|ch: char| !ch.is_whitespace())?;
            let start = u32_at(at + rel);
            return Some(Span { start, end: start });
        }
    }
    None
}

/// `{ 1 => "one", }` stopped on `1` because `_` was expected. The `}`
/// after the trailing comma is the failure token.
fn match_closer(source: &str, node: &Syntax) -> Option<Span> {
    let at = node_start(node);
    let rest = source.get(at..)?;
    let comma = rest.find(',')?;
    let after = &rest[comma + 1..];
    let rel = after.find(|ch: char| ch == '}' || !ch.is_whitespace())?;
    if after.as_bytes().get(rel) != Some(&b'}') {
        return None;
    }
    let start = u32_at(at + comma + 1 + rel);
    Some(Span {
        start,
        end: start + 1,
    })
}

/// The first non-whitespace of the line after `at`.
fn each_line_after(source: &str, at: usize) -> Option<Span> {
    let rest = source.get(at..)?;
    let nl = rest.find('\n')?;
    let line_start = at + nl + 1;
    let mut end = line_start;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| *byte != b'\n')
    {
        end += 1;
    }
    // The missing brace is the end of that line. A span on `<` is the
    // element, not the failure.
    let start = u32_at(end);
    Some(Span { start, end: start })
}

/// `{each (...)}` wanted a braced body on the next line. Upstream
/// points at the start of that line, not at `each`.
fn each_body_line(source: &str, nodes: &[Syntax], index: usize) -> Option<Span> {
    let group = nodes.iter().skip(index).find(|node| {
        matches!(node, Syntax::Group { children, .. } if children.first().and_then(macro_head) == Some("each"))
    })?;
    let at = node_end(group);
    let rest = source.get(at..)?;
    let nl = rest.find('\n')?;
    let start = u32_at(at + nl + 1);
    Some(Span { start, end: start })
}

/// `assert()` — the call has no argument. The `)` is the failure token.
fn empty_call_paren(source: &str, node: &Syntax) -> Option<Span> {
    let at = node_end(node);
    let rest = source.get(at..)?;
    if !rest.starts_with("()") {
        return None;
    }
    let start = u32_at(at + 1);
    Some(Span {
        start,
        end: start + 1,
    })
}

/// `lower(1)` where the rule wanted `upper(`. The `(` of the missing
/// segment is the next one in the source, not the segment name that was written.
fn segment_paren(source: &str, node: &Syntax) -> Option<Span> {
    let at = node_end(node);
    let rel = source.get(at..)?.find('(')?;
    let start = u32_at(at + rel);
    Some(Span {
        start,
        end: start + 1,
    })
}

/// `value <-;` inside the group the repetition stopped on. The semicolon
/// is the missing expression.
fn bind_semi(node: Option<&Syntax>) -> Option<Span> {
    let Syntax::Group { children, .. } = node? else {
        return None;
    };
    if !children
        .iter()
        .any(|child| operator_spelling(child).is_some_and(|raw| raw == "<-"))
    {
        return None;
    }
    let arrow = children
        .iter()
        .position(|child| operator_spelling(child).is_some_and(|raw| raw == "<-"))?;
    children
        .get(arrow + 1)
        .filter(|child| operator_is(Some(child), ";"))
        .map(failure_token)
}

/// An expression hole that did not take an expression. `name <-;` reports
/// the semicolon. A binder with no operator is itself.
fn expr_failure_token(nodes: &[Syntax], cursor: usize) -> Option<Span> {
    let mut look = cursor;
    skip_comments(nodes, &mut look);
    if nodes
        .get(look)
        .and_then(|node| match node {
            Syntax::Token { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .is_some_and(is_ident_text)
    {
        let mut op = look + 1;
        skip_comments(nodes, &mut op);
        if nodes
            .get(op)
            .and_then(operator_spelling)
            .is_some_and(|raw| raw == "<-" || raw == "->")
        {
            let after = op + 1;
            return nodes.get(after).map(failure_token);
        }
    }
    nodes.get(look).map(failure_token)
}

/// A type hole that did not take a type. `value:` reports `)`, and
/// `head:, tail` reports the comma. A name with no colon is itself.
fn type_failure_token(nodes: &[Syntax], cursor: usize) -> Option<Span> {
    let mut look = cursor;
    skip_comments(nodes, &mut look);
    if nodes
        .get(look)
        .is_some_and(|node| operator_is(Some(node), ":"))
    {
        look += 1;
        skip_comments(nodes, &mut look);
        return nodes.get(look).map(failure_token);
    }
    if nodes
        .get(look)
        .and_then(|node| match node {
            Syntax::Token { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .is_some_and(is_ident_text)
        && nodes
            .get(look + 1)
            .is_some_and(|node| operator_is(Some(node), ":"))
    {
        let after = look + 2;
        return nodes.get(after).map(failure_token);
    }
    nodes.get(look).map(failure_token)
}

/// `name:` with the type missing. The match consumed the closer, so the
/// class did not really accept the form. The closer is the failure token.
fn missing_type_closer(source: &str, nodes: &[Syntax], from: usize, to: usize) -> Option<Span> {
    if to <= from || to > nodes.len() {
        return None;
    }
    let text = source.get(node_start(&nodes[from])..node_end(&nodes[to - 1]))?;
    let mut depth = 0i32;
    let mut saw_colon = false;
    for ch in text.chars() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                if saw_colon && depth == 1 {
                    return nodes.get(to - 1).map(failure_token);
                }
                depth -= 1;
            }
            ':' if depth > 0 => saw_colon = true,
            ',' if depth > 0 && saw_colon => return nodes.get(to - 1).map(failure_token),
            ch if ch.is_whitespace() => {}
            _ => saw_colon = false,
        }
    }
    None
}

/// The token a syntax class could not accept. Each rule is tried; the one
/// that gets furthest supplies the span and its `expect`.
fn class_failure(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    class_: &SyntaxClass,
    classes: &[SyntaxClass],
) -> Option<RuleStop> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let mut best: Option<RuleStop> = None;
    let mut any = false;
    for rule in &class_.rules {
        any = true;
        let mut child = index;
        let mut raw = HashMap::new();
        let mut nested = nodes.get(index).map(node_start).unwrap_or(0) as i32;
        let mut expect = None;
        if let Some(mut span) = match_pats_fail(
            source,
            nodes,
            &mut child,
            &rule.pattern,
            &mut raw,
            classes,
            &mut nested,
            &mut expect,
        ) {
            // `value:` failed on the name. The type is missing, so the
            // closer after the colon is the token the rule was expecting.
            // That colon is inside the constructor group, not a sibling.
            let adjusted = expr_failure_token(nodes, child.saturating_sub(1))
                .filter(|found| found.start > span.start)
                .unwrap_or_else(|| closer_after_colon(source, nodes, index, span));
            if adjusted.start != span.start {
                nested = adjusted.end as i32;
                span = adjusted;
            } else if let Some(inside) =
                nodes
                    .get(child.saturating_sub(1))
                    .and_then(|node| match node {
                        Syntax::Group { children, .. } => closer_in(children, span),
                        _ => None,
                    })
            {
                nested = inside.end as i32;
                span = inside;
            } else {
                nested = nested.max(span.start as i32);
            }
            let expect = expect.or_else(|| rule.expect.clone());
            let better = best.as_ref().is_none_or(|have| nested > have.progress);
            if better {
                best = Some(RuleStop {
                    span,
                    expect,
                    progress: nested,
                });
            }
        } else if match_pats(source, nodes, &mut child, &rule.pattern, &mut raw, classes).is_some()
        {
            // `Some(value:)` matches only because the type hole ate `)`. A
            // colon with no type is still the class failure. The closer is
            // the token upstream names.
            if let Some(span) = missing_type_closer(source, nodes, index, child) {
                let progress = span.start as i32;
                let better = best.as_ref().is_none_or(|have| progress > have.progress);
                if better {
                    best = Some(RuleStop {
                        span,
                        expect: rule.expect.clone(),
                        progress,
                    });
                }
                continue;
            }
            return None;
        }
    }
    if any {
        best.or_else(|| {
            nodes.get(index).map(|node| RuleStop {
                span: failure_token(node),
                expect: None,
                progress: node_start(node) as i32,
            })
        })
    } else {
        nodes.get(index).map(|node| RuleStop {
            span: failure_token(node),
            expect: None,
            progress: node_start(node) as i32,
        })
    }
}

fn match_rule_with(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    pattern: &[Pat],
    classes: &[SyntaxClass],
    refines: &[Refine],
) -> Option<Match> {
    let mut captures = HashMap::new();
    let mut cursor = index;
    // A rule may write `export function` while the call site is already at
    // `function`. The skipped words have to be the modifiers that were
    // actually written. `async function` does not match `function`.
    if !prefix_was_written(source, nodes, index, pattern) {
        return None;
    }
    let pattern = skip_matched_prefix(nodes, index, pattern);
    // `(-> value, call(...), ...)` is one group in the macro source. The
    // reader already opened that group, so the call site is the operator
    // and its siblings, not another paren around them.
    let unwrapped = unwrap_call_group(nodes, index, pattern).to_vec();
    let owned = comma_separated_rest(&unwrapped);
    let end = match_pats(source, nodes, &mut cursor, &owned, &mut captures, classes)?;
    if !refine_accepts(refines, &captures) {
        return None;
    }
    Some(Match {
        next: cursor,
        end,
        captures,
    })
}

/// `$($rest:tt)+` was written with the comma inside the repetition.
/// The call site separates steps with a comma, so the repetition's
/// separator is that comma and the hole is one step.
fn comma_separated_rest(pattern: &[Pat]) -> Vec<Pat> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < pattern.len() {
        let pat = &pattern[index];
        if matches!(pat, Pat::Lit(lit) if lit == ",")
            && matches!(pattern.get(index + 1), Some(Pat::Rep { inner, sep: None, .. }) if inner.iter().any(|part| matches!(part, Pat::Hole { kind: HoleKind::TokenTree, .. })))
        {
            if let Pat::Rep { inner, kind, .. } = &pattern[index + 1] {
                out.push(Pat::Rep {
                    inner: comma_separated_rest(inner),
                    sep: Some(",".to_owned()),
                    kind: *kind,
                });
                index += 2;
                continue;
            }
        }
        out.push(match pat {
            Pat::Group { delimiter, inner } => Pat::Group {
                delimiter: *delimiter,
                inner: comma_separated_rest(inner),
            },
            Pat::Rep {
                inner,
                sep: None,
                kind,
            } => {
                let inner = comma_separated_rest(inner);
                let hole = inner.iter().any(|part| {
                    matches!(
                        part,
                        Pat::Hole {
                            kind: HoleKind::TokenTree,
                            ..
                        }
                    )
                });
                let comma = inner
                    .iter()
                    .any(|part| matches!(part, Pat::Lit(lit) if lit == ","));
                if hole && comma {
                    Pat::Rep {
                        inner: inner
                            .into_iter()
                            .filter(|part| !matches!(part, Pat::Lit(lit) if lit == ","))
                            .collect(),
                        sep: Some(",".to_owned()),
                        kind: *kind,
                    }
                } else {
                    Pat::Rep {
                        inner,
                        sep: None,
                        kind: *kind,
                    }
                }
            }
            other => other.clone(),
        });
        index += 1;
    }
    out
}

/// `filter(map(...), (value) => value > 2)` is the expansion. The
/// reference also has a trailing comma on that call, and on a call whose
/// last argument is itself a call. An inner `map(...)` does not.
fn comma_before_close(chars: &[char], close: usize) -> bool {
    let mut at = close;
    while at > 0 && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    at > 0 && chars[at - 1] == ','
}

fn trailing_thread_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut insert = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '(' {
            if let Some(close) = matching_thread_paren(&chars, index) {
                let last = last_arg_end(&chars, index + 1, close);
                let outermost = close + 1 == chars.len()
                    || chars[close + 1..]
                        .iter()
                        .all(|ch| ch.is_whitespace() || *ch == ';');
                let already = comma_before_close(&chars, close);
                // A nested call (`vector(...)`) does not take the comma.
                // The outer call does, including when its last argument is
                // a number (`3`) or another call.
                if outermost && !already && last.is_some() {
                    insert.push(close);
                }
            }
        }
        index += 1;
    }
    let mut out = String::new();
    for (at, ch) in chars.iter().enumerate() {
        if insert.contains(&at) {
            out.push(',');
        }
        out.push(*ch);
    }
    out
}

fn matching_thread_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (at, ch) in chars.iter().enumerate().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

fn last_arg_end(chars: &[char], start: usize, close: usize) -> Option<usize> {
    let mut at = close;
    while at > start && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    if at == start {
        return None;
    }
    Some(at - 1)
}

/// A comma whose next token is also a comma. One trailing comma after
/// the last call is the closer, not a missing step.
fn empty_thread_step(children: &[Syntax]) -> Option<Span> {
    let mut cursor = 0;
    while cursor < children.len() {
        if operator_is(children.get(cursor), ",") {
            let mut next = cursor + 1;
            while matches!(children.get(next), Some(Syntax::Comment { .. })) {
                next += 1;
            }
            if next < children.len() && operator_is(children.get(next), ",") {
                return Some(node_span(&children[next]));
            }
        }
        cursor += 1;
    }
    None
}

fn grouped_prefix(nodes: &[Syntax], index: usize) -> Option<String> {
    let Syntax::Group {
        children,
        delimiter: Delimiter::Paren,
        ..
    } = nodes.get(index)?
    else {
        return None;
    };
    let first = children.first().and_then(operator_spelling)?;
    if first == "->" && children.get(1).and_then(operator_spelling) == Some(">") {
        let adjacent = node_end(&children[0]) == node_start(&children[1]);
        if adjacent {
            return Some("->>".to_owned());
        }
    }
    if first == "->" || first == "->>" || first == "<-" {
        return Some(first.to_owned());
    }
    None
}

fn unwrap_call_group<'a>(nodes: &[Syntax], index: usize, pattern: &'a [Pat]) -> &'a [Pat] {
    let [
        Pat::Group {
            delimiter: Delimiter::Paren,
            inner,
        },
    ] = pattern
    else {
        return pattern;
    };
    let head = match nodes.get(index) {
        Some(Syntax::Token { raw, .. } | Syntax::Operator { raw, .. }) => raw.as_str(),
        _ => return pattern,
    };
    let starts = inner
        .first()
        .is_some_and(|pat| matches!(pat, Pat::Lit(lit) if lit == head));
    if starts { inner.as_slice() } else { pattern }
}

fn skip_matched_prefix<'a>(nodes: &[Syntax], index: usize, pattern: &'a [Pat]) -> &'a [Pat] {
    let Some(head) = nodes.get(index).and_then(|node| match node {
        Syntax::Token { raw, .. } => Some(raw.as_str()),
        _ => None,
    }) else {
        return pattern;
    };
    // The matcher VM compares each literal with the token under the cursor.
    // It does not search ahead for a later copy of the same spelling, and it
    // does not treat an arbitrary leading literal as optional. A declaration
    // modifier (`export`, `declare`, `default`) may already have been printed
    // when matching resumes at the macro name. `async` is not one of those:
    // `async function` does not match a rule written `function`.
    let Some(at) = pattern.iter().position(|pat| match pat {
        Pat::Lit(lit) if lit == head => true,
        Pat::Lit(lit) if declaration_modifier(lit) => false,
        _ => return false,
    }) else {
        return pattern;
    };
    if at == 0
        || !pattern[..at]
            .iter()
            .all(|pat| matches!(pat, Pat::Lit(lit) if declaration_modifier(lit)))
    {
        return pattern;
    }
    &pattern[at..]
}

fn declaration_modifier(word: &str) -> bool {
    matches!(word, "export" | "declare" | "default")
}

/// A rule matches from its first literal. A declaration modifier that the
/// call site already printed may stand in front. Any other token there is
/// part of the form: `async function` does not match a rule written
/// `function`. A group boundary (the `}` that closed the macro) is not a
/// token of the form.
fn prefix_was_written(source: &str, nodes: &[Syntax], index: usize, pattern: &[Pat]) -> bool {
    let Some(head) = nodes.get(index).and_then(|node| match node {
        Syntax::Token { raw, .. } => Some(raw.as_str()),
        _ => None,
    }) else {
        return true;
    };
    let starts = pattern.first().is_some_and(
        |pat| matches!(pat, Pat::Lit(lit) if lit == head || declaration_modifier(lit)),
    );
    if !starts || index == 0 {
        return true;
    }
    // A previous member (`parse` above `overloaded`) is not a prefix. A word
    // on the same line is: `async function` does not match `function`.
    let Syntax::Token { raw, span, .. } = &nodes[index - 1] else {
        return true;
    };
    let between = source
        .get(span.end as usize..node_start(&nodes[index]))
        .unwrap_or("");
    if between.contains('\n') {
        return true;
    }
    // A keyword that takes an expression (`return id(6)`, `yield m(x)`)
    // leaves an operand position; the invocation starts here.
    if matches!(
        raw.as_str(),
        "return"
            | "yield"
            | "await"
            | "typeof"
            | "void"
            | "delete"
            | "throw"
            | "case"
            | "in"
            | "of"
            | "else"
            | "do"
            | "extends"
    ) {
        return true;
    }
    declaration_modifier(raw)
}

/// Words that open the class hole after this repetition. `where` opens
/// the filter that follows `join ... on $condition`, so the condition's
/// last expression stops on `where` instead of consuming it.
fn following_clause_words(
    pattern: &[Pat],
    pat_index: usize,
    classes: &[SyntaxClass],
) -> Vec<String> {
    let mut inner = match pattern.get(pat_index + 1) {
        Some(Pat::Rep {
            inner, sep: None, ..
        }) => inner.as_slice(),
        _ => return Vec::new(),
    };
    // `$($filter:Filter)?` is itself a repetition. The word is the class
    // inside it (`where`), not the repetition.
    while let [
        Pat::Rep {
            inner: nested,
            sep: None,
            ..
        },
    ] = inner
    {
        inner = nested.as_slice();
    }
    let Some(Pat::Hole {
        kind: HoleKind::Class(name),
        ..
    }) = inner.first()
    else {
        return Vec::new();
    };
    let Some(class_) = classes.iter().find(|class_| class_.name == *name) else {
        return Vec::new();
    };
    let mut words = Vec::new();
    for rule in &class_.rules {
        if let Some(Pat::Lit(word)) = rule.pattern.first() {
            if !words.contains(word) {
                words.push(word.clone());
            }
        }
    }
    words
}

fn match_pats(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
) -> Option<usize> {
    match_pats_in(source, nodes, cursor, pattern, captures, classes, false)
}

/// `in_rep` is true while matching the inner pattern of a `Pat::Rep`. A
/// `$($rest:tt)+` step spans a call (`map(...)`); a standalone `$marker:token`
/// is exactly one token, so it must not swallow the `/` after it.
fn match_pats_in(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
    in_rep: bool,
) -> Option<usize> {
    match_pats_stop(
        source,
        nodes,
        cursor,
        pattern,
        captures,
        classes,
        in_rep,
        &[],
    )
}

fn match_pats_stop(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
    in_rep: bool,
    // Words that end this pattern even when it does not name them. A
    // repetition step stops before `where`, which belongs to the next clause.
    rep_stop: &[String],
) -> Option<usize> {
    match_pats_cont(
        source,
        nodes,
        cursor,
        pattern,
        captures,
        classes,
        in_rep,
        rep_stop,
        &Cont::Free,
    )
}

/// What must still match after the pattern being matched, so a choice can
/// backtrack: upstream's choice tries each alternative against the whole
/// rest of the pattern, not only against itself.
enum Cont<'a> {
    /// Nothing is required (a rule's own tail: the invocation may stop).
    Free,
    /// The node list must be used up (the inside of a group).
    End,
    /// `pats`, then `then`.
    Seq(&'a [Pat], &'a Cont<'a>),
    /// Inside a repetition step: the separator or another step, or `then`.
    Rep {
        inner: &'a [Pat],
        sep: Option<&'a str>,
        then: &'a Cont<'a>,
    },
}

/// Whether `cont` can match at `cursor`. Matching commits inside the
/// continuation itself, but every choice in it backtracks the same way.
fn cont_ok(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
    cont: &Cont<'_>,
) -> bool {
    match cont {
        Cont::Free => true,
        Cont::End => {
            let mut at = cursor;
            skip_comments(nodes, &mut at);
            at >= nodes.len()
        }
        Cont::Seq(pats, then) => {
            let mut at = cursor;
            let mut scratch = captures.clone();
            match_pats_cont(
                source,
                nodes,
                &mut at,
                pats,
                &mut scratch,
                classes,
                false,
                &[],
                then,
            )
            .is_some()
                && cont_ok(source, nodes, at, &scratch, classes, then)
        }
        Cont::Rep { inner, sep, then } => {
            let mut at = cursor;
            skip_comments(nodes, &mut at);
            if let Some(sep) = sep {
                if nodes.get(at).is_some_and(|node| node_is_lit(node, sep)) {
                    return true;
                }
            } else if at > cursor || !inner.is_empty() {
                let mut again = at;
                let mut scratch = captures.clone();
                if match_pats_stop(
                    source,
                    nodes,
                    &mut again,
                    inner,
                    &mut scratch,
                    classes,
                    true,
                    &[],
                )
                .is_some()
                    && again > at
                {
                    return true;
                }
            }
            cont_ok(source, nodes, cursor, captures, classes, then)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn match_pats_cont(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
    in_rep: bool,
    rep_stop: &[String],
    cont: &Cont<'_>,
) -> Option<usize> {
    if let Err(message) = super::sweetener_budget::charge(
        super::sweetener_budget::Kind::MatcherSteps,
        u64::try_from(pattern.len()).unwrap_or(u64::MAX).max(1),
    ) {
        note_budget(message);
        return None;
    }
    let start_end = nodes.get(*cursor).map(node_start).unwrap_or(0);
    let mut last_end = start_end;
    for (pat_index, pat) in pattern.iter().enumerate() {
        match pat {
            Pat::Choice(alternatives) => {
                let saved = *cursor;
                let mut chosen = false;
                let last = pat_index + 1 == pattern.len();
                let rest = Cont::Seq(&pattern[pat_index + 1..], cont);
                // Full backtracking: the first alternative, in written order,
                // after which the rest of the pattern still matches.
                let backtracked = (!matches!(cont, Cont::Free) || !last)
                    .then(|| {
                        alternatives.iter().find(|alternative| {
                            let mut tried = saved;
                            let mut taken = captures.clone();
                            match_pats_cont(
                                source,
                                nodes,
                                &mut tried,
                                alternative,
                                &mut taken,
                                classes,
                                in_rep,
                                rep_stop,
                                &rest,
                            )
                            .is_some()
                                && cont_ok(source, nodes, tried, &taken, classes, &rest)
                        })
                    })
                    .flatten();
                let order = match backtracked {
                    Some(found) => vec![found],
                    None => choice_order(source, nodes, saved, alternatives, classes, last),
                };
                for alternative in order {
                    let mut tried = saved;
                    let mut taken = captures.clone();
                    if match_pats_cont(
                        source,
                        nodes,
                        &mut tried,
                        alternative,
                        &mut taken,
                        classes,
                        in_rep,
                        rep_stop,
                        &rest,
                    )
                    .is_some()
                    {
                        *cursor = tried;
                        *captures = taken;
                        if tried > saved {
                            last_end = node_end(&nodes[tried - 1]);
                        }
                        chosen = true;
                        break;
                    }
                }
                if !chosen {
                    return None;
                }
            }
            Pat::Lit(expected) => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                if let Some(width) = lit_width(nodes, *cursor, expected) {
                    last_end = node_end(&nodes[*cursor + width - 1]);
                    *cursor += width;
                } else if node_is_lit(node, expected) {
                    last_end = node_end(node);
                    *cursor += 1;
                } else {
                    return None;
                }
            }
            Pat::Group { delimiter, inner } => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                let Syntax::Group {
                    delimiter: found,
                    children,
                    ..
                } = node
                else {
                    return None;
                };
                if found != delimiter {
                    return None;
                }
                let mut child = 0;
                if match_pats_cont(
                    source,
                    children,
                    &mut child,
                    inner,
                    captures,
                    classes,
                    false,
                    &[],
                    &Cont::End,
                )
                .is_none()
                {
                    return None;
                }
                // `record Name()` is an empty group. It must not accept
                // `record Name(head: number)`, whose fields are a later rule.
                skip_comments(children, &mut child);
                if child < children.len() {
                    return None;
                }
                // A single hole or `$($body:stmt)*` owns the group's interior,
                // including operators the flat walker stopped at.
                let whole = match inner.as_slice() {
                    [Pat::Hole { name, .. }] => Some(name.clone()),
                    [
                        Pat::Rep {
                            inner: rep_inner,
                            sep: None,
                            ..
                        },
                    ] => match rep_inner.as_slice() {
                        [
                            Pat::Hole {
                                name,
                                kind: HoleKind::Stmt,
                            },
                        ] => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = whole {
                    let already = captures.get(&name).is_some_and(|have| {
                        matches!(have, Capture::Many(_) | Capture::Record { .. })
                    });
                    if !already {
                        let text = sweetener_reader::slice_range(
                            source,
                            node_start(node) + 1,
                            node_end(node).saturating_sub(1),
                        )
                        .to_owned();
                        captures.insert(name, Capture::Text(text));
                    }
                }
                last_end = node_end(node);
                *cursor += 1;
            }
            Pat::Hole { name, kind } => {
                if let HoleKind::Class(class_name) = kind {
                    let class_ = classes.iter().find(|class_| class_.name == *class_name)?;
                    let (capture, end, next) = match take_class(
                        source, nodes, *cursor, class_, classes, false, rep_stop,
                    ) {
                        Some(found) => found,
                        None => return None,
                    };
                    captures.insert(name.clone(), capture);
                    last_end = end;
                    *cursor = next;
                } else {
                    // A literal right after an expression hole is that
                    // hole's stop. `$left:expr === $right:expr` has the
                    // reader's split `==` then `=`, so the hole must stop
                    // at `==`. With no following literal the hole is
                    // greedy, as in `unless($condition:expr) { ... }`.
                    let stop = match pattern.get(pat_index + 1) {
                        Some(Pat::Lit(text)) => Some(text.as_str()),
                        _ => None,
                    };
                    let taken = if matches!(kind, HoleKind::Ident | HoleKind::Binding)
                        && stop.is_some_and(|word| {
                            nodes
                                .get(*cursor)
                                .is_some_and(|node| node_is_lit(node, word))
                        }) {
                        // `from $table:ident $($joins)` and the next token is
                        // `join`. The ident is absent, not the word `join`.
                        None
                    } else if *kind == HoleKind::Expr {
                        // `$left:expr == $right` stops before `==`. The
                        // operator is the next pattern piece, not part of
                        // the operand. `take_expr_until` already stops on a
                        // word; this covers an operator spelling. A word
                        // the repetition stops on (`where`) is the same.
                        let word = stop
                            .filter(|word| !word.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch)));
                        let stop = stop
                            .filter(|word| word.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch)))
                            .or(word);
                        let clause = rep_stop.iter().find(|word| {
                            nodes
                                .get(*cursor..)
                                .into_iter()
                                .flatten()
                                .take(8)
                                .any(|node| node_is_lit(node, word))
                        });
                        take_expr_until(source, nodes, *cursor, stop.or(clause.map(String::as_str)))
                    } else {
                        take_hole(source, nodes, *cursor, kind, in_rep)
                    };
                    let (text, end, next) = taken?;
                    captures.insert(name.clone(), Capture::Text(text));
                    last_end = end;
                    *cursor = next;
                }
            }
            Pat::Rep { inner, sep, kind } => {
                // `condition($($terms) or +)` is written without a paren, and
                // the call has one: `condition(a or b)`. Match the repetition
                // against the arguments. A token-tree repetition is a list of
                // groups, so it still matches the group itself.
                let class_rep = inner.iter().any(|part| {
                    matches!(
                        part,
                        Pat::Hole {
                            kind: HoleKind::Class(_),
                            ..
                        }
                    )
                });
                if class_rep {
                    if let Some(Syntax::Group {
                        delimiter: Delimiter::Paren,
                        children,
                        ..
                    }) = nodes.get(*cursor)
                    {
                        let mut sub = 0;
                        let wrapped = [pat.clone()];
                        if match_pats_stop(
                            source, children, &mut sub, &wrapped, captures, classes, in_rep,
                            rep_stop,
                        )
                        .is_some()
                        {
                            last_end = node_end(nodes.get(*cursor).unwrap());
                            *cursor += 1;
                            continue;
                        }
                    }
                }
                let mut many = Vec::new();
                let saved_start = *cursor;
                let mut row_end: Option<usize> = None;
                let mut row_starts: Vec<usize> = Vec::new();
                let mut resume_next = saved_start;
                // A nested `$($parameter ...),*` reuses the head's hole names.
                // Matching into the outer map overwrites `parameter`, so the
                // last segment is the only one left. Each iteration matches
                // on a copy and only its own holes are kept.
                loop {
                    let saved = *cursor;
                    let mut one = captures.clone();
                    let before: Vec<String> = one.keys().cloned().collect();
                    let follows = match pattern.get(pat_index + 1) {
                        Some(Pat::Lit(text))
                            if !text.chars().all(|ch| "!=<>&|?+-*/%^~.,;()".contains(ch)) =>
                        {
                            Some(text.as_str())
                        }
                        _ => None,
                    };
                    // `where` opens the class after this repetition. The last
                    // expression of `$($terms) and +` must stop on that word,
                    // not run into the next clause. `and` only separates steps.
                    let mut step_stop = following_clause_words(pattern, pat_index, classes);
                    for word in std::iter::once(follows)
                        .chain(rep_stop.iter().map(String::as_str).map(Some))
                        .chain(std::iter::once(sep.as_deref()))
                        .flatten()
                    {
                        if !step_stop.iter().any(|have| have == word) {
                            step_stop.push(word.to_owned());
                        }
                    }
                    let after_rep = Cont::Seq(&pattern[pat_index + 1..], cont);
                    let step = Cont::Rep {
                        inner,
                        sep: sep.as_deref(),
                        then: &after_rep,
                    };
                    if match_pats_cont(
                        source, nodes, cursor, inner, &mut one, classes, true, &step_stop, &step,
                    )
                    .is_none()
                    {
                        // A separator with no step after it belongs to what
                        // follows the repetition (`$($arm),+ , _ => ...`).
                        // A trailing separator the rest of the pattern does
                        // not need stays consumed.
                        *cursor = if sep.is_some()
                            && !many.is_empty()
                            && !cont_ok(source, nodes, saved, captures, classes, &after_rep)
                        {
                            resume_next
                        } else {
                            saved
                        };
                        break;
                    }
                    for key in before {
                        one.remove(&key);
                    }
                    let mut one: Vec<(String, Capture)> = one.into_iter().collect();
                    // A separator that was written inside the repetition
                    // (`$($rest:tt,)+`) is not part of the captured step.
                    if sep.is_some() {
                        for (_, value) in &mut one {
                            if let Capture::Text(text) = value {
                                *text = text.trim().trim_end_matches(',').trim().to_owned();
                            }
                        }
                    }
                    if sep.is_none() {
                        // The source between two separator-less steps
                        // (`map(f), filter(g)` or `return x` `;`) is how the
                        // template joins them again.
                        let mut first = saved;
                        skip_comments(nodes, &mut first);
                        if let (Some(prev), Some(node)) = (row_end, nodes.get(first)) {
                            if *cursor > first {
                                one.push((
                                    REP_GAP.to_owned(),
                                    Capture::Text(
                                        sweetener_reader::slice_range(
                                            source,
                                            prev,
                                            node_start(node),
                                        )
                                        .to_owned(),
                                    ),
                                ));
                            }
                        }
                        if *cursor > first {
                            row_end = Some(node_end(&nodes[*cursor - 1]));
                        }
                    }
                    row_starts.push(if sep.is_some() { resume_next } else { saved });
                    many.push(one);
                    if *kind == RepKind::Optional {
                        break;
                    }
                    if let Some(sep) = sep {
                        let saved = *cursor;
                        skip_comments(nodes, cursor);
                        if nodes
                            .get(*cursor)
                            .is_some_and(|node| node_is_lit(node, sep))
                        {
                            resume_next = saved;
                            *cursor += 1;
                            continue;
                        }
                        *cursor = saved;
                        break;
                    }
                }
                // A greedy repetition gives steps back when
                // the rest of the pattern cannot match after it, as upstream's
                // backtracking matcher does: `$($statements:stmt)*
                // $result:expr` leaves the last expression to `$result`.
                if !(pat_index + 1 == pattern.len() && matches!(cont, Cont::Free)) {
                    let rest = Cont::Seq(&pattern[pat_index + 1..], cont);
                    while !many.is_empty()
                        && !cont_ok(source, nodes, *cursor, captures, classes, &rest)
                    {
                        many.pop();
                        *cursor = row_starts.pop().unwrap_or(saved_start);
                    }
                }
                if *kind == RepKind::OneOrMore && many.is_empty() {
                    return None;
                }
                // A repetition that matched nothing still binds its holes, to
                // an empty sequence (upstream). `refine $binders ...` on
                // `None()` is then vacuously true, not a missing capture.
                if many.is_empty() {
                    let mut holes = Vec::new();
                    pattern_holes(inner, &mut holes);
                    for (name, _) in holes {
                        captures
                            .entry(name)
                            .or_insert_with(|| Capture::Many(Vec::new()));
                    }
                }
                // A repetition's captures are stored under each inner name.
                let mut grouped: HashMap<String, Vec<Vec<(String, Capture)>>> = HashMap::new();
                for one in &many {
                    // Every hole in this iteration stays on the row. Walking
                    // `$initial` must still see the `$value` captured beside it.
                    let full = one.clone();
                    for (name, _) in one {
                        grouped.entry(name.clone()).or_default().push(full.clone());
                    }
                }
                // `$($body:stmt)*` with no separator is the whole interior,
                // including the newlines between the braces. A syntax-class
                // repetition keeps the records; the blob would hide `name`
                // and `moves`.
                if sep.is_none() {
                    if let Some(name) = many
                        .iter()
                        .find_map(|one| one.first().map(|(name, _)| name.clone()))
                    {
                        let class_rows = many
                            .iter()
                            .all(|one| matches!(one.first(), Some((_, Capture::Record { .. }))));
                        // `$($body:item)*` is several items. One blob hides
                        // `record` from the next expansion. Keep each item.
                        let item_rows = many.iter().all(|one| {
                            matches!(one.first(), Some((_, Capture::Text(text))) if !text.contains('\n') || text.trim_end().ends_with(';'))
                        }) && many.len() > 1;
                        if !class_rows && !item_rows {
                            if let (Some(start_node), Some(end_node)) = (
                                nodes.get(saved_start),
                                cursor.checked_sub(1).and_then(|at| nodes.get(at)),
                            ) {
                                let text = sweetener_reader::slice_range(
                                    source,
                                    node_start(start_node),
                                    node_end(end_node),
                                );
                                if !grouped.contains_key(&name) {
                                    captures.insert(name, Capture::Text(text.to_owned()));
                                }
                            }
                        }
                    }
                }
                for (name, rows) in grouped {
                    if captures.contains_key(&name) {
                        continue;
                    }
                    // Each row is one class match. Key it by the field the
                    // repetition named (`state`, `moves`) so `$state.name`
                    // walks that record instead of the first field text.
                    // A record's own `name` field is not that path: the
                    // template writes `$field.name`, and the row is stored
                    // under `field`.
                    let rows: Vec<Vec<(String, Capture)>> = rows
                        .into_iter()
                        .map(|one| {
                            let mut row = if one.len() == 1 {
                                let (key, value) = one.into_iter().next().unwrap();
                                let mut row = vec![(name.clone(), value.clone())];
                                // The one entry is the class record. A repeated
                                // field lives inside it (`modifiers`), and the
                                // template writes `$column.modifiers`, so copy
                                // those fields onto the row.
                                if let Capture::Record { fields, .. } = &value {
                                    for (field, child) in fields {
                                        if matches!(child, Capture::Many(_)) {
                                            row.push((format!("{name}.{field}"), child.clone()));
                                        }
                                    }
                                }
                                if key != name {
                                    row[0].0 = name.clone();
                                }
                                let _ = key;
                                row
                            } else {
                                one
                            };
                            // `$field.name` is not the row key. The class
                            // record is stored as `field`; copy its fields
                            // onto that path so the repetition can see them.
                            let mut extras: Vec<(String, Capture)> = row
                                .iter()
                                .filter_map(|(key, value)| match value {
                                    Capture::Record { fields, .. } if key == &name => {
                                        Some(fields.clone())
                                    }
                                    _ => None,
                                })
                                .flatten()
                                .map(|(field, value)| (format!("{name}.{field}"), value))
                                .collect();
                            // `$column.modifiers` is a sibling of the record,
                            // not one of its fields. Copy it onto the record
                            // so `$column.modifiers.primary` walks one path.
                            let siblings: Vec<(String, Capture)> = row
                                .iter()
                                .filter(|(key, _)| key.starts_with(&format!("{name}.")))
                                .map(|(key, value)| {
                                    (key[name.len() + 1..].to_owned(), value.clone())
                                })
                                .filter(|(field, _)| !field.contains('.'))
                                .collect();
                            if let Some(Capture::Record { fields, .. }) = row
                                .iter_mut()
                                .find_map(|(key, value)| (key == &name).then_some(value))
                            {
                                for (field, value) in &siblings {
                                    if !fields.contains_key(field) {
                                        fields.insert(field.clone(), value.clone());
                                    }
                                }
                            }
                            extras.extend(
                                siblings
                                    .into_iter()
                                    .map(|(field, value)| (format!("{name}.{field}"), value)),
                            );
                            row.extend(extras);
                            row
                        })
                        .collect();
                    captures.insert(name, Capture::Many(rows));
                }
                if let Some(node) = cursor.checked_sub(1).and_then(|at| nodes.get(at)) {
                    last_end = node_end(node);
                }
            }
        }
    }
    Some(last_end)
}

/// A class rule may open with the macro head it is matched under.
/// `function $name(...)` inside `syntax function` starts at `function`,
/// which is already `nodes[index]`. Drop that one literal and match the
/// parameters. A rule that does not repeat the head is unchanged.
fn class_pattern_at<'a>(
    nodes: &[Syntax],
    index: usize,
    pattern: &'a [Pat],
    at_macro_head: bool,
) -> &'a [Pat] {
    if !at_macro_head {
        return pattern;
    }
    let Some(Pat::Lit(head)) = pattern.first() else {
        return pattern;
    };
    // A class matched as `syntax function` already sits on `function`.
    // Its rule repeats that word, so the rest of the rule is the form.
    // A class used as a hole (`$joins:Join`) does not: `join posts on`
    // starts at `join`, and stripping it reads `posts` as the keyword.
    if nodes.get(index).is_some_and(|node| node_is_lit(node, head)) {
        &pattern[1..]
    } else {
        pattern
    }
}

/// Match one syntax-class form. The first rule whose pattern accepts
/// the input wins. Fields the rule did not bind stay absent so
/// `#if(present ...)` can tell them from an empty capture.
fn take_class(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    class_: &SyntaxClass,
    classes: &[SyntaxClass],
    at_macro_head: bool,
    outer_stop: &[String],
) -> Option<(Capture, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start = node_start(nodes.get(index)?);
    for rule in &class_.rules {
        let mut child = index;
        let mut raw = HashMap::new();
        // `syntax function` matches at the keyword. A class rule written
        // `function $name(...)` repeats that head; the keyword is already
        // the token this hole started on, so the rest of the rule is the form.
        let pattern = class_pattern_at(nodes, index, &rule.pattern, at_macro_head);
        let Some(end) = match_pats_stop(
            source, nodes, &mut child, pattern, &mut raw, classes, false, outer_stop,
        ) else {
            continue;
        };
        // `refine $left spelling equals "left"` rejects a match the pattern
        // took, so the next rule of this class can take the same tokens.
        if !refine_accepts(&rule.refines, &raw) {
            continue;
        }
        let text = sweetener_reader::slice_range(source, start, end).to_owned();
        let mut fields = HashMap::new();
        for field in &class_.fields {
            let Some(value) = raw.remove(&field.name) else {
                // A repeated field that matched nothing is an empty
                // sequence, not a missing one: `None()` has no `fields`.
                if field.repeated {
                    fields.insert(field.name.clone(), Capture::Many(Vec::new()));
                }
                continue;
            };
            if field.repeated {
                let rows = match value {
                    Capture::Many(rows) => rows,
                    other => vec![vec![(field.name.clone(), other)]],
                };
                fields.insert(field.name.clone(), Capture::Many(rows));
            } else if let Capture::Many(rows) = value {
                // `ident` and `binding` are one name. A hole that swallowed
                // the rest of the form is not the field.
                if let Some(one) = rows
                    .into_iter()
                    .next()
                    .and_then(|row| row.into_iter().next())
                {
                    fields.insert(field.name.clone(), one.1);
                }
            } else {
                fields.insert(field.name.clone(), value);
            }
        }
        return Some((Capture::Record { text, fields }, end, child));
    }
    None
}

/// The index just after a JSX element that starts at `index`.
/// `<name .../>` and `<name>...</name>` both count, including fragments.
fn jsx_element_end(nodes: &[Syntax], index: usize) -> Option<usize> {
    if !operator_is(nodes.get(index), "<") || operator_is(nodes.get(index + 1), "/") {
        return None;
    }
    let mut cursor = index + 1;
    if operator_is(nodes.get(cursor), ">") {
        return jsx_children_end(nodes, cursor + 1, None);
    }
    let name = jsx_tag_name(nodes, &mut cursor)?;
    let self_closing = jsx_skip_attributes(nodes, &mut cursor)?;
    if self_closing {
        return Some(cursor);
    }
    jsx_children_end(nodes, cursor, Some(name))
}

fn jsx_tag_name(nodes: &[Syntax], cursor: &mut usize) -> Option<String> {
    let mut name = String::new();
    loop {
        match nodes.get(*cursor)? {
            Syntax::Token { raw, .. } if is_ident_text(raw) => {
                name.push_str(raw);
                *cursor += 1;
            }
            Syntax::Operator { raw, .. } if raw == "." && !name.is_empty() => {
                name.push('.');
                *cursor += 1;
            }
            _ => break,
        }
    }
    if name.is_empty() { None } else { Some(name) }
}

/// `Some(true)` when the tag ended at `/>`.
fn jsx_skip_attributes(nodes: &[Syntax], cursor: &mut usize) -> Option<bool> {
    let start = *cursor;
    while *cursor < nodes.len() {
        if operator_is(nodes.get(*cursor), "/") && operator_is(nodes.get(*cursor + 1), ">") {
            *cursor += 2;
            return Some(true);
        }
        if operator_is(nodes.get(*cursor), ">") {
            *cursor += 1;
            return Some(false);
        }
        if operator_is(nodes.get(*cursor), "<") {
            *cursor = jsx_element_end(nodes, *cursor)?;
            continue;
        }
        *cursor += 1;
        if *cursor - start > 400 {
            return None;
        }
    }
    None
}

fn jsx_children_end(nodes: &[Syntax], mut cursor: usize, name: Option<String>) -> Option<usize> {
    let mut stack = vec![name];
    while cursor < nodes.len() && !stack.is_empty() {
        if operator_is(nodes.get(cursor), "<") && operator_is(nodes.get(cursor + 1), "/") {
            cursor += 2;
            if operator_is(nodes.get(cursor), ">") {
                cursor += 1;
                stack.pop();
                continue;
            }
            let close = jsx_tag_name(nodes, &mut cursor)?;
            if !operator_is(nodes.get(cursor), ">") {
                return None;
            }
            cursor += 1;
            match stack.last() {
                Some(Some(open)) if open == &close => {
                    stack.pop();
                }
                Some(None) => return None,
                _ => return None,
            }
            continue;
        }
        if operator_is(nodes.get(cursor), "<") {
            cursor = jsx_element_end(nodes, cursor)?;
            continue;
        }
        cursor += 1;
    }
    if stack.is_empty() { Some(cursor) } else { None }
}

fn take_hole(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    kind: &HoleKind,
    in_rep: bool,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    match kind {
        HoleKind::Class(_) => None,
        HoleKind::Stmt | HoleKind::Item => {
            // An import or export declaration is a module item, not a
            // statement. `import("x")` and `import.meta` are expressions.
            if matches!(kind, HoleKind::Stmt)
                && (token_is(Some(node), "export")
                    || (token_is(Some(node), "import")
                        && !matches!(
                            nodes.get(index + 1),
                            Some(Syntax::Group {
                                delimiter: Delimiter::Paren,
                                ..
                            })
                        )
                        && !operator_is(nodes.get(index + 1), ".")))
            {
                return None;
            }
            // `<li>item</li>` is one JSX child. A later `{else}` or `{end}`
            // is the next pattern token, not part of this element, and a
            // `{name}` inside the element is a child, not the end of it.
            if operator_is(Some(node), "<") {
                if let Some(end_index) = jsx_element_end(nodes, index) {
                    let end = node_end(&nodes[end_index - 1]);
                    return Some((source[node_start(node)..end].to_owned(), end, end_index));
                }
            }
            // `record Empty();` is an item. The name is not a brace, and
            // the semicolon is the closer. A following item in the same
            // module must not be swallowed with it.
            if matches!(node, Syntax::Token { raw, .. } if is_ident_text(raw)) {
                let mut end_index = index;
                let mut depth_groups = 0i32;
                while end_index < nodes.len() {
                    match &nodes[end_index] {
                        Syntax::Group { .. } => depth_groups += 1,
                        Syntax::Operator { raw, .. } if raw == ";" && depth_groups == 0 => {
                            let end = node_end(&nodes[end_index]);
                            return Some((
                                source[node_start(node)..end].to_owned(),
                                end,
                                end_index + 1,
                            ));
                        }
                        _ => {}
                    }
                    end_index += 1;
                    if end_index - index > 40 {
                        break;
                    }
                }
            }
            if is_closed_group(node, Delimiter::Brace) {
                return Some((
                    sweetener_reader::slice(source, node).to_owned(),
                    node_end(node),
                    index + 1,
                ));
            }
            let start = node_start(node);
            let mut end_index = index;
            while end_index < nodes.len() {
                // `return value;` ends at the semicolon. `if (...) { ... }`
                // and `try { ... } catch { ... }` have no semicolon. The
                // brace is the statement; a following `catch` or `finally`
                // is still that statement, not the next body item.
                if operator_is(Some(&nodes[end_index]), ";") {
                    let end = node_end(&nodes[end_index]);
                    return Some((source[start..end].to_owned(), end, end_index + 1));
                }
                // Automatic semicolon insertion, the same question the
                // enforester asks in `asiAllowed`: a line break in front of
                // the next token, a `}` that closes the enclosing statement
                // list, or the end of the file. The statement already read
                // is whole; the following token is the next item.
                if end_index > index && statement_asi(source, nodes, end_index) {
                    let end = node_end(&nodes[end_index - 1]);
                    return Some((source[start..end].to_owned(), end, end_index));
                }
                if is_closed_group(&nodes[end_index], Delimiter::Brace) {
                    let mut after = end_index + 1;
                    skip_comments(nodes, &mut after);
                    if token_is(nodes.get(after), "catch") || token_is(nodes.get(after), "finally")
                    {
                        end_index = after;
                        continue;
                    }
                    let end = node_end(&nodes[end_index]);
                    return Some((source[start..end].to_owned(), end, end_index + 1));
                }
                end_index += 1;
                if end_index - index > 80 {
                    break;
                }
            }
            // The file ended with no semicolon. That is a terminator.
            if end_index > index && end_index >= nodes.len() {
                let end = node_end(&nodes[end_index - 1]);
                return Some((source[start..end].to_owned(), end, end_index));
            }
            None
        }
        HoleKind::Ident => take_ident(nodes, index),
        HoleKind::Binding => {
            if is_closed_group(node, Delimiter::Brace) || is_closed_group(node, Delimiter::Bracket)
            {
                Some((
                    sweetener_reader::slice(source, node).to_owned(),
                    node_end(node),
                    index + 1,
                ))
            } else {
                take_ident(nodes, index)
            }
        }
        HoleKind::Member { type_member } => take_member(source, nodes, index, *type_member),
        HoleKind::Type => take_type(source, nodes, index),
        HoleKind::JsxChild => take_jsx_child(source, nodes, index),
        HoleKind::TokenTree => {
            // `$($rest:tt)+` is the remaining steps of a thread, which start
            // with an identifier (`map(...)`). A repetition step spans a
            // whole call; a standalone `$marker:token` is exactly one token
            // and must not swallow the `/` after it.
            if in_rep {
                take_token_tree(source, nodes, index)
                    .or_else(|| take_ident(nodes, index))
                    .or_else(|| take_semicolon(nodes, index))
            } else {
                take_ident(nodes, index).or_else(|| take_one_token(source, nodes, index))
            }
        }
        HoleKind::Expr => {
            // One primary, then tight postfix, then infix operators whose
            // precedence is below a surrounding custom operator. The simple
            // slice stops at a comma, semicolon, or a closer the pattern owns.
            take_expr(source, nodes, index)
        }
    }
}

/// One class element or type member: through its `;` (or `,` in a type),
/// through the brace body of a method or accessor, or to a line break that
/// ends it by automatic semicolon insertion.
fn take_member(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    type_member: bool,
) -> Option<(String, usize, usize)> {
    let start = node_start(nodes.get(index)?);
    if operator_is(nodes.get(index), ";") || operator_is(nodes.get(index), "}") {
        return None;
    }
    let mut end_index = index;
    while end_index < nodes.len() {
        let node = &nodes[end_index];
        if operator_is(Some(node), ";") || (type_member && operator_is(Some(node), ",")) {
            let end = node_end(node);
            return Some((source[start..end].to_owned(), end, end_index + 1));
        }
        if end_index > index && is_closed_group(node, Delimiter::Brace) {
            // `move() { ... }` ends at its body; `x = { a: 1 };` does not.
            let body = matches!(
                nodes.get(end_index - 1),
                Some(Syntax::Group {
                    delimiter: Delimiter::Paren,
                    ..
                })
            ) || matches!(nodes.get(end_index - 1), Some(Syntax::Token { .. }) if end_index >= 2
                    && matches!(nodes.get(end_index - 2), Some(Syntax::Operator { raw, .. }) if raw == ":"));
            if body && !type_member {
                let end = node_end(node);
                return Some((source[start..end].to_owned(), end, end_index + 1));
            }
        }
        if end_index > index && statement_asi(source, nodes, end_index) {
            let end = node_end(&nodes[end_index - 1]);
            return Some((source[start..end].to_owned(), end, end_index));
        }
        end_index += 1;
    }
    let end = node_end(&nodes[end_index - 1]);
    Some((source[start..end].to_owned(), end, end_index))
}

/// The order a choice tries its alternatives. Upstream backtracks through
/// the whole pattern; this matcher commits to the first alternative that
/// matches, so where the choice ends its sequence an alternative that
/// consumes every remaining node (the rest of a group) is tried first.
fn choice_order<'p>(
    source: &str,
    nodes: &[Syntax],
    at: usize,
    alternatives: &'p [Vec<Pat>],
    classes: &[SyntaxClass],
    last: bool,
) -> Vec<&'p Vec<Pat>> {
    let mut order: Vec<&Vec<Pat>> = alternatives.iter().collect();
    if last {
        let whole = |alternative: &Vec<Pat>| {
            let mut cursor = at;
            let mut scratch = HashMap::new();
            match_pats(
                source,
                nodes,
                &mut cursor,
                alternative,
                &mut scratch,
                classes,
            )
            .is_some()
                && {
                    skip_comments(nodes, &mut cursor);
                    cursor >= nodes.len()
                }
        };
        order.sort_by_key(|alternative| !whole(alternative));
    }
    order
}

/// Whether a statement already begun may end at `index` by automatic
/// semicolon insertion.
///
/// The enforester's `asiAllowed` is true at the end of the cursor or when a
/// line break stands in front of the next token. A `}` closes the statement
/// list that holds this statement, so it is a terminator too: the group is
/// the next pattern token, not part of the statement.
fn statement_asi(source: &str, nodes: &[Syntax], index: usize) -> bool {
    let Some(node) = nodes.get(index) else {
        return true;
    };
    if is_closed_group(node, Delimiter::Brace) || operator_is(Some(node), "}") {
        return true;
    }
    if operator_is(Some(node), ";") {
        return false;
    }
    let Some(previous) = nodes.get(index - 1) else {
        return false;
    };
    // A line break in front of the next token. The reader drops whitespace,
    // so the break is the source between the two spans.
    source
        .get(node_end(previous)..node_start(node))
        .is_some_and(|between| between.contains('\n'))
}

/// Where printing resumes after an item that no rule accepted.
///
/// Upstream recovery keeps the unread run through its `;`, or through the
/// line break in front of the next statement, and then keeps expanding.
/// Stopping at the macro's own head left the rest of that statement to be
/// matched again.
fn failed_item_resume(source: &str, nodes: &[Syntax], index: usize) -> usize {
    let mut end = index + 1;
    while end < nodes.len() {
        // A group is one node of this list. Its interior is not walked
        // here, so a `;` inside it does not end the item.
        if operator_is(Some(&nodes[end]), ";") {
            return end + 1;
        }
        // The next statement begins here. The failed item stops in front
        // of it, semicolon or not.
        if statement_asi(source, nodes, end) && statement_boundary(nodes, end) {
            return end;
        }
        end += 1;
        if end - index > 80 {
            break;
        }
    }
    end.min(nodes.len())
}

/// A token that can begin the statement after an unterminated item.
///
/// Same words as the enforester's `statementStarts`, plus a brace group,
/// which is itself a statement.
fn statement_boundary(nodes: &[Syntax], index: usize) -> bool {
    match nodes.get(index) {
        Some(Syntax::Group {
            delimiter: Delimiter::Brace,
            ..
        }) => true,
        Some(Syntax::Token { raw, .. }) => matches!(
            raw.as_str(),
            "abstract"
                | "await"
                | "break"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "declare"
                | "do"
                | "enum"
                | "export"
                | "for"
                | "function"
                | "if"
                | "import"
                | "interface"
                | "let"
                | "module"
                | "namespace"
                | "operator"
                | "rec"
                | "return"
                | "switch"
                | "syntax"
                | "throw"
                | "try"
                | "type"
                | "var"
                | "using"
                | "while"
                | "with"
        ),
        _ => false,
    }
}

/// A type ends at `{`, `;`, `,`, or `=`. It is not the function body.
fn take_type(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start_node = nodes.get(index)?;
    if !is_expr_start(start_node) {
        return None;
    }
    let start = node_start(start_node);
    let mut end = node_end(start_node);
    index += 1;
    // `Tree<T>` and `Map<K, V>`: a `>` that closes an argument list this
    // type opened, and the commas inside it, are part of the type.
    let mut angles = 0usize;
    loop {
        skip_comments(nodes, &mut index);
        let Some(node) = nodes.get(index) else {
            break;
        };
        if let Syntax::Operator { raw, .. } = node {
            if raw.chars().all(|ch| ch == '<') {
                angles += raw.len();
                end = node_end(node);
                index += 1;
                continue;
            }
            if angles > 0 && raw.chars().all(|ch| ch == '>') && raw.len() <= angles {
                angles -= raw.len();
                end = node_end(node);
                index += 1;
                continue;
            }
            if angles > 0 && raw == "," {
                end = node_end(node);
                index += 1;
                continue;
            }
        }
        if matches!(
            node,
            Syntax::Group {
                delimiter: Delimiter::Brace,
                ..
            }
        ) {
            break;
        }
        if matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ";" | "," | "=" | ")" | "]" | "}" | ">"))
        {
            break;
        }
        // `Empty | Cons` is one type. `|` is not a field separator; `,` is.
        if operator_is(Some(node), "|") {
            end = node_end(node);
            index += 1;
            continue;
        }
        // `value:` with the type missing stops at the colon. A following
        // closer or comma is not the type: `Some(value:)` and `head:, tail`.
        if operator_is(nodes.get(index.wrapping_sub(1)), ":")
            && matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ")" | "," | ";" | "}" | "]"))
        {
            return None;
        }
        // `string[]` and `readonly string` are part of the type. A following
        // `>` closes `optional<...>`, so it is not consumed here.
        end = node_end(node);
        index += 1;
        if index > cursor + 24 {
            break;
        }
    }
    Some((source[start..end].to_owned(), end, index))
}

/// One JSX child: an element through its matching closing tag (or `/>`), a
/// `{...}` container, or a run of text up to the next tag or container.
fn take_jsx_child(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let first = nodes.get(index)?;
    let start = node_start(first);
    if is_closed_group(first, Delimiter::Brace) {
        let end = node_end(first);
        return Some((source[start..end].to_owned(), end, index + 1));
    }
    let is_open = |node: Option<&Syntax>| operator_is(node, "<");
    if !is_open(Some(first)) {
        // Text: everything up to the next tag or container.
        let mut end_index = index;
        while let Some(node) = nodes.get(end_index) {
            if is_open(Some(node))
                || matches!(
                    node,
                    Syntax::Group {
                        delimiter: Delimiter::Brace,
                        ..
                    }
                )
                || is_expr_stop(node)
            {
                break;
            }
            end_index += 1;
        }
        if end_index == index {
            return None;
        }
        let end = node_end(&nodes[end_index - 1]);
        return Some((source[start..end].to_owned(), end, end_index));
    }
    let mut depth = 0usize;
    let mut at = index;
    while at < nodes.len() {
        if !is_open(nodes.get(at)) {
            at += 1;
            continue;
        }
        let closing = operator_is(nodes.get(at + 1), "/");
        // Walk to the `>` that ends this tag. Attribute values are groups or
        // strings, so a bare `>` is the tag's end.
        let mut tag_end = at + 1;
        let mut self_closing = false;
        while let Some(node) = nodes.get(tag_end) {
            if let Syntax::Operator { raw, .. } = node {
                if raw == "/>" {
                    self_closing = true;
                    break;
                }
                if raw == ">" {
                    self_closing = !closing && operator_is(nodes.get(tag_end - 1), "/");
                    break;
                }
            }
            tag_end += 1;
        }
        if tag_end >= nodes.len() {
            return None;
        }
        if closing {
            depth = depth.checked_sub(1)?;
        } else if !self_closing {
            depth += 1;
        }
        at = tag_end + 1;
        if depth == 0 {
            let end = node_end(&nodes[tag_end]);
            return Some((source[start..end].to_owned(), end, at));
        }
    }
    None
}

/// `$name:binding` is one identifier, not the call or type that follows it.
fn take_ident(nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    let Syntax::Token { raw, .. } = node else {
        return None;
    };
    if !is_ident_text(raw) {
        return None;
    }
    Some((raw.clone(), node_end(node), index + 1))
}

/// One token of a `$($segment)/+` repetition. A group is the whole group.
fn take_token_tree(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    if is_expr_stop(node) {
        return None;
    }
    // `$($rest:tt)+` is the remaining steps, not one character. A call
    // (`map((value) => value + 1)`) is one step; the comma after it belongs
    // to the repetition's separator, not to this hole.
    let start = node_start(node);
    // A brace group is a whole body. `kinded Functor<F> { ... }` stops at
    // the closing brace and does not swallow the next item
    // (`Functor FunctorArray for Array`).
    if is_closed_group(node, Delimiter::Brace) {
        let end = node_end(node);
        return Some((source[start..end].to_owned(), end, index + 1));
    }
    let mut end_index = index + 1;
    while end_index < nodes.len() {
        if operator_is(nodes.get(end_index), ",") || is_expr_stop(&nodes[end_index]) {
            break;
        }
        end_index += 1;
        if end_index - index > 24 {
            break;
        }
    }
    let end = node_end(&nodes[end_index - 1]);
    Some((source[start..end].to_owned(), end, end_index))
}

/// `;` as one token tree. Upstream `tt` reads any single token, so the
/// statements after a `doSteps` step (`$($rest:tt)+`) keep their
/// semicolons; the greedy step reader stops in front of one.
fn take_semicolon(nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    operator_is(Some(node), ";").then(|| (";".to_owned(), node_end(node), index + 1))
}

/// One token or group, whatever its kind. A standalone `token` hole uses
/// this; the greedy [`take_token_tree`] is only for a repetition step.
fn take_one_token(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    if is_expr_stop(node) {
        return None;
    }
    Some((
        sweetener_reader::slice(source, node).to_owned(),
        node_end(node),
        index + 1,
    ))
}

fn take_expr(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    take_expr_until(source, nodes, cursor, None)
}

/// True when every character is one an operator can be spelled with.
/// The reader splits `===` into `==` and `=`, so a fixed list of spellings
/// misses the second half.
fn is_operator_spelling(raw: &str) -> bool {
    !raw.is_empty() && raw.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch))
}

/// Operand of a right-associative operator, including another copy of that
/// operator. `2 ** 3 ** 2` captures `3 ** 2`, not just `3`.
fn take_right_assoc_operand(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start_node = nodes.get(index)?;
    if !is_expr_start(start_node) {
        return None;
    }
    let start = node_start(start_node);
    index += 1;
    while index < nodes.len() {
        skip_comments(nodes, &mut index);
        let Some(node) = nodes.get(index) else {
            break;
        };
        if is_expr_stop(node) {
            break;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ":") {
            break;
        }
        // `=>` is `=` then `>`. The arrow is part of the expression.
        if operator_is(Some(node), "=") && operator_is(nodes.get(index + 1), ">") {
            index += 2;
            continue;
        }
        if operator_is(Some(node), "=") {
            break;
        }
        if matches!(node, Syntax::Group { .. })
            || matches!(node, Syntax::Token { .. } | Syntax::String { .. })
        {
            index += 1;
            continue;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == ".") {
            index += 1;
            if matches!(nodes.get(index), Some(Syntax::Token { .. })) {
                index += 1;
            }
            continue;
        }
        if matches!(node, Syntax::Operator { .. })
            && nodes.get(index + 1).is_some_and(is_expr_start)
        {
            index += 1;
            continue;
        }
        break;
    }
    if index == cursor {
        return None;
    }
    let end = node_end(&nodes[index - 1]);
    Some((source[start..end].to_owned(), end, index))
}

fn take_expr_until(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    stop_op: Option<&str>,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start_node = nodes.get(index)?;
    let start = node_start(start_node);
    if !is_expr_start(start_node) {
        return None;
    }
    index += 1;
    // `-4` starts on the operator. The operand is the next primary, not a
    // trailing operator the walk would drop.
    if matches!(start_node, Syntax::Operator { .. }) && nodes.get(index).is_some_and(is_expr_start)
    {
        index += 1;
    }
    // Open `?` of conditionals; the matching `:` belongs to the expression.
    let mut pending_conditionals = 0usize;
    loop {
        skip_comments(nodes, &mut index);
        let Some(node) = nodes.get(index) else {
            break;
        };
        if is_expr_stop(node) {
            break;
        }
        if operator_is(Some(node), "?")
            && nodes.get(index + 1).is_some_and(is_expr_start)
            && !operator_is(nodes.get(index + 1), ".")
        {
            pending_conditionals += 1;
            index += 1;
            continue;
        }
        if pending_conditionals > 0
            && operator_is(Some(node), ":")
            && nodes.get(index + 1).is_some_and(is_expr_start)
        {
            pending_conditionals -= 1;
            index += 1;
            continue;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ":") {
            break;
        }
        // `posts.authorId == users.id where ...`. The following pattern
        // piece is the word `where`, so the expression stops before it. A
        // word that is not the stop stays part of the expression.
        if let Some(stop) = stop_op {
            if matches!(node, Syntax::Token { raw, .. } if raw == stop) {
                break;
            }
        }
        // The reader splits `=>` into `=` and `>`. An arrow is part of the
        // expression (`(value) => value + 1`), not a type annotation.
        if operator_is(Some(node), "=") && operator_is(nodes.get(index + 1), ">") {
            index += 2;
            continue;
        }
        // A block after a complete operand is not part of the expression:
        // `handle report(id) { ... }` has the operand `report(id)`. A
        // function or class expression owns its body.
        if is_closed_group(node, Delimiter::Brace)
            && matches!(
                nodes.get(index - 1),
                Some(Syntax::Token { .. } | Syntax::String { .. } | Syntax::Group { .. })
            )
            && !nodes[cursor..index]
                .iter()
                .any(|node| token_is(Some(node), "function") || token_is(Some(node), "class"))
        {
            break;
        }
        // Postfix call or index, or an infix operator followed by another primary.
        if matches!(node, Syntax::Group { .. }) {
            index += 1;
            continue;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == ".") {
            if matches!(nodes.get(index + 1), Some(Syntax::Token { .. })) {
                index += 2;
                continue;
            }
            break;
        }
        if matches!(node, Syntax::Token { .. } | Syntax::String { .. }) {
            // The operator branch lands on the operand (`0` in `!== 0`).
            // Include it; a bare token after a finished primary is the next statement.
            // `where` after that operand ends the expression.
            if stop_op.is_some_and(|stop| matches!(node, Syntax::Token { raw, .. } if raw == stop))
            {
                break;
            }
            if index > cursor && matches!(nodes.get(index - 1), Some(Syntax::Operator { .. })) {
                index += 1;
                continue;
            }
            break;
        }
        if matches!(node, Syntax::Operator { .. }) {
            if stop_op.is_some() && operator_spelling(node) == stop_op && index > cursor {
                break;
            }
            let op_end = index + 1;
            if nodes.get(op_end).is_some_and(is_expr_start) {
                index = op_end;
                continue;
            }
            // `!==` and `&&` are several operator characters. The reader
            // splits `===` into `==` and `=`, so the spelling is not one of
            // a fixed list; every character is an operator character. An
            // expression hole includes them; a custom infix still stops at
            // its own spelling.
            if matches!(node, Syntax::Operator { raw, .. } if is_operator_spelling(raw))
                && nodes.get(op_end).is_some_and(|next| {
                    matches!(
                        next,
                        Syntax::Operator { .. } | Syntax::Token { .. } | Syntax::String { .. }
                    )
                })
            {
                index = op_end;
                continue;
            }
            break;
        }
        if token_is(Some(node), "as") {
            index += 1;
            continue;
        }
        break;
    }
    // Walk back if we consumed a trailing infix operator with no
    // right-hand side. A leading `-4` is the expression, not a trailing op.
    while index > cursor + 1 && matches!(nodes.get(index - 1), Some(Syntax::Operator { .. })) {
        // `!==` was consumed as operators. Do not walk back onto `!` when
        // `==` and its operand are already part of the expression.
        let chained = matches!(nodes.get(index - 1), Some(Syntax::Operator { raw, .. }) if is_operator_spelling(raw))
            && nodes.get(index).is_some_and(|next| {
                matches!(
                    next,
                    Syntax::Operator { .. } | Syntax::Token { .. } | Syntax::String { .. }
                )
            });
        if chained {
            break;
        }
        if stop_op.is_some()
            && operator_spelling(&nodes[index - 1]) != stop_op
            && nodes.get(index).is_some_and(is_expr_start)
        {
            break;
        }
        if stop_op.is_some() && operator_spelling(&nodes[index - 1]) == stop_op {
            break;
        }
        let leading = index - 1 == cursor;
        index -= 1;
        if leading {
            break;
        }
    }
    let end = node_end(&nodes[index - 1]);
    Some((source[start..end].to_owned(), end, index))
}

fn is_expr_start(node: &Syntax) -> bool {
    match node {
        Syntax::Token { .. } | Syntax::String { .. } | Syntax::Group { .. } => true,
        Syntax::Operator { raw, .. } => {
            matches!(
                raw.as_str(),
                "!" | "-" | "+" | "~" | "#" | "..." | "." | "<"
            ) || is_syntax_parameter(raw)
        }
        Syntax::Comment { .. } => false,
    }
}

fn is_expr_stop(node: &Syntax) -> bool {
    match node {
        Syntax::Operator { raw, .. } => matches!(raw.as_str(), ";" | ")" | "]" | "}"),
        _ => false,
    }
}

/// The rename renderer prints a `#text` capture raw. Theirs quotes it.
/// Where the ordinary rendering has the quoted form and the renamed one has
/// the raw capture, put the quotes back.
/// Rename identifiers in an already-rendered replacement. Each plan entry
/// is `(spelling, occurrence, printed)`. A binder takes the next occurrence;
/// a reference takes the one in scope. Captures are not scanned: they were
/// already substituted, and a user spelling inside one stays as written when
/// it is not a binder the template introduced.
/// Replace capture text with a string marker so hygiene does not rename it.
/// Markers keep their order. [`unshield`] puts the original text back.
fn shield_captures(text: &str, protected: &[(usize, usize)]) -> (String, Vec<String>) {
    let mut spans: Vec<(usize, usize)> = protected
        .iter()
        .copied()
        .filter(|(from, to)| from < to && *to <= text.len())
        .collect();
    spans.sort_by_key(|span| span.0);
    spans.dedup();
    let mut out = text.to_owned();
    let mut originals = vec![String::new(); spans.len()];
    for (index, (from, to)) in spans.iter().enumerate().rev() {
        originals[index] = out[*from..*to].to_owned();
        out.replace_range(*from..*to, &format!("\"__swcap{index}__\""));
    }
    (out, originals)
}

fn unshield(text: &str, originals: &[String]) -> String {
    let mut out = text.to_owned();
    for (index, original) in originals.iter().enumerate() {
        out = out.replace(&format!("\"__swcap{index}__\""), original);
    }
    out
}

fn apply_plan(text: &str, plan: &[(String, String)]) -> String {
    let read = sweetener_reader::read_syntax(text);
    let mut seen = HashMap::<String, usize>::new();
    let mut out = String::new();
    let mut cursor = 0usize;
    apply_plan_nodes(
        text,
        &read.children,
        plan,
        &mut seen,
        false,
        false,
        &mut out,
        &mut cursor,
    );
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    out
}

fn plan_name(key: &str) -> &str {
    key.split('\0').next().unwrap_or(key)
}

fn occurrence_plan(
    plan: &[(String, String)],
    seen: &HashMap<String, usize>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, printed) in plan {
        let name = plan_name(key);
        let opened = seen.get(name).copied().unwrap_or(0);
        // The parameter was already counted. A use inside `${...}` refers
        // to that binding, the same way `planned` does for a non-binder.
        let nth = if key.contains('\0') && opened > 0 {
            opened - 1
        } else {
            opened
        };
        if key == &format!("{name}\0{nth}") || key == name || (!key.contains('\0') && nth == 0) {
            map.insert(name.to_owned(), printed.clone());
        }
    }
    map
}

fn apply_plan_nodes(
    source: &str,
    nodes: &[Syntax],
    plan: &[(String, String)],
    seen: &mut HashMap<String, usize>,
    after_dot: bool,
    // True while the nodes are a destructuring pattern. Its names are
    // binders, so `const { id }` prints `id_1` when `id` is taken.
    pattern: bool,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut index = 0;
    let mut dotted = after_dot;
    while index < nodes.len() {
        match &nodes[index] {
            Syntax::Token { span, raw } if !dotted && is_ident_text(raw) => {
                let binder = is_binder_at(nodes, index)
                    || in_type_params(nodes, index)
                    || (pattern && !operator_is(nodes.get(index + 1), ":"));
                if let Some(printed) = planned(raw, binder, plan, seen) {
                    push_until(source, out, cursor, span.start as usize);
                    // `{ id }` keeps the property and renames the binding:
                    // `{ id: id_1 }`. `cond: conditionInput` already names both.
                    let shorthand = pattern
                        && printed != *raw
                        && !operator_is(nodes.get(index.wrapping_sub(1)), ":");
                    // A placeholder's shorthand is decided on the output.
                    if shorthand && !super::sweetener_hygiene::is_placeholder(&printed) {
                        out.push_str(raw);
                        out.push_str(": ");
                    }
                    out.push_str(&printed);
                    *cursor = span.end as usize;
                    if binder {
                        let n = seen.get(raw).copied().unwrap_or(0);
                        seen.insert(raw.clone(), n + 1);
                    }
                } else if binder {
                    // `[Brand]` is a later sibling group. Count the declaration
                    // now so that reference uses this occurrence, not a fresh one.
                    let n = seen.get(raw).copied().unwrap_or(0);
                    seen.insert(raw.clone(), n + 1);
                }
                dotted = false;
            }
            Syntax::Group { span, children, .. } if span.end as usize <= *cursor => {}
            Syntax::Group { children, .. } => {
                let method_params = index > 0
                    && operator_is(nodes.get(index - 1), ">")
                    && matches!(
                        nodes.get(index),
                        Some(Syntax::Group {
                            delimiter: Delimiter::Paren,
                            ..
                        })
                    );
                if method_params {
                    if let Syntax::Group { span, children, .. } = &nodes[index] {
                        // `map<A, B>(value: F<A>)` — count `A` before the
                        // parameter types, or `F<A>` keeps the user's `A`.
                        if let Some(angle) = nodes[..index]
                            .iter()
                            .rposition(|node| operator_is(Some(node), "<"))
                            .filter(|angle| {
                                nodes[*angle..index]
                                    .iter()
                                    .position(|node| operator_is(Some(node), ">"))
                                    .is_some_and(|close| angle + close + 1 == index)
                            })
                        {
                            if let Some(params) = type_param_names(nodes, angle) {
                                for name in params {
                                    // One count for the declaration. A second
                                    // count here makes the return type look
                                    // up an occurrence the plan never made.
                                    if seen.get(&name).copied().unwrap_or(0) == 0 {
                                        seen.insert(name, 1);
                                    }
                                }
                            }
                        }
                        push_until(source, out, cursor, span.start as usize);
                        let mut local = span.start as usize;
                        apply_arrow_params(source, children, plan, seen, out, &mut local);
                        if local < span.end as usize {
                            out.push_str(&source[local..span.end as usize]);
                        }
                        *cursor = span.end as usize;
                        // `: F<B>` is the return type, still in this signature.
                        let mut rest = index + 1;
                        if operator_is(nodes.get(rest), ":") {
                            rest += 1;
                            let mut end = rest;
                            while end < nodes.len()
                                && !operator_is(nodes.get(end), ";")
                                && !operator_is(nodes.get(end), "{")
                                && !matches!(
                                    nodes.get(end),
                                    Some(Syntax::Group {
                                        delimiter: Delimiter::Brace,
                                        ..
                                    })
                                )
                            {
                                end += 1;
                                if end > rest + 8 {
                                    break;
                                }
                            }
                            apply_plan_nodes(
                                source,
                                &nodes[rest..end],
                                plan,
                                seen,
                                false,
                                false,
                                out,
                                cursor,
                            );
                            if end > rest {
                                let close = node_end(&nodes[end - 1]);
                                if *cursor < close {
                                    out.push_str(&source[*cursor..close]);
                                    *cursor = close;
                                }
                            }
                            index = end;
                            continue;
                        }
                        index += 1;
                        continue;
                    }
                }
                if let Some(_params) = arrow_params(nodes, index) {
                    if let Syntax::Group { span, children, .. } = &nodes[index] {
                        // `map<A, B>(a: A)` — the type parameters are binders
                        // of this method. Count them before the parameter list
                        // or `a` takes `A`'s suffix. The prefix is still copied;
                        // rewrite the `<...>` here so `A` becomes `A_1`.
                        if let Some(angle) = nodes[..index]
                            .iter()
                            .rposition(|node| operator_is(Some(node), "<"))
                        {
                            if let Some(close) = nodes[angle..index]
                                .iter()
                                .position(|node| operator_is(Some(node), ">"))
                            {
                                let close_at = angle + close;
                                // Only the type-parameter list that ends at
                                // this parameter list. `F<A>` earlier in the
                                // signature is a use, not this binder.
                                if close_at + 1 == index && type_param_names(nodes, angle).is_some()
                                {
                                    push_until(source, out, cursor, node_start(&nodes[angle]));
                                    out.push('<');
                                    let mut local = node_end(&nodes[angle]);
                                    apply_plan_nodes(
                                        source,
                                        &nodes[angle + 1..close_at],
                                        plan,
                                        seen,
                                        false,
                                        false,
                                        out,
                                        &mut local,
                                    );
                                    if local < node_start(&nodes[close_at]) {
                                        out.push_str(&source[local..node_start(&nodes[close_at])]);
                                    }
                                    out.push('>');
                                    *cursor = node_end(&nodes[close_at]);
                                }
                            }
                        }
                        push_until(source, out, cursor, span.start as usize);
                        let mut local_cursor = span.start as usize;
                        apply_arrow_params(source, children, plan, seen, out, &mut local_cursor);
                        if local_cursor < span.end as usize {
                            out.push_str(&source[local_cursor..span.end as usize]);
                        }
                        *cursor = span.end as usize;
                        let mut rest_index = index + 1;
                        if operator_is(nodes.get(rest_index), "=>") {
                            rest_index += 1;
                        }
                        apply_plan_nodes(
                            source,
                            &nodes[rest_index..],
                            plan,
                            seen,
                            false,
                            false,
                            out,
                            cursor,
                        );
                        index = nodes.len();
                        continue;
                    }
                } else {
                    let binding_pattern = pattern
                        || (matches!(
                            nodes.get(index),
                            Some(Syntax::Group {
                                delimiter: Delimiter::Brace,
                                ..
                            })
                        ) && index > 0
                            && (token_at(nodes, index - 1, "const")
                                || token_at(nodes, index - 1, "let")
                                || token_at(nodes, index - 1, "var")));
                    apply_plan_nodes(
                        source,
                        children,
                        plan,
                        seen,
                        false,
                        binding_pattern,
                        out,
                        cursor,
                    );
                }
                dotted = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => dotted = true,
            Syntax::String { span, raw } if raw.starts_with('`') && raw.contains("${") => {
                let map = occurrence_plan(plan, seen);
                // A name is not only `${inspected}`; it can be nested inside
                // `${globalThis.JSON.stringify(inspected)}`. Split the
                // template on non-identifier characters so the word is found
                // wherever it appears.
                let needs = map.keys().any(|name| {
                    raw.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .any(|word| word == name)
                });
                if needs {
                    let renamed = rename_template_literal(
                        raw,
                        &map,
                        &[],
                        &HashMap::new(),
                        seen,
                        span.start as usize,
                    );
                    push_until(source, out, cursor, span.start as usize);
                    out.push_str(&renamed);
                    *cursor = span.end as usize;
                }
                dotted = false;
            }
            _ => dotted = false,
        }
        index += 1;
    }
}

fn planned(
    raw: &str,
    binder: bool,
    plan: &[(String, String)],
    seen: &HashMap<String, usize>,
) -> Option<String> {
    let opened = seen.get(raw).copied().unwrap_or(0);
    if !binder && opened == 0 {
        return plan
            .iter()
            .find(|(key, _)| plan_name(key) == raw && !key.contains('\0'))
            .map(|(_, printed)| printed.clone());
    }
    let nth = if binder {
        opened
    } else {
        opened.saturating_sub(1)
    };
    plan.iter()
        .find(|(key, _)| key == &format!("{raw}\0{nth}"))
        .map(|(_, printed)| printed.clone())
}

fn apply_arrow_params(
    source: &str,
    children: &[Syntax],
    plan: &[(String, String)],
    seen: &mut HashMap<String, usize>,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut index = 0;
    while index < children.len() {
        if let Syntax::Token { span, raw } = &children[index] {
            let after_colon = index > 0 && operator_is(children.get(index - 1), ":");
            if is_ident_text(raw) && !is_type_name(raw) {
                // A name after `:` is a type reference, not a new binder.
                let binder = !after_colon;
                if let Some(printed) = planned(raw, binder, plan, seen) {
                    push_until(source, out, cursor, span.start as usize);
                    out.push_str(&printed);
                    *cursor = span.end as usize;
                }
                if binder {
                    let n = seen.get(raw).copied().unwrap_or(0);
                    seen.insert(raw.clone(), n + 1);
                }
            }
        }
        if let Syntax::Group {
            children: nested, ..
        } = &children[index]
        {
            apply_arrow_params(source, nested, plan, seen, out, cursor);
        }
        index += 1;
    }
}

fn is_type_name(raw: &str) -> bool {
    matches!(
        raw,
        "number"
            | "string"
            | "boolean"
            | "any"
            | "unknown"
            | "never"
            | "void"
            | "object"
            | "bigint"
            | "symbol"
            | "undefined"
            | "null"
    )
}

fn u32_at(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

fn node_span(node: &Syntax) -> Span {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => *span,
    }
}

fn node_start(node: &Syntax) -> usize {
    node_span(node).start as usize
}

fn node_end(node: &Syntax) -> usize {
    node_span(node).end as usize
}

fn token_at(nodes: &[Syntax], index: usize, expected: &str) -> bool {
    matches!(nodes.get(index), Some(Syntax::Token { raw, .. }) if raw == expected)
}

fn token_is(node: Option<&Syntax>, expected: &str) -> bool {
    matches!(node, Some(Syntax::Token { raw, .. }) if raw == expected)
}

fn operator_is(node: Option<&Syntax>, expected: &str) -> bool {
    match node {
        Some(Syntax::Operator { raw, .. } | Syntax::Token { raw, .. }) => raw == expected,
        _ => false,
    }
}

fn operator_spelling(node: &Syntax) -> Option<&str> {
    match node {
        Syntax::Operator { raw, .. } => Some(raw.as_str()),
        // `implements` is a word, not a punctuation operator.
        Syntax::Token { raw, .. } if is_ident_text(raw) => Some(raw.as_str()),
        _ => None,
    }
}

fn group_is(node: Option<&Syntax>, delimiter: Delimiter) -> bool {
    matches!(node, Some(Syntax::Group { delimiter: found, .. }) if *found == delimiter)
}

fn is_closed_group(node: &Syntax, delimiter: Delimiter) -> bool {
    matches!(node, Syntax::Group { delimiter: found, close: Some(_), .. } if *found == delimiter)
}

/// `<-` and `->` are one macro name, but the reader stores each character.
/// `===` and `!==` are the same: the reader keeps `==` then `=`. A literal
/// matches that run when the characters are adjacent.
fn lit_width(nodes: &[Syntax], index: usize, expected: &str) -> Option<usize> {
    if expected.chars().count() < 2 || !expected.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch)) {
        return None;
    }
    let mut cursor = index;
    let mut got = String::new();
    let mut width = 0usize;
    while got.len() < expected.len() {
        let raw = match nodes.get(cursor) {
            Some(Syntax::Operator { raw, .. }) => raw.as_str(),
            _ => return None,
        };
        if !raw.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch)) {
            return None;
        }
        if width > 0 {
            let prev_end = node_end(&nodes[cursor - 1]);
            if node_start(&nodes[cursor]) != prev_end {
                return None;
            }
        }
        got.push_str(raw);
        width += 1;
        cursor += 1;
        if width > 4 {
            return None;
        }
    }
    (got == expected).then_some(width)
}

fn node_is_lit(node: &Syntax, expected: &str) -> bool {
    match node {
        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } | Syntax::String { raw, .. } => {
            raw == expected
        }
        _ => false,
    }
}

fn is_ident_text(raw: &str) -> bool {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

fn open_char(delimiter: Delimiter) -> char {
    match delimiter {
        Delimiter::Paren => '(',
        Delimiter::Brace => '{',
        Delimiter::Bracket => '[',
    }
}

fn skip_comments(nodes: &[Syntax], cursor: &mut usize) {
    while nodes
        .get(*cursor)
        .is_some_and(|node| matches!(node, Syntax::Comment { .. }))
    {
        *cursor += 1;
    }
}

fn push_until(source: &str, out: &mut String, cursor: &mut usize, until: usize) {
    if until < *cursor {
        // A nested walk started past this token. The caller is about to
        // write the token itself, so do not also copy it from source.
        *cursor = until;
        return;
    }
    if until <= source.len() {
        out.push_str(&source[*cursor..until]);
        *cursor = until;
    }
}

fn macro_head(node: &Syntax) -> Option<&str> {
    match node {
        Syntax::Token { raw, .. } if is_ident_text(raw) => Some(raw.as_str()),
        _ => None,
    }
}

/// True when the node at `index` is an identifier that a property access
/// owns: `x.service` (the name follows a `.`) or `service.q` (the name is
/// followed by a `.`). A macro call never has a `.` on either side.
fn property_access_at(nodes: &[Syntax], index: usize) -> bool {
    if nodes.get(index).and_then(macro_head).is_none() {
        return false;
    }
    let prev_dot = index > 0 && operator_is(nodes.get(index - 1), ".");
    let next_dot = operator_is(nodes.get(index + 1), ".");
    prev_dot || next_dot
}

/// True when the identifier at `index` could start a macro rule. A macro
/// head is followed by a token, a group, or `<`. A bare identifier that a
/// parameter list or an expression owns (`(service) =>`, `service;`) is not.
/// A name a property access owns (`Effect.gen`) is a member, never a head.
/// A form rewrite can print such an identifier that shares a macro name.
/// `export syntax` / `export operator` in the file being expanded. The
/// body is the next brace. The definition is a macro, not TypeScript.
fn local_syntax_end(nodes: &[Syntax], index: usize) -> Option<usize> {
    let mut cursor = index;
    if token_at(nodes, cursor, "export") {
        cursor += 1;
    }
    if token_at(nodes, cursor, "rec") {
        cursor += 1;
    }
    if !token_at(nodes, cursor, "syntax") && !token_at(nodes, cursor, "operator") {
        return None;
    }
    let mut look = cursor + 1;
    while look < nodes.len() && look < cursor + 16 {
        if let Syntax::Group {
            delimiter: Delimiter::Brace,
            span,
            ..
        } = &nodes[look]
        {
            return Some(span.end as usize);
        }
        look += 1;
    }
    None
}

/// True when every top-level item is a `for syntax` import or a macro
/// definition, so the file can invoke no macro.
fn only_definitions(nodes: &[Syntax]) -> bool {
    let mut index = 0;
    while index < nodes.len() {
        match &nodes[index] {
            Syntax::Comment { .. } => index += 1,
            node if operator_is(Some(node), ";") => index += 1,
            _ if is_syntax_import_at(nodes, index) => index = skip_syntax_import(nodes, index),
            _ => {
                let Some(end) = local_syntax_end(nodes, index) else {
                    return false;
                };
                while index < nodes.len() && (node_span(&nodes[index]).start as usize) < end {
                    index += 1;
                }
            }
        }
    }
    true
}

/// A following group or word is an attempted call. A name at the end of
/// `(service)` is not, once no rule has claimed it.
/// `deferred {name}` inside `<p>…</p>` is text, not the statement macro.
fn join_spellings(
    template: &[Tpl],
    captures: &HashMap<String, Capture>,
    out: &mut std::collections::HashSet<String>,
) {
    for part in template {
        match part {
            Tpl::Join {
                name,
                prefix,
                suffix,
                casing,
            } => {
                if let Some(text) = capture_text(captures, name) {
                    out.insert(join_identifier(prefix, text.trim(), suffix, casing));
                }
            }
            Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::Define(inner) => {
                join_spellings(inner, captures, out);
            }
            Tpl::If {
                then_body,
                else_body,
                ..
            } => {
                join_spellings(then_body, captures, out);
                if let Some(else_body) = else_body {
                    join_spellings(else_body, captures, out);
                }
            }
            _ => {}
        }
    }
}

fn jsx_child_context(nodes: &[Syntax], index: usize) -> bool {
    let mut stack = 0i32;
    let mut cursor = 0usize;
    while cursor < index {
        if operator_is(nodes.get(cursor), "<") && !operator_is(nodes.get(cursor + 1), "/") {
            stack += 1;
        } else if operator_is(nodes.get(cursor), "<") && operator_is(nodes.get(cursor + 1), "/") {
            stack -= 1;
            cursor += 1;
        }
        cursor += 1;
    }
    stack > 0
}

fn macro_invocation(nodes: &[Syntax], index: usize) -> bool {
    match nodes.get(index + 1) {
        Some(Syntax::Group { .. } | Syntax::Token { .. } | Syntax::String { .. }) => true,
        Some(Syntax::Operator { raw, .. }) => {
            matches!(
                raw.as_str(),
                "<" | "#" | "=" | "+=" | "-=" | "*=" | "/=" | "%="
            ) || raw.ends_with('=')
        }
        _ => false,
    }
}

fn macro_call_position(nodes: &[Syntax], index: usize) -> bool {
    if nodes.get(index).and_then(macro_head).is_none() {
        return false;
    }
    if index > 0 && operator_is(nodes.get(index - 1), ".") {
        return false;
    }
    // `flag` at the end of `[count, flag]` has no following sibling. A
    // generated expression macro still owns that name.
    let Some(next) = nodes.get(index + 1) else {
        return true;
    };
    match next {
        Syntax::Group { .. } => true,
        Syntax::Token { .. } | Syntax::String { .. } => true,
        // `=` and `+=` are how a generated `syntax count` is called.
        // `<<=` used to match only because the reader splits it and the
        // first character is `<`. A closer is not a call: `(service)` is a
        // parameter, and treating `)` as a follower failed that macro.
        // `,` separates arguments (`[count, flag]`). `)` does not: `(service)`
        // is a parameter, and a failed rule there used to discard the file.
        Syntax::Operator { raw, .. } => !matches!(raw.as_str(), "." | ")" | "]" | "}"),
        _ => false,
    }
}

/// The macro a failed form was matching, including an operator inside a group.
fn macro_name_of(source: &str, nodes: &[Syntax], index: usize) -> Option<String> {
    if let Some(name) = nodes.get(index).and_then(macro_head) {
        return Some(name.to_owned());
    }
    let Syntax::Group { children, .. } = nodes.get(index)? else {
        return nodes
            .get(index)
            .and_then(operator_spelling)
            .map(str::to_owned);
    };
    let _ = source;
    if children.iter().any(|node| operator_is(Some(node), "#")) {
        return Some("#".to_owned());
    }
    if let Some(name) = children.iter().find_map(|node| {
        let raw = operator_spelling(node)?;
        if raw == "->" || raw == "->>" || raw == "<-" {
            Some(raw.to_owned())
        } else {
            None
        }
    }) {
        return Some(name);
    }
    joined_operator(children)
}

fn joined_operator(nodes: &[Syntax]) -> Option<String> {
    let first = nodes.first().and_then(operator_spelling)?;
    if first == "->" {
        if nodes.get(1).and_then(operator_spelling) == Some(">") {
            return Some(format!("{first}>"));
        }
        return Some(first.to_owned());
    }
    if first == "<-" {
        return Some(first.to_owned());
    }
    None
}

fn specifier_body(raw: &str) -> Option<&str> {
    let quote = raw.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    raw.strip_prefix(quote)?.strip_suffix(quote)
}

fn rewrite_specifier(raw: &str) -> String {
    let Some(body) = specifier_body(raw) else {
        return raw.to_owned();
    };
    for ext in [".sts", ".stsx"] {
        if let Some(stripped) = body.strip_suffix(ext) {
            let quote = raw.chars().next().unwrap_or('"');
            return format!("{quote}{stripped}{quote}");
        }
    }
    raw.to_owned()
}

fn user_spellings(nodes: &[Syntax]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    collect_user_binders(nodes, &mut names);
    names
}

fn collect_user_binders(nodes: &[Syntax], names: &mut std::collections::HashSet<String>) {
    let mut index = 0;
    while index < nodes.len() {
        if is_binder_keyword(nodes, index) {
            if let Some(name) = binder_name_after(nodes, index) {
                names.insert(name);
            }
            // `const [id, setId] = ...` and `const { a, b } = ...`.
            if ["const", "let", "var"]
                .iter()
                .any(|word| token_at(nodes, index, word))
            {
                if let Some(Syntax::Group {
                    delimiter: Delimiter::Bracket | Delimiter::Brace,
                    children,
                    ..
                }) = nodes.get(index + 1)
                {
                    let mut bound = Vec::new();
                    pattern_binders(children, &mut bound);
                    names.extend(bound);
                }
            }
            // `function Component({ cond = false, id }: Props)` binds `id`.
            // The template's own `id` is then printed `id_1`.
            let params = if token_at(nodes, index, "function") {
                nodes.get(index + 2)
            } else {
                nodes.get(index + 1)
            };
            if let Some(Syntax::Group {
                delimiter: Delimiter::Paren,
                children,
                ..
            }) = params
            {
                let mut bound = Vec::new();
                for child in children {
                    if let Syntax::Group {
                        delimiter: Delimiter::Brace,
                        children,
                        ..
                    } = child
                    {
                        pattern_binders(children, &mut bound);
                    }
                }
                names.extend(bound);
            }
        }
        // `declare class Option<A>` binds `A` only when a macro also
        // introduces `A`. Recording every type argument would rename `F`
        // because the user wrote `F<A>`.
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_user_binders(children, names);
        }
        index += 1;
    }
}

/// Names bound by one destructuring pattern. `cond: conditionInput`
/// binds `conditionInput`; a bare `id` binds `id`. The property name is not
/// a binder.
fn pattern_binders(nodes: &[Syntax], names: &mut Vec<String>) {
    let mut index = 0;
    while index < nodes.len() {
        if let Syntax::Group {
            delimiter: Delimiter::Brace,
            children,
            ..
        } = &nodes[index]
        {
            pattern_binders(children, names);
            index += 1;
            continue;
        }
        let Some(Syntax::Token { raw, .. }) = nodes.get(index) else {
            index += 1;
            continue;
        };
        if !is_ident_text(raw) {
            index += 1;
            continue;
        }
        if operator_is(nodes.get(index + 1), ":") {
            if let Some(Syntax::Token { raw: bound, .. }) = nodes.get(index + 2) {
                if is_ident_text(bound) {
                    names.push(bound.clone());
                }
            }
            index += 3;
            continue;
        }
        names.push(raw.clone());
        index += 1;
    }
}

fn is_binder_keyword(nodes: &[Syntax], index: usize) -> bool {
    token_at(nodes, index, "const")
        || token_at(nodes, index, "let")
        || token_at(nodes, index, "var")
        || token_at(nodes, index, "function")
        || token_at(nodes, index, "class")
        || token_at(nodes, index, "enum")
        || token_at(nodes, index, "type")
        || token_at(nodes, index, "interface")
}

fn binder_name_after(nodes: &[Syntax], index: usize) -> Option<String> {
    let mut cursor = index + 1;
    if operator_is(nodes.get(cursor), "*") {
        cursor += 1;
    }
    match nodes.get(cursor) {
        Some(Syntax::Token { raw, .. }) if is_ident_text(raw) => Some(raw.clone()),
        _ => None,
    }
}

fn is_binder_at(nodes: &[Syntax], index: usize) -> bool {
    let Some(Syntax::Token { raw, .. }) = nodes.get(index) else {
        return false;
    };
    if !is_ident_text(raw) {
        return false;
    }
    if index > 0 && is_binder_keyword(nodes, index - 1) {
        return true;
    }
    if index > 1 && operator_is(nodes.get(index - 1), "*") && is_binder_keyword(nodes, index - 2) {
        return true;
    }
    // `map<A, B>(value, transform)` — the names in that paren list are binders.
    if method_param_name(nodes, index) {
        return true;
    }
    false
}

fn method_param_name(nodes: &[Syntax], index: usize) -> bool {
    let Some(open) = nodes[..index].iter().rposition(|node| {
        matches!(
            node,
            Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            }
        )
    }) else {
        return false;
    };
    if open == 0 || !operator_is(nodes.get(open - 1), ">") {
        return false;
    }
    let Syntax::Group { children, .. } = &nodes[open] else {
        return false;
    };
    let Some(Syntax::Token { span, .. }) = nodes.get(index) else {
        return false;
    };
    children.iter().any(|child| match child {
        Syntax::Token {
            span: child_span,
            raw,
        } => child_span.start == span.start && is_ident_text(raw) && !is_type_name(raw),
        _ => false,
    }) && !children.iter().enumerate().any(|(at, child)| {
        node_start(child) == span.start as usize && at > 0 && operator_is(children.get(at - 1), ":")
    })
}

fn syntax_imports(source: &str, nodes: &[Syntax]) -> Vec<SyntaxImport> {
    let mut imports = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if is_syntax_import_at(nodes, index)
            && super::sweetener_validate::syntax_import_well_formed(source, nodes, index)
        {
            if let Some(import) = syntax_import_at(nodes, index) {
                imports.push(import);
            }
        }
        index += 1;
    }
    imports
}

fn inside_syntax_import(nodes: &[Syntax], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if operator_is(nodes.get(cursor), ";") {
            return false;
        }
        if token_is(Some(&nodes[cursor]), "import") {
            return nodes[cursor..=index]
                .iter()
                .any(|node| token_is(Some(node), "syntax") || token_is(Some(node), "from"));
        }
        if cursor + 8 < index {
            return false;
        }
    }
    false
}

fn is_syntax_import_at(nodes: &[Syntax], index: usize) -> bool {
    if !token_at(nodes, index, "import") {
        return false;
    }
    // Only this statement. A later `export syntax` in the same module is not
    // part of the import, and treating it as one dropped the runtime import.
    nodes
        .iter()
        .skip(index)
        .take_while(|node| !operator_is(Some(node), ";"))
        .any(|node| token_is(Some(node), "syntax"))
}

fn syntax_import_at(nodes: &[Syntax], index: usize) -> Option<SyntaxImport> {
    let span = node_span(&nodes[index]);
    let mut names = Vec::new();
    let mut aliases = Vec::new();
    if let Some(Syntax::Group {
        children,
        delimiter: Delimiter::Brace,
        ..
    }) = nodes.get(index + 1)
    {
        let mut child = 0;
        while child < children.len() {
            if let Syntax::Token { raw, .. } = &children[child] {
                if is_ident_text(raw) && raw != "as" {
                    if token_at(children, child + 1, "as") {
                        if let Some(Syntax::Token { raw: local, .. }) = children.get(child + 2) {
                            aliases.push((raw.clone(), local.clone()));
                            names.push(local.clone());
                            child += 3;
                            continue;
                        }
                    }
                    names.push(raw.clone());
                }
            }
            if let Syntax::Group {
                delimiter: Delimiter::Paren,
                children: inner,
                ..
            } = &children[child]
            {
                let name = inner
                    .iter()
                    .filter_map(|node| match node {
                        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => {
                            Some(raw.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>();
                if !name.is_empty() {
                    names.push(name);
                }
            }
            child += 1;
        }
    }
    let specifier = nodes
        .iter()
        .skip(index)
        .take(12)
        .find_map(|node| match node {
            Syntax::String { raw, .. } => Some(raw.clone()),
            _ => None,
        })?;
    let shadows_core = nodes
        .iter()
        .skip(index)
        .take_while(|node| !operator_is(Some(node), ";"))
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| token_is(Some(pair[0]), "shadows") && token_is(Some(pair[1]), "core"));
    Some(SyntaxImport {
        names,
        shadows_core,
        aliases,
        specifier,
        span,
    })
}

fn syntax_import_end(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return node_end(&nodes[cursor]);
        }
        cursor += 1;
        if cursor > index + 16 {
            break;
        }
    }
    node_end(&nodes[index])
}

fn skip_syntax_import(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return cursor + 1;
        }
        cursor += 1;
        if cursor > index + 16 {
            break;
        }
    }
    index + 1
}

fn instantiate(
    source: &str,
    template: &[Tpl],
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
) -> String {
    instantiate_marked(source, template, captures, classes).0
}

fn instantiate_marked(
    source: &str,
    template: &[Tpl],
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
) -> (String, Vec<(usize, usize)>) {
    let _ = source;
    let mut out = String::new();
    let mut protected = Vec::new();
    render(&mut out, template, captures, classes, &mut protected);
    let start = out.len() - out.trim_start().len();
    let text = normalize_replacement(&out);
    let protected = protected
        .into_iter()
        .filter_map(|(from, to)| {
            let from = from.checked_sub(start)?;
            let to = to.checked_sub(start)?;
            if from >= text.len() {
                return None;
            }
            Some((from, to.min(text.len())))
        })
        .collect();
    (text, protected)
}

fn normalize_replacement(text: &str) -> String {
    text.trim().to_owned()
}

fn unknown_directives(template: &[Tpl]) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for part in template {
        match part {
            Tpl::Unknown { name, at } => found.push((name.clone(), *at)),
            Tpl::Metavar { .. } | Tpl::Trim { .. } | Tpl::Index => {}
            Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::Define(inner) => {
                found.extend(unknown_directives(inner));
            }
            Tpl::Let { value, body, .. } | Tpl::Parameterize { value, body, .. } => {
                found.extend(unknown_directives(value));
                found.extend(unknown_directives(body));
            }
            Tpl::Fold { init, body, .. } => {
                found.extend(unknown_directives(init));
                found.extend(unknown_directives(body));
            }
            Tpl::If {
                then_body,
                else_body,
                ..
            } => {
                found.extend(unknown_directives(then_body));
                if let Some(else_body) = else_body {
                    found.extend(unknown_directives(else_body));
                }
            }
            _ => {}
        }
    }
    found
}

fn template_is_core(template: &[Tpl]) -> bool {
    template.iter().any(|part| matches!(part, Tpl::Core(_)))
        && template.iter().all(|part| match part {
            Tpl::Core(_) => true,
            Tpl::Text(text) => text.trim().is_empty(),
            _ => false,
        })
}

fn introduced_of(source: &str, nodes: &[Syntax]) -> Vec<String> {
    let mut names = Vec::new();
    collect_introduced_source(source, nodes, &mut names);
    names
}

fn introduced_names(
    declared: &[String],
    rendered: &str,
    captures: &HashMap<String, Capture>,
) -> Vec<String> {
    let read = sweetener_reader::read_syntax(rendered);
    let mut names = introduced_of(rendered, &read.children);
    // An empty replacement introduced nothing. Falling back to the template
    // source would count binders inside `#syntax`, which belong to the
    // generated macro, not this expansion.
    if names.is_empty() && !rendered.trim().is_empty() {
        names.extend(declared.iter().cloned());
    }
    // A capture such as `$name` is the user's spelling. Renaming `Seconds`
    // because the template also binds it drops the quantity the rule created.
    let captured = capture_spellings(captures);
    // The template's own binder stays in the plan when a capture uses the
    // same spelling. The capture text is shielded, so only the binder moves.
    names.retain(|name| declared.iter().any(|item| item == name) || !captured.contains(name));
    // `#fresh` and `#let` already print a placeholder of their own.
    names.retain(|name| !super::sweetener_hygiene::is_placeholder(name));
    names
}

fn capture_spellings(captures: &HashMap<String, Capture>) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for value in captures.values() {
        collect_capture_idents(value, &mut names);
    }
    names
}

/// A syntax-class field such as `parameters.second` is the user's spelling.
/// Renaming it because the rendered `#core` signature also binds it changes
/// a parameter theirs keeps.
fn collect_capture_idents(value: &Capture, names: &mut std::collections::HashSet<String>) {
    match value {
        Capture::Text(text) => {
            let trimmed = text.trim();
            if is_ident_text(trimmed) {
                names.insert(trimmed.to_owned());
            } else if !trimmed.is_empty() {
                // `const doubled = value * 2;` captured as a statement is
                // the user's binder, wherever the template puts it.
                let read = sweetener_reader::read_syntax(trimmed);
                names.extend(introduced_of(trimmed, &read.children));
            }
        }
        Capture::Record { fields, text } => {
            let trimmed = text.trim();
            if is_ident_text(trimmed) {
                names.insert(trimmed.to_owned());
            }
            for field in fields.values() {
                collect_capture_idents(field, names);
            }
        }
        Capture::Many(rows) => {
            for row in rows {
                for (_, field) in row {
                    collect_capture_idents(field, names);
                }
            }
        }
    }
}

fn collect_introduced_source(source: &str, nodes: &[Syntax], names: &mut Vec<String>) {
    let _ = source;
    let mut index = 0;
    while index < nodes.len() {
        if dollar_at(nodes, index) {
            index += 2;
            continue;
        }
        if is_binder_keyword(nodes, index) {
            if let Some(name) = binder_name_after(nodes, index) {
                names.push(name);
            }
            // `const { cond: conditionInput, id } = input` binds the
            // pattern names. `id` is already a user spelling, so the
            // printed binding is `id_1`.
            if let Some(Syntax::Group {
                delimiter: Delimiter::Brace,
                children,
                ..
            }) = nodes.get(index + 1)
            {
                pattern_binders(children, names);
            }
        }
        if let Some(params) = type_param_names(nodes, index) {
            names.extend(params);
        }
        if let Syntax::Group {
            delimiter: Delimiter::Paren,
            children,
            ..
        } = &nodes[index]
        {
            // Arrow parameters and `map<A, B>(value, transform)`. The second
            // expansion must not reprint `value` once the first bound it.
            // `function add(left, right)` is not that list. The reader's
            // `=>` is `=` then `>`, so a following `>` is a type annotation
            // on an arrow, not `map<A, B>(...)`. `#core` keeps `right`.
            let after_type_params = index > 0
                && operator_is(nodes.get(index - 1), ">")
                && !operator_is(nodes.get(index.wrapping_sub(2)), "=");
            if paren_is_arrow_params(nodes, index) || after_type_params {
                names.extend(param_names(children));
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_introduced_source(source, children, names);
        }
        index += 1;
    }
}

/// True when `index` is an identifier inside the first type-parameter
/// list, `map<A, B>`, not a later argument such as `F<A>`.
fn in_type_params(nodes: &[Syntax], index: usize) -> bool {
    let mut depth = 0i32;
    let mut open = None;
    for (at, node) in nodes.iter().enumerate().take(index + 1) {
        if operator_is(Some(node), "<") {
            if depth == 0 {
                open = Some(at);
            }
            depth += 1;
        } else if operator_is(Some(node), ">") && depth > 0 {
            depth -= 1;
            if depth == 0 {
                open = None;
            }
        }
    }
    let Some(open) = open else {
        return false;
    };
    if depth == 0 || open == 0 {
        return false;
    }
    matches!(nodes.get(open - 1), Some(Syntax::Token { raw, .. }) if is_ident_text(raw))
        && type_param_names(nodes, open).is_some_and(|params| {
            nodes
                .get(index)
                .and_then(|node| match node {
                    Syntax::Token { raw, .. } => Some(raw),
                    _ => None,
                })
                .is_some_and(|raw| params.iter().any(|name| name == raw))
        })
}

/// `type F<Inner>` and `map<A, B>` introduce the names between `<` and `>`.
fn type_param_names(nodes: &[Syntax], index: usize) -> Option<Vec<String>> {
    if !operator_is(nodes.get(index), "<") {
        return None;
    }
    let mut names = Vec::new();
    let mut cursor = index + 1;
    let mut depth = 1i32;
    while cursor < nodes.len() && depth > 0 {
        if operator_is(nodes.get(cursor), "<") {
            depth += 1;
        } else if operator_is(nodes.get(cursor), ">") {
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if depth == 1 {
            if let Syntax::Token { raw, .. } = &nodes[cursor] {
                if is_ident_text(raw) && !is_type_name(raw) && raw != "extends" && raw != "keyof" {
                    names.push(raw.clone());
                }
            }
        }
        cursor += 1;
        if cursor > index + 24 {
            break;
        }
    }
    if names.is_empty() { None } else { Some(names) }
}

fn arrow_params(nodes: &[Syntax], index: usize) -> Option<Vec<String>> {
    let Syntax::Group {
        delimiter: Delimiter::Paren,
        children,
        ..
    } = nodes.get(index)?
    else {
        return None;
    };
    if !paren_is_arrow_params(nodes, index) {
        return None;
    }
    Some(param_names(children))
}

fn paren_is_arrow_params(nodes: &[Syntax], index: usize) -> bool {
    // `of: (value: number): Meters =>` — the arrow is a few siblings later.
    // A second paren before `=>` is another method, not this arrow.
    for node in nodes.iter().skip(index + 1).take(8) {
        if operator_is(Some(node), "=>") {
            return true;
        }
        if matches!(
            node,
            Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            } | Syntax::Group {
                delimiter: Delimiter::Brace,
                ..
            }
        ) {
            return false;
        }
    }
    false
}

fn param_names(children: &[Syntax]) -> Vec<String> {
    let mut names = Vec::new();
    let mut child = 0;
    while child < children.len() {
        if let Syntax::Token { raw, .. } = &children[child] {
            let after_colon = child > 0 && operator_is(children.get(child - 1), ":");
            if is_ident_text(raw) && !after_colon && !is_type_name(raw) {
                names.push(raw.clone());
            }
        }
        child += 1;
    }
    names
}

/// Render a `#syntax` body. A hole this invocation captured is
/// substituted. A hole of the macro being defined is written back as
/// `$name`, including `$name.field`.
fn render_keeping_holes(out: &mut String, template: &[Tpl], captures: &HashMap<String, Capture>) {
    for part in template {
        match part {
            Tpl::Text(text) => out.push_str(&substitute_text(text, captures)),
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) => {
                if capture_at(captures, name).is_some() {
                    let mut protected = Vec::new();
                    render(
                        out,
                        std::slice::from_ref(part),
                        captures,
                        &[],
                        &mut protected,
                    );
                } else {
                    out.push('$');
                    out.push_str(name);
                }
            }
            Tpl::Join {
                name,
                prefix,
                suffix,
                casing,
            } => {
                if capture_at(captures, name).is_some() {
                    let mut protected = Vec::new();
                    render(
                        out,
                        std::slice::from_ref(part),
                        captures,
                        &[],
                        &mut protected,
                    );
                } else {
                    out.push_str("#join($");
                    out.push_str(name);
                    if !prefix.is_empty() {
                        out.push_str(&format!(", prefix: \"{prefix}\""));
                    }
                    if !suffix.is_empty() {
                        out.push_str(&format!(", suffix: \"{suffix}\""));
                    }
                    if casing != "preserve" {
                        out.push_str(&format!(", casing: \"{casing}\""));
                    }
                    out.push(')');
                }
            }
            Tpl::Core(inner) => {
                out.push_str("#core(");
                render_keeping_holes(out, inner, captures);
                out.push(')');
            }
            Tpl::If {
                path,
                then_body,
                else_body,
            } => {
                out.push_str("#if(present $");
                out.push_str(path);
                out.push_str(") {");
                render_keeping_holes(out, then_body, captures);
                out.push('}');
                if let Some(else_body) = else_body {
                    out.push_str(" #else {");
                    render_keeping_holes(out, else_body, captures);
                    out.push('}');
                }
            }
            Tpl::Rep { inner, sep } => {
                render_keeping_rep(out, inner, sep.as_deref(), captures);
            }
            Tpl::Define(inner) => {
                out.push_str("#syntax {");
                render_keeping_holes(out, inner, captures);
                out.push('}');
            }
            Tpl::Metavar { hole, key } => {
                out.push('$');
                out.push_str(&metavar_hole(hole, key, captures));
            }
            Tpl::Trim { path } => {
                if capture_at(captures, path).is_some() {
                    let mut protected = Vec::new();
                    render(
                        out,
                        std::slice::from_ref(part),
                        captures,
                        &[],
                        &mut protected,
                    );
                } else {
                    out.push_str("#trim($");
                    out.push_str(path);
                    out.push(')');
                }
            }
            Tpl::Index => out.push_str("#index()"),
            Tpl::Let { name, value, body } => {
                out.push_str(&format!("#let({name} = "));
                render_keeping_holes(out, value, captures);
                out.push_str(") {");
                render_keeping_holes(out, body, captures);
                out.push('}');
            }
            Tpl::Fresh(hint) => out.push_str(&format!("#fresh(\"{hint}\")")),
            Tpl::Fold {
                path,
                init,
                acc,
                item,
                index,
                body,
            } => {
                out.push_str(&format!("#fold(${path}, init: {{"));
                render_keeping_holes(out, init, captures);
                out.push_str(&format!("}}) {{ (${acc}, ${item}, ${index}) => {{"));
                render_keeping_holes(out, body, captures);
                out.push_str("} }");
            }
            Tpl::Parameterize {
                param,
                required,
                value,
                body,
            } => {
                out.push_str("#parameterize(");
                if *required {
                    out.push_str("required ");
                }
                out.push_str(&format!("{param} = "));
                render_keeping_holes(out, value, captures);
                out.push_str(") {");
                render_keeping_holes(out, body, captures);
                out.push('}');
            }
            Tpl::Unknown { name, .. } => out.push_str(name),
        }
    }
}

/// A repetition inside `#syntax`. Rows this invocation already captured
/// are written out, so `$segment` becomes `lower`. A repetition that is a
/// hole of the macro being defined stays `$($name)*`.
fn render_keeping_rep(
    out: &mut String,
    inner: &[Tpl],
    sep: Option<&str>,
    captures: &HashMap<String, Capture>,
) {
    let sep_was_inferred = sep.is_none();
    let (inner, sep) = rep_layout(inner, sep);
    let (path, rows) = rep_rows_for(&inner, captures);
    if rows.is_empty() {
        out.push_str("$(");
        render_keeping_holes(out, &inner, captures);
        out.push(')');
        if let Some(sep) = &sep {
            out.push_str(sep);
        }
        out.push('*');
        return;
    }
    for (index, row) in rows.iter().enumerate() {
        push_row_sep(out, index, row, sep_was_inferred, sep.as_deref());
        let scoped = with_rep_index(rep_row_scope(captures, &path, row), index);
        render_keeping_holes(out, &inner, &scoped);
    }
}

fn with_rep_index(mut scoped: HashMap<String, Capture>, index: usize) -> HashMap<String, Capture> {
    // A nested `$($parameter)*` is its own sequence. The outer row stored
    // every iteration of that sequence, so a hole such as `#metavar` read
    // the first parameter (`minimum`) for `lower` and for `upper`. Keep
    // only the row this iteration captured.
    retain_row_sequences(&mut scoped);
    scoped.insert("__rep_index".to_owned(), Capture::Text(index.to_string()));
    scoped
}

fn retain_row_sequences(scoped: &mut HashMap<String, Capture>) {
    let keys: Vec<String> = scoped
        .iter()
        .filter(|(_, value)| matches!(value, Capture::Many(rows) if rows.len() > 1))
        .map(|(key, _)| key.clone())
        .collect();
    for key in keys {
        let Some(Capture::Many(rows)) = scoped.get(&key).cloned() else {
            continue;
        };
        let Some(one) = rows.into_iter().find(|row| row_belongs_here(scoped, row)) else {
            continue;
        };
        scoped.insert(key, Capture::Many(vec![one]));
    }
}

/// True when this nested row is the iteration already in scope. A row of
/// `$parameter` carries `minimum` only while the segment is `lower`.
/// Text that also sits on another row (`number` for both parameters) does
/// not decide: the row has to name this segment (`upper`), not the first.
fn row_belongs_here(scoped: &HashMap<String, Capture>, row: &[(String, Capture)]) -> bool {
    let texts: Vec<(&str, &str)> = row
        .iter()
        .filter_map(|(key, value)| match value {
            Capture::Text(text) => Some((key.as_str(), text.trim())),
            _ => None,
        })
        .filter(|(_, text)| !text.is_empty())
        .collect();
    if texts.is_empty() {
        return false;
    }
    // The outer iteration already bound `segment` to `upper`. A nested row
    // that still says `lower` belongs to the other segment, even when its
    // parameter type (`number`) matches this one too.
    if texts.iter().any(|(key, text)| {
        capture_text(scoped, key).is_some_and(|have| have != *text && !have.is_empty())
    }) {
        return false;
    }
    texts.iter().any(|(key, text)| {
        let Some(have) = capture_text(scoped, key) else {
            return false;
        };
        if have != *text {
            return false;
        }
        // A value every sibling row shares does not pick this iteration.
        texts.iter().filter(|(_, own)| own == text).count() == 1 || texts.len() == 1
    })
}

/// `#let(name = value) { body }`: the body sees a fresh binder for `name`.
/// A body that awaits or yields cannot move into a function of its own, so
/// it assigns a binder declared at the top of the enclosing function.
fn render_let(
    out: &mut String,
    name: &str,
    value: &[Tpl],
    body: &[Tpl],
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
    protected: &mut Vec<(usize, usize)>,
) {
    let binder = super::sweetener_hygiene::placeholder(name);
    let mut bound = String::new();
    render(&mut bound, value, captures, classes, &mut Vec::new());
    let mut text = String::new();
    let mut spans = Vec::new();
    super::sweetener_scoped::push_let(name, &binder);
    render(&mut text, body, captures, classes, &mut spans);
    super::sweetener_scoped::pop_let();
    let (text, spans) = super::sweetener_scoped::rename_outside(&text, &spans, name, &binder);
    let text_trimmed = text.trim();
    let lead = text.len() - text.trim_start().len();
    let suspends = super::sweetener_scoped::suspends(text_trimmed);
    let head = if suspends {
        format!("({binder} = {}, ", bound.trim())
    } else {
        format!("(({binder}) => ")
    };
    out.push_str(&head);
    let base = out.len();
    let object = !suspends && text_trimmed.starts_with('{');
    if object {
        out.push('(');
    }
    let shift = out.len() - base;
    out.push_str(text_trimmed);
    for (from, to) in spans {
        if from >= lead {
            protected.push((
                base + shift + from - lead,
                base + shift + (to - lead).min(text_trimmed.len()),
            ));
        }
    }
    if object {
        out.push(')');
    }
    if suspends {
        out.push_str(&super::sweetener_scoped::hoist_marker(&binder));
        out.push(')');
    } else {
        out.push_str(&format!(")({})", bound.trim()));
    }
}

/// Each element of the sequence makes the next accumulator from the last;
/// an empty sequence leaves the initial one. A spliced accumulator is one
/// expression, so it is parenthesized unless it is a single primary.
fn render_fold(
    out: &mut String,
    (path, init, acc, item, index, body): (&str, &[Tpl], &str, &str, &str, &[Tpl]),
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
) {
    let mut current = String::new();
    render(&mut current, init, captures, classes, &mut Vec::new());
    let mut current = current.trim().to_owned();
    let rows = capture_sequence(captures, path).unwrap_or_default();
    let root = path.rsplit('.').next().unwrap_or(path);
    for (position, row) in rows.iter().enumerate() {
        let mut scoped = rep_row_scope(captures, path, row);
        let element = row
            .iter()
            .find(|(name, _)| name == root)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| Capture::Record {
                text: String::new(),
                fields: row.iter().cloned().collect(),
            });
        scoped.insert(item.to_owned(), element);
        let spliced = if is_primary_text(&current) {
            current.clone()
        } else {
            format!("({current})")
        };
        scoped.insert(acc.to_owned(), Capture::Text(spliced));
        scoped.insert(index.to_owned(), Capture::Text(position.to_string()));
        scope_record_fields(&mut scoped);
        let mut next = String::new();
        render(&mut next, body, &scoped, classes, &mut Vec::new());
        current = next.trim().to_owned();
    }
    out.push_str(&current);
}

/// One identifier, literal, or closed group: safe to splice anywhere.
/// Upstream's `maxNestingDepth` is 1024 syntax levels; a nested printer is
/// far heavier than one level, so ours bounds replacement re-expansion.
const MAX_NESTING_DEPTH: usize = 64;

thread_local! {
    /// Fingerprints of the replacements being expanded, outermost first.
    static ACTIVE_EXPANSIONS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The first budget diagnostic of the current file-level invocation.
    static BUDGET: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// This file's macros whose templates parameterize a name that is not a
    /// syntax parameter where they are defined, with that name.
    static UNKNOWN_PARAMETERIZED: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
    /// Where this file defines each of its own macros.
    static LOCAL_HEADS: std::cell::RefCell<HashMap<String, u32>> =
        std::cell::RefCell::new(HashMap::new());
}

/// A macro some rule of which is just its name (`syntax count` read as a
/// value) accepts the name on its own.
fn compiled_accepts_bare(definitions: &[&Macro]) -> bool {
    definitions
        .iter()
        .any(|macro_| macro_.rules.iter().any(|rule| rule.pattern.len() <= 1))
}

thread_local! {
    /// Macro modules whose rules this pass instantiated.
    static INVOKED_OWNERS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// Set for a pass that knows which modules were invoked: only their
    /// runtime imports are printed.
    static OWNER_FILTER: std::cell::RefCell<Option<std::collections::HashSet<String>>> =
        const { std::cell::RefCell::new(None) };
    /// Every module some runtime import of this pass belongs to.
    static IMPORT_OWNERS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

fn note_invoked(owner: Option<&str>) {
    if let Some(owner) = owner {
        INVOKED_OWNERS.with(|set| {
            set.borrow_mut().insert(owner.to_owned());
        });
    }
}

fn note_budget(message: String) {
    BUDGET.with(|budget| {
        budget.borrow_mut().get_or_insert(message);
    });
}

fn is_primary_text(text: &str) -> bool {
    let read = sweetener_reader::read_syntax(text);
    match read.children.as_slice() {
        [Syntax::Token { .. } | Syntax::String { .. }] => true,
        [Syntax::Group { delimiter, .. }] => *delimiter == Delimiter::Paren,
        _ => false,
    }
}

fn render(
    out: &mut String,
    template: &[Tpl],
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
    protected: &mut Vec<(usize, usize)>,
) {
    let template = trim_template(template);
    if let Err(message) = super::sweetener_budget::charge(
        super::sweetener_budget::Kind::TemplateSteps,
        u64::try_from(template.len()).unwrap_or(u64::MAX),
    ) {
        note_budget(message);
        return;
    }
    for (index, part) in template.iter().enumerate() {
        match part {
            Tpl::Text(text) => {
                let next_rest = template.get(index + 1).is_some_and(|next| {
                    matches!(next, Tpl::Rep { inner, sep: None } if inner.iter().any(|part| matches!(part, Tpl::Var(name) if name == "rest")))
                });
                let text = if next_rest && text.trim_end().ends_with(',') {
                    text.trim_end().trim_end_matches(',').to_owned()
                } else {
                    text.clone()
                };
                let base = out.len();
                let (substituted, spans) = substitute_text_protecting(&text, captures);
                for (from, to) in spans {
                    protected.push((base + from, base + to));
                }
                out.push_str(&substituted);
            }
            Tpl::Var(name) => {
                if let Some(text) = capture_text(captures, name) {
                    let start = out.len();
                    out.push_str(text.trim());
                    protected.push((start, out.len()));
                } else if let Some((root, rest)) = name.split_once('.') {
                    // `$protocol.register` is a text capture plus a member.
                    // A field path such as `$method.parameters.name` is not.
                    if matches!(captures.get(root), Some(Capture::Text(_))) {
                        if let Some(text) = capture_text(captures, root) {
                            let start = out.len();
                            out.push_str(text.trim());
                            protected.push((start, out.len()));
                            out.push('.');
                            out.push_str(rest);
                        }
                    }
                }
            }
            Tpl::Quote(name) => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(&string_literal(text.trim()));
                }
            }
            Tpl::Core(inner) => {
                // `#core` is ordinary TypeScript. Do not wrap `new`, calls,
                // or types in extra parentheses. The capture compare treats
                // those as the same shape as theirs.
                render(out, inner, captures, classes, protected);
            }
            Tpl::Join {
                name,
                prefix,
                suffix,
                casing,
            } => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(&join_identifier(prefix, text.trim(), suffix, casing));
                }
            }
            Tpl::Count(name) => out.push_str(&capture_count(captures, name).to_string()),
            Tpl::If {
                path,
                then_body,
                else_body,
            } => {
                if capture_present(captures, path) {
                    render(out, then_body, captures, classes, protected);
                } else if let Some(else_body) = else_body {
                    render(out, else_body, captures, classes, protected);
                }
            }
            Tpl::Define(_) => {}
            Tpl::Let { name, value, body } => {
                render_let(out, name, value, body, captures, classes, protected)
            }
            Tpl::Fresh(hint) => out.push_str(&super::sweetener_hygiene::placeholder(hint)),
            Tpl::Fold {
                path,
                init,
                acc,
                item,
                index,
                body,
            } => render_fold(out, (path, init, acc, item, index, body), captures, classes),
            Tpl::Parameterize {
                param,
                required,
                value,
                body,
            } => {
                let mut bound = String::new();
                render(&mut bound, value, captures, classes, &mut Vec::new());
                // `#parameterize(% = topic)` inside `#let(topic = ...)` names
                // the let's binder.
                let bound = super::sweetener_scoped::let_binder_in(bound.trim());
                let (open, close) =
                    super::sweetener_scoped::parameter_markers(param, &bound, *required);
                out.push_str(&open);
                render(out, body, captures, classes, protected);
                out.push_str(&close);
            }
            Tpl::Unknown { .. } => {}
            Tpl::Metavar { hole, key } => {
                let name = metavar_hole(hole, key, captures);
                if let Some(text) =
                    capture_text(captures, &name).or_else(|| capture_text(captures, hole))
                {
                    let start = out.len();
                    out.push_str(text.trim());
                    protected.push((start, out.len()));
                }
            }
            Tpl::Trim { path } => {
                if let Some(text) = capture_text(captures, path) {
                    out.push_str(text.trim());
                }
            }
            Tpl::Index => {
                if let Some(text) = capture_text(captures, "__rep_index") {
                    out.push_str(text.trim());
                }
            }
            Tpl::Rep { inner, sep } => {
                render_rep(out, inner, sep.as_deref(), captures, classes, protected);
            }
        }
    }
}

/// `$($state.name: { $($state.moves.event: #text($state.moves.target)),* };)*`
/// walks one sequence per repetition. The sequence is the longest path
/// this repetition itself names. Nested repetitions are rendered with
/// that one record in scope, so `moves` is this state's moves.
fn render_rep(
    out: &mut String,
    inner: &[Tpl],
    sep: Option<&str>,
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
    protected: &mut Vec<(usize, usize)>,
) {
    let inferred = sep.is_none();
    let (inner, sep) = rep_layout(inner, sep);
    let (path, rows) = rep_rows_for(&inner, captures);
    for (index, row) in rows.iter().enumerate() {
        push_row_sep(out, index, row, inferred, sep.as_deref());
        let scoped = with_rep_index(rep_row_scope(captures, &path, row), index);
        render(out, &inner, &scoped, classes, protected);
    }
}

/// Row key holding the source text between a separator-less repetition
/// step and the one before it.
const REP_GAP: &str = "\u{1}gap";

/// `$($rest:tt,)+` writes the comma inside the repetition. Rendering it
/// there glues `), ,` onto the next step. The comma is the separator.
fn rep_layout(inner: &[Tpl], sep: Option<&str>) -> (Vec<Tpl>, Option<String>) {
    if sep.is_none()
        && inner
            .iter()
            .any(|part| matches!(part, Tpl::Var(name) if name == "rest"))
    {
        // `$($rest)+` steps are separated by commas at the call site.
        // The template copied no separator, so the steps would join.
        (inner.to_vec(), Some(", ".to_owned()))
    } else if sep.is_none()
        && inner
            .last()
            .is_some_and(|part| matches!(part, Tpl::Text(text) if text.trim() == ","))
    {
        (inner[..inner.len() - 1].to_vec(), Some(",".to_owned()))
    } else {
        (inner.to_vec(), sep.map(str::to_owned))
    }
}

/// The separator in front of repetition row `index`. A template that wrote
/// no separator joins steps the way the source did: a comma stays a comma,
/// whitespace stays a space, and adjacent tokens (`x` `;`) stay adjacent.
fn push_row_sep(
    out: &mut String,
    index: usize,
    row: &[(String, Capture)],
    inferred: bool,
    sep: Option<&str>,
) {
    let gap = row.iter().find_map(|(name, capture)| match capture {
        Capture::Text(text) if name == REP_GAP => Some(text.as_str()),
        _ => None,
    });
    match gap {
        Some(gap) if index > 0 && inferred => {
            if gap.contains(',') {
                out.push_str(", ");
            } else if !gap.is_empty() {
                out.push(' ');
            }
        }
        _ => push_sep(out, index, sep),
    }
}

fn push_sep(out: &mut String, index: usize, sep: Option<&str>) {
    if index == 0 {
        return;
    }
    if let Some(sep) = sep {
        out.push_str(sep);
        if sep == "," {
            out.push(' ');
        }
    }
}

/// The sequence a repetition walks and the row for each element. The path
/// is the longest one this repetition names. `$($state.name: {
/// $($state.moves.event),* })*` has no variable of its own on the outer
/// row only when this is the inner row; then the nested path is the
/// sequence.
fn rep_rows_for(
    inner: &[Tpl],
    captures: &HashMap<String, Capture>,
) -> (String, Vec<Vec<(String, Capture)>>) {
    let mut own = Vec::new();
    for part in inner {
        match part {
            Tpl::Var(name)
            | Tpl::Quote(name)
            | Tpl::Count(name)
            | Tpl::Join { name, .. }
            | Tpl::Trim { path: name }
            | Tpl::Metavar { key: name, .. } => {
                if !own.contains(name) {
                    own.push(name.clone());
                }
            }
            _ => {}
        }
    }
    if own.is_empty() {
        collect_vars(inner, &mut own);
    }
    let mut path = own
        .iter()
        .max_by_key(|name| name.len())
        .cloned()
        .unwrap_or_default();
    let mut rows = Vec::new();
    while !path.is_empty() {
        if let Some(found) = capture_sequence(captures, &path) {
            if !found.is_empty() {
                rows = found;
                break;
            }
            // `None()` has an empty `fields` sequence. Its rows are none;
            // walking up to `constructor` would repeat the outer rows.
            return (path, Vec::new());
        }
        path = path
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_owned())
            .unwrap_or_default();
    }
    if rows.is_empty() {
        rows = rep_rows(inner, captures);
    }
    (path, rows)
}

fn rep_row_scope(
    captures: &HashMap<String, Capture>,
    path: &str,
    row: &[(String, Capture)],
) -> HashMap<String, Capture> {
    let mut scoped = HashMap::new();
    for (key, value) in captures {
        // Keep bindings that are not this sequence. A field named `name`
        // is stored on its dotted path; the root `name` stays the protocol.
        if key == path || key == path.split('.').next().unwrap_or(path) {
            continue;
        }
        if path.starts_with(&format!("{key}.")) {
            continue;
        }
        scoped.insert(key.clone(), value.clone());
    }
    for (name, value) in scope_rep_row(path, row) {
        // A later row of the same record (`column.kind`) must not replace
        // the record that already has the repeated fields (`modifiers`).
        if let Some(Capture::Record { fields: have, .. }) = scoped.get(&name) {
            if let Capture::Record { fields: next, .. } = &value {
                if next.len() < have.len() {
                    continue;
                }
            }
        }
        scoped.insert(name, value);
    }
    scope_record_fields(&mut scoped);
    scoped
}

fn scope_record_fields(scoped: &mut HashMap<String, Capture>) {
    let records: Vec<(String, Capture)> = scoped
        .iter()
        .filter(|(_, value)| matches!(value, Capture::Record { .. }))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in records {
        let Capture::Record { fields, .. } = value else {
            continue;
        };
        for (field, child) in fields {
            let dotted = format!("{key}.{field}");
            // Do not overwrite a binding already called `name`. The protocol
            // name and a method's `name` field are different captures.
            if matches!(child, Capture::Text(_))
                && !scoped.contains_key(&field)
                && !key.ends_with(&format!(".{field}"))
                && key != field
            {
                // A field whose leaf is already a root binding (`name`) stays
                // on the dotted path. Inserting it as `name` hides the protocol.
                if field != "name" {
                    scoped.insert(field.clone(), child.clone());
                }
            }
            scoped.insert(dotted, child);
        }
    }
}

fn trim_template(template: &[Tpl]) -> &[Tpl] {
    let mut start = 0;
    let mut end = template.len();
    while start < end {
        match &template[start] {
            Tpl::Text(text) if text.trim().is_empty() => start += 1,
            _ => break,
        }
    }
    while end > start {
        match &template[end - 1] {
            Tpl::Text(text) if text.trim().is_empty() => end -= 1,
            _ => break,
        }
    }
    &template[start..end]
}

fn substitute_text(text: &str, captures: &HashMap<String, Capture>) -> String {
    substitute_text_protecting(text, captures).0
}

fn substitute_text_protecting(
    text: &str,
    captures: &HashMap<String, Capture>,
) -> (String, Vec<(usize, usize)>) {
    if !text.contains('$') && !text.contains("#text") {
        return (text.to_owned(), Vec::new());
    }
    if text.contains("${") || text.contains('`') {
        return (substitute_template_literal(text, captures), Vec::new());
    }
    let read = sweetener_reader::read_syntax(text);
    let mut out = String::new();
    let mut protected = Vec::new();
    let mut cursor = 0usize;
    substitute_nodes(
        text,
        &read.children,
        captures,
        &mut out,
        &mut cursor,
        &mut protected,
    );
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    (out, protected)
}

fn substitute_nodes(
    source: &str,
    nodes: &[Syntax],
    captures: &HashMap<String, Capture>,
    out: &mut String,
    cursor: &mut usize,
    protected: &mut Vec<(usize, usize)>,
) {
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "#text") && group_is(nodes.get(index + 1), Delimiter::Paren) {
            if let Some(name) = text_hole_name(&nodes[index + 1]) {
                if let Some(value) = capture_text(captures, &name) {
                    push_until(source, out, cursor, node_start(&nodes[index]));
                    let start = out.len();
                    out.push_str(&string_literal(value.trim()));
                    protected.push((start, out.len()));
                    *cursor = node_end(&nodes[index + 1]);
                    index += 2;
                    continue;
                }
            }
        }
        if dollar_at(nodes, index) {
            if let Some((path, end_at, next)) = read_var_path(nodes, index) {
                // `$protocol.register` is a capture followed by a member.
                // The member is not part of the path.
                let value = capture_text(captures, &path).or_else(|| {
                    path.split('.')
                        .next()
                        .and_then(|root| capture_text(captures, root))
                });
                let member = if capture_text(captures, &path).is_none() {
                    path.split_once('.').map(|(_, rest)| rest.to_owned())
                } else {
                    None
                };
                let (end_at, next) = if member.is_some() {
                    let root = path.split('.').next().unwrap_or(path.as_str());
                    (node_start(&nodes[index]) + 1 + root.len(), index + 2)
                } else {
                    (end_at, next)
                };
                if let Some(value) = value {
                    let value = if let Some(rest) = member {
                        format!("{}.{rest}", value.trim())
                    } else {
                        value.trim().to_owned()
                    };
                    push_until(source, out, cursor, node_start(&nodes[index]));
                    let start = out.len();
                    out.push_str(value.trim());
                    protected.push((start, out.len()));
                    *cursor = end_at;
                    index = next;
                    continue;
                }
            } else if let Some(Syntax::Token { span, raw, .. }) = nodes.get(index + 1) {
                if let Some(value) = capture_text(captures, raw) {
                    push_until(source, out, cursor, node_start(&nodes[index]));
                    let start = out.len();
                    out.push_str(value.trim());
                    protected.push((start, out.len()));
                    *cursor = span.end as usize;
                    index += 2;
                    continue;
                }
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            substitute_nodes(source, children, captures, out, cursor, protected);
        }
        if let Syntax::String { span, raw, .. } = &nodes[index] {
            if raw.contains('$') || raw.contains("#text") {
                push_until(source, out, cursor, span.start as usize);
                let substituted = substitute_template_literal(raw, captures);
                let start = out.len();
                out.push_str(&substituted);
                protected.push((start, out.len()));
                *cursor = span.end as usize;
            }
        }
        index += 1;
    }
}

fn substitute_template_literal(raw: &str, captures: &HashMap<String, Capture>) -> String {
    let mut out = String::new();
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'#' && raw[index..].starts_with("#text") {
            let after = index + "#text".len();
            if bytes.get(after) == Some(&b'(') {
                if let Some(end) = matching_paren(&raw[after + 1..]) {
                    let body = raw[after + 1..after + 1 + end].trim();
                    let name = body.strip_prefix('$').unwrap_or(body);
                    if let Some(value) = capture_text(captures, name) {
                        out.push_str(&string_literal(value.trim()));
                        index = after + 1 + end + 1;
                        continue;
                    }
                }
            }
        }
        if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'(') {
            if let Some(end) = matching_paren(&raw[index + 2..]) {
                let body = &raw[index + 2..index + 2 + end];
                let after = index + 2 + end + 1;
                let (sep, quant_end) = template_rep_tail(&raw[after..]);
                let repeated = render_text_rep(body, sep.as_deref(), captures);
                if !repeated.is_empty() || quant_end > 0 {
                    out.push_str(&repeated);
                    index = after + quant_end;
                    continue;
                }
            }
        }
        if bytes[index] == b'$' {
            let rest = &raw[index + 1..];
            let name: String = rest
                .chars()
                .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric() || *ch == '.')
                .collect();
            if !name.is_empty() && name != "." {
                if let Some(value) = capture_text(captures, &name) {
                    out.push_str(value.trim());
                    index += 1 + name.len();
                    continue;
                }
            }
        }
        let ch = raw[index..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// `,$` or `*` after `$(...)` inside a template literal.
fn template_rep_tail(after: &str) -> (Option<String>, usize) {
    let bytes = after.as_bytes();
    if bytes.first() == Some(&b',') && bytes.get(1) == Some(&b'*') {
        return (Some(",".to_owned()), 2);
    }
    if bytes.first() == Some(&b'*') || bytes.first() == Some(&b'+') {
        return (None, 1);
    }
    (None, 0)
}

fn render_text_rep(body: &str, sep: Option<&str>, captures: &HashMap<String, Capture>) -> String {
    let read = sweetener_reader::read_syntax(body);
    let inner = tpls_from(&read.children, String::new());
    let rows = rep_rows(&inner, captures);
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            if let Some(sep) = sep {
                out.push_str(sep);
                if sep == "," {
                    out.push(' ');
                }
            }
        }
        let mut scoped = captures.clone();
        for (name, value) in row {
            scoped.insert(name.clone(), value.clone());
        }
        out.push_str(&substitute_template_literal(body, &scoped));
    }
    out
}

fn matching_paren(after: &str) -> Option<usize> {
    let mut depth = 1i32;
    let mut chars = after.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            '"' | '\'' | '`' => {
                let quote = ch;
                while let Some((_, inner)) = chars.next() {
                    if inner == '\\' {
                        let _ = chars.next();
                        continue;
                    }
                    if inner == quote {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_brace(after: &str) -> Option<usize> {
    let mut depth = 1i32;
    let mut chars = after.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            '"' | '\'' | '`' => {
                let quote = ch;
                while let Some((_, inner)) = chars.next() {
                    if inner == '\\' {
                        let _ = chars.next();
                        continue;
                    }
                    if inner == quote {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn capture_pieces(captures: &HashMap<String, Capture>, name: &str) -> Vec<String> {
    match captures.get(name) {
        Some(Capture::Many(rows)) => rows
            .iter()
            .filter_map(|row| {
                row.iter()
                    .find(|(key, _)| key == name)
                    .or_else(|| row.first())
                    .and_then(|(_, value)| capture_spelling(value))
                    .map(str::to_owned)
            })
            .collect(),
        _ => capture_text(captures, name)
            .map(|text| vec![text.to_owned()])
            .unwrap_or_default(),
    }
}

fn capture_text<'a>(captures: &'a HashMap<String, Capture>, name: &str) -> Option<&'a str> {
    // `$name` is the root binding. A scoped field also called `name`
    // belongs to `$method.name`, not to the protocol.
    if !name.contains('.') {
        if let Some(Capture::Text(text)) = captures.get(name) {
            if !text.trim().is_empty() {
                return Some(text.as_str());
            }
        }
    }
    // A repetition scopes `state.name` as text on that path. That wins
    // over the parent record. A different field that also ends in `name`
    // (`method.name`) must not fill `method.parameters.name`.
    if name.contains('.') {
        if let Some(Capture::Text(text)) = captures.get(name) {
            if !text.trim().is_empty() {
                return Some(text.as_str());
            }
        }
        if let Some((parent, leaf)) = name.rsplit_once('.') {
            if matches!(captures.get(parent), Some(Capture::Record { .. })) {
                if let Some(Capture::Text(text)) = captures.get(leaf) {
                    if !text.trim().is_empty() {
                        return Some(text.as_str());
                    }
                }
            }
        }
    }
    let value = capture_at(captures, name)?;
    if name.contains('.') {
        if let Capture::Text(text) = value {
            return Some(text.as_str());
        }
        if let Capture::Record { fields, .. } = value {
            if let Some(leaf) = name.rsplit('.').next() {
                if let Some(field) = fields.get(leaf) {
                    return capture_spelling(field);
                }
            }
        }
    }
    capture_spelling_key(value, name)
}

fn capture_spelling(value: &Capture) -> Option<&str> {
    capture_spelling_key(value, "")
}

fn capture_spelling_key<'a>(value: &'a Capture, key: &str) -> Option<&'a str> {
    match value {
        Capture::Text(text) => Some(text.as_str()),
        Capture::Record { fields, text } => {
            // A field path that stopped on the record wants the field, not
            // the whole form. Callers that want the form use the text only
            // when no field was asked. This branch is the form itself.
            let _ = fields;
            Some(text.as_str())
        }
        Capture::Many(rows) => {
            let leaf = key.rsplit('.').next().unwrap_or(key);
            rows.iter().find_map(|row| row_spelling(row, leaf))
        }
    }
}

/// The text this row stores under `key`. A nested repetition keeps this
/// row's own sequence under the same name (`parameter` -> `maximum`) and
/// also copies the sibling sequence (`parameterType` -> `number`). Walking
/// every nested sequence stamps the hole with the type. A sibling text on
/// the row is the outer segment (`lower`), which is not the parameter.
fn row_spelling<'a>(row: &'a [(String, Capture)], key: &str) -> Option<&'a str> {
    if !key.is_empty() {
        if let Some((_, value)) = row.iter().find(|(name, _)| name == key) {
            return match value {
                Capture::Text(text) => Some(text.as_str()),
                Capture::Many(inner) => inner
                    .iter()
                    .find_map(|inner_row| row_spelling(inner_row, key)),
                Capture::Record { text, .. } => Some(text.as_str()),
            };
        }
    }
    row.iter().find_map(|(_, value)| match value {
        Capture::Text(text) => Some(text.as_str()),
        _ => None,
    })
}

fn capture_present(captures: &HashMap<String, Capture>, path: &str) -> bool {
    match capture_at(captures, path) {
        Some(Capture::Many(rows)) => !rows.is_empty(),
        Some(Capture::Text(text)) => !text.trim().is_empty(),
        Some(Capture::Record { .. }) => true,
        None => false,
    }
}

/// `$state.name` and `$state.moves.event` walk record fields. A repeated
/// field keeps the repetition so the template row splitter can see it.
fn capture_at<'a>(captures: &'a HashMap<String, Capture>, path: &str) -> Option<&'a Capture> {
    let (root, rest) = path.split_once('.').unwrap_or((path, ""));
    // The row stores `$column.modifiers` both inside the record and as its
    // own key. The record copy can be a later row that dropped the repeated
    // field. The direct key is the one this path names.
    if captures.contains_key(path) {
        return captures.get(path);
    }
    let mut value = captures.get(root)?;
    if rest.is_empty() {
        return Some(value);
    }
    for field in rest.split('.') {
        value = match value {
            Capture::Record { fields, .. } => fields.get(field)?,
            Capture::Text(_) => {
                // `$protocol.register` is a text capture plus a member.
                // The member is not stored on the text, so this path is
                // absent and the caller keeps `.register` in the output.
                return captures
                    .get(path)
                    .filter(|found| matches!(found, Capture::Text(_)));
            }
            Capture::Many(rows) => {
                // `$column.modifiers` is a sequence of records. The next
                // field (`primary`) is on this row's record. `rest` is the
                // whole path after `column`, so skip the fields already
                // walked. A path that stops at the sequence is
                // `capture_sequence`'s.
                let walked = rest.split('.').position(|part| part == field).unwrap_or(0);
                let rest_fields: Vec<&str> = rest.split('.').skip(walked + 1).collect();
                if rest_fields.is_empty() {
                    return None;
                }
                return rows.iter().find_map(|row| {
                    let record = row.iter().find_map(|(_, element)| match element {
                        Capture::Record { fields, .. } => Some(fields),
                        _ => None,
                    })?;
                    let mut child = record.get(rest_fields[0])?;
                    for field in &rest_fields[1..] {
                        child = match child {
                            Capture::Record { fields, .. } => fields.get(*field)?,
                            _ => return None,
                        };
                    }
                    Some(child)
                });
            }
        };
    }
    Some(value)
}

/// `$state.moves` while `state` is a sequence of records. Each row contributes
/// that field, so a nested repetition sees every move, not the first state's.
fn capture_sequence(
    captures: &HashMap<String, Capture>,
    path: &str,
) -> Option<Vec<Vec<(String, Capture)>>> {
    if let Some(Capture::Many(rows)) = captures.get(path) {
        return Some(rows.clone());
    }
    // `$segment.marker` names a field of the `segment` sequence. Walk to the
    // nearest ancestor that is a sequence, so a repetition over the marked
    // segments still sees the literal ones (`users`, `posts`).
    let mut parent = path;
    while let Some((head, _)) = parent.rsplit_once('.') {
        if let Some(Capture::Many(rows)) = captures.get(head) {
            return Some(rows.clone());
        }
        parent = head;
    }
    let (root, rest) = path.split_once('.')?;
    let mut value = captures.get(root)?.clone();
    let mut walked = root.to_owned();
    for field in rest.split('.') {
        walked.push('.');
        walked.push_str(field);
        // A dotted path walks the record already in hand. A stored sequence
        // on this path is only the answer when that record has no such field.
        if let Capture::Record { fields, .. } = &value {
            if let Some(child) = fields.get(field) {
                value = child.clone();
                continue;
            }
        }
        if let Some(Capture::Many(rows)) = captures.get(&walked) {
            value = Capture::Many(rows.clone());
            continue;
        }
        value = match value {
            Capture::Record { fields, .. } => fields.get(field)?.clone(),
            Capture::Many(rows) => {
                let projected: Vec<Vec<(String, Capture)>> = rows
                    .iter()
                    .filter_map(|row| {
                        let record = row.iter().find_map(|(_, element)| match element {
                            Capture::Record { .. } => Some(element.clone()),
                            _ => None,
                        });
                        let element = row.iter().find_map(|(_, element)| match element {
                            Capture::Record { fields, .. } => fields.get(field).cloned(),
                            other if row.len() == 1 => Some(other.clone()),
                            _ => None,
                        })?;
                        let mut one = vec![(field.to_owned(), element)];
                        // The field text is what this path names. The record
                        // stays so a nested field (`moves`) is still reachable.
                        if let Some(record) = record {
                            one.push(("".to_owned(), record));
                        }
                        Some(one)
                    })
                    .collect();
                if projected.is_empty() {
                    return None;
                }
                Capture::Many(projected)
            }
            Capture::Text(_) => return None,
        };
    }
    match value {
        Capture::Many(rows) => Some(rows),
        other => Some(vec![vec![(
            path.rsplit('.').next().unwrap_or(path).to_owned(),
            other,
        )]]),
    }
}

fn capture_count(captures: &HashMap<String, Capture>, name: &str) -> usize {
    match capture_at(captures, name) {
        Some(Capture::Many(rows)) => rows.len(),
        Some(Capture::Text(text)) if !text.trim().is_empty() => 1,
        Some(Capture::Record { .. }) => 1,
        _ => 0,
    }
}

fn join_identifier(prefix: &str, captured: &str, suffix: &str, casing: &str) -> String {
    let cased = match casing {
        "upper-first" => {
            let mut chars = captured.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
        "lower-first" => {
            let mut chars = captured.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_lowercase(), chars.as_str()),
                None => String::new(),
            }
        }
        "upper" => captured.to_uppercase(),
        "lower" => captured.to_lowercase(),
        _ => captured.to_owned(),
    };
    format!("{prefix}{cased}{suffix}")
}

fn rep_rows(inner: &[Tpl], captures: &HashMap<String, Capture>) -> Vec<Vec<(String, Capture)>> {
    let mut names = Vec::new();
    collect_vars(inner, &mut names);
    // A nested field (`$state.moves.event`) repeats on the sequence it
    // walks through, not on the leaf. The longest path that is a sequence
    // is that repetition.
    let mut best: Option<(String, Vec<Vec<(String, Capture)>>, usize)> = None;
    // Names this repetition writes itself. A nested `$($moves),*` is a
    // different repetition; using its path here prints moves where the
    // outer row should print states.
    let mut own = Vec::new();
    for part in inner {
        match part {
            Tpl::Var(name)
            | Tpl::Quote(name)
            | Tpl::Count(name)
            | Tpl::Join { name, .. }
            | Tpl::Trim { path: name } => {
                if !own.contains(name) {
                    own.push(name.clone());
                }
            }
            _ => {}
        }
    }
    let walked = if own.is_empty() { &names } else { &own };
    for name in walked {
        let mut path = name.as_str();
        while !path.is_empty() {
            if let Some(rows) = capture_sequence(captures, path) {
                // A scoped row still has the outer `state` sequence in the
                // cloned map. Walk a sequence only when this scope stored
                // it, or when nothing has been scoped yet.
                let scoped_here = captures.contains_key(path)
                    && matches!(captures.get(path), Some(Capture::Many(_)));
                let unscoped = !captures.keys().any(|key| key.contains('.'));
                if !scoped_here && !unscoped {
                    path = path
                        .rsplit_once('.')
                        .map(|(parent, _)| parent)
                        .unwrap_or("");
                    continue;
                }
                let covers = walked
                    .iter()
                    .filter(|name| name.starts_with(path) || path.starts_with(name.as_str()))
                    .count();
                let better = best.as_ref().is_none_or(|(have_path, _have, have_covers)| {
                    // The longest sequence the template names is the one
                    // this repetition walks. `$state.moves.event` repeats
                    // moves, not states, even when both cover the names.
                    path.len() > have_path.len()
                        || (path.len() == have_path.len() && covers > *have_covers)
                });
                if better {
                    best = Some((path.to_owned(), rows, covers));
                }
                break;
            }
            path = path
                .rsplit_once('.')
                .map(|(parent, _)| parent)
                .unwrap_or("");
        }
    }
    if let Some((path, rows, _)) = best {
        let path = path.as_str();
        let scoped: Vec<Vec<(String, Capture)>> =
            rows.iter().map(|row| scope_rep_row(path, row)).collect();
        // `$state.moves.event` while repeating `$state` is still a sequence
        // on each row. Repeat that sequence too, one element per row.
        if names
            .iter()
            .any(|name| name.starts_with(path) && name.len() > path.len())
        {
            if let Some(field) = names.iter().find_map(|name| {
                let rest = name.strip_prefix(path)?.strip_prefix('.')?;
                rest.split('.').next().map(str::to_owned)
            }) {
                let nested_path = names
                    .iter()
                    .find_map(|name| {
                        let rest = name.strip_prefix(path)?.strip_prefix('.')?;
                        let head = rest.split('.').next()?;
                        if head == field {
                            Some(format!("{path}.{field}"))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| format!("{path}.{field}"));
                let mut flat = Vec::new();
                for row in &scoped {
                    let nested = row.iter().find_map(|(key, value)| {
                        (key == &nested_path || key == &field).then_some(value)
                    });
                    if let Some(Capture::Many(inner)) = nested {
                        if inner.len() > 1 {
                            for inner_row in inner {
                                let mut one = row.clone();
                                one.extend(scope_rep_row(&nested_path, inner_row));
                                // The inner row may be keyed by its field.
                                // Re-scope it under the sequence the template names.
                                if let Some(rekeyed) = one
                                    .iter()
                                    .find(|(key, value)| {
                                        key == &nested_path
                                            && matches!(value, Capture::Record { .. })
                                    })
                                    .map(|(_, value)| value.clone())
                                {
                                    one.extend(scope_rep_row(
                                        &nested_path,
                                        &[(field.clone(), rekeyed)],
                                    ));
                                }
                                flat.push(one);
                            }
                            continue;
                        }
                    }
                    flat.push(row.clone());
                }
                if flat.len() > scoped.len() {
                    return flat;
                }
            }
        }
        return scoped;
    }
    let Some(name) = names.first() else {
        return Vec::new();
    };
    match capture_at(captures, name) {
        Some(Capture::Many(rows)) => rows.clone(),
        Some(value) => vec![vec![(name.clone(), value.clone())]],
        None => Vec::new(),
    }
}

fn scope_rep_row(path: &str, row: &[(String, Capture)]) -> Vec<(String, Capture)> {
    let mut scoped = Vec::new();
    let leaf = path.rsplit('.').next().unwrap_or(path);
    let element = row
        .iter()
        .find(|(key, value)| {
            (key == leaf || key == path) && matches!(value, Capture::Record { .. })
        })
        .or_else(|| {
            // `$body` beside `$arm:Pat` in one row is the body's own text,
            // not the arm record next to it.
            (!path.contains('.'))
                .then(|| row.iter().find(|(key, _)| key == path))
                .flatten()
        })
        .or_else(|| {
            // `state.name` is text, but the same row still holds the state
            // record. Keep that record so a nested `$state.moves` can walk it.
            row.iter()
                .find(|(_, value)| matches!(value, Capture::Record { .. }))
        })
        .or_else(|| {
            row.iter().find(|(key, value)| {
                (key == leaf || key == path) && !matches!(value, Capture::Text(_))
            })
        })
        .or_else(|| row.iter().find(|(key, _)| key == leaf || key == path))
        .or_else(|| row.first())
        .map(|(_, value)| value.clone());
    if let Some(element) = element {
        // A row keyed by a field (`event`) still holds the class record.
        // Store that record on the sequence path (`state.moves`) and the
        // field text on the field path. A text row (`name: "Closed"`)
        // stays under the leaf so `$state.name` does not look up the
        // machine binding also called `name`.
        if let Capture::Text(text) = &element {
            // `method.name` is text, but `name` is also the protocol binding.
            // Keep the field on the path the template wrote.
            if path.contains('.') {
                scoped.push((path.to_owned(), Capture::Text(text.clone())));
            } else {
                scoped.push((leaf.to_owned(), Capture::Text(text.clone())));
                scoped.push((path.to_owned(), Capture::Text(text.clone())));
            }
            for (key, value) in row {
                if key == leaf || key == path {
                    continue;
                }
                // A nested sequence (`$($request)+` inside the outer row)
                // is this iteration's, not every iteration's.
                if matches!(value, Capture::Text(_) | Capture::Many(_)) {
                    scoped.push((key.clone(), value.clone()));
                }
            }
            return scoped;
        }
        let record_path =
            if matches!(element, Capture::Record { .. }) && path.ends_with(&format!(".{leaf}")) {
                path.trim_end_matches(&format!(".{leaf}")).to_owned()
            } else {
                path.to_owned()
            };
        // A repeated field (`modifiers`) is stored beside the record, not
        // inside it. Copy it onto the record so `$column.modifiers.primary`
        // walks the same path the template wrote.
        let mut element = element;
        if let Capture::Record { fields, .. } = &mut element {
            for (key, value) in row {
                if key.starts_with(&format!("{record_path}.")) {
                    let field = &key[record_path.len() + 1..];
                    if !field.contains('.') && !fields.contains_key(field) {
                        fields.insert(field.to_owned(), value.clone());
                    }
                }
            }
        }
        scoped.push((record_path.clone(), element.clone()));
        if let Capture::Record { fields, .. } = &element {
            for (field, value) in fields {
                // `name: text("Closed")` must stay visible as text. The
                // record on `state` is the form; `$state.name` is the field.
                // Do not also store it as the bare leaf when that leaf is
                // already the outer binding (`protocol $name`).
                if let Capture::Text(text) = value {
                    // `$field.name` is the field. A bare `name` is also the
                    // record binding (`record $name`), so the field must not
                    // overwrite it. The dotted path below is the one the
                    // template writes.
                    if field != "name" || path == field || path.ends_with(&format!(".{field}")) {
                        scoped.push((field.clone(), Capture::Text(text.clone())));
                    }
                }
                let mut child = record_path.clone();
                child.push('.');
                child.push_str(field);
                scoped.push((child, value.clone()));
            }
        }
    }
    for (key, value) in row {
        if !scoped.iter().any(|(have, _)| have == key) {
            scoped.push((key.clone(), value.clone()));
        }
    }
    scoped
}

fn collect_vars(template: &[Tpl], names: &mut Vec<String>) {
    for part in template {
        match part {
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) | Tpl::Trim { path: name } => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            Tpl::Join { name, .. } => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            Tpl::Define(inner) | Tpl::Core(inner) | Tpl::Rep { inner, .. } => {
                collect_vars(inner, names)
            }
            Tpl::Let { value, body, .. } | Tpl::Parameterize { value, body, .. } => {
                collect_vars(value, names);
                collect_vars(body, names);
            }
            Tpl::Fold { path, init, .. } => {
                if !names.contains(path) {
                    names.push(path.clone());
                }
                collect_vars(init, names);
            }
            Tpl::Fresh(_) => {}
            Tpl::If {
                path,
                then_body,
                else_body,
            } => {
                if !names.contains(path) {
                    names.push(path.clone());
                }
                collect_vars(then_body, names);
                if let Some(else_body) = else_body {
                    collect_vars(else_body, names);
                }
            }
            Tpl::Metavar { key, .. } => {
                if !key.is_empty() && !names.contains(key) {
                    names.push(key.clone());
                }
            }
            Tpl::Text(_) | Tpl::Unknown { .. } | Tpl::Index => {}
        }
    }
}

fn rename_template_literal(
    raw: &str,
    plan: &HashMap<String, String>,
    capture_spans: &[(usize, usize)],
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
    base: usize,
) -> String {
    let mut out = String::new();
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
            let start = index + 2;
            if let Some(end) = matching_brace(&raw[start..]) {
                let body = &raw[start..start + end];
                if in_capture(capture_spans, base + index) || body.contains("#text") {
                    out.push_str(&raw[index..start + end + 1]);
                } else {
                    out.push_str("${");
                    out.push_str(&rename_interpolation(body, plan, occurrences, seen));
                    out.push('}');
                }
                index = start + end + 1;
                continue;
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

fn rename_interpolation(
    body: &str,
    plan: &HashMap<String, String>,
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
) -> String {
    let read = sweetener_reader::read_syntax(body);
    rename_tokens(body, &read.children, plan, &[], occurrences, seen)
}

fn in_capture(spans: &[(usize, usize)], start: usize) -> bool {
    spans.iter().any(|(from, to)| start >= *from && start < *to)
}

fn rename_tokens(
    source: &str,
    nodes: &[Syntax],
    plan: &HashMap<String, String>,
    capture_spans: &[(usize, usize)],
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;
    rename_nodes(
        source,
        nodes,
        plan,
        capture_spans,
        occurrences,
        seen,
        false,
        &mut out,
        &mut cursor,
    );
    if cursor < source.len() {
        out.push_str(&source[cursor..]);
    }
    out
}

fn rename_nodes(
    source: &str,
    nodes: &[Syntax],
    plan: &HashMap<String, String>,
    capture_spans: &[(usize, usize)],
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
    after_dot: bool,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut index = 0;
    let mut dotted = after_dot;
    while index < nodes.len() {
        let binder_here = is_binder_at(nodes, index);
        match &nodes[index] {
            Syntax::Token { span, raw }
                if !dotted
                    && !in_capture(capture_spans, span.start as usize)
                    && printed_for(raw, binder_here, plan, occurrences, seen).is_some() =>
            {
                let printed = printed_for(raw, binder_here, plan, occurrences, seen).unwrap();
                push_until(source, out, cursor, span.start as usize);
                out.push_str(&printed);
                *cursor = span.end as usize;
                if binder_here {
                    let n = seen.get(raw).copied().unwrap_or(0);
                    seen.insert(raw.clone(), n + 1);
                }
                dotted = false;
            }
            Syntax::Group { children, .. } => {
                rename_nodes(
                    source,
                    children,
                    plan,
                    capture_spans,
                    occurrences,
                    seen,
                    false,
                    out,
                    cursor,
                );
                dotted = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => dotted = true,
            Syntax::String { span, raw } if raw.starts_with('`') && raw.contains("${") => {
                push_until(source, out, cursor, span.start as usize);
                out.push_str(&rename_template_literal(
                    raw,
                    plan,
                    capture_spans,
                    occurrences,
                    seen,
                    span.start as usize,
                ));
                *cursor = span.end as usize;
                dotted = false;
            }
            _ => dotted = false,
        }
        index += 1;
    }
}

fn printed_for(
    raw: &str,
    is_binder: bool,
    plan: &HashMap<String, String>,
    occurrences: &HashMap<String, Vec<String>>,
    seen: &HashMap<String, usize>,
) -> Option<String> {
    if let Some(names) = occurrences.get(raw) {
        if names.len() > 1 || names.first().is_some_and(|name| name != raw) {
            let index = seen.get(raw).copied().unwrap_or(0);
            if is_binder {
                return names.get(index).cloned().or_else(|| names.last().cloned());
            }
            let current = index.saturating_sub(1);
            return names
                .get(current)
                .cloned()
                .or_else(|| names.first().cloned());
        }
    }
    plan.get(raw).cloned()
}

fn runtime_imports_of(nodes: &[Syntax]) -> Vec<RuntimeImport> {
    let mut imports = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "import") && !is_syntax_import_at(nodes, index) {
            let import = value_import_at(nodes, index);
            if let Some(import) = import {
                imports.push(import);
            }
        }
        index += 1;
    }
    imports
}

fn value_import_at(nodes: &[Syntax], index: usize) -> Option<RuntimeImport> {
    let Syntax::Group {
        delimiter: Delimiter::Brace,
        children,
        ..
    } = nodes.get(index + 1)?
    else {
        return None;
    };
    let specifier = nodes
        .iter()
        .skip(index)
        .take(8)
        .find_map(|node| match node {
            Syntax::String { raw, .. } => Some(raw.clone()),
            _ => None,
        })?;
    let mut bindings = Vec::new();
    let mut cursor = 0;
    while cursor < children.len() {
        let Some(Syntax::Token { raw, .. }) = children.get(cursor) else {
            cursor += 1;
            continue;
        };
        if !is_ident_text(raw) {
            cursor += 1;
            continue;
        }
        let exported = raw.clone();
        let local = if token_at(children, cursor + 1, "as") {
            match children.get(cursor + 2) {
                Some(Syntax::Token { raw, .. }) if is_ident_text(raw) => {
                    cursor += 3;
                    raw.clone()
                }
                _ => {
                    cursor += 1;
                    exported.clone()
                }
            }
        } else {
            cursor += 1;
            exported.clone()
        };
        bindings.push(ImportBinding { exported, local });
        if operator_is(children.get(cursor), ",") {
            cursor += 1;
        }
    }
    if bindings.is_empty() {
        return None;
    }
    Some(RuntimeImport {
        specifier,
        bindings,
        owners: Vec::new(),
    })
}

fn runtime_used_by(imports: &[RuntimeImport], macros: &[Macro]) -> Vec<RuntimeImport> {
    let mut used_names = std::collections::HashSet::new();
    for macro_ in macros {
        for rule in &macro_.rules {
            collect_free_idents(&rule.template, &mut used_names);
        }
    }
    let mut out = Vec::new();
    for import in imports {
        let bindings: Vec<ImportBinding> = import
            .bindings
            .iter()
            .filter(|binding| used_names.contains(&binding.local))
            .cloned()
            .collect();
        if !bindings.is_empty() {
            out.push(RuntimeImport {
                specifier: import.specifier.clone(),
                bindings,
                owners: import.owners.clone(),
            });
        }
    }
    out
}

fn collect_free_idents(template: &[Tpl], names: &mut std::collections::HashSet<String>) {
    for part in template {
        match part {
            Tpl::Text(text) => {
                // A template literal is one string, so a name inside `${}`
                // is not a token. Scan the text as well as the token tree.
                let read = sweetener_reader::read_syntax(text);
                idents_in(&read.children, names);
                idents_in_text(text, names);
            }
            Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::Define(inner) => {
                collect_free_idents(inner, names)
            }
            Tpl::Let { value, body, .. } | Tpl::Parameterize { value, body, .. } => {
                collect_free_idents(value, names);
                collect_free_idents(body, names);
            }
            Tpl::Fold { init, body, .. } => {
                collect_free_idents(init, names);
                collect_free_idents(body, names);
            }
            Tpl::Fresh(_) => {}
            Tpl::If {
                then_body,
                else_body,
                ..
            } => {
                collect_free_idents(then_body, names);
                if let Some(else_body) = else_body {
                    collect_free_idents(else_body, names);
                }
            }
            Tpl::Join { .. }
            | Tpl::Var(_)
            | Tpl::Quote(_)
            | Tpl::Count(_)
            | Tpl::Metavar { .. }
            | Tpl::Trim { .. }
            | Tpl::Index
            | Tpl::Unknown { .. } => {}
        }
    }
}

fn idents_in_text(text: &str, names: &mut std::collections::HashSet<String>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if ch == '_' || ch.is_ascii_alphabetic() {
            let start = index;
            index += 1;
            while index < bytes.len() {
                let next = bytes[index] as char;
                if next == '_' || next.is_ascii_alphanumeric() {
                    index += 1;
                } else {
                    break;
                }
            }
            let word = &text[start..index];
            if is_ident_text(word) {
                names.insert(word.to_owned());
            }
            continue;
        }
        index += 1;
    }
}

fn idents_in(nodes: &[Syntax], names: &mut std::collections::HashSet<String>) {
    let mut index = 0;
    let mut after_dot = false;
    while index < nodes.len() {
        match &nodes[index] {
            Syntax::Token { raw, .. } if !after_dot && is_ident_text(raw) => {
                names.insert(raw.clone());
                after_dot = false;
            }
            Syntax::Group { children, .. } => {
                idents_in(children, names);
                after_dot = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => after_dot = true,
            _ => after_dot = false,
        }
        index += 1;
    }
}

fn merge_runtime_imports(into: &mut Vec<RuntimeImport>, more: Vec<RuntimeImport>) {
    for import in more {
        if let Some(existing) = into
            .iter_mut()
            .find(|found| found.specifier == import.specifier)
        {
            for owner in import.owners {
                if !existing.owners.contains(&owner) {
                    existing.owners.push(owner);
                }
            }
            for binding in import.bindings {
                if !existing
                    .bindings
                    .iter()
                    .any(|have| have.local == binding.local)
                {
                    existing.bindings.push(binding);
                }
            }
        } else {
            into.push(import);
        }
    }
}

fn pattern_ends_with_semi(pattern: &[Pat]) -> bool {
    matches!(pattern.last(), Some(Pat::Lit(lit)) if lit == ";")
}

fn infix_rule_binds(
    pattern: &[Pat],
    name: &str,
    left: &str,
    right: &str,
    captures: &mut HashMap<String, Capture>,
) -> bool {
    // `left OP right`, with the operator a literal.
    let mut holes = Vec::new();
    let mut saw_op = false;
    for pat in pattern {
        match pat {
            Pat::Lit(lit) if lit == name => saw_op = true,
            Pat::Hole { name, .. } => holes.push(name.clone()),
            _ => {}
        }
    }
    // `$value:expr |> await`: the right operand is the literal run.
    if saw_op
        && holes.len() == 1
        && let Some(run) = literal_right_run(pattern, name)
    {
        let written: String = right.split_whitespace().collect();
        if written != run.concat() {
            return false;
        }
        captures.insert(holes[0].clone(), Capture::Text(left.trim().to_owned()));
        return true;
    }
    if !saw_op || holes.len() < 2 {
        return false;
    }
    captures.insert(holes[0].clone(), Capture::Text(left.trim().to_owned()));
    captures.insert(holes[1].clone(), Capture::Text(right.trim().to_owned()));
    true
}

/// The literal tokens a rule spells after its operator, when that is all
/// of its right side: `$value:expr |> await`.
fn literal_right_run(pattern: &[Pat], name: &str) -> Option<Vec<String>> {
    let op = pattern
        .iter()
        .position(|pat| matches!(pat, Pat::Lit(lit) if lit == name))?;
    let run: Vec<String> = pattern[op + 1..]
        .iter()
        .map(|pat| match pat {
            Pat::Lit(lit) => Some(lit.clone()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    (!run.is_empty()).then_some(run)
}

/// Whether `node`, after an operand, continues it: a call, an index, a
/// member access, or another operand word.
fn continues_operand(node: Option<&Syntax>) -> bool {
    match node {
        None => false,
        Some(Syntax::Group { delimiter, .. }) => *delimiter != Delimiter::Brace,
        Some(Syntax::Token { .. } | Syntax::String { .. }) => true,
        Some(Syntax::Operator { raw, .. }) => matches!(raw.as_str(), "." | "?." | "!"),
        Some(Syntax::Comment { .. }) => false,
    }
}

/// Whether an arrow function starts at `at`: `x =>`, `(a, b) =>`,
/// `async x =>`, `async (a) =>`.
fn arrow_starts(nodes: &[Syntax], at: usize) -> bool {
    let at = if token_is(nodes.get(at), "async") {
        at + 1
    } else {
        at
    };
    let head = matches!(
        nodes.get(at),
        Some(Syntax::Token { .. })
            | Some(Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            })
    );
    head && operator_spelling_is(nodes.get(at + 1), "=>")
}

fn operator_spelling_is(node: Option<&Syntax>, spelling: &str) -> bool {
    node.and_then(operator_spelling) == Some(spelling) || token_is(node, spelling)
}

fn strip_already_printed(source: &str, head: usize, expanded: &str) -> String {
    let before = source[..head].trim_end();
    let mut out = expanded.to_owned();
    // `async` is part of the form. Stripping it made `async function`
    // match a rule written `function`. Only a modifier the rule itself
    // wrote, and that the call site already printed, is removed.
    for word in ["export", "declare", "default"] {
        if before.ends_with(word) {
            let trimmed = out.trim_start();
            if let Some(rest) = trimmed.strip_prefix(word) {
                if rest.starts_with(|ch: char| ch.is_whitespace()) || rest.is_empty() {
                    out = rest.trim_start().to_owned();
                }
            }
        }
    }
    out
}

fn fresh_name(
    name: &str,
    used: &std::collections::HashSet<String>,
    hygiene: &HashMap<String, String>,
) -> String {
    if let Some(printed) = hygiene.get(name) {
        if !used.contains(printed) {
            return printed.clone();
        }
    }
    if !used.contains(name) {
        return name.to_owned();
    }
    let mut nth = 1u32;
    loop {
        let candidate = format!("{name}_{nth}");
        if !used.contains(&candidate) {
            return candidate;
        }
        nth += 1;
    }
}
