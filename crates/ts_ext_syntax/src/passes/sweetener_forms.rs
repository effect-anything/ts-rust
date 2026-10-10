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
    if let Some(item) = match_method_item(source, &nodes[index..]) {
        let end = method_item_end(nodes, index);
        let head_params = param_pairs(source, item.head_params);
        let mut segments = Vec::new();
        for (_name, params) in &item.segments {
            segments.push(param_pairs(source, params));
        }
        return Some(FixtureRewrite {
            start: node_start(&nodes[index]),
            end,
            next: end_index(nodes, index, end),
            text: String::new(),
            binds: vec![(item.head.clone(), "method".to_owned())],
            method: Some(MethodBinding {
                head: item.head,
                head_params,
                segments,
                result: item.result.to_owned(),
                body: item.body.to_owned(),
            }),
        });
    }
    if let Some(call) = match_method_call(source, nodes, index, methods) {
        return Some(call);
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
    if let Some(mut effect) = match_effect_item(source, nodes, index) {
        effect.text = substitute_signal_reads(&effect.text, signals);
        return Some(effect);
    }
    None
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
    })
}

/// Pieces of a multi-part method item. Parameter groups are still token trees;
/// [`expand_method_call`] wants the printed pairs.
pub struct MethodItem<'a> {
    pub head: String,
    pub head_params: &'a [Syntax],
    pub segments: Vec<(String, &'a [Syntax])>,
    pub result: &'a str,
    pub body: &'a str,
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

fn match_method_call(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    methods: &[MethodBinding],
) -> Option<FixtureRewrite> {
    let name = token_raw(nodes.get(index))?;
    let method = methods.iter().find(|method| method.head == name)?;
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

fn match_effect_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
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
    Some(FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next,
        text: apply_signal_use(&name, &signal, Some(&op), Some(value.trim())),
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
    let mut cursor = index;
    while cursor < nodes.len() && cursor < index + 48 {
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
        // `(-> value, sum(),)` keeps the empty step's trailing comma.
        // A step that already has arguments does not.
        let trailing = args.is_empty() && call_args_trail(source, args_node);
        let mut call_args = Vec::new();
        if first {
            call_args.push(current);
            call_args.extend(args);
        } else {
            call_args.extend(args);
            call_args.push(current);
        }
        let mut joined = call_args.join(", ");
        if trailing {
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
