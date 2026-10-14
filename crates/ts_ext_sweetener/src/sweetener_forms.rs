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
    if let Some(use_) = match_signal_use(source, nodes, index, signals) {
        return Some(use_);
    }
    if let Some(read) = match_signal_read(nodes, index, signals) {
        return Some(read);
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
const HANDWRITTEN_FORMS: &[(&str, &str)] = &[];

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
        .iter()
        .find(|(name, _)| name == local)
        .map(|(_, export)| export.clone())
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
    if group_children(nodes.get(index), Delimiter::Brace).is_some() {
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
pub(crate) fn imported_syntax_bindings(nodes: &[Syntax]) -> std::rc::Rc<Vec<(String, String)>> {
    let active = FILE_IMPORTS.with(|active| {
        active
            .borrow()
            .as_ref()
            .filter(|(range, _)| *range == nodes.as_ptr_range())
            .map(|(_, imports)| imports.clone())
    });
    active.unwrap_or_else(|| {
        let mut names = Vec::new();
        collect_imported_syntax_names(nodes, &mut names);
        std::rc::Rc::new(names)
    })
}

thread_local! {
    /// The top-level `for syntax` bindings of the file being printed. Every
    /// form head asks for them; scanning the file per head was quadratic.
    static FILE_IMPORTS: std::cell::RefCell<
        Option<(std::ops::Range<*const Syntax>, std::rc::Rc<Vec<(String, String)>>)>,
    > = const { std::cell::RefCell::new(None) };
}

/// Run `f` with `file`'s imports computed once. Only this exact slice uses
/// them, and the previous value is restored after, so a nested or later
/// expansion never sees another file's bindings.
pub fn with_file_imports<T>(file: &[Syntax], f: impl FnOnce() -> T) -> T {
    let mut names = Vec::new();
    collect_imported_syntax_names(file, &mut names);
    let saved = FILE_IMPORTS.with(|active| {
        active
            .borrow_mut()
            .replace((file.as_ptr_range(), std::rc::Rc::new(names)))
    });
    type Saved = Option<(
        std::ops::Range<*const Syntax>,
        std::rc::Rc<Vec<(String, String)>>,
    )>;
    struct Restore(Saved);
    impl Drop for Restore {
        fn drop(&mut self) {
            let saved = self.0.take();
            FILE_IMPORTS.with(|active| *active.borrow_mut() = saved);
        }
    }
    let _restore = Restore(saved);
    f()
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

fn is_word(raw: &str) -> bool {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
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
    }

    /// A library form with no `for syntax` import is not rewritten. The same
    /// form expands through its imported rules once the file imports its
    /// spelling; a nested-block import does not authorize it.
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
        let macros = "export syntax match:expr {\n  rule { match ($value:expr) { $($arms:tt)* } } => { matched($value) }\n}\nexport syntax (#):expr {\n  rule { #[$($element:expr),*] } => { vector($($element),*) }\n}\nexport syntax data:item {\n  rule { data $name:binding<$t:binding> = $($rest:tt)*; } => { type $name = 1; }\n}\n";
        std::fs::write(dir.join("macros.sts"), macros).expect("macro fixture");
        let imported_path = dir.join("imported.sts");
        let imported = "import { match, (#), data } from \"./macros.sts\" for syntax;\nconst a = match (1) { 1 => 2, _ => 3 };\nconst v = #[1, 2];\ndata Box<T> = Full(value: T) | Empty();\n";
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

#[cfg(test)]
mod file_import_tests {
    use super::{imported_syntax_bindings, with_file_imports};
    use crate::sweetener_reader::read_syntax;

    /// The once-per-file bindings answer only for that exact slice, and the
    /// previous scope comes back after, so another file is never confused.
    #[test]
    fn file_imports_are_scoped_to_their_slice() {
        let a = read_syntax("import { each } from \"./a.sts\" for syntax;\nconst x = 1;\n");
        let b = read_syntax("import { when as w } from \"./b.sts\" for syntax;\n");
        let pairs =
            |nodes| -> Vec<(String, String)> { imported_syntax_bindings(nodes).as_ref().clone() };
        let fresh_a = pairs(&a.children);
        assert_eq!(fresh_a, vec![("each".to_owned(), "each".to_owned())]);
        with_file_imports(&a.children, || {
            assert_eq!(pairs(&a.children), fresh_a);
            assert_eq!(
                pairs(&b.children),
                vec![("w".to_owned(), "when".to_owned())]
            );
            assert!(pairs(&a.children[1..]).is_empty());
            with_file_imports(&b.children, || {
                assert_eq!(
                    pairs(&b.children),
                    vec![("w".to_owned(), "when".to_owned())]
                );
            });
            assert_eq!(pairs(&a.children), fresh_a);
        });
    }
}
