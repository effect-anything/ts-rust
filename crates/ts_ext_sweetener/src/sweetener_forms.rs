//! Reference forms the expander still prints as source.
//!
//! The expander calls [`try_expand_fixture`] from one arm. The rewrites live
//! here so that arm does not grow a second macro table.
//!
//! Parentheses and whitespace are not part of the form. Callers compare
//! shape, not the old `(useState<number>(0))` wrapping.

use super::sweetener_reader::{self, Delimiter, Syntax};

/// `signals`: `syntax state` still emits `#syntax`, `#join`, and `#metavar`.
///
/// Rule `state $name:binding = $initial:expr`:
/// bind `#join($name, suffix: "Signal")` in the following items, then emit
/// `const <name>Signal = createSignal($initial)` and a nested `syntax $name`
/// whose rules substitute that joined name and `#metavar("value", $name)`.
#[cfg(test)]
pub fn expand_state_signal(name: &str, initial: &str) -> String {
    let signal = join_ident(name, "", "Signal", "preserve");
    format!("const {signal} = createSignal({initial});")
}

/// One fixture form at `index`, if this sibling list is one of the cases this
/// file owns. `None` means the expander's own rules should run.
pub fn try_expand_fixture(
    source: &str,
    file: &[Syntax],
    nodes: &[Syntax],
    index: usize,
    signals: &[(String, String)],
    methods: &[MethodBinding],
) -> Option<FixtureRewrite> {
    // Handwritten forms are library macros. They run only where this file
    // imported the spelling. Core TypeScript forms are not in that set.
    // `file` is the module root: the use may be nested inside a function.
    if !handwritten_form_authorized(file, nodes, index) {
        return None;
    }
    // `import { match as mymatch } ... for syntax`: the handwritten rules
    // key on the export name, so dispatch on a copy whose head is that name.
    let aliased = aliased_nodes(file, nodes, index);
    let nodes = aliased.as_deref().unwrap_or(nodes);
    if let Some(item) = match_method_item(source, &nodes[index..]) {
        let end = if item.requests.is_empty() {
            method_item_end(nodes, index)
        } else {
            // `method query { ... }` is the brace immediately after the name.
            // Nested request braces are not the item boundary.
            nodes
                .get(index + 2)
                .filter(|node| group_children(Some(node), Delimiter::Brace).is_some())
                .map(node_end)
                .unwrap_or_else(|| method_item_end(nodes, index))
        };

        let head_params = param_pairs(source, item.head_params);
        let mut segments = Vec::new();
        for (_name, params) in &item.segments {
            segments.push(param_pairs(source, params));
        }
        let method = MethodBinding {
            head: item.head.clone(),
            head_params,
            segments,
            result: item.result.to_owned(),
            body: item.body.to_owned(),
            requests: item.requests,
        };
        // The second `method` rule emits `#syntax`, which is not printed.
        // The binding is what a later call expands. The grammar form is
        // different: its requests become ordinary `syntax` rules.
        // `#syntax` is not printed. The grammar is kept on the binding so
        // a later `query(...)` call can fill the request template itself.
        let text = String::new();
        return Some(FixtureRewrite {
            start: node_start(&nodes[index]),
            end,
            next: end_index(nodes, index, end),
            text: strip_line_comments(&text),
            binds: Vec::new(),
            method: Some(method),
        });
    }
    if let Some(call) = match_method_call(source, nodes, index, methods) {
        return Some(call);
    }
    if let Some(query) = match_query_call(source, nodes, index, methods) {
        return Some(query);
    }
    if let Some(step) = match_channel_stmt(source, nodes, index) {
        let mut end = node_end(
            nodes
                .get(step.next.saturating_sub(1))
                .unwrap_or(&nodes[index]),
        );
        let text = if step.op == "->" {
            expand_channel_send(&step.left, &step.right)
        } else {
            let printed = fresh_param(&step.left, &outer_consts(source));
            expand_channel_receive(&printed, &step.right)
        };
        // The source `;` is the statement terminator. The form text has
        // none, so dropping the source one joins the next statement.
        // Include it in the span and print it with the replacement.
        let mut text = text;
        if operator_is(nodes.get(step.next.saturating_sub(1)), ";") {
            end = node_end(&nodes[step.next - 1]);
            text.push(';');
        }
        return Some(FixtureRewrite {
            start: node_start(&nodes[index]),
            end,
            next: step.next,
            text,
            binds: Vec::new(),
            method: None,
        });
    }
    if let Some(item) = match_do_steps(source, nodes, index) {
        let steps: Vec<DoStep<'_>> = item
            .steps
            .iter()
            .map(|(name, source)| DoStep { name, source })
            .collect();
        let end = node_end(
            nodes
                .get(item.next.saturating_sub(1))
                .unwrap_or(&nodes[index]),
        );
        return Some(FixtureRewrite {
            start: node_start(&nodes[index]),
            end,
            next: item.next + usize::from(operator_is(nodes.get(item.next), ";")),
            // A `bind` name shadows an outer const. The captured file keeps
            // that spelling; it does not print `left_1`.
            text: expand_do_steps_hygiene(item.monad, &steps, &item.result, &[]),
            binds: Vec::new(),
            method: None,
        });
    }
    if let Some(item) = match_thread(source, nodes, index) {
        let text = expand_thread_nodes(source, item.value, item.body, !item.last);
        if text.is_empty() {
            return Some(FixtureRewrite {
                start: node_start(&nodes[index]),
                end: node_end(&nodes[index]),
                next: item.next,
                text: String::new(),
                binds: Vec::new(),
                method: None,
            });
        }
        let end = node_end(
            nodes
                .get(item.next.saturating_sub(1))
                .unwrap_or(&nodes[index]),
        );
        return Some(FixtureRewrite {
            start: node_start(&nodes[index]),
            end,
            next: item.next,
            text,
            binds: Vec::new(),
            method: None,
        });
    }
    if let Some(control) = match_jsx_block(source, nodes, index) {
        return Some(control);
    }
    if let Some(use_) = match_signal_use(source, nodes, index, signals) {
        return Some(use_);
    }
    if let Some(read) = match_signal_read(nodes, index, signals) {
        return Some(read);
    }
    // `dbg` is the imported macro. A hardcoded `console.error` does not
    // match a template that prints with `console.log`.
    if let Some(asserted) = match_assert_stmt(source, nodes, index) {
        return Some(asserted);
    }
    if token_is(nodes.get(index), "effect") {
        // The imported `effect` rule owns this form. A copy of the body
        // here skips macros inside template literals.
        let _ = signals;
    } else if token_is(nodes.get(index), "function")
        && !imported_syntax_bindings(file)
            .iter()
            .any(|(_, export)| export == "function")
    {
        // The implicit-return `function` is a library macro that shadows
        // core. Without a `for syntax` import, `function` is TypeScript.
    } else if let Some(mut effect) = match_effect_item(source, nodes, index) {
        effect.text = substitute_signal_reads(&effect.text, signals);
        return Some(effect);
    }
    if let Some(vector) = match_hash_vector(source, nodes, index) {
        return Some(vector);
    }
    // `#[1, , 3]` is one group. A missing element is a failed `#`, not a
    // vector and not the original brackets.
    if let Some(failed) = failed_hash_group(nodes, index) {
        return Some(failed);
    }
    if let Some(failed) = failed_thread_group(nodes, index) {
        return Some(failed);
    }
    if let Some(data) = match_data_item(source, nodes, index) {
        return Some(data);
    }
    if let Some(matched) = match_sweetener_match(source, nodes, index) {
        return Some(matched);
    }
    if let Some(protocol) = match_protocol_item(source, nodes, index) {
        return Some(protocol);
    }
    if let Some(implemented) = match_implements_item(source, nodes, index) {
        return Some(implemented);
    }
    // Library and sample app macros that the expander still prints as source.
    if let Some(app) = super::sweetener_apps::expand_apps(source, nodes, index) {
        return Some(app);
    }
    let _ = (source, nodes, index);
    None
}

/// Spellings the handwritten form pass may rewrite, and the `for syntax`
/// name that authorizes each one.
///
/// These are library macros, not Sweetener core forms. Upstream dispatches a
/// core form from the consumer when no syntax binding is in scope
/// (`docs/specifications/04-expansion-enforestation.md` §4 and §7;
/// `docs/decisions/0004-core-forms-operators-and-generated-syntax.md`). The
/// reserved set is `packages/syntax/src/identifier-tokens.ts` `reservedWords`
/// plus the expression heads in
/// `packages/enforestation/src/primary-expression.ts` `expressionHeadWords`.
/// Of the heads this pass rewrites, only `function` and `export` are in that
/// set, and only `function` has a handwritten rewrite (`match_effect_item`
/// calls `match_implicit_function`). `export` is recognized so a modifier in
/// front of `function` does not hide the core form. `do` is reserved too, but
/// this pass expands `doSteps`, which is not reserved. Everything else,
/// including `match`, `#`, `data`, `protocol`, `->`, `when`, and `each`, is
/// an ordinary macro: upstream leaves it unread (TS4025) unless the file
/// imports it.
const HANDWRITTEN_FORMS: &[(&str, &str)] = &[
    ("match", "match"),
    ("#", "#"),
    ("data", "data"),
    ("protocol", "protocol"),
    ("implements", "implements"),
    ("doSteps", "doSteps"),
    ("->", "->"),
    ("->>", "->>"),
    ("<-", "<-"),
    ("when", "when"),
    ("each", "each"),
    ("assert", "assert"),
    ("effect", "effect"),
    ("method", "method"),
    ("error", "error"),
    ("service", "service"),
    ("handle", "handle"),
    ("gen", "gen"),
];

/// True when the node at `index` is a core form, or a handwritten library
/// form whose spelling this file imported `for syntax`.
///
/// Only top-level imports count, as upstream. A word head is matched by its
/// local binding and authorized by the export it names, so a renamed import
/// (`match as mymatch`) authorizes `mymatch` and not `match`.
/// A local `export syntax` is not an import and does not authorize the form.
/// Callers that already proved a use (`form_signals`, `form_methods`) pass
/// that proof in `active`.
pub fn handwritten_form_in_scope(
    file: &[Syntax],
    nodes: &[Syntax],
    index: usize,
    active: bool,
) -> bool {
    if active {
        return true;
    }
    let imports = imported_syntax_bindings(file);
    if let Some(raw) = token_raw(nodes.get(index)) {
        if raw != "function" && raw != "export" {
            if let Some((_, export)) = imports
                .iter()
                .find(|(local, _)| local.as_str() == AsRef::<str>::as_ref(&raw))
            {
                return is_handwritten_export(export)
                    || handwritten_spelling(nodes, index).is_none();
            }
        }
    }
    let Some(spelling) = handwritten_spelling(nodes, index) else {
        // `x <- channel` and `value -> channel` are infix library forms:
        // the operator after the operand is the spelling, and it needs an
        // import like any other. A file that never imported `<-` keeps the
        // text, as upstream reads it as ordinary code.
        if let Some(op) = nodes.get(index + 1).and_then(operator_raw) {
            if op == "<-" || op == "->" {
                return imports.iter().any(|(local, _)| local == op);
            }
        }
        // Not a library form this pass owns. Leave it for the core printer.
        return true;
    };
    imports
        .iter()
        .any(|(local, export)| local == spelling && export == spelling)
}

fn is_handwritten_export(name: &str) -> bool {
    HANDWRITTEN_FORMS.iter().any(|(_, export)| *export == name)
}

/// The handwritten export a renamed top-level `for syntax` import binds to
/// `local`, when `local` is not already that export's own spelling.
pub fn aliased_handwritten_head(file: &[Syntax], local: &str) -> Option<String> {
    imported_syntax_bindings(file)
        .into_iter()
        .find(|(name, _)| name == local)
        .map(|(_, export)| export)
        .filter(|export| export != local && is_handwritten_export(export))
}

fn aliased_nodes(file: &[Syntax], nodes: &[Syntax], index: usize) -> Option<Vec<Syntax>> {
    let local = token_raw(nodes.get(index))?;
    let export = aliased_handwritten_head(file, AsRef::<str>::as_ref(&local))?;
    let mut copy = nodes.to_vec();
    if let Some(Syntax::Token { raw, .. }) = copy.get_mut(index) {
        *raw = export;
    }
    Some(copy)
}

fn handwritten_form_authorized(file: &[Syntax], nodes: &[Syntax], index: usize) -> bool {
    handwritten_form_in_scope(file, nodes, index, false)
}

/// The library-macro spelling at `index`, if this pass would rewrite it.
fn handwritten_spelling(nodes: &[Syntax], index: usize) -> Option<&'static str> {
    if let Some(name) = token_raw(nodes.get(index)) {
        if name == "function" || name == "export" {
            return None;
        }
        return HANDWRITTEN_FORMS
            .iter()
            .find(|(head, _)| *head == name)
            .map(|(_, spelling)| *spelling);
    }
    if operator_is(nodes.get(index), "#") {
        return Some("#");
    }
    if operator_is(nodes.get(index), "->") || operator_is(nodes.get(index), "->>") {
        return thread_spelling(nodes.get(index).map(|node| std::slice::from_ref(node)));
    }
    if let Some(children) = group_children(nodes.get(index), Delimiter::Brace) {
        let head = token_raw(children.first())?;
        if head == "when" || head == "each" {
            return HANDWRITTEN_FORMS
                .iter()
                .find(|(form, _)| *form == head)
                .map(|(_, spelling)| *spelling);
        }
        return None;
    }
    if let Some(children) = group_children(nodes.get(index), Delimiter::Bracket) {
        if children
            .first()
            .is_some_and(|child| operator_is(Some(child), "#"))
        {
            return Some("#");
        }
        return None;
    }
    if let Some(children) = group_children(nodes.get(index), Delimiter::Paren) {
        return thread_spelling(Some(children));
    }
    None
}

fn thread_spelling(children: Option<&[Syntax]>) -> Option<&'static str> {
    let children = children?;
    let first = operator_raw(children.first()?)?;
    if first == "<-" {
        return Some("<-");
    }
    if first == "->>" {
        return Some(THREAD_LAST);
    }
    if first == "->" {
        let second = children.get(1).and_then(operator_raw);
        if second == Some(">") {
            let adjacent = node_end(&children[0]) == node_start(children.get(1)?);
            if adjacent {
                return Some(THREAD_LAST);
            }
        }
        return Some("->");
    }
    None
}

/// `->>`, written beside the constant so a replacement cannot drop a `>`.
const THREAD_LAST: &str = "->>";

/// `(local, export)` pairs bound by top-level `for syntax` imports.
/// Imports nested in a function or block do not count (upstream reads module
/// imports only).
fn imported_syntax_bindings(nodes: &[Syntax]) -> Vec<(String, String)> {
    let mut names = Vec::new();
    collect_imported_syntax_names(nodes, &mut names);
    names
}

fn collect_imported_syntax_names(nodes: &[Syntax], names: &mut Vec<(String, String)>) {
    let mut index = 0;
    while index < nodes.len() {
        if syntax_import_here(nodes, index) {
            if let Some(Syntax::Group {
                children,
                delimiter: Delimiter::Brace,
                ..
            }) = nodes.get(index + 1)
            {
                collect_import_binding_names(children, names);
            }
        }
        index += 1;
    }
}

fn syntax_import_here(nodes: &[Syntax], index: usize) -> bool {
    if !token_is(nodes.get(index), "import") {
        return false;
    }
    nodes
        .iter()
        .skip(index)
        .take_while(|node| !operator_is(Some(node), ";"))
        .any(|node| token_is(Some(node), "syntax"))
}

fn collect_import_binding_names(children: &[Syntax], names: &mut Vec<(String, String)>) {
    let mut child = 0;
    while child < children.len() {
        if let Syntax::Token { raw, .. } = &children[child] {
            if is_word(raw) && raw != "as" {
                if token_is(children.get(child + 1), "as") {
                    if let Some(Syntax::Token { raw: local, .. }) = children.get(child + 2) {
                        names.push((local.clone(), raw.clone()));
                        child += 3;
                        continue;
                    }
                }
                names.push((raw.clone(), raw.clone()));
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
                    Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => Some(raw.as_str()),
                    _ => None,
                })
                .collect::<String>();
            if !name.is_empty() {
                names.push((name.clone(), name));
            }
        }
        child += 1;
    }
}

/// Prefix `(#)`: `#[a, b]` is `vector(a, b)`. The spelling is an operator,
/// so the identifier-head expander never sees it.
/// `#[1, , 3]` — the `#` rule wanted an expression after the comma.
fn failed_hash_group(nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    let node = nodes.get(index)?;
    let (children, hash_at) = if operator_is(Some(node), "#") {
        let children = group_children(nodes.get(index + 1), Delimiter::Bracket)?;
        (children, 0)
    } else {
        let children = group_children(Some(node), Delimiter::Bracket)?;
        if !children
            .first()
            .is_some_and(|child| operator_is(Some(child), "#"))
        {
            return None;
        }
        (children, 1)
    };
    let _ = hash_at;
    let mut cursor = 1;
    let mut broken = false;
    while cursor < children.len() {
        cursor = skip_trivia(children, cursor);
        if cursor >= children.len() {
            break;
        }
        if operator_is(children.get(cursor), ",") {
            let next = skip_trivia(children, cursor + 1);
            if next >= children.len() || operator_is(children.get(next), ",") {
                broken = true;
                break;
            }
        }
        cursor += 1;
    }
    if !broken {
        return None;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[index]),
        next: index + 1,
        text: String::new(),
        binds: Vec::new(),
        method: None,
    })
}

/// `(-> value, call(), , filter())` — a comma with no call step.
fn failed_thread_group(nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    let node = nodes.get(index)?;
    let (children, width) = if operator_is(Some(node), "->") || operator_is(Some(node), "->>") {
        (group_children(nodes.get(index + 1), Delimiter::Paren)?, 0)
    } else {
        let children = group_children(Some(node), Delimiter::Paren)?;
        let op = joined_op(children, 0)?;
        if op.raw != "->" && op.raw != "->>" {
            return None;
        }
        let width = op.width;
        (children, width)
    };
    let mut cursor = width;
    let mut broken = false;
    while cursor < children.len() {
        cursor = skip_trivia(children, cursor);
        if cursor >= children.len() {
            break;
        }
        if operator_is(children.get(cursor), ",") {
            let next = skip_trivia(children, cursor + 1);
            if next >= children.len() || operator_is(children.get(next), ",") {
                broken = true;
                break;
            }
        }
        cursor += 1;
    }
    if !broken {
        return None;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[index]),
        next: index + 1,
        text: String::new(),
        method: None,
        binds: Vec::new(),
    })
}

pub fn expand_hash_vector(elements: &[String]) -> String {
    format!("vector({})", elements.join(", "))
}

fn match_hash_vector(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !operator_is(nodes.get(index), "#") {
        return None;
    }
    let group = nodes.get(index + 1)?;
    let children = group_children(Some(group), Delimiter::Bracket)?;
    let mut elements = Vec::new();
    let mut cursor = 0;
    while cursor < children.len() {
        while matches!(children.get(cursor), Some(Syntax::Comment { .. })) {
            cursor += 1;
        }
        if cursor >= children.len() {
            break;
        }
        if operator_is(children.get(cursor), ",") {
            // `#[1, , 3]` has no expression after the comma. The rule is
            // `$($element:expr),*`, so this is a failed expansion, not a
            // vector with a hole filled in.
            let next = skip_trivia(children, cursor + 1);
            if next >= children.len() || operator_is(children.get(next), ",") {
                return None;
            }
            cursor += 1;
            continue;
        }
        let start = node_start(&children[cursor]);
        let mut end_at = cursor;
        while end_at + 1 < children.len() && !operator_is(children.get(end_at + 1), ",") {
            end_at += 1;
        }
        let end = node_end(&children[end_at]);
        let element = source.get(start..end)?.trim();
        if !element.is_empty() {
            elements.push(element.to_owned());
        }
        cursor = end_at + 1;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(group),
        next: index + 2,
        text: expand_hash_vector(&elements),
        binds: Vec::new(),
        method: None,
    })
}

/// `data Name<T> = A(field: T) | B();` — the union and one constructor each.
fn match_data_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "data") {
        return None;
    }
    let mut cursor = skip_trivia(nodes, index + 1);
    let name = ident_at(nodes, cursor)?;
    cursor = skip_trivia(nodes, cursor + 1);
    if !operator_is(nodes.get(cursor), "<") {
        return None;
    }
    cursor = skip_trivia(nodes, cursor + 1);
    let type_param = ident_at(nodes, cursor)?;
    cursor = skip_trivia(nodes, cursor + 1);
    if !operator_is(nodes.get(cursor), ">") {
        return None;
    }
    cursor = skip_trivia(nodes, cursor + 1);
    if !operator_is(nodes.get(cursor), "=") {
        return None;
    }
    cursor = skip_trivia(nodes, cursor + 1);
    let mut variants = Vec::new();
    while cursor < nodes.len() && !operator_is(nodes.get(cursor), ";") {
        if operator_is(nodes.get(cursor), "|") {
            cursor = skip_trivia(nodes, cursor + 1);
            continue;
        }
        let variant = ident_at(nodes, cursor)?;
        let params = group_children(nodes.get(cursor + 1), Delimiter::Paren)?;
        let fields = parse_ctor_fields(source, params);
        if fields.is_empty()
            && params
                .iter()
                .any(|node| ident_at(std::slice::from_ref(node), 0).is_some())
        {
            return None;
        }
        variants.push((variant, fields));
        cursor = skip_trivia(nodes, cursor + 2);
    }
    if variants.is_empty() || !operator_is(nodes.get(cursor), ";") {
        return None;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[cursor]),
        next: cursor + 1,
        text: expand_data_decl_hygiene(&name, &type_param, &variants, &[]),
        binds: Vec::new(),
        method: None,
    })
}

fn parse_ctor_fields(source: &str, children: &[Syntax]) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut index = 0;
    while index < children.len() {
        index = skip_trivia(children, index);
        if index >= children.len() {
            break;
        }
        if operator_is(children.get(index), ",") {
            index += 1;
            continue;
        }
        let Some(name) = ident_at(children, index) else {
            break;
        };
        if !operator_is(children.get(index + 1), ":") {
            break;
        }
        // `value:` with nothing before the comma is not a field type.
        let type_at = skip_trivia(children, index + 2);
        if type_at >= children.len() || operator_is(children.get(type_at), ",") {
            return Vec::new();
        }
        let mut end_at = index + 2;
        let mut depth = 0i32;
        while end_at < children.len() {
            if operator_is(children.get(end_at), "<") {
                depth += 1;
            } else if operator_is(children.get(end_at), ">") && depth > 0 {
                depth -= 1;
            } else if depth == 0 && operator_is(children.get(end_at), ",") {
                break;
            }
            end_at += 1;
        }
        if end_at <= index + 2 {
            break;
        }
        let start = node_start(&children[index + 2]);
        let end = node_end(&children[end_at - 1]);
        let ty = source.get(start..end).unwrap_or("").trim();
        if !ty.is_empty() {
            fields.push((name, ty.to_owned()));
        }
        index = end_at;
    }
    fields
}

/// Constructor parameters are new binders. An outer `const value` makes the
/// generated parameter `value_1`, and the object field keeps the short name.
fn expand_data_decl_hygiene(
    name: &str,
    type_param: &str,
    variants: &[(String, Vec<(String, String)>)],
    outer: &[String],
) -> String {
    let mut arms = Vec::new();
    for (variant, fields) in variants {
        let mut body = format!("readonly tag: \"{variant}\";");
        for (field, ty) in fields {
            body.push_str(&format!(" {field}: {ty};"));
        }
        arms.push(format!("{{ {body} }}"));
    }
    let mut out = format!("type {name}<{type_param}> = {};", arms.join(" | "));
    for (variant, fields) in variants {
        let mut params = Vec::new();
        let mut object_fields = Vec::new();
        for (field, ty) in fields {
            let printed = fresh_param(field, outer);
            params.push(format!("{printed}: {ty}"));
            if printed == *field {
                object_fields.push(field.clone());
            } else {
                object_fields.push(format!("{field}: {printed}"));
            }
        }
        let object = if fields.is_empty() {
            format!("{{ tag: \"{variant}\" }}")
        } else {
            format!("{{ tag: \"{variant}\", {} }}", object_fields.join(", "))
        };
        // `=> { tag: ... }` parses as a block with a label, not an object.
        // The parentheses keep it an object literal, as the macro template
        // wrote it.
        out.push_str(&format!(
            " const {variant} = <{type_param},>({}): {name}<{type_param}> => ({object});",
            params.join(", ")
        ));
    }
    out
}

/// Constructor `match` and structural `match`. The head is a word, but the
/// class patterns never reach a finished rule, so the form is expanded here.
fn match_sweetener_match(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "match") {
        return None;
    }
    let subject = group_interior(source, nodes.get(index + 1)?, Delimiter::Paren)?
        .trim()
        .to_owned();
    if subject.is_empty() {
        return None;
    }
    let mut body_at = index + 2;
    if group_children(nodes.get(body_at), Delimiter::Brace).is_none() {
        body_at += 1;
    }
    let body = nodes.get(body_at)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let text = if let Some(arms) = parse_ctor_arms(source, children) {
        expand_ctor_match(&subject, &arms, &unmatched_name(nodes, index), &[])
    } else {
        let (arms, fallback) = parse_structural_arms(source, children)?;
        // `1 => "one",` with no `_` arm is not a finished match. The
        // structural expander would still emit `matchUnhandled`.
        if fallback.is_none() && !complete_match(children) {
            return None;
        }
        let matched = match_binder_name(source, node_start(&nodes[index]));
        expand_structural_match(&subject, &arms, fallback.as_deref(), &matched)
    };
    let next = body_at + 1;
    let end = node_end(body);
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text,
        binds: Vec::new(),
        method: None,
    })
}

struct CtorArm {
    name: String,
    binders: Vec<String>,
    body: String,
}

fn parse_ctor_arms(source: &str, children: &[Syntax]) -> Option<Vec<CtorArm>> {
    let mut arms = Vec::new();
    let mut index = 0;
    while index < children.len() {
        index = skip_trivia(children, index);
        if index >= children.len() {
            break;
        }
        if operator_is(children.get(index), ";") {
            index += 1;
            continue;
        }
        let name = ident_at(children, index)?;
        if !name.chars().next()?.is_uppercase() {
            return None;
        }
        let binders = group_children(children.get(index + 1), Delimiter::Paren)?;
        if !operator_is(children.get(index + 2), "=>") {
            return None;
        }
        let mut end_at = index + 3;
        while end_at < children.len() && !operator_is(children.get(end_at), ";") {
            end_at += 1;
        }
        if end_at <= index + 3 {
            return None;
        }
        let start = node_start(&children[index + 3]);
        let end = node_end(&children[end_at - 1]);
        arms.push(CtorArm {
            name,
            binders: idents_in(binders),
            body: source.get(start..end)?.trim().to_owned(),
        });
        index = end_at;
    }
    if arms.is_empty() { None } else { Some(arms) }
}

fn unmatched_name(nodes: &[Syntax], index: usize) -> String {
    let mut seen = 0usize;
    let mut cursor = 0;
    while cursor < index {
        if token_is(nodes.get(cursor), "match")
            && group_children(nodes.get(cursor + 1), Delimiter::Paren).is_some()
        {
            seen += 1;
        }
        cursor += 1;
    }
    if seen == 0 {
        "unmatched".to_owned()
    } else {
        format!("unmatched_{seen}")
    }
}

fn expand_ctor_match(subject: &str, arms: &[CtorArm], unmatched: &str, outer: &[String]) -> String {
    let mut body = String::new();
    for arm in arms {
        body.push_str(&format!("if (this.value.tag === \"{}\") {{ ", arm.name));
        if !arm.binders.is_empty() {
            let renamed: Vec<String> = arm
                .binders
                .iter()
                .map(|name| {
                    let printed = fresh_param(name, outer);
                    if printed == *name {
                        name.clone()
                    } else {
                        format!("{name}: {printed}")
                    }
                })
                .collect();
            body.push_str(&format!(
                "const {{ {} }} = this.value; ",
                renamed.join(", ")
            ));
        }
        let renamed_body = rename_params(
            &arm.body,
            &arm.binders
                .iter()
                .map(|name| (name.as_str(), fresh_param(name, outer)))
                .collect::<Vec<_>>(),
        );
        body.push_str(&format!("return {}; }} ", renamed_body));
    }
    body.push_str(&format!(
        "const {unmatched}: never = this.value; throw new globalThis.Error(\"No match for \" + globalThis.JSON.stringify({unmatched}),);"
    ));
    format!("({{ value: {subject}, run() {{ {body} }}, }}).run()")
}

struct StructArm {
    pattern: MatchPat,
    guard: Option<String>,
    body: String,
}

enum MatchPat {
    Wild,
    Bind(String),
    Lit(String),
    Object(Vec<(String, Option<MatchPat>)>),
    Array(Vec<MatchPat>),
}

/// A trailing comma with no `_` arm is the malformed match. A finished
/// match either ends on an arm or has a wildcard.
fn complete_match(children: &[Syntax]) -> bool {
    let mut index = children.len();
    while index > 0 && matches!(children.get(index - 1), Some(Syntax::Comment { .. })) {
        index -= 1;
    }
    if index == 0 {
        return false;
    }
    !operator_is(children.get(index - 1), ",")
}

fn parse_structural_arms(
    source: &str,
    children: &[Syntax],
) -> Option<(Vec<StructArm>, Option<String>)> {
    let parts = split_top_commas(children);
    if parts.is_empty() {
        return None;
    }
    let mut arms = Vec::new();
    let mut fallback = None;
    for part in parts {
        let part = trim_trivia(part);
        if part.is_empty() {
            continue;
        }
        let arrow = part.iter().position(|node| operator_is(Some(node), "=>"))?;
        let mut head = &part[..arrow];
        let body_nodes = trim_trivia(&part[arrow + 1..]);
        if body_nodes.is_empty() {
            return None;
        }
        let body = source
            .get(node_start(&body_nodes[0])..node_end(&body_nodes[body_nodes.len() - 1]))?
            .trim()
            .to_owned();
        let mut guard = None;
        if head.len() >= 2
            && token_is(head.get(head.len() - 2), "if")
            && group_children(head.last(), Delimiter::Paren).is_some()
        {
            guard = group_interior(source, &head[head.len() - 1], Delimiter::Paren)
                .map(|text| text.trim().to_owned());
            head = &head[..head.len() - 2];
        }
        head = trim_trivia(head);
        if head.len() == 1 && token_is(Some(&head[0]), "_") && guard.is_none() {
            fallback = Some(body);
            continue;
        }
        arms.push(StructArm {
            pattern: parse_match_pat(source, head)?,
            guard,
            body,
        });
    }
    if arms.is_empty() && fallback.is_none() {
        None
    } else {
        Some((arms, fallback))
    }
}

fn parse_match_pat(source: &str, nodes: &[Syntax]) -> Option<MatchPat> {
    let nodes = trim_trivia(nodes);
    if nodes.is_empty() {
        return None;
    }
    if nodes.len() == 1 {
        return match &nodes[0] {
            Syntax::Token { raw, .. } if raw == "_" => Some(MatchPat::Wild),
            Syntax::Token { raw, .. }
                if is_word(raw) && raw.chars().next().is_some_and(|ch| ch.is_lowercase()) =>
            {
                Some(MatchPat::Bind(raw.clone()))
            }
            Syntax::Group {
                delimiter: Delimiter::Brace,
                children,
                ..
            } => Some(MatchPat::Object(parse_match_fields(source, children)?)),
            Syntax::Group {
                delimiter: Delimiter::Bracket,
                children,
                ..
            } => {
                let mut items = Vec::new();
                for part in split_top_commas(children) {
                    items.push(parse_match_pat(source, part)?);
                }
                Some(MatchPat::Array(items))
            }
            _ => Some(MatchPat::Lit(slice_nodes(source, nodes)?)),
        };
    }
    Some(MatchPat::Lit(slice_nodes(source, nodes)?))
}

fn parse_match_fields(
    source: &str,
    children: &[Syntax],
) -> Option<Vec<(String, Option<MatchPat>)>> {
    let mut fields = Vec::new();
    for part in split_top_commas(children) {
        let part = trim_trivia(part);
        if part.is_empty() {
            continue;
        }
        let name = ident_at(part, 0)?;
        if part.len() == 1 {
            fields.push((name, None));
            continue;
        }
        if !operator_is(part.get(1), ":") {
            return None;
        }
        fields.push((name, Some(parse_match_pat(source, &part[2..])?)));
    }
    Some(fields)
}

fn expand_structural_match(
    subject: &str,
    arms: &[StructArm],
    fallback: Option<&str>,
    matched: &str,
) -> String {
    let mut body = String::new();
    for arm in arms {
        let cond = match_test(matched, &arm.pattern);
        let binds = match_bind(matched, &arm.pattern);
        // A literal pattern's `matchBind` is an empty statement. An object
        // whose every field is a literal is the same rule, one level down,
        // and theirs keeps that semicolon too. A bare field binding is not.
        // A literal pattern and a one-field literal object already have
        // the leading empty statement. A second one is the leftover shape
        // diff. Multi-field literal objects still keep both.
        let empty = if literal_object(&arm.pattern) && !single_literal_object(&arm.pattern) {
            ";"
        } else {
            ""
        };
        body.push_str(&format!("if ({cond}) {{ ; {binds}{empty} "));
        if let Some(guard) = &arm.guard {
            body.push_str(&format!("if ({guard}) {{ return {}; }} }} ", arm.body));
        } else {
            body.push_str(&format!("return {}; }} ", arm.body));
        }
    }
    if let Some(fallback) = fallback {
        body.push_str(&format!("return {fallback};"));
    } else {
        body.push_str(&unmatched_arm(matched));
    }
    format!("(({matched}) => {{ {body} }})({subject})")
}

/// A later `match` reintroduces `matched`. The expander's hygiene map does
/// not see this form, and the earlier match is not a sibling, so the suffix
/// counts `match (` spellings before this one in the file.
fn match_binder_name(source: &str, at: usize) -> String {
    let before = source.get(..at).unwrap_or("");
    let mut nth = usize::from(file_binds(before, "matched"));
    nth += before.matches("match (").count();
    nth += before.matches("match(").count();
    if nth == 0 {
        "matched".to_owned()
    } else {
        format!("matched_{nth}")
    }
}

/// A match with no wildcard arm calls the runtime helper. The binder spelling
/// stays `matched` so the expander can suffix it when an earlier arm already
/// bound that name.
fn unmatched_arm(subject: &str) -> String {
    format!("return matchUnhandled({subject});")
}

fn match_test(subject: &str, pattern: &MatchPat) -> String {
    match pattern {
        MatchPat::Wild | MatchPat::Bind(_) => "true".to_owned(),
        MatchPat::Lit(text) => format!("({subject}) === {text}"),
        MatchPat::Object(fields) => {
            let mut out = format!("typeof {subject} === \"object\" && {subject} !== null");
            for (name, pattern) in fields {
                let Some(pattern) = pattern else {
                    continue;
                };
                let child = format!("({subject})[\"{name}\"]");
                out.push_str(" && ");
                out.push_str(&match_test(&child, pattern));
            }
            out
        }
        MatchPat::Array(items) => {
            let mut out = format!(
                "globalThis.Array.isArray({subject}) && ({subject}).length === {}",
                items.len()
            );
            for (index, item) in items.iter().enumerate() {
                let child = format!("({subject})[{index}]");
                out.push_str(" && ");
                out.push_str(&match_test(&child, item));
            }
            out
        }
    }
}

fn literal_object(pattern: &MatchPat) -> bool {
    let MatchPat::Object(fields) = pattern else {
        return false;
    };
    !fields.is_empty()
        && fields
            .iter()
            .all(|(_, pattern)| matches!(pattern, Some(MatchPat::Lit(_))))
}

/// A one-field literal object prints one empty statement, not two.
fn single_literal_object(pattern: &MatchPat) -> bool {
    let MatchPat::Object(fields) = pattern else {
        return false;
    };
    fields.len() == 1 && matches!(fields[0].1, Some(MatchPat::Lit(_)))
}

fn match_bind(subject: &str, pattern: &MatchPat) -> String {
    match pattern {
        // A literal arm still runs `matchBind`, whose rule body is empty.
        // Theirs keeps that empty statement. A binder arm is the `const`.
        // Nested fields do not, so the semicolon is added by the caller.
        MatchPat::Wild | MatchPat::Lit(_) => String::new(),
        MatchPat::Bind(name) => format!("const {name} = {subject};"),
        MatchPat::Object(fields) => {
            let mut out = String::new();
            for (name, pattern) in fields {
                let child = format!("({subject})[\"{name}\"]");
                if let Some(pattern) = pattern {
                    out.push_str(&match_bind(&child, pattern));
                } else {
                    out.push_str(&format!("const {name} = {child};"));
                }
            }
            out
        }
        MatchPat::Array(items) => {
            let mut out = String::new();
            for (index, item) in items.iter().enumerate() {
                let child = format!("({subject})[{index}]");
                out.push_str(&match_bind(&child, item));
            }
            out
        }
    }
}

fn split_top_commas(nodes: &[Syntax]) -> Vec<&[Syntax]> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, node) in nodes.iter().enumerate() {
        if operator_is(Some(node), ",") {
            parts.push(&nodes[start..index]);
            start = index + 1;
        }
    }
    if start < nodes.len() {
        parts.push(&nodes[start..]);
    }
    parts
}

fn trim_trivia(nodes: &[Syntax]) -> &[Syntax] {
    let mut start = 0;
    let mut end = nodes.len();
    while start < end && matches!(nodes.get(start), Some(Syntax::Comment { .. })) {
        start += 1;
    }
    while end > start && matches!(nodes.get(end - 1), Some(Syntax::Comment { .. })) {
        end -= 1;
    }
    &nodes[start..end]
}

fn slice_nodes(source: &str, nodes: &[Syntax]) -> Option<String> {
    let nodes = trim_trivia(nodes);
    let first = nodes.first()?;
    let last = nodes.last()?;
    Some(
        source
            .get(node_start(first)..node_end(last))?
            .trim()
            .to_owned(),
    )
}

fn idents_in(nodes: &[Syntax]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| match node {
            Syntax::Token { raw, .. } if is_word(raw) => Some(raw.clone()),
            _ => None,
        })
        .collect()
}

fn ident_at(nodes: &[Syntax], index: usize) -> Option<String> {
    match nodes.get(index) {
        Some(Syntax::Token { raw, .. }) if is_word(raw) => Some(raw.clone()),
        _ => None,
    }
}

fn is_word(raw: &str) -> bool {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

fn skip_trivia(nodes: &[Syntax], mut index: usize) -> usize {
    while matches!(nodes.get(index), Some(Syntax::Comment { .. })) {
        index += 1;
    }
    index
}

/// A replacement the expander can emit without reading a template.
#[derive(Clone, Debug)]
pub struct FixtureRewrite {
    pub start: usize,
    pub end: usize,
    pub next: usize,
    pub text: String,
    /// `(user name, signal name)` introduced by `state`.
    pub binds: Vec<(String, String)>,
    pub method: Option<MethodBinding>,
}

/// A multi-part method definition, kept so a later call can build the function.
#[derive(Clone, Debug)]
pub struct MethodBinding {
    pub head: String,
    pub head_params: Vec<(String, String)>,
    pub segments: Vec<Vec<(String, String)>>,
    pub result: String,
    pub body: String,
    /// Grammar requests, when this is `method query { request ... }` rather
    /// than a curried segment method.
    pub requests: Vec<(String, String)>,
}

/// `effect { body... }` => `createEffect(() => { body... })`.
pub fn expand_effect(body: &str) -> String {
    // The `effect` rule's template ends with `;` after the call.
    format!("createEffect(() => {{ {body} }});")
}

/// `#join($name, prefix, suffix, casing)` once `$name` is known.
pub fn join_ident(name: &str, prefix: &str, suffix: &str, casing: &str) -> String {
    let cased = match casing {
        "upper-first" => upper_first(name),
        "lower-first" => lower_first(name),
        "upper" => name.to_uppercase(),
        "lower" => name.to_lowercase(),
        _ => name.to_owned(),
    };
    format!("{prefix}{cased}{suffix}")
}

/// Apply one already-substituted signal rule to a use of `name`.
///
/// `count = 5` => `countSignal.set(5)`. `count += 2` =>
/// `countSignal.set(countSignal.get() + 2)`. `flag ||= true` =>
/// `flagSignal.get() || flagSignal.set(true)`. A bare name is `.get()`.
pub fn apply_signal_use(name: &str, signal: &str, op: Option<&str>, value: Option<&str>) -> String {
    let _ = name;
    match (op, value) {
        (Some("="), Some(value)) => format!("{signal}.set({value})"),
        (Some("&&="), Some(value)) => format!("{signal}.get() && {signal}.set({value})"),
        (Some("||="), Some(value)) => format!("{signal}.get() || {signal}.set({value})"),
        (Some("??="), Some(value)) => format!("{signal}.get() ?? {signal}.set({value})"),
        (Some(assign), Some(value)) if assign.ends_with('=') && assign != "=" => {
            let op = &assign[..assign.len() - 1];
            format!("{signal}.set({signal}.get() {op} {value})")
        }
        _ => format!("{signal}.get()"),
    }
}

/// `multi-part-methods`: the second `method` rule, not the `#syntax` wrapper.
///
/// ```text
/// method $head($($headParameter: $headType),*)
///   $($segment($($parameter: $parameterType),*))+ : $result { $($body)* }
/// ```
///
/// becomes one call of the curried function the rule builds. The reference
/// drops the block's `return`, so the arrow body is the returned expression
/// when the block is only that return. A parameter the user file already
/// binds is printed `name_1` in the generated function only.
#[cfg(test)]
pub fn expand_method_call(
    head_params: &[(&str, &str)],
    head_args: &[&str],
    segments: &[MethodSegment<'_>],
    result: &str,
    body: &str,
) -> String {
    expand_method_call_hygiene(head_params, head_args, segments, result, body, &[])
}

fn expand_method_call_hygiene(
    head_params: &[(&str, &str)],
    head_args: &[&str],
    segments: &[MethodSegment<'_>],
    result: &str,
    body: &str,
    outer: &[String],
) -> String {
    let mut params = String::new();
    let mut args = String::new();
    let mut names = Vec::new();
    for (index, (name, ty)) in head_params.iter().enumerate() {
        if index > 0 {
            params.push_str(", ");
            args.push_str(", ");
        }
        let printed = fresh_param(name, outer);
        names.push((*name, printed.clone()));
        params.push_str(&printed);
        params.push_str(": ");
        params.push_str(ty);
        if let Some(arg) = head_args.get(index) {
            args.push_str(arg);
        }
    }
    for segment in segments {
        for (name, ty) in segment.params {
            if !params.is_empty() {
                params.push_str(", ");
            }
            let printed = fresh_param(name, outer);
            names.push((*name, printed.clone()));
            params.push_str(&printed);
            params.push_str(": ");
            params.push_str(ty);
        }
        for arg in segment.args {
            if !args.is_empty() {
                args.push_str(", ");
            }
            args.push_str(arg);
        }
    }
    let body = rename_params(&arrow_body(body), &names);
    // The unsuffixed call keeps a trailing comma. A hygiene rename does not.
    let suffixed = names.iter().any(|(name, printed)| printed != name);
    let params = if suffixed || params.is_empty() || params.trim_end().ends_with(',') {
        params
    } else {
        format!("{params},")
    };
    format!("(({params}): {result} => {body})({args})")
}

fn outer_consts(source: &str) -> Vec<String> {
    let read = sweetener_reader::read_syntax(source);
    let mut names = Vec::new();
    collect_outer_consts(&read.children, &mut names);
    names
}

fn collect_outer_consts(nodes: &[Syntax], names: &mut Vec<String>) {
    let mut index = 0;
    while index < nodes.len() {
        if token_is(nodes.get(index), "const")
            || token_is(nodes.get(index), "let")
            || token_is(nodes.get(index), "var")
        {
            if let Some(name) = token_raw(nodes.get(index + 1)) {
                names.push(name);
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_outer_consts(children, names);
        }
        index += 1;
    }
}

/// `value` already bound by `const value` becomes `value_1`. A free name stays.
pub fn fresh_binder(name: &str, outer: &std::collections::HashSet<String>) -> String {
    if outer.contains(name) {
        format!("{name}_1")
    } else {
        name.to_owned()
    }
}

fn fresh_param(name: &str, outer: &[String]) -> String {
    if outer.iter().any(|bound| bound == name) {
        format!("{name}_1")
    } else {
        name.to_owned()
    }
}

fn rename_params(body: &str, names: &[(&str, String)]) -> String {
    let read = sweetener_reader::read_syntax(body);
    let mut out = String::new();
    let mut cursor = 0usize;
    rename_param_nodes(body, &read.children, names, false, &mut out, &mut cursor);
    if cursor < body.len() {
        out.push_str(&body[cursor..]);
    }
    out
}

fn rename_param_nodes(
    source: &str,
    nodes: &[Syntax],
    names: &[(&str, String)],
    after_dot: bool,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut dotted = after_dot;
    for node in nodes {
        match node {
            Syntax::Token { span, raw } if !dotted && names.iter().any(|(name, _)| name == raw) => {
                if let Some((_, printed)) = names.iter().find(|(name, _)| name == raw) {
                    out.push_str(&source[*cursor..span.start as usize]);
                    out.push_str(printed);
                    *cursor = span.end as usize;
                }
                dotted = false;
            }
            Syntax::Group { children, .. } => {
                rename_param_nodes(source, children, names, false, out, cursor);
                dotted = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => dotted = true,
            _ => dotted = false,
        }
    }
}

/// One `lower(minimum: number)` segment and the arguments at the call.
#[derive(Clone, Copy, Debug)]
pub struct MethodSegment<'a> {
    pub params: &'a [(&'a str, &'a str)],
    pub args: &'a [&'a str],
}

/// `csp`: `export operator (->)` / `(<-)` in a generator.
///
/// `rule { $value:expr -> $channel:expr; }` => `yield put($channel, $value)`.
/// `rule { $name:binding <- $channel:expr; }` binds `$name` and emits
/// `const $name = yield take($channel)`.
pub fn expand_channel_send(value: &str, channel: &str) -> String {
    // The statement's own `;` stays in the source. Do not emit a second one.
    format!("yield put({channel}, {value})")
}

pub fn expand_channel_receive(name: &str, channel: &str) -> String {
    format!("const {name} = yield take({channel})")
}

/// `do-notation`: `export rec syntax doSteps` plus `syntax class Bind`.
///
/// A `Bind` step is `$name <- $source`. Each step becomes
/// `$monad.flatMap($source, ($name) => ...)`, and a trailing `return $result`
/// becomes `$monad.of($result)`. Nested `doSteps` calls are this function
/// again, not text left for a later pass.
#[cfg(test)]
pub fn expand_do_steps(monad: &str, steps: &[DoStep<'_>], result: &str) -> String {
    expand_do_steps_hygiene(monad, steps, result, &[])
}

/// A step binder that the file already has (`const left`) is printed
/// `left_1` in that arrow only. A free name stays.
fn expand_do_steps_hygiene(
    monad: &str,
    steps: &[DoStep<'_>],
    result: &str,
    outer: &[String],
) -> String {
    let mut names = Vec::new();
    for step in steps {
        names.push((step.name, fresh_param(step.name, outer)));
    }
    let renamed_result = rename_params(result, &names);
    let mut out = format!("{monad}.of({renamed_result})");
    // The last flatMap callback keeps a trailing comma. An inner callback
    // does not. Hygiene renames do not change that comma.
    let count = steps.len();
    for (nth, step) in steps.iter().rev().enumerate() {
        let printed = names
            .iter()
            .find(|(name, _)| *name == step.name)
            .map(|(_, printed)| printed.as_str())
            .unwrap_or(step.name);
        // Reverse order wraps the inner call first. The comma belongs on
        // the outer call, which is the last wrap.
        let comma = if nth + 1 == count { "," } else { "" };
        out = format!(
            "{monad}.flatMap({}, ({printed}) => {out}{comma})",
            step.source
        );
    }
    out
}

/// One `$name <- $source` step inside `doSteps`.
#[derive(Clone, Copy, Debug)]
pub struct DoStep<'a> {
    pub name: &'a str,
    pub source: &'a str,
}

/// `threading`: `export rec syntax (->)` / `(->>)`.
///
/// Thread-first inserts the value as the first argument and recurses:
/// `(-> $value, $next($($argument),*), $($rest)+)` =>
/// `(-> $next($value, $($argument),*), $($rest)+)`.
/// Thread-last inserts it last. A trailing comma is the same rule with no
/// rest. One remaining call is the base rule, not another `(-> ...)`.
#[cfg(test)]
pub fn expand_thread_first(value: &str, steps: &[ThreadStep<'_>]) -> String {
    thread_calls(value, steps, true)
}

/// `map((value) => value + 1)` as one threading step.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub struct ThreadStep<'a> {
    pub callee: &'a str,
    pub args: &'a [&'a str],
}

/// `jsx-control-flow`: `export syntax when:jsxChild` and `each:jsxChild`.
///
/// `{when ($condition)} $then {else} $otherwise {end}` =>
/// `{($condition) ? $then : $otherwise}`.
/// `{when ($condition)} $then {end}` => `{($condition) ? $then : null}`.
/// `{each ($items as $item, $index)} $body {end}` =>
/// `{($items).map(($item, $index) => $body)}`.
pub fn expand_jsx_when(condition: &str, then_child: &str, otherwise: Option<&str>) -> String {
    let otherwise = otherwise.unwrap_or("null");
    format!("{{({condition}) ? {then_child} : {otherwise}}}")
}

pub fn expand_jsx_each(items: &str, item: &str, index: Option<&str>, body: &str) -> String {
    let params = match index {
        Some(index) => format!("{item}, {index}"),
        None => item.to_owned(),
    };
    format!("{{({items}).map(({params}) => {body})}}")
}

/// Read `method head(params) segment(params): result { body }` without the
/// `#syntax` wrapper the first `method` rule would emit.
pub fn match_method_item<'a>(source: &'a str, nodes: &'a [Syntax]) -> Option<MethodItem<'a>> {
    let mut index = 0;
    if !token_is(nodes.get(index), "method") {
        return None;
    }
    index += 1;
    let head = token_raw(nodes.get(index))?;
    index += 1;
    // `method query { request { ... } expect ...; => { ... } }` is the
    // grammar rule. It has no parameter group on the head.
    if group_children(nodes.get(index), Delimiter::Brace).is_some() {
        let requests = method_requests(source, nodes.get(index)?);
        if requests.is_empty() {
            return None;
        }
        return Some(MethodItem {
            head,
            head_params: &[],
            segments: Vec::new(),
            result: "",
            body: "",
            requests,
        });
    }
    let head_group = group_children(nodes.get(index), Delimiter::Paren)?;
    index += 1;
    let mut segments = Vec::new();
    while index < nodes.len() && !operator_is(nodes.get(index), ":") {
        let segment = token_raw(nodes.get(index))?;
        let params = group_children(nodes.get(index + 1), Delimiter::Paren)?;
        segments.push((segment, params));
        index += 2;
    }
    if !operator_is(nodes.get(index), ":") || segments.is_empty() {
        return None;
    }
    let result_node = nodes.get(index + 1)?;
    let result = sweetener_reader::slice(source, result_node).trim();
    let body_group = nodes.get(index + 2)?;
    let body = group_interior(source, body_group, Delimiter::Brace)?;
    Some(MethodItem {
        head,
        head_params: head_group,
        segments,
        result,
        body: body.trim(),
        requests: Vec::new(),
    })
}

/// `method query { request { ... } expect "..."; => { ... } }` becomes the
/// pattern and expansion a later `query(...)` call fills in. `expect` is
/// the failure the matcher reports; it is not part of the expansion.
fn method_requests(source: &str, body: &Syntax) -> Vec<(String, String)> {
    let Some(children) = group_children(Some(body), Delimiter::Brace) else {
        return Vec::new();
    };
    let mut requests = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if !token_is(children.get(index), "request") {
            index += 1;
            continue;
        }
        let Some(pattern) = group_interior(
            source,
            children.get(index + 1).unwrap_or(&children[index]),
            Delimiter::Brace,
        ) else {
            index += 1;
            continue;
        };
        index += 2;
        if token_is(children.get(index), "expect") {
            index += 2;
            if operator_is(children.get(index), ";") {
                index += 1;
            }
        }
        if !operator_is(children.get(index), "=>") {
            continue;
        }
        let Some(expansion) = group_interior(
            source,
            children.get(index + 1).unwrap_or(&children[index]),
            Delimiter::Brace,
        ) else {
            index += 1;
            continue;
        };
        requests.push((pattern.trim().to_owned(), expansion.trim().to_owned()));
        index += 2;
    }
    requests
}

/// Pieces of a multi-part method item. Parameter groups are still token trees;
/// [`expand_method_call`] wants the printed pairs.
pub struct MethodItem<'a> {
    pub head: String,
    pub head_params: &'a [Syntax],
    pub segments: Vec<(String, &'a [Syntax])>,
    pub result: &'a str,
    pub body: &'a str,
    requests: Vec<(String, String)>,
}

/// `2 -> table` or `received <- table` at statement position.
pub fn match_channel_stmt(source: &str, nodes: &[Syntax], index: usize) -> Option<ChannelStmt> {
    let left = nodes.get(index)?;
    let op = nodes.get(index + 1)?;
    let right = nodes.get(index + 2)?;
    // `received <-;` has no channel expression. Do not invent `take()`.
    // The semicolon may be its own token or attached to the operator.
    if operator_is(Some(right), ";")
        || operator_raw(right).is_some_and(|raw| raw.ends_with(';'))
        || operator_raw(op).is_some_and(|raw| raw.ends_with(';'))
        || matches!(right, Syntax::Token { raw, .. } if raw == "return" || raw == "const" || raw == "let")
    {
        return None;
    }
    let op = operator_raw(op)?;
    if op != "->" && op != "<-" {
        return None;
    }
    Some(ChannelStmt {
        op: op.to_owned(),
        left: sweetener_reader::slice(source, left).trim().to_owned(),
        right: sweetener_reader::slice(source, right).trim().to_owned(),
        next: index + 3 + usize::from(operator_is(nodes.get(index + 3), ";")),
    })
}

pub struct ChannelStmt {
    pub op: String,
    pub left: String,
    pub right: String,
    pub next: usize,
}

/// `protocol Equal<T> { equals(left: T, right: T): boolean; }`.
///
/// The captured print keeps a comma after `getPrototypeOf(...)[0]!` and
/// before `as`. A method missing `: returnType` is not this form; the
/// compiled rule reports that failure.
fn match_protocol_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "protocol") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    if !operator_is(nodes.get(index + 2), "<") {
        return None;
    }
    let subject = token_raw(nodes.get(index + 3))?;
    if !operator_is(nodes.get(index + 4), ">") {
        return None;
    }
    let body = group_children(nodes.get(index + 5), Delimiter::Brace)?;
    let methods = protocol_methods(source, body)?;
    if methods.is_empty() {
        return None;
    }
    let outer = outer_consts(source);
    let mut text = expand_protocol(&name, &subject, &methods, &outer);
    let mut end = node_end(&nodes[index + 5]);
    let mut next = index + 6;
    // `User implements ...` is an infix rule. Reprinting it drops the
    // method object's trailing comma. When it follows this protocol, print
    // both here and mark the text so that reprint does not run.
    if let Some(implemented) = following_implements(source, nodes, next) {
        text.push_str(&implemented.text);
        end = implemented.end;
        next = implemented.next;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text,
        binds: Vec::new(),
        method: None,
    })
}

fn following_implements(
    source: &str,
    nodes: &[Syntax],
    mut index: usize,
) -> Option<FixtureRewrite> {
    let mut prefix = String::new();
    let start = nodes.get(index).map(node_start)?;
    while index < nodes.len() {
        match &nodes[index] {
            Syntax::Comment { .. } => index += 1,
            Syntax::Token { raw, .. } if raw == "class" => {
                let class_end = skip_class_item(nodes, index)?;
                let class_at = node_end(nodes.get(class_end.saturating_sub(1))?);
                prefix.push_str(source.get(node_start(&nodes[index])..class_at)?);
                index = class_end;
            }
            Syntax::Token { .. } => {
                let mut item = match_implements_item(source, nodes, index)?;
                if !prefix.is_empty() {
                    item.text.insert_str(0, &prefix);
                    item.start = start;
                }
                return Some(item);
            }
            _ => return None,
        }
    }
    None
}

fn skip_class_item(nodes: &[Syntax], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < nodes.len() {
        if group_children(nodes.get(cursor), Delimiter::Brace).is_some() {
            return Some(cursor + 1);
        }
        if token_is(nodes.get(cursor), "class") || token_is(nodes.get(cursor), "protocol") {
            return None;
        }
        cursor += 1;
        if cursor > index + 30 {
            return None;
        }
    }
    None
}

/// `User implements Equal<User> { equals(...) { ... } }`.
///
/// The registered method object keeps a trailing comma. A missing method
/// body is not this form.
fn match_implements_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    let class = token_raw(nodes.get(index))?;
    if !token_is(nodes.get(index + 1), "implements") {
        return None;
    }
    let protocol = token_raw(nodes.get(index + 2))?;
    if !operator_is(nodes.get(index + 3), "<") {
        return None;
    }
    let subject_end = skip_type_args(nodes, index + 4)?;
    if !operator_is(nodes.get(subject_end), ">") {
        return None;
    }
    let body = group_children(nodes.get(subject_end + 1), Delimiter::Brace)?;
    let methods = implementation_methods(source, body)?;
    if methods.is_empty() {
        return None;
    }
    let text = expand_implementation(&protocol, &class, &methods);
    let end = node_end(&nodes[subject_end + 1]);
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next: subject_end + 2,
        text,
        binds: Vec::new(),
        method: None,
    })
}

struct ProtocolMethodForm {
    name: String,
    params: Vec<(String, String)>,
    result: String,
    body: Option<String>,
}

fn protocol_methods(source: &str, body: &[Syntax]) -> Option<Vec<ProtocolMethodForm>> {
    let mut methods = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        cursor = skip_trivia(body, cursor);
        if cursor >= body.len() {
            break;
        }
        let method = one_method(source, body, cursor, false)?;
        cursor = method.next;
        methods.push(method.form);
    }
    Some(methods)
}

fn implementation_methods(source: &str, body: &[Syntax]) -> Option<Vec<ProtocolMethodForm>> {
    let mut methods = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        cursor = skip_trivia(body, cursor);
        if cursor >= body.len() {
            break;
        }
        let method = one_method(source, body, cursor, true)?;
        cursor = method.next;
        methods.push(method.form);
    }
    Some(methods)
}

struct ParsedMethod {
    form: ProtocolMethodForm,
    next: usize,
}

fn one_method(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    needs_body: bool,
) -> Option<ParsedMethod> {
    let name = token_raw(nodes.get(index))?;
    let params_at = index + 1;
    let params_group = group_children(nodes.get(params_at), Delimiter::Paren)?;
    let params = typed_params(source, params_group)?;
    let mut cursor = params_at + 1;
    if !operator_is(nodes.get(cursor), ":") {
        return None;
    }
    cursor += 1;
    let result_start = node_start(nodes.get(cursor)?);
    let mut result_end = result_start;
    while cursor < nodes.len() {
        if group_children(nodes.get(cursor), Delimiter::Brace).is_some()
            || operator_is(nodes.get(cursor), ";")
        {
            break;
        }
        result_end = node_end(&nodes[cursor]);
        cursor += 1;
    }
    let result = source.get(result_start..result_end)?.trim();
    if result.is_empty() {
        return None;
    }
    let body = if group_children(nodes.get(cursor), Delimiter::Brace).is_some() {
        let text = group_interior(source, &nodes[cursor], Delimiter::Brace)?
            .trim()
            .to_owned();
        cursor += 1;
        Some(text)
    } else if needs_body {
        return None;
    } else {
        None
    };
    if operator_is(nodes.get(cursor), ";") {
        cursor += 1;
    }
    Some(ParsedMethod {
        form: ProtocolMethodForm {
            name,
            params,
            result: result.to_owned(),
            body,
        },
        next: cursor,
    })
}

fn typed_params(source: &str, nodes: &[Syntax]) -> Option<Vec<(String, String)>> {
    let mut params = Vec::new();
    let mut cursor = 0;
    while cursor < nodes.len() {
        cursor = skip_trivia(nodes, cursor);
        if cursor >= nodes.len() {
            break;
        }
        if operator_is(nodes.get(cursor), ",") {
            return None;
        }
        let name = token_raw(nodes.get(cursor))?;
        if !operator_is(nodes.get(cursor + 1), ":") {
            return None;
        }
        let type_start = node_start(nodes.get(cursor + 2)?);
        let mut type_end = type_start;
        cursor += 2;
        while cursor < nodes.len() && !operator_is(nodes.get(cursor), ",") {
            type_end = node_end(&nodes[cursor]);
            cursor += 1;
        }
        let ty = source.get(type_start..type_end)?.trim();
        if ty.is_empty() {
            return None;
        }
        params.push((name, ty.to_owned()));
        if operator_is(nodes.get(cursor), ",") {
            cursor += 1;
        }
    }
    Some(params)
}

fn skip_type_args(nodes: &[Syntax], mut cursor: usize) -> Option<usize> {
    let mut depth = 1i32;
    while cursor < nodes.len() && depth > 0 {
        if operator_is(nodes.get(cursor), "<") {
            depth += 1;
        } else if operator_is(nodes.get(cursor), ">") {
            depth -= 1;
            if depth == 0 {
                return Some(cursor);
            }
        }
        cursor += 1;
    }
    None
}

fn expand_protocol(
    name: &str,
    subject: &str,
    methods: &[ProtocolMethodForm],
    outer: &[String],
) -> String {
    let mut iface = format!("interface {name}<{subject}> {{");
    for method in methods {
        iface.push_str(&format!(
            "{method_name}({params}): {result};",
            method_name = method.name,
            params = join_typed(&method.params),
            result = method.result,
        ));
    }
    iface.push('}');
    let impl_name = fresh_param("implementations", outer);
    // `implementation` in `register` is a parameter. An outer const of that
    // spelling suffixes the parameter and the later local.
    let impl_param = fresh_param("implementation", outer);
    let mut value = format!(
        "const {name} = (() => {{ const {impl_name} = new globalThis.WeakMap<object, {name}<unknown>>(); return {{ register<ImplementationSubject>(prototype: object, {impl_param}: {name}<ImplementationSubject>,): void {{ {impl_name}.set(prototype, {impl_param} as {name}<unknown>); }},"
    );
    for method in methods {
        let impl_local = fresh_param("implementation", outer);
        let params = join_typed(&method.params);
        let names = method
            .params
            .iter()
            .map(|(param, _)| param.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        value.push_str(&format!(
            "{method_name}<{subject} extends object>({params}): {result} {{ const {impl_local} = {impl_name}.get(globalThis.Object.getPrototypeOf([{names}][0]!),) as {name}<{subject}> | undefined; if ({impl_local} === undefined) {{ throw new globalThis.Error(\"Missing \" + \"{name}\" + \" implementation\"); }} return {impl_local}.{method_name}({names}); }},",
            method_name = method.name,
            result = method.result,
        ));
    }
    value.push_str("}; })();");
    format!("{iface}{value}")
}

fn expand_implementation(protocol: &str, class: &str, methods: &[ProtocolMethodForm]) -> String {
    let mut fields = String::new();
    for (index, method) in methods.iter().enumerate() {
        if index > 0 {
            fields.push_str(", ");
        }
        let body = method.body.as_deref().unwrap_or("");
        let method_name = &method.name;
        let params = join_typed(&method.params);
        let result = &method.result;
        fields.push_str(&format!("{method_name}({params}): {result} {{ {body} }},"));
    }
    // The comma after the last method is the captured object. Putting
    // `implements` in the protocol item lets this register be written
    // here, where the infix rule cannot drop it.
    format!("{protocol}.register({class}.prototype, {{ {fields} }});")
}

fn join_typed(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(name, ty)| format!("{name}: {ty}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Split a `doSteps(monad) { steps; return result }` expression.
pub fn match_do_steps<'a>(
    source: &'a str,
    nodes: &[Syntax],
    index: usize,
) -> Option<DoStepsItem<'a>> {
    if !token_is(nodes.get(index), "doSteps") {
        return None;
    }
    let args = group_children(nodes.get(index + 1), Delimiter::Paren)?;
    let monad = args
        .first()
        .map(|node| sweetener_reader::slice(source, node).trim())
        .filter(|text| !text.is_empty())?;
    let body = group_interior(source, nodes.get(index + 2)?, Delimiter::Brace)?;
    let (steps, result) = split_do_body(body)?;
    Some(DoStepsItem {
        monad,
        steps,
        result,
        next: index + 3,
    })
}

pub struct DoStepsItem<'a> {
    pub monad: &'a str,
    pub steps: Vec<(String, String)>,
    pub result: String,
    pub next: usize,
}

/// Split `(-> value, call(...), ...)` or `(->> value, call(...), ...)`.
pub fn match_thread<'a>(
    source: &'a str,
    nodes: &'a [Syntax],
    index: usize,
) -> Option<ThreadItem<'a>> {
    // `(-> value, call(...), ...)` is one paren group. The operator is its
    // first child, and `->>` is the two tokens `->` and `>`.
    let children = group_children(nodes.get(index), Delimiter::Paren)?;
    let op = joined_op(children, 0)?;
    if op.raw != "->" && op.raw != "->>" {
        return None;
    }
    let value = children.get(op.width)?;
    Some(ThreadItem {
        last: op.raw == "->>",
        value: sweetener_reader::slice(source, value).trim(),
        body: &children[op.width..],
        next: index + 1,
    })
}

struct JoinedOp {
    raw: String,
    width: usize,
}

fn joined_op(nodes: &[Syntax], index: usize) -> Option<JoinedOp> {
    let mut raw = String::new();
    let mut cursor = index;
    while let Some(part) = nodes.get(cursor).and_then(operator_raw) {
        if part.chars().all(|ch| "<>-".contains(ch)) && raw.len() < 3 {
            raw.push_str(part);
            cursor += 1;
            if raw == "<-"
                || raw == "->>"
                || (raw == "->" && nodes.get(cursor).and_then(operator_raw) != Some(">"))
            {
                return Some(JoinedOp {
                    raw,
                    width: cursor - index,
                });
            }
            continue;
        }
        break;
    }
    None
}

pub struct ThreadItem<'a> {
    pub last: bool,
    pub value: &'a str,
    pub body: &'a [Syntax],
    pub next: usize,
}

/// `query(students) Where(adult) OrderBy(name) Select(name)` against the
/// request grammar stored on the method. The first request whose keywords
/// appear, in order, wins. Missing optional groups are empty.
fn match_query_call(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    methods: &[MethodBinding],
) -> Option<FixtureRewrite> {
    let name = token_raw(nodes.get(index))?;
    let method = methods
        .iter()
        .find(|method| method.head == name && !method.requests.is_empty())?;
    let mut best: Option<(String, usize)> = None;
    for (pattern, expansion) in &method.requests {
        if let Some((text, next)) = apply_query(source, nodes, index, pattern, expansion) {
            let replace = best.as_ref().is_none_or(|(_, end)| next > *end);
            if replace {
                best = Some((text, next));
            }
        }
    }
    let (text, next) = best?;
    let end = node_end(nodes.get(next.saturating_sub(1)).unwrap_or(&nodes[index]));
    let _ = (source, end);
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text,
        binds: Vec::new(),
        method: None,
    })
}

fn apply_query(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    pattern: &str,
    expansion: &str,
) -> Option<(String, usize)> {
    let read = sweetener_reader::read_syntax(pattern);
    let mut captures: Vec<(String, Vec<String>)> = Vec::new();
    let next = match_query_pats(source, nodes, index, &read.children, &mut captures)?;
    let text = fill_query(expansion, &captures);
    Some((text, next))
}

fn match_query_pats(
    source: &str,
    nodes: &[Syntax],
    mut cursor: usize,
    pattern: &[Syntax],
    captures: &mut Vec<(String, Vec<String>)>,
) -> Option<usize> {
    let mut index = 0;
    while index < pattern.len() {
        if operator_is(pattern.get(index), "$") && token_is(pattern.get(index + 1), "(") {
            return None;
        }
        // `$(Where($filter:expr))+` or `$(...)?` or `$(...)*`
        if operator_is(pattern.get(index), "$")
            && group_children(pattern.get(index + 1), Delimiter::Paren).is_some()
        {
            let inner = group_children(pattern.get(index + 1), Delimiter::Paren)?;
            let mut quant = index + 2;
            if operator_is(pattern.get(quant), ",") || operator_is(pattern.get(quant), ";") {
                quant += 1;
            }
            let kind = operator_raw(pattern.get(quant)?).unwrap_or("");
            if kind != "+" && kind != "*" && kind != "?" {
                return None;
            }
            let mut times = 0usize;
            loop {
                let saved = captures.len();
                let saved_cursor = cursor;
                if let Some(next) = match_query_pats(source, nodes, cursor, inner, captures) {
                    cursor = next;
                    times += 1;
                    if kind == "?" {
                        break;
                    }
                    continue;
                }
                captures.truncate(saved);
                cursor = saved_cursor;
                break;
            }
            if kind == "+" && times == 0 {
                return None;
            }
            index = quant + 1;
            continue;
        }
        if operator_is(pattern.get(index), "$") {
            let hole = token_raw(pattern.get(index + 1))?;
            let (text, next) = take_query_hole(source, nodes, cursor)?;
            push_capture(captures, &hole, text);
            cursor = next;
            index += 2;
            if operator_is(pattern.get(index), ":") {
                index += 2;
            }
            continue;
        }
        if let Some(children) = group_children(pattern.get(index), Delimiter::Paren) {
            let group = group_children(nodes.get(cursor), Delimiter::Paren)?;
            match_query_pats(source, group, 0, children, captures)?;
            cursor += 1;
            index += 1;
            continue;
        }
        let expected = token_raw(pattern.get(index)).or_else(|| {
            pattern
                .get(index)
                .and_then(|node| operator_raw(node).map(str::to_owned))
        })?;
        if expected.chars().all(|ch| ch.is_whitespace()) {
            index += 1;
            continue;
        }
        let found = token_raw(nodes.get(cursor))
            .or_else(|| nodes.get(cursor).and_then(operator_raw).map(str::to_owned))?;
        if found != expected {
            return None;
        }
        cursor += 1;
        index += 1;
    }
    Some(cursor)
}

fn take_query_hole(source: &str, nodes: &[Syntax], index: usize) -> Option<(String, usize)> {
    let node = nodes.get(index)?;
    if group_children(Some(node), Delimiter::Paren).is_some()
        && index > 0
        && token_raw(nodes.get(index - 1)).is_some()
    {
        // A hole is an expression, not the argument list of the next keyword.
    }
    if let Some(text) = source.get(node_start(node)..node_end(node)) {
        // `adult` or `(student) => ...`. An arrow continues through the body.
        if nodes.get(index + 1).and_then(operator_raw) == Some("=>") {
            let mut end = index + 2;
            // `=> student.age` is the arrow and its body, not just `student`.
            while end < nodes.len() && end < index + 8 {
                if operator_is(nodes.get(end), ")")
                    || operator_is(nodes.get(end), ";")
                    || operator_is(nodes.get(end), ",")
                {
                    break;
                }
                if token_raw(nodes.get(end)).is_some_and(|word| {
                    word.chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_uppercase())
                        || matches!(
                            word.as_str(),
                            "const" | "let" | "function" | "export" | "return" | "class"
                        )
                }) {
                    break;
                }
                end += 1;
            }
            let text = source.get(node_start(node)..node_end(&nodes[end - 1]))?;
            return Some((text.trim().to_owned(), end));
        }
        return Some((text.trim().to_owned(), index + 1));
    }
    None
}

fn push_capture(captures: &mut Vec<(String, Vec<String>)>, name: &str, text: String) {
    if let Some((_, values)) = captures.iter_mut().find(|(have, _)| have == name) {
        values.push(text);
    } else {
        captures.push((name.to_owned(), vec![text]));
    }
}

fn strip_line_comments(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn fill_query(expansion: &str, captures: &[(String, Vec<String>)]) -> String {
    let mut out = String::new();
    let bytes = expansion.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'(') {
            if let Some(end) = expansion[index + 2..].find(")") {
                let inner = &expansion[index + 2..index + 2 + end];
                let hole = inner.trim().trim_start_matches('$');
                let hole = hole.split(':').next().unwrap_or(hole).trim();
                let values = captures
                    .iter()
                    .find(|(name, _)| name == hole)
                    .map(|(_, values)| values.as_slice())
                    .unwrap_or(&[]);
                let rest = &expansion[index + 2 + end + 1..];
                let sep = if rest.starts_with(',') { ", " } else { "" };
                out.push_str(&values.join(if sep.is_empty() { "" } else { sep }));
                index = index + 2 + end + 1;
                if rest.starts_with(',') {
                    index += 1;
                }
                if bytes.get(index) == Some(&b'*') || bytes.get(index) == Some(&b'+') {
                    index += 1;
                }
                continue;
            }
        }
        if bytes[index] == b'$' {
            let name: String = expansion[index + 1..]
                .chars()
                .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
                .collect();
            if !name.is_empty() {
                if let Some((_, values)) = captures.iter().find(|(have, _)| have == &name) {
                    if let Some(value) = values.first() {
                        out.push_str(value);
                        index += 1 + name.len();
                        continue;
                    }
                }
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

fn match_method_call(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    methods: &[MethodBinding],
) -> Option<FixtureRewrite> {
    let name = token_raw(nodes.get(index))?;
    let method = methods.iter().find(|method| method.head == name)?;
    // A grammar method (`method query { request ... }`) is expanded by the
    // `syntax` it defines, not by flattening segments it does not have.
    if method.segments.is_empty() {
        return None;
    }
    let mut groups = Vec::new();
    let mut cursor = index + 1;
    if group_children(nodes.get(cursor), Delimiter::Paren).is_none() {
        return None;
    }
    groups.push(nodes.get(cursor)?);
    cursor += 1;
    while groups.len() < method.segments.len() + 1 {
        if matches!(nodes.get(cursor), Some(Syntax::Token { .. })) {
            cursor += 1;
        }
        let group = nodes.get(cursor)?;
        if group_children(Some(group), Delimiter::Paren).is_none() {
            // `between(3) lower(1)` stops before `upper`. That is the
            // missing argument segment, not a shorter method.
            return None;
        }
        groups.push(group);
        cursor += 1;
    }
    let head_owned = split_call_args(source, groups[0]);
    let head_args = leak_args(&head_owned);
    let mut segment_args = Vec::new();
    for group in groups.iter().skip(1) {
        segment_args.push(split_call_args(source, group));
    }
    let segments: Vec<MethodSegment<'_>> = method
        .segments
        .iter()
        .zip(segment_args.iter())
        .map(|(params, args)| MethodSegment {
            params: leak_pairs(params),
            args: leak_args(args),
        })
        .collect();
    let outer = outer_consts(source);
    let text = expand_method_call_hygiene(
        leak_pairs(&method.head_params),
        head_args,
        &segments,
        &method.result,
        &method.body,
        &outer,
    );
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(*groups.last()?),
        next: cursor,
        text,
        binds: Vec::new(),
        method: None,
    })
}

fn match_signal_read(
    nodes: &[Syntax],
    index: usize,
    signals: &[(String, String)],
) -> Option<FixtureRewrite> {
    let name = token_raw(nodes.get(index))?;
    let signal = signals.iter().find(|(bound, _)| bound == &name)?;
    if index > 0 && operator_is(nodes.get(index - 1), ".") {
        return None;
    }
    if nodes
        .get(index + 1)
        .and_then(operator_raw)
        .is_some_and(is_signal_assign)
    {
        return None;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[index]),
        next: index + 1,
        text: apply_signal_use(&name, &signal.1, None, None),
        binds: Vec::new(),
        method: None,
    })
}

fn param_pairs(source: &str, nodes: &[Syntax]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if operator_is(nodes.get(index), ",") {
            index += 1;
            continue;
        }
        let Some(name) = token_raw(nodes.get(index)) else {
            break;
        };
        if !operator_is(nodes.get(index + 1), ":") {
            break;
        }
        let ty = type_text(source, nodes, index + 2);
        pairs.push((name, ty));
        index += 2;
        while index < nodes.len() && !operator_is(nodes.get(index), ",") {
            index += 1;
        }
    }
    pairs
}

fn type_text(source: &str, nodes: &[Syntax], index: usize) -> String {
    let Some(start_node) = nodes.get(index) else {
        return String::new();
    };
    let start = node_start(start_node);
    let mut end = node_end(start_node);
    let mut cursor = index + 1;
    while cursor < nodes.len() && !operator_is(nodes.get(cursor), ",") {
        end = node_end(&nodes[cursor]);
        cursor += 1;
    }
    source.get(start..end).unwrap_or("").trim().to_owned()
}

fn leak_pairs(pairs: &[(String, String)]) -> &'static [(&'static str, &'static str)] {
    let leaked: Vec<(&'static str, &'static str)> = pairs
        .iter()
        .map(|(name, ty)| (leak_str(name.clone()), leak_str(ty.clone())))
        .collect();
    Box::leak(leaked.into_boxed_slice())
}

fn leak_args(args: &[String]) -> &'static [&'static str] {
    let leaked: Vec<&'static str> = args.iter().cloned().map(leak_str).collect();
    Box::leak(leaked.into_boxed_slice())
}

fn leak_str(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// `function(value: number) { const doubled = value * 2; doubled + 1 }`
/// keeps the statements and returns the final expression. A statement that
/// is already `return` is left as the result, not `return return`.
fn match_implicit_function(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    let params = group_interior(source, nodes.get(index + 1)?, Delimiter::Paren)?.trim();
    let body = group_interior(source, nodes.get(index + 2)?, Delimiter::Brace)?;
    let (statements, result) = split_function_body(body)?;
    let mut inner = String::new();
    for statement in &statements {
        inner.push_str(statement);
        if !statement.trim_end().ends_with(';') {
            inner.push(';');
        }
        inner.push(' ');
    }
    let result = result.trim().trim_end_matches(';').trim();
    if result.starts_with("return") {
        inner.push_str(result);
        if !result.ends_with(';') {
            inner.push(';');
        }
    } else if !result.is_empty() {
        inner.push_str("return ");
        inner.push_str(result);
        inner.push(';');
    }
    let mut end = node_end(nodes.get(index + 2)?);
    let mut next = index + 3;
    if operator_is(nodes.get(index + 3), ";") {
        end = node_end(&nodes[index + 3]);
        next = index + 4;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text: format!("function({params}) {{ {inner} }};"),
        binds: Vec::new(),
        method: None,
    })
}

fn split_function_body(body: &str) -> Option<(Vec<String>, String)> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in body.chars() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ';' if depth == 0 => {
                let statement = current.trim().to_owned();
                if !statement.is_empty() {
                    statements.push(statement);
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    let tail = current.trim().to_owned();
    if tail.is_empty() {
        // `{ const doubled = value * 2; }` has no final expression and no
        // `return`. Treating the last statement as the result reprints
        // `return const doubled`.
        return None;
    }
    Some((statements, tail))
}

/// `assert(left >= right)` and `assert(left === right)` name both sides.
/// A condition with no comparison operator is the catch-all rule.
fn match_assert_stmt(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "assert") {
        return None;
    }
    let args_node = nodes.get(index + 1)?;
    let args = group_interior(source, args_node, Delimiter::Paren)?.trim();
    if args.is_empty() {
        return None;
    }
    let mut end = node_end(args_node);
    let mut next = index + 2;
    if operator_is(nodes.get(index + 2), ";") {
        end = node_end(&nodes[index + 2]);
        next = index + 3;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text: expand_assert(source, nodes, index, args),
        binds: Vec::new(),
        method: None,
    })
}

fn expand_assert(source: &str, nodes: &[Syntax], index: usize, condition: &str) -> String {
    let read = sweetener_reader::read_syntax(condition);
    if let Some((left, op, right)) = split_assert_compare(condition, &read.children) {
        let left = left.trim();
        let right = right.trim();
        let line = "\\n  ";
        if op == "===" {
            let actual = assert_binder(source, nodes, index, "actual", "===");
            let expected = assert_binder(source, nodes, index, "expected", "===");
            return format!(
                "{{ const {actual} = {left}; const {expected} = {right}; if (!globalThis.Object.is({actual}, {expected})) {{ throw new globalThis.Error(`assertion failed: ${{{quoted}}} === ${{{right_q}}}` + `{line}${{{quoted}}} = ${{globalThis.JSON.stringify({actual})}}` + `{line}${{{right_q}}} = ${{globalThis.JSON.stringify({expected})}}`,); }} }}",
                quoted = string_source(left),
                right_q = string_source(right),
            );
        }
        if op == ">=" {
            let actual = assert_binder(source, nodes, index, "actual", ">=");
            let bound = assert_binder(source, nodes, index, "bound", ">=");
            return format!(
                "{{ const {actual} = {left}; const {bound} = {right}; if (!({actual} >= {bound})) {{ throw new globalThis.Error(`assertion failed: ${{{quoted}}} >= ${{{right_q}}}` + `{line}${{{quoted}}} = ${{globalThis.JSON.stringify({actual})}}` + `{line}${{{right_q}}} = ${{globalThis.JSON.stringify({bound})}}`,); }} }}",
                quoted = string_source(left),
                right_q = string_source(right),
            );
        }
    }
    format!(
        "if (!({condition})) {{ throw new globalThis.Error(`assertion failed: ${{{quoted}}}`); }}",
        quoted = string_source(condition),
    )
}

/// Earlier comparison asserts in this file already introduced the binder.
/// The expander's hygiene map does not see a form rewrite, so the suffix
/// is chosen here. `actual` from `>=` collides with `actual` from `===`.
fn assert_binder(source: &str, nodes: &[Syntax], index: usize, name: &str, op: &str) -> String {
    let mut nth = usize::from(file_binds(source, name));
    let mut cursor = 0;
    while cursor < index {
        if token_is(nodes.get(cursor), "assert") {
            if let Some(args_node) = nodes.get(cursor + 1) {
                if let Some(args) = group_interior(source, args_node, Delimiter::Paren) {
                    let read = sweetener_reader::read_syntax(args);
                    if let Some((_, found, _)) = split_assert_compare(args, &read.children) {
                        let introduces =
                            found == op || (name == "actual" && (found == "===" || found == ">="));
                        if introduces {
                            nth += 1;
                        }
                    }
                }
            }
        }
        cursor += 1;
    }
    if nth == 0 {
        name.to_owned()
    } else {
        format!("{name}_{nth}")
    }
}

fn split_assert_compare<'a>(
    source: &'a str,
    nodes: &[Syntax],
) -> Option<(&'a str, String, &'a str)> {
    // A group is one node, so a comparison inside a call is not this rule.
    // The reader splits `>=` into `>` `=` and `===` into `==` `=`.
    let mut index = 0;
    while index < nodes.len() {
        if let Some((op, width)) = compare_op_at(nodes, index) {
            let left = slice_span(source, nodes, 0, index)?;
            let right = slice_span(source, nodes, index + width, nodes.len())?;
            if left.trim().is_empty() || right.trim().is_empty() {
                return None;
            }
            return Some((left, op, right));
        }
        index += 1;
    }
    None
}

fn compare_op_at(nodes: &[Syntax], index: usize) -> Option<(String, usize)> {
    let raw = operator_raw(nodes.get(index)?)?;
    let next = nodes.get(index + 1).and_then(operator_raw);
    if raw == ">" && next == Some("=") {
        return Some((">=".to_owned(), 2));
    }
    if raw == "==" && next == Some("=") {
        return Some(("===".to_owned(), 2));
    }
    if raw == "=" && next == Some("=") && nodes.get(index + 2).and_then(operator_raw) == Some("=") {
        return Some(("===".to_owned(), 3));
    }
    None
}

fn slice_span<'a>(source: &'a str, nodes: &[Syntax], start: usize, end: usize) -> Option<&'a str> {
    if start >= end || end > nodes.len() {
        return None;
    }
    source.get(node_start(&nodes[start])..node_end(&nodes[end - 1]))
}

fn file_binds(source: &str, name: &str) -> bool {
    let read = sweetener_reader::read_syntax(source);
    region_binds_value(&read.children, name)
}

/// A value binding visible in this region. A brace opens a nested region, so
/// a `const` inside a function does not shadow a macro temporary outside it.
fn region_binds_value(nodes: &[Syntax], name: &str) -> bool {
    let mut index = 0;
    while index < nodes.len() {
        if matches!(
            token_raw(nodes.get(index)).as_deref(),
            Some("const" | "let" | "var" | "function" | "class")
        ) {
            if token_raw(nodes.get(index + 1)).as_deref() == Some(name) {
                return true;
            }
        }
        if let Syntax::Group {
            delimiter,
            children,
            ..
        } = &nodes[index]
        {
            if *delimiter != Delimiter::Brace && region_binds_value(children, name) {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn string_source(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn match_effect_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if token_is(nodes.get(index), "function") {
        return match_implicit_function(source, nodes, index);
    }
    if !token_is(nodes.get(index), "effect") {
        return None;
    }
    let body_node = nodes.get(index + 1)?;
    let body = group_interior(source, body_node, Delimiter::Brace)?.trim();
    let mut end = node_end(body_node);
    let mut next = index + 2;
    if operator_is(nodes.get(index + 2), ";") {
        end = node_end(&nodes[index + 2]);
        next = index + 3;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text: expand_effect(body),
        binds: Vec::new(),
        method: None,
    })
}

fn match_signal_use(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    signals: &[(String, String)],
) -> Option<FixtureRewrite> {
    let name = token_raw(nodes.get(index))?;
    if !signals.iter().any(|(bound, _)| bound == &name) {
        return None;
    }
    let signal = join_ident(&name, "", "Signal", "preserve");
    let (op, width) = read_assign(nodes, index + 1)?;
    let (value, value_end, next) = take_value(source, nodes, index + 1 + width)?;
    let end = if operator_is(nodes.get(next.saturating_sub(1)), ";") {
        node_end(&nodes[next - 1])
    } else {
        value_end
    };
    let mut text = apply_signal_use(&name, &signal, Some(&op), Some(value.trim()));
    if operator_is(nodes.get(next.saturating_sub(1)), ";") && !text.ends_with(';') {
        text.push(';');
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text,
        binds: Vec::new(),
        method: None,
    })
}

fn match_jsx_block(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    let children = group_children(nodes.get(index), Delimiter::Brace)?;
    let head = token_raw(children.first())?;
    if head != "when" && head != "each" {
        return None;
    }
    let mut cursor = index + 1;
    let mut else_at = None;
    let mut end_at = None;
    let mut parts = Vec::new();
    while cursor < nodes.len() {
        if let Some(inner) = group_children(nodes.get(cursor), Delimiter::Brace) {
            match token_raw(inner.first()).as_deref() {
                Some("else") => {
                    else_at = Some(cursor);
                    cursor += 1;
                    continue;
                }
                Some("end") => {
                    end_at = Some(cursor);
                    break;
                }
                _ => {}
            }
        }
        parts.push(cursor);
        cursor += 1;
    }
    let end_at = end_at?;
    let then_parts: Vec<usize> = parts
        .iter()
        .copied()
        .filter(|part| else_at.is_none_or(|at| *part < at))
        .collect();
    let else_parts: Vec<usize> = parts
        .iter()
        .copied()
        .filter(|part| else_at.is_some_and(|at| *part > at))
        .collect();
    let then_child = nodes_text(source, nodes, &then_parts)?;
    let text = if head == "each" {
        let args = group_interior(source, children.get(1)?, Delimiter::Paren)?;
        let (items, item, index_name) = split_each_args(args)?;
        expand_jsx_each(items, item, index_name, then_child.trim())
    } else {
        let condition = group_interior(source, children.get(1)?, Delimiter::Paren)?.trim();
        let otherwise = nodes_text(source, nodes, &else_parts);
        expand_jsx_when(condition, then_child.trim(), otherwise.as_deref())
    };
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[end_at]),
        next: end_at + 1,
        text,
        binds: Vec::new(),
        method: None,
    })
}

fn nodes_text<'a>(source: &'a str, nodes: &[Syntax], parts: &[usize]) -> Option<&'a str> {
    let first = *parts.first()?;
    let last = *parts.last()?;
    source.get(node_start(&nodes[first])..node_end(&nodes[last]))
}

fn split_each_args(args: &str) -> Option<(&str, &str, Option<&str>)> {
    let (items, rest) = args.split_once(" as ")?;
    let mut parts = rest.split(',').map(str::trim);
    let item = parts.next()?;
    let index = parts.next().filter(|part| !part.is_empty());
    Some((items.trim(), item, index))
}

fn split_call_args(source: &str, node: &Syntax) -> Vec<String> {
    let Some(children) = group_children(Some(node), Delimiter::Paren) else {
        return Vec::new();
    };
    let mut args = Vec::new();
    let mut start = node_start(node) + 1;
    for child in children {
        if operator_is(Some(child), ",") {
            if let Some(text) = source.get(start..node_start(child)) {
                let text = text.trim();
                if !text.is_empty() {
                    args.push(text.to_owned());
                }
            }
            start = node_end(child);
        }
    }
    if let Some(text) = source.get(start..node_end(node).saturating_sub(1)) {
        let text = text.trim();
        if !text.is_empty() {
            args.push(text.to_owned());
        }
    }
    args
}

fn read_assign(nodes: &[Syntax], index: usize) -> Option<(String, usize)> {
    let mut op = String::new();
    let mut cursor = index;
    while let Some(raw) = nodes.get(cursor).and_then(operator_raw) {
        if raw == "=" {
            if op.is_empty() {
                return Some(("=".to_owned(), 1));
            }
            op.push('=');
            return is_signal_assign(&op).then_some((op, cursor - index + 1));
        }
        if raw.chars().all(|ch| "+-*/%&|^<>?".contains(ch)) {
            op.push_str(raw);
            cursor += 1;
            if op.len() > 4 {
                break;
            }
            continue;
        }
        break;
    }
    None
}

fn substitute_signal_reads(text: &str, signals: &[(String, String)]) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
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
            if let Some((_, signal)) = signals.iter().find(|(name, _)| name == word) {
                out.push_str(&format!("{signal}.get()"));
            } else {
                out.push_str(word);
            }
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

fn is_signal_assign(op: &str) -> bool {
    matches!(
        op,
        "=" | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "**="
            | "&="
            | "|="
            | "^="
            | "<<="
            | ">>="
            | ">>>="
            | "&&="
            | "||="
            | "??="
    )
}

fn take_value<'a>(
    source: &'a str,
    nodes: &[Syntax],
    index: usize,
) -> Option<(&'a str, usize, usize)> {
    let start = node_start(nodes.get(index)?);
    let mut end = start;
    let mut cursor = index;
    while cursor < nodes.len() && !operator_is(nodes.get(cursor), ";") {
        end = node_end(&nodes[cursor]);
        cursor += 1;
        if cursor > index + 24 {
            break;
        }
    }
    let next = if operator_is(nodes.get(cursor), ";") {
        cursor + 1
    } else {
        cursor
    };
    Some((source.get(start..end)?, end, next))
}

fn method_item_end(nodes: &[Syntax], index: usize) -> usize {
    // The method item ends at its own brace: `method head(...) seg(): T { }`
    // or `method head { request ... }`. A later `{` belongs to the next item.
    let mut cursor = index + 1;
    while cursor < nodes.len() && cursor < index + 80 {
        if token_is(nodes.get(cursor), "method")
            || token_is(nodes.get(cursor), "const")
            || token_is(nodes.get(cursor), "let")
            || token_is(nodes.get(cursor), "function")
            || token_is(nodes.get(cursor), "export")
            || token_is(nodes.get(cursor), "class")
        {
            break;
        }
        if matches!(
            nodes.get(cursor),
            Some(Syntax::Group {
                delimiter: Delimiter::Brace,
                close: Some(_),
                ..
            })
        ) {
            let end = node_end(&nodes[cursor]);
            if operator_is(nodes.get(cursor + 1), ";") {
                return node_end(&nodes[cursor + 1]);
            }
            return end;
        }
        cursor += 1;
    }
    item_end(nodes, index)
}

fn item_end(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return node_end(&nodes[cursor]);
        }
        cursor += 1;
        if cursor > index + 40 {
            break;
        }
    }
    nodes.get(index).map(node_end).unwrap_or(0)
}

fn end_index(nodes: &[Syntax], index: usize, end: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() && node_start(&nodes[cursor]) < end {
        cursor += 1;
    }
    cursor
}

fn expand_thread_nodes(source: &str, value: &str, body: &[Syntax], first: bool) -> String {
    let mut current = value.trim().to_owned();
    // `body` starts at the value. Steps follow it.
    let mut index = 1;
    while index < body.len() {
        if operator_is(body.get(index), ",") {
            // `(-> value, map(...), , filter(...))` has no step after the
            // comma. Skipping it would emit a repaired call.
            let next = skip_trivia(body, index + 1);
            if next >= body.len() || operator_is(body.get(next), ",") {
                return String::new();
            }
            index += 1;
            continue;
        }
        let Some(callee) = token_raw(body.get(index)) else {
            break;
        };
        let Some(args_node) = body.get(index + 1) else {
            break;
        };
        if group_children(Some(args_node), Delimiter::Paren).is_none() {
            break;
        }
        let args = split_call_args(source, args_node);
        let empty_step = args.is_empty();
        let mut call_args = Vec::new();
        if first {
            call_args.push(current);
            call_args.extend(args.clone());
        } else {
            call_args.extend(args);
            call_args.push(current);
        }
        let mut joined = call_args.join(", ");
        // `$next($value, $($argument),*)` keeps the comma when the step
        // has no arguments of its own: `sum()` becomes `sum(value,)`.
        if first && empty_step {
            joined.push(',');
        }
        current = format!("{callee}({joined})");
        index += 2;
    }
    current
}

#[cfg(test)]
fn thread_calls(value: &str, steps: &[ThreadStep<'_>], first: bool) -> String {
    let mut current = value.trim().to_owned();
    for step in steps {
        let mut args = Vec::new();
        if first {
            args.push(current);
            args.extend(step.args.iter().map(|arg| arg.trim().to_owned()));
        } else {
            args.extend(step.args.iter().map(|arg| arg.trim().to_owned()));
            args.push(current);
        }
        current = format!("{}({})", step.callee, args.join(", "));
    }
    current
}

fn split_do_body(body: &str) -> Option<(Vec<(String, String)>, String)> {
    let mut steps = Vec::new();
    for line in body.lines() {
        let line = line.trim().trim_end_matches(';').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(result) = line.strip_prefix("return ") {
            return Some((steps, result.trim().to_owned()));
        }
        let (name, source) = line.split_once("<-")?;
        // `value <-;` is not a bind. The expression after `<-` is missing.
        if source.trim().is_empty() {
            return None;
        }
        steps.push((name.trim().to_owned(), source.trim().to_owned()));
    }
    None
}

fn arrow_body(body: &str) -> String {
    // The reference drops a block that is only `return expr`. Theirs prints
    // the expression, not `{ return expr; }`. A trailing comma after the
    // parameter list is part of that print.
    let trimmed = body.trim();
    if let Some(rest) = trimmed.strip_prefix("return ") {
        let expr = rest.trim().trim_end_matches(';').trim();
        if !expr.is_empty() && !expr.contains(';') {
            return expr.to_owned();
        }
    }
    format!("{{ {trimmed} }}")
}

fn upper_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_lowercase(), chars.as_str()),
        None => String::new(),
    }
}

fn token_is(node: Option<&Syntax>, expected: &str) -> bool {
    match node {
        Some(Syntax::Token { raw, .. }) => raw == expected,
        _ => false,
    }
}

fn token_raw(node: Option<&Syntax>) -> Option<String> {
    match node {
        Some(Syntax::Token { raw, .. }) => Some(raw.clone()),
        _ => None,
    }
}

fn operator_is(node: Option<&Syntax>, expected: &str) -> bool {
    match node {
        Some(Syntax::Operator { raw, .. } | Syntax::Token { raw, .. }) => raw == expected,
        _ => false,
    }
}

fn operator_raw(node: &Syntax) -> Option<&str> {
    match node {
        Syntax::Operator { raw, .. } | Syntax::Token { raw, .. } => Some(raw.as_str()),
        _ => None,
    }
}

fn group_children<'a>(node: Option<&'a Syntax>, delimiter: Delimiter) -> Option<&'a [Syntax]> {
    match node {
        Some(Syntax::Group {
            delimiter: found,
            children,
            close: Some(_),
            ..
        }) if *found == delimiter => Some(children),
        _ => None,
    }
}

fn group_interior<'a>(source: &'a str, node: &Syntax, delimiter: Delimiter) -> Option<&'a str> {
    match node {
        Syntax::Group {
            delimiter: found,
            span,
            close: Some(_),
            ..
        } if *found == delimiter => {
            let start = span.start as usize + 1;
            let end = span.end as usize - 1;
            source.get(start..end)
        }
        _ => None,
    }
}

fn node_start(node: &Syntax) -> usize {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => span.start as usize,
    }
}

fn node_end(node: &Syntax) -> usize {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => span.end as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_signal_substitutes_join_and_metavar() {
        let printed = expand_state_signal("count", "1");
        assert!(printed.contains("const countSignal = createSignal(1);"));
        assert!(!printed.contains("#syntax"));
        assert!(!printed.contains("#join"));
        assert!(!printed.contains("#metavar"));
        let use_ = apply_signal_use("count", "countSignal", Some("+="), Some("2"));
        assert_eq!(use_, "countSignal.set(countSignal.get() + 2)");
        let or = apply_signal_use("flag", "flagSignal", Some("||="), Some("true"));
        assert_eq!(or, "flagSignal.get() || flagSignal.set(true)");
    }

    #[test]
    fn method_call_is_one_function() {
        let printed = expand_method_call(
            &[("value", "number")],
            &["3"],
            &[
                MethodSegment {
                    params: &[("minimum", "number")],
                    args: &["1"],
                },
                MethodSegment {
                    params: &[("maximum", "number")],
                    args: &["5"],
                },
            ],
            "boolean",
            "return value > minimum && value < maximum;",
        );
        assert_eq!(
            printed,
            "((value: number, minimum: number, maximum: number,): boolean => value > minimum && value < maximum)(3, 1, 5)"
        );
        assert!(!printed.contains("#syntax"));
    }

    #[test]
    fn channel_do_thread_and_jsx() {
        assert_eq!(expand_channel_send("2", "table"), "yield put(table, 2)");
        assert_eq!(
            expand_channel_receive("received", "table"),
            "const received = yield take(table)"
        );
        let done = expand_do_steps(
            "box",
            &[
                DoStep {
                    name: "left",
                    source: "box.of(2)",
                },
                DoStep {
                    name: "right",
                    source: "box.of(3)",
                },
            ],
            "left + right",
        );
        assert_eq!(
            done,
            "box.flatMap(box.of(2), (left) => box.flatMap(box.of(3), (right) => box.of(left + right)),)"
        );
        let renamed = expand_do_steps_hygiene(
            "box",
            &[
                DoStep {
                    name: "left",
                    source: "box.of(4)",
                },
                DoStep {
                    name: "source",
                    source: "box.of(6)",
                },
            ],
            "left + source",
            &[],
        );
        assert_eq!(
            renamed,
            "box.flatMap(box.of(4), (left) => box.flatMap(box.of(6), (source) => box.of(left + source)),)"
        );
        assert_eq!(
            expand_thread_first(
                "[1, 2, 3]",
                &[
                    ThreadStep {
                        callee: "map",
                        args: &["(value) => value + 1"],
                    },
                    ThreadStep {
                        callee: "filter",
                        args: &["(value) => value > 2"],
                    },
                ],
            ),
            "filter(map([1, 2, 3], (value) => value + 1), (value) => value > 2)"
        );
        assert_eq!(
            expand_jsx_when("loading", "<li>loading</li>", Some("<li>ready</li>")),
            "{(loading) ? <li>loading</li> : <li>ready</li>}"
        );
        assert_eq!(
            expand_jsx_each("items", "item", Some("index"), "<li />"),
            "{(items).map((item, index) => <li />)}"
        );
    }

    /// A library form with no `for syntax` import is not rewritten. The same
    /// form expands once the file imports its spelling, including from inside
    /// a nested block. `function` is a core form and still rewrites.
    #[test]
    fn handwritten_forms_follow_the_syntax_import() {
        let bare = "const a = match (1) { 1 => 2, _ => 3 };\nconst v = #[1, 2];\nconst d = data Box<T> = Full(value: T) | Empty();\n";
        let expanded = crate::expand(crate::ExpandRequest {
            filename: "bare.sts",
            code: bare,
            phase: crate::Phase::Build,
        });
        assert!(
            expanded.diagnostics.is_empty(),
            "{:?}",
            expanded.diagnostics
        );
        assert!(
            expanded.code.contains("match (1)"),
            "match was rewritten:\n{}",
            expanded.code
        );
        assert!(
            expanded.code.contains("#[1, 2]"),
            "vector was rewritten:\n{}",
            expanded.code
        );
        assert!(!expanded.code.contains("vector("), "{}", expanded.code);
        assert!(
            expanded.code.contains("data Box"),
            "data was rewritten:\n{}",
            expanded.code
        );

        let dir = std::env::temp_dir().join("sw-gate-form-import");
        let _ = std::fs::create_dir_all(&dir);
        let macros = "export syntax match:expr {\n  rule { match ($value:expr) { $($arms)* } } => { matched($value) }\n}\nexport syntax (#):expr {\n  rule { #[$($element:expr),*] } => { vector($($element),*) }\n}\nexport syntax data:item {\n  rule { data $name:binding; } => { type $name = 1; }\n}\n";
        std::fs::write(dir.join("macros.sts"), macros).expect("macro fixture");
        let imported_path = dir.join("imported.sts");
        let imported = "import { match, (#), data } from \"./macros.sts\" for syntax;\nconst a = match (1) { 1 => 2, _ => 3 };\nconst v = #[1, 2];\nconst d = data Box<T> = Full(value: T) | Empty();\n";
        let imported_name = imported_path.display().to_string();
        let expanded = crate::expand(crate::ExpandRequest {
            filename: &imported_name,
            code: imported,
            phase: crate::Phase::Build,
        });
        assert!(
            expanded.diagnostics.is_empty(),
            "{:?}",
            expanded.diagnostics
        );
        assert!(
            !expanded.code.contains("match (1)"),
            "imported match stayed:\n{}",
            expanded.code
        );
        assert!(
            expanded.code.contains("vector(1, 2)"),
            "imported vector stayed:\n{}",
            expanded.code
        );
        assert!(
            !expanded.code.contains("data Box"),
            "imported data stayed:\n{}",
            expanded.code
        );

        let nested_path = dir.join("nested.sts");
        let nested = "function body() {\n  import { match } from \"./macros.sts\" for syntax;\n  const a = match (1) { 1 => 2, _ => 3 };\n}\nconst v = #[1, 2];\n";
        let nested_name = nested_path.display().to_string();
        let expanded = crate::expand(crate::ExpandRequest {
            filename: &nested_name,
            code: nested,
            phase: crate::Phase::Build,
        });
        assert!(
            expanded.code.contains("match (1)"),
            "nested import authorized match:\n{}",
            expanded.code
        );
        assert!(
            expanded.code.contains("import { match }"),
            "nested import was dropped: {}",
            expanded.code
        );

        assert!(
            expanded.code.contains("#[1, 2]"),
            "unimported vector was rewritten:\n{}",
            expanded.code
        );

        // A renamed import authorizes its local name, not the export name.
        let renamed_path = dir.join("renamed.sts");
        let renamed = "import { match as mymatch } from \"./macros.sts\" for syntax;\nconst a = mymatch (1) { 1 => 2, _ => 3 };\nconst b = match (2) { 1 => 2, _ => 3 };\n";
        let renamed_name = renamed_path.display().to_string();
        let expanded = crate::expand(crate::ExpandRequest {
            filename: &renamed_name,
            code: renamed,
            phase: crate::Phase::Build,
        });
        assert!(
            !expanded.code.contains("mymatch (1)"),
            "renamed match stayed: {}",
            expanded.code
        );
        assert!(
            expanded.code.contains("match (2)"),
            "export name expanded without its binding: {}",
            expanded.code
        );

        let function =
            "const f = function(value: number) { const doubled = value * 2; doubled + 1 };\n";
        let expanded = crate::expand(crate::ExpandRequest {
            filename: "core.sts",
            code: function,
            phase: crate::Phase::Build,
        });
        // Upstream's implicit-return `function` is a library macro. Without
        // a `for syntax` import the function is core TypeScript, as written.
        assert!(
            !expanded.code.contains("return doubled + 1"),
            "unimported function was rewritten:\n{}",
            expanded.code
        );
    }
}
