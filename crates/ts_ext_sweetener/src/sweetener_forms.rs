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
) -> Option<FixtureRewrite> {
    // One form expansion is one hygiene scope.
    super::sweetener_hygiene::in_scope(|| {
        try_expand_fixture_scoped(source, file, nodes, index, signals)
    })
}

fn try_expand_fixture_scoped(
    source: &str,
    file: &[Syntax],
    nodes: &[Syntax],
    index: usize,
    signals: &[(String, String)],
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
    if let Some(control) = match_jsx_block(source, nodes, index) {
        return Some(control);
    }
    if let Some(use_) = match_signal_use(source, nodes, index, signals) {
        return Some(use_);
    }
    if let Some(read) = match_signal_read(nodes, index, signals) {
        return Some(read);
    }
    if let Some(data) = match_data_item(source, nodes, index) {
        return Some(data);
    }
    if let Some(matched) = match_sweetener_match(source, nodes, index) {
        return Some(matched);
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
    ("data", "data"),
    ("when", "when"),
    ("each", "each"),
];

/// True when the node at `index` is a core form, or a handwritten library
/// form whose spelling this file imported `for syntax`.
///
/// Only top-level imports count, as upstream. A word head is matched by its
/// local binding and authorized by the export it names, so a renamed import
/// (`match as mymatch`) authorizes `mymatch` and not `match`.
/// A local `export syntax` is not an import and does not authorize the form.
/// Callers that already proved a use (`form_signals`) pass
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
        text: expand_data_decl_hygiene(&name, &type_param, &variants),
        binds: Vec::new(),
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
            let printed = user_binder(field);
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
        expand_ctor_match(&subject, &arms, &unmatched_name(nodes, index))
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

fn expand_ctor_match(subject: &str, arms: &[CtorArm], unmatched: &str) -> String {
    let mut body = String::new();
    for arm in arms {
        body.push_str(&format!("if (this.value.tag === \"{}\") {{ ", arm.name));
        if !arm.binders.is_empty() {
            let renamed: Vec<String> = arm
                .binders
                .iter()
                .map(|name| {
                    let printed = user_binder(name);
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
                .map(|name| (name.as_str(), user_binder(name)))
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

/// A channel binder the form introduces. Its printed name is chosen on the
/// output (`sweetener_hygiene::assign`). The receive form that prints the
/// declaration takes the same placeholder.
pub fn fresh_binder(name: &str) -> String {
    let printed = super::sweetener_hygiene::placeholder(name);
    super::sweetener_hygiene::remember(name, &printed);
    printed
}

/// A binder the user spelled. It is the user's name and is never renamed.
fn user_binder(name: &str) -> String {
    name.to_owned()
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
    })
}

pub struct ChannelStmt {
    pub op: String,
    pub left: String,
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
    })
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
    fn channel_do_thread_and_jsx() {
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
        // Upstream warns that it could not read these (SWR4025) and passes
        // them through; nothing is an error.
        assert!(
            expanded
                .diagnostics
                .iter()
                .all(|item| item.severity == crate::Severity::Warning),
            "{:?}",
            expanded.diagnostics
        );
        assert_eq!(expanded.diagnostics.len(), 3, "{:?}", expanded.diagnostics);
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
