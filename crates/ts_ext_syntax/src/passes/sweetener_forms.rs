//! Reference forms the expander still prints as source.
//!
//! The expander calls [`try_expand_fixture`] from one arm. The rewrites live
//! here so that arm does not grow a second macro table.
//!
//! Parentheses and whitespace are not part of the form. Callers compare
//! shape, not the old `(useState<number>(0))` wrapping.

#![allow(dead_code)]

use super::sweetener_reader::{self, Delimiter, Syntax};

/// `signals`: `syntax state` still emits `#syntax`, `#join`, and `#metavar`.
///
/// Rule `state $name:binding = $initial:expr`:
/// bind `#join($name, suffix: "Signal")` in the following items, then emit
/// `const <name>Signal = createSignal($initial)` and a nested `syntax $name`
/// whose rules substitute that joined name and `#metavar("value", $name)`.
pub fn expand_state_signal(name: &str, initial: &str) -> String {
    let signal = join_ident(name, "", "Signal", "preserve");
    format!("const {signal} = createSignal({initial});")
}

/// One fixture form at `index`, if this sibling list is one of the cases this
/// file owns. `None` means the expander's own rules should run.
pub fn try_expand_fixture(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    signals: &[(String, String)],
    methods: &[MethodBinding],
) -> Option<FixtureRewrite> {
    // `createSignal` is the signals fixture. A React module imports
    // `useState` for the same `state name = initial` spelling.
    if source.contains("createSignal") {
        if let Some((name, initial)) = match_state_item(source, &nodes[index..]) {
            let end = item_end(nodes, index);
            return Some(FixtureRewrite {
                start: node_start(&nodes[index]),
                end,
                next: end_index(nodes, index, end),
                text: expand_state_signal(&name, &initial),
                binds: vec![(name.clone(), join_ident(&name, "", "Signal", "preserve"))],
                method: None,
            });
        }
    }
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
            requests: item
                .requests
                .iter()
                .map(|request| (request.pattern.clone(), request.expansion.clone()))
                .collect(),
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
        let end = node_end(nodes.get(step.next.saturating_sub(1)).unwrap_or(&nodes[index]));
        let text = if step.op == "->" {
            expand_channel_send(&step.left, &step.right)
        } else {
            expand_channel_receive(&step.left, &step.right)
        };
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
        let end = node_end(nodes.get(item.next.saturating_sub(1)).unwrap_or(&nodes[index]));
        return Some(FixtureRewrite {
            start: node_start(&nodes[index]),
            end,
            next: item.next + usize::from(operator_is(nodes.get(item.next), ";")),
            text: expand_do_steps(item.monad, &steps, &item.result),
            binds: Vec::new(),
            method: None,
        });
    }
    if let Some(item) = match_thread(source, nodes, index) {
        let text = expand_thread_nodes(source, item.value, item.body, !item.last);
        let end = node_end(nodes.get(item.next.saturating_sub(1)).unwrap_or(&nodes[index]));
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
    if let Some(dbg) = match_dbg_call(source, nodes, index) {
        return Some(dbg);
    }
    if let Some(asserted) = match_assert_stmt(source, nodes, index) {
        return Some(asserted);
    }
    if let Some(mut effect) = match_effect_item(source, nodes, index) {
        effect.text = substitute_signal_reads(&effect.text, signals);
        return Some(effect);
    }
    if let Some(vector) = match_hash_vector(source, nodes, index) {
        return Some(vector);
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

/// Prefix `(#)`: `#[a, b]` is `vector(a, b)`. The spelling is an operator,
/// so the identifier-head expander never sees it.
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
        variants.push((variant, parse_ctor_fields(source, params)));
        cursor = skip_trivia(nodes, cursor + 2);
    }
    if variants.is_empty() || !operator_is(nodes.get(cursor), ";") {
        return None;
    }
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[cursor]),
        next: cursor + 1,
        text: expand_data_decl(&name, &type_param, &variants),
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

fn expand_data_decl(name: &str, type_param: &str, variants: &[(String, Vec<(String, String)>)]) -> String {
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
        let params = fields
            .iter()
            .map(|(field, ty)| format!("{field}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        let names = fields
            .iter()
            .map(|(field, _)| field.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let object = if fields.is_empty() {
            format!("{{ tag: \"{variant}\" }}")
        } else {
            format!("{{ tag: \"{variant}\", {names} }}")
        };
        out.push_str(&format!(
            " const {variant} = <{type_param},>({params}): {name}<{type_param}> => {object};"
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
    let subject = group_interior(source, nodes.get(index + 1)?, Delimiter::Paren)?.trim().to_owned();
    if subject.is_empty() {
        return None;
    }
    let body = nodes.get(index + 2)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let text = if let Some(arms) = parse_ctor_arms(source, children) {
        expand_ctor_match(&subject, &arms, &unmatched_name(nodes, index))
    } else {
        let (arms, fallback) = parse_structural_arms(source, children)?;
        let matched = match_binder_name(source, node_start(&nodes[index]));
        expand_structural_match(&subject, &arms, fallback.as_deref(), &matched)
    };
    let next = index + 3;
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

fn expand_ctor_match(subject: &str, arms: &[CtorArm], unmatched: &str) -> String {
    let mut body = String::new();
    for arm in arms {
        body.push_str(&format!("if (this.value.tag === \"{}\") {{ ", arm.name));
        if !arm.binders.is_empty() {
            body.push_str(&format!(
                "const {{ {} }} = this.value; ",
                arm.binders.join(", ")
            ));
        }
        body.push_str(&format!("return {}; }} ", arm.body));
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

fn parse_structural_arms(source: &str, children: &[Syntax]) -> Option<(Vec<StructArm>, Option<String>)> {
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
            Syntax::Token { raw, .. } if is_word(raw) && raw.chars().next().is_some_and(|ch| ch.is_lowercase()) => {
                Some(MatchPat::Bind(raw.clone()))
            }
            Syntax::Group { delimiter: Delimiter::Brace, children, .. } => {
                Some(MatchPat::Object(parse_match_fields(source, children)?))
            }
            Syntax::Group { delimiter: Delimiter::Bracket, children, .. } => {
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

fn parse_match_fields(source: &str, children: &[Syntax]) -> Option<Vec<(String, Option<MatchPat>)>> {
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
        let empty = if matches!(arm.pattern, MatchPat::Lit(_)) || literal_object(&arm.pattern) {
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
    !fields.is_empty() && fields.iter().all(|(_, pattern)| matches!(pattern, Some(MatchPat::Lit(_))))
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
    Some(source.get(node_start(first)..node_end(last))?.trim().to_owned())
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

/// The nested `syntax $name:expr` produced by `state`, with every directive
/// already substituted. Assignment operators that read-then-write keep the
/// operator between `.get()` and the value; `&&=` / `||=` / `??=` short-circuit
/// into `.set`. A bare name is `.get()`.
pub fn signal_syntax(name: &str, signal: &str) -> String {
    let value = "value";
    let mut rules = Vec::new();
    rules.push(format!(
        "rule {{ {name} = {value}:expr }} => {{ {signal}.set({value}) }}"
    ));
    for op in ["+", "-", "*", "/", "%", "**", "&", "|", "^", "<<", ">>", ">>>"] {
        let assign = format!("{op}=");
        rules.push(format!(
            "rule {{ {name} {assign} {value}:expr }} => {{ {signal}.set({signal}.get() {op} {value}) }}"
        ));
    }
    for op in ["&&", "||", "??"] {
        let assign = format!("{op}=");
        rules.push(format!(
            "rule {{ {name} {assign} {value}:expr }} => {{ {signal}.get() {op} {signal}.set({value}) }}"
        ));
    }
    rules.push(format!("rule {{ {name} }} => {{ {signal}.get() }}"));
    format!("syntax {name}:expr {{\n{}\n}}", rules.join("\n"))
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

/// `#metavar("value", $name)` is the expression hole of the rule being
/// applied, not text to print. The call-site capture is that hole.
pub fn metavar_capture<'a>(hole: &str, captures: &'a [(&str, &'a str)]) -> Option<&'a str> {
    captures
        .iter()
        .find_map(|(name, value)| (*name == hole).then_some(*value))
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
/// when the block is only that return.
pub fn expand_method_call(
    head_params: &[(&str, &str)],
    head_args: &[&str],
    segments: &[MethodSegment<'_>],
    result: &str,
    body: &str,
) -> String {
    let mut params = String::new();
    let mut args = String::new();
    for (index, (name, ty)) in head_params.iter().enumerate() {
        if index > 0 {
            params.push_str(", ");
            args.push_str(", ");
        }
        params.push_str(name);
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
            params.push_str(name);
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
    let body = arrow_body(body);
    format!("(({params}): {result} => {body})({args})")
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
pub fn expand_do_steps(monad: &str, steps: &[DoStep<'_>], result: &str) -> String {
    let mut out = format!("{monad}.of({result})");
    for step in steps.iter().rev() {
        out = format!(
            "{monad}.flatMap({}, ({}) => {out})",
            step.source, step.name
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
pub fn expand_thread_first(value: &str, steps: &[ThreadStep<'_>]) -> String {
    thread_calls(value, steps, true)
}

pub fn expand_thread_last(value: &str, steps: &[ThreadStep<'_>]) -> String {
    thread_calls(value, steps, false)
}

/// `map((value) => value + 1)` as one threading step.
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

/// Read a `state name = initial` item out of a token tree. Returns the
/// binding and the initializer text. This is the pattern the `state` rule
/// matches before [`expand_state_signal`].
pub fn match_state_item(source: &str, nodes: &[Syntax]) -> Option<(String, String)> {
    let mut index = 0;
    if token_is(nodes.get(index), "export") {
        index += 1;
    }
    if !token_is(nodes.get(index), "state") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    if !operator_is(nodes.get(index + 2), "=") {
        return None;
    }
    let start = node_start(nodes.get(index + 3)?);
    let mut end = start;
    let mut cursor = index + 3;
    while cursor < nodes.len() && !operator_is(nodes.get(cursor), ";") {
        end = node_end(&nodes[cursor]);
        cursor += 1;
    }
    let initial = source.get(start..end)?.trim().to_owned();
    Some((name, initial))
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
/// `syntax query` the first `method` rule would define. `#syntax` is not
/// printed; the rules are.
fn method_requests(source: &str, body: &Syntax) -> Vec<MethodRequest> {
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
        let Some(pattern) = group_interior(source, children.get(index + 1).unwrap_or(&children[index]), Delimiter::Brace) else {
            index += 1;
            continue;
        };
        index += 2;
        let mut expectation = "";
        if token_is(children.get(index), "expect") {
            expectation = children
                .get(index + 1)
                .map(|node| sweetener_reader::slice(source, node).trim())
                .unwrap_or("");
            index += 2;
            if operator_is(children.get(index), ";") {
                index += 1;
            }
        }
        if !operator_is(children.get(index), "=>") {
            continue;
        }
        let Some(expansion) = group_interior(source, children.get(index + 1).unwrap_or(&children[index]), Delimiter::Brace) else {
            index += 1;
            continue;
        };
        requests.push(MethodRequest {
            pattern: pattern.trim().to_owned(),
            expectation: expectation.trim_matches('"').to_owned(),
            expansion: expansion.trim().to_owned(),
        });
        index += 2;
    }
    requests
}

struct MethodRequest {
    pattern: String,
    expectation: String,
    expansion: String,
}

pub fn expand_method_grammar(head: &str, requests: &[MethodRequest]) -> String {
    let mut rules = Vec::new();
    for request in requests {
        let expect = if request.expectation.is_empty() {
            String::new()
        } else {
            format!("\nexpect \"{}\";", request.expectation)
        };
        rules.push(format!(
            "rule {{ {} }}{expect}\n=> {{ {} }}",
            request.pattern.trim(),
            request.expansion.trim()
        ));
    }
    format!("syntax {head}:expr {{\n{}\n}}", rules.join("\n"))
}

/// Pieces of a multi-part method item. Parameter groups are still token trees;
/// [`expand_method_call`] wants the printed pairs.
pub struct MethodItem<'a> {
    pub head: String,
    pub head_params: &'a [Syntax],
    pub segments: Vec<(String, &'a [Syntax])>,
    pub result: &'a str,
    pub body: &'a str,
    requests: Vec<MethodRequest>,
}

/// `2 -> table` or `received <- table` at statement position.
pub fn match_channel_stmt(source: &str, nodes: &[Syntax], index: usize) -> Option<ChannelStmt> {
    let left = nodes.get(index)?;
    let op = nodes.get(index + 1)?;
    let right = nodes.get(index + 2)?;
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

/// Split a `doSteps(monad) { steps; return result }` expression.
pub fn match_do_steps<'a>(source: &'a str, nodes: &[Syntax], index: usize) -> Option<DoStepsItem<'a>> {
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
pub fn match_thread<'a>(source: &'a str, nodes: &'a [Syntax], index: usize) -> Option<ThreadItem<'a>> {
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
            if raw == "<-" || raw == "->>" || (raw == "->" && nodes.get(cursor).and_then(operator_raw) != Some(">")) {
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

/// `{when (cond)} child {else} other {end}` or `{each (items as item, index)}`.
pub fn match_jsx_control<'a>(source: &'a str, nodes: &[Syntax], index: usize) -> Option<JsxControl<'a>> {
    let children = group_children(nodes.get(index), Delimiter::Brace)?;
    let head = token_raw(children.first())?;
    if head != "when" && head != "each" {
        return None;
    }
    let args = group_interior(source, children.get(1)?, Delimiter::Paren)?;
    Some(JsxControl {
        kind: head,
        args: args.trim(),
        at: index,
    })
}

pub struct JsxControl<'a> {
    pub kind: String,
    pub args: &'a str,
    pub at: usize,
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
    let method = methods.iter().find(|method| method.head == name && !method.requests.is_empty())?;
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
        let expected = token_raw(pattern.get(index)).or_else(|| pattern.get(index).and_then(|node| operator_raw(node).map(str::to_owned)))?;
        if expected.chars().all(|ch| ch.is_whitespace()) {
            index += 1;
            continue;
        }
        let found = token_raw(nodes.get(cursor)).or_else(|| nodes.get(cursor).and_then(operator_raw).map(str::to_owned))?;
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
                    word.chars().next().is_some_and(|ch| ch.is_ascii_uppercase())
                        || matches!(word.as_str(), "const" | "let" | "function" | "export" | "return" | "class")
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
    let text = expand_method_call(
        leak_pairs(&method.head_params),
        head_args,
        &segments,
        &method.result,
        &method.body,
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
    if nodes.get(index + 1).and_then(operator_raw).is_some_and(is_signal_assign) {
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
        let result = statements.pop()?;
        return Some((statements, result));
    }
    Some((statements, tail))
}

/// `dbg(expr)` prints the source and returns the value. The binder is
/// `inspected`. When the user file already binds that spelling, the copy
/// inside the template string is the same suffixed name as the parameter.
/// The generic rename stops at `JSON.stringify(inspected)` and leaves the
/// unsuffixed spelling there.
fn match_dbg_call(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "dbg") {
        return None;
    }
    let args = group_interior(source, nodes.get(index + 1)?, Delimiter::Paren)?.trim();
    if args.is_empty() {
        return None;
    }
    let name = if file_binds(source, "inspected") {
        "inspected_1"
    } else {
        "inspected"
    };
    let quoted = string_source(args);
    let text = format!(
        "(({name}) => {{ globalThis.console.error(`${{{quoted}}} = ${{globalThis.JSON.stringify({name})}}`,); return {name}; }})({args})"
    );
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end: node_end(&nodes[index + 1]),
        next: index + 2,
        text,
        binds: Vec::new(),
        method: None,
    })
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
                        let introduces = found == op || (name == "actual" && (found == "===" || found == ">="));
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
    source.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_').any(|word| {
        word == name
    }) && source.contains(&format!("const {name}"))
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

fn call_args_trail(source: &str, node: &Syntax) -> bool {
    let Some(children) = group_children(Some(node), Delimiter::Paren) else {
        return false;
    };
    let Some(last) = children.last() else {
        return source
            .get(node_start(node)..node_end(node))
            .is_some_and(|text| text.trim_end().ends_with(",)") || text.trim_end().ends_with(", )"));
    };
    if operator_is(Some(last), ",") {
        return true;
    }
    source
        .get(node_end(last)..node_end(node))
        .is_some_and(|text| text.contains(','))
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

fn take_value<'a>(source: &'a str, nodes: &[Syntax], index: usize) -> Option<(&'a str, usize, usize)> {
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
        if matches!(nodes.get(cursor), Some(Syntax::Group { delimiter: Delimiter::Brace, close: Some(_), .. })) {
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
        steps.push((name.trim().to_owned(), source.trim().to_owned()));
    }
    None
}

fn arrow_body(body: &str) -> String {
    // The method template copies the statement block, including `return`.
    format!("{{ {} }}", body.trim())
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
            "((value: number, minimum: number, maximum: number): boolean => { return value > minimum && value < maximum; })(3, 1, 5)"
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
            "box.flatMap(box.of(2), (left) => box.flatMap(box.of(3), (right) => box.of(left + right)))"
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
}
