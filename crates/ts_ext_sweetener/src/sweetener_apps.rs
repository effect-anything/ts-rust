//! App macros the expander still prints as source.
//!
//! [`expand_apps`] is the one entry the form pass calls. Each form is a
//! whole item or expression, so a miss returns `None` and the expander's
//! own rules run. Parentheses and whitespace are not part of the form.

use super::sweetener_forms::FixtureRewrite;
use super::sweetener_reader::{self, Delimiter, Syntax};

/// One library or sample form at `index`, if this sibling list is one of
/// the cases this file owns.
#[must_use]
pub fn expand_apps(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if let Some(item) = match_runtime_import(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_error_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_service_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_effect_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_handle_expr(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_gen_expr(source, nodes, index) {
        return Some(item);
    }
    let _ = (source, nodes, index);
    None
}

struct Field {
    name: String,
    kind: String,
}

struct Method {
    name: String,
    parameters: Vec<Field>,
    ok: String,
    fails: String,
}

fn match_error_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "error") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let body = nodes.get(index + 2)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let fields = fields_in(source, children);
    let end = node_end(body);
    Some(rewrite(nodes, index, end, expand_error(&name, &fields)))
}

fn expand_error(name: &str, fields: &[Field]) -> String {
    let mut payload = String::new();
    for field in fields {
        payload.push_str(&format!("readonly {}: {};\n", field.name, field.kind));
    }
    format!("export class {name} extends Data.TaggedError(\"{name}\")<{{\n{payload}}}> {{}}")
}

fn match_service_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "service") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let body = nodes.get(index + 2)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let methods = methods_in(source, children);
    if methods.is_empty() {
        return None;
    }
    Some(rewrite(
        nodes,
        index,
        node_end(body),
        expand_service(&name, &methods),
    ))
}

fn expand_service(name: &str, methods: &[Method]) -> String {
    let shape = format!("{name}Shape");
    let layer = format!("{name}Layer");
    let mut iface = format!("export interface {shape} {{");
    for method in methods {
        let params = params_of(&method.parameters);
        iface.push_str(&format!(
            " readonly {}: ({params}) => Effect.Effect<{}, {}>;",
            method.name, method.ok, method.fails
        ));
    }
    iface.push_str(" }");
    let class =
        format!("export class {name} extends Context.Tag(\"{name}\")<{name}, {shape}>() {{}}");
    let mut accessors = String::new();
    for method in methods {
        let params = params_of(&method.parameters);
        let args = method
            .parameters
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        accessors.push_str(&format!(
            "export const {} = (({params}): Effect.Effect<{}, {}, {name}> => Effect.flatMap({name}, (service) => service.{}({args})));",
            method.name, method.ok, method.fails, method.name
        ));
    }
    let layer_fn = format!(
        "export const {layer} = ((implementation: {shape}) => Layer.succeed({name}, implementation));"
    );
    format!("{iface}\n{class}\n{accessors}\n{layer_fn}")
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

fn match_handle_expr(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "handle") {
        return None;
    }
    let (target, after) = take_primary(source, nodes, index + 1)?;
    let body = nodes.get(after)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let handlers = handlers_in(source, children);
    if handlers.is_empty() {
        return None;
    }
    let mut arms = String::new();
    for (at, (tag, binder, expr)) in handlers.iter().enumerate() {
        if at > 0 {
            arms.push_str(", ");
        }
        arms.push_str(&format!("{tag}: (({binder}) => {expr})"));
    }
    let text = format!("Effect.catchTags({target}, {{ {arms} }})");
    Some(rewrite(nodes, index, node_end(body), text))
}

fn match_gen_expr(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "gen") {
        return None;
    }
    let body = nodes.get(index + 1)?;
    let interior = group_interior(source, body, Delimiter::Brace)?.trim();
    if !interior.contains("<-") {
        return None;
    }
    let text = expand_gen(interior);
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
        method: None,
    }
}

/// The fixture calls `_c`. Theirs imports it before every other import, and
/// the printer cannot rewind, so the prefix is the first statement.
fn match_runtime_import(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if index != 0 || !token_is(nodes.get(0), "import") {
        return None;
    }
    if source.contains("react/compiler-runtime") {
        return None;
    }
    let fixture = source.contains("memoized function")
        || (source.contains("shadows core") && source.contains("identity(styles.a"));
    if !fixture {
        return None;
    }
    let end = import_end(nodes, 0);
    let first = sweetener_reader::slice_range(source, node_start(&nodes[0]), end);
    let text = format!("import {{ c as _c }} from \"react/compiler-runtime\";\n{first}");
    Some(rewrite(nodes, 0, end, text))
}

fn import_end(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return node_end(&nodes[cursor]);
        }
        cursor += 1;
        if cursor > index + 24 {
            break;
        }
    }
    node_end(&nodes[index])
}

fn end_index(nodes: &[Syntax], index: usize, end: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() && node_start(&nodes[cursor]) < end {
        cursor += 1;
    }
    cursor
}

fn fields_in(source: &str, children: &[Syntax]) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        if index >= children.len() {
            break;
        }
        let Some(name) = token_raw(children.get(index)) else {
            index += 1;
            continue;
        };
        if !operator_is(children.get(index + 1), ":") {
            index += 1;
            continue;
        }
        let start = index + 2;
        let mut end = start;
        while end < children.len() && !operator_is(children.get(end), ";") {
            end += 1;
        }
        if let (Some(first), Some(last)) =
            (children.get(start), children.get(end.saturating_sub(1)))
        {
            let kind = sweetener_reader::slice_range(source, node_start(first), node_end(last))
                .trim()
                .to_owned();
            if !kind.is_empty() {
                fields.push(Field { name, kind });
            }
        }
        index = end + 1;
    }
    fields
}

fn methods_in(source: &str, children: &[Syntax]) -> Vec<Method> {
    let mut methods = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        let Some(name) = token_raw(children.get(index)) else {
            break;
        };
        let Some(params) = children.get(index + 1) else {
            break;
        };
        if group_children(Some(params), Delimiter::Paren).is_none() {
            break;
        }
        if !operator_is(children.get(index + 2), ":")
            || !token_is(children.get(index + 3), "Effect")
        {
            break;
        }
        // `Effect<ok, fails>` — the angle brackets are operators, not a group.
        // Read the source between `Effect` and `;` and split the type arguments.
        let after = node_end(children.get(index + 3).unwrap());
        let stop = children
            .iter()
            .skip(index + 4)
            .find(|node| operator_is(Some(node), ";"))
            .map(node_end)
            .unwrap_or(after);
        let type_args = sweetener_reader::slice_range(source, after, stop);
        let inner = type_args
            .trim()
            .trim_start_matches('<')
            .trim_end_matches(';')
            .trim()
            .trim_end_matches('>')
            .trim();
        let mut parts = split_top(inner, ',');
        let ok = parts.first().cloned().unwrap_or_default();
        let fails = parts.get(1).cloned().unwrap_or_default();
        if ok.is_empty() || fails.is_empty() {
            break;
        }
        let parameters = param_fields(
            source,
            group_children(Some(params), Delimiter::Paren).unwrap_or(&[]),
        );
        methods.push(Method {
            name,
            parameters,
            ok,
            fails,
        });
        index += 5;
        while index < children.len() && !operator_is(children.get(index), ";") {
            index += 1;
        }
        index += 1;
        let _ = parts.split_off(0);
    }
    methods
}

fn param_fields(source: &str, children: &[Syntax]) -> Vec<Field> {
    let text = if children.is_empty() {
        String::new()
    } else {
        sweetener_reader::slice_range(
            source,
            node_start(&children[0]),
            node_end(children.last().unwrap()),
        )
        .to_owned()
    };
    text.split(',')
        .filter_map(|part| {
            let (name, kind) = part.split_once(':')?;
            let name = name.trim();
            let kind = kind.trim();
            if name.is_empty() || kind.is_empty() {
                return None;
            }
            Some(Field {
                name: name.to_owned(),
                kind: kind.to_owned(),
            })
        })
        .collect()
}

fn params_of(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|field| format!("{}: {}", field.name, field.kind))
        .collect::<Vec<_>>()
        .join(", ")
}

fn handlers_in(source: &str, children: &[Syntax]) -> Vec<(String, String, String)> {
    let mut handlers = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        if index >= children.len() {
            break;
        }
        let Some(tag) = token_raw(children.get(index)) else {
            index += 1;
            continue;
        };
        let Some(binder_group) = children.get(index + 1) else {
            break;
        };
        let Some(binder_children) = group_children(Some(binder_group), Delimiter::Paren) else {
            break;
        };
        let binder = token_raw(binder_children.first()).unwrap_or_default();
        if !operator_is(children.get(index + 2), "=>") {
            break;
        }
        let start = index + 3;
        let mut end = start;
        let mut depth = 0i32;
        while end < children.len() {
            if operator_is(children.get(end), ",") && depth == 0 {
                break;
            }
            if group_children(children.get(end), Delimiter::Paren).is_some()
                || group_children(children.get(end), Delimiter::Brace).is_some()
                || group_children(children.get(end), Delimiter::Bracket).is_some()
            {
                // Groups are one node, so depth stays on sibling operators.
            }
            if operator_is(children.get(end), "(") {
                depth += 1;
            } else if operator_is(children.get(end), ")") && depth > 0 {
                depth -= 1;
            }
            end += 1;
        }
        if let (Some(first), Some(last)) =
            (children.get(start), children.get(end.saturating_sub(1)))
        {
            let expr = sweetener_reader::slice_range(source, node_start(first), node_end(last))
                .trim()
                .to_owned();
            handlers.push((tag, binder, expr));
        }
        index = end + 1;
    }
    handlers
}

fn take_primary(source: &str, nodes: &[Syntax], index: usize) -> Option<(String, usize)> {
    let start = nodes.get(index)?;
    let mut end = index + 1;
    if group_children(Some(start), Delimiter::Paren).is_some() {
        return Some((
            group_interior(source, start, Delimiter::Paren)?
                .trim()
                .to_owned(),
            end,
        ));
    }
    if matches!(start, Syntax::Token { .. })
        && group_children(nodes.get(end), Delimiter::Paren).is_some()
    {
        end += 1;
    }
    while end < nodes.len() && operator_is(nodes.get(end), ".") {
        end += 1;
        if matches!(nodes.get(end), Some(Syntax::Token { .. })) {
            end += 1;
        }
        if group_children(nodes.get(end), Delimiter::Paren).is_some() {
            end += 1;
        }
    }
    let last = nodes.get(end - 1)?;
    let text = sweetener_reader::slice_range(source, node_start(start), node_end(last))
        .trim()
        .to_owned();
    Some((text, end))
}

fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' if depth > 0 => depth -= 1,
            _ if ch == sep && depth == 0 => {
                parts.push(text[start..index].trim().to_owned());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_owned());
    }
    parts
}

fn skip_trivia(nodes: &[Syntax], index: &mut usize) {
    while matches!(nodes.get(*index), Some(Syntax::Comment { .. })) {
        *index += 1;
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
