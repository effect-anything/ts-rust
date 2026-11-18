//! App macros the expander still prints as source.
//!
//! [`expand_apps`] is the one entry the form pass calls. Each form is a
//! whole item or expression, so a miss returns `None` and the expander's
//! own rules run. Parentheses and whitespace are not part of the form.

use super::sweetener_forms::FixtureRewrite;
use super::sweetener_reader::{Delimiter, Syntax};

/// One library or sample form at `index`, if this sibling list is one of
/// the cases this file owns.
#[must_use]
pub fn expand_apps(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if let Some(item) = match_effect_item(source, nodes, index) {
        return Some(item);
    }
    let _ = (source, nodes, index);
    None
}

fn match_effect_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "effect") {
        return None;
    }
    // `effect [deps] { ... }` is the hook form, not this declaration.
    if !token_is(nodes.get(index + 1), "effect") && operator_is(nodes.get(index + 1), "[") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let params = nodes.get(index + 2)?;
    if group_children(Some(params), Delimiter::Paren).is_none() {
        return None;
    }
    let body = nodes.get(index + 3)?;
    if group_children(Some(body), Delimiter::Brace).is_none() {
        return None;
    }
    let param_text = group_interior(source, params, Delimiter::Paren)?
        .trim()
        .to_owned();
    let body_text = group_interior(source, body, Delimiter::Brace)?
        .trim()
        .to_owned();
    // `gen` is itself a macro. Nested expansion reprints this replacement
    // as source and does not run the form pass on it, so finish the body here.
    let generated = if body_text.contains("<-") {
        expand_gen(&body_text)
    } else {
        format!("gen {{ {body_text} }}")
    };
    let text = format!("export const {name} = (({param_text}) => {generated});");
    Some(rewrite(nodes, index, node_end(body), text))
}

fn expand_gen(body: &str) -> String {
    let mut steps = String::new();
    for line in body.lines() {
        let trimmed = line.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((name, value)) = trimmed.split_once("<-") {
            steps.push_str(&format!(
                "const {} = (yield* {});\n",
                name.trim(),
                value.trim()
            ));
        } else if let Some(value) = trimmed.strip_prefix("do ") {
            steps.push_str(&format!("yield* {};\n", value.trim()));
        } else {
            steps.push_str(trimmed);
            if !trimmed.ends_with('}') {
                steps.push(';');
            }
            steps.push('\n');
        }
    }
    format!("Effect.gen(function* () {{ {steps} }})")
}

fn rewrite(nodes: &[Syntax], index: usize, end: usize, text: String) -> FixtureRewrite {
    FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next: end_index(nodes, index, end),
        text,
        binds: Vec::new(),
    }
}

fn end_index(nodes: &[Syntax], index: usize, end: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() && node_start(&nodes[cursor]) < end {
        cursor += 1;
    }
    cursor
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
        } if *found == delimiter => source.get(span.start as usize + 1..span.end as usize - 1),
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
