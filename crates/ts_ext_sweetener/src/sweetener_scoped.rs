//! `#let` and `#parameterize` on printed text.
//!
//! A `#parameterize(p = value) { body }` region is printed between two
//! markers. Macros inside the body expand first, so an inner region that
//! binds the same parameter has already replaced its own uses; what is left
//! in the outer region is the outer binding's. Resolution replaces each
//! parameter use, innermost region first. An operator spelled parameter
//! (`%`) is a use only where an operand may start, so `% % 5` keeps the
//! remainder.
//!
//! A `#let` whose body awaits or yields assigns a binder declared at the top
//! of the enclosing function; `hoist` declares those once the file is
//! printed.

use std::sync::atomic::{AtomicUsize, Ordering};

use ts_goport::astdata::SyntaxKind;
use ts_goport::core::Node;
use ts_goport::flags::ScriptKind;
use ts_goport::frontend::parser::{SourceFileParseOptions, parse_source_file};

use crate::Segment;

pub(crate) const OPEN: &str = "__swp";
const HOIST: &str = "/*__swlet ";
static NEXT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static REGIONS: std::cell::RefCell<std::collections::HashMap<usize, (String, String, bool)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The text around one `#parameterize` body: a call to a marker, so the
/// reader keeps the body together as one group and no capture can take a
/// marker without the rest of the region.
pub(crate) fn parameter_markers(param: &str, value: &str, required: bool) -> (String, String) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    REGIONS.with(|regions| {
        regions
            .borrow_mut()
            .insert(id, (param.to_owned(), value.to_owned(), required));
    });
    (format!("{OPEN}{id}("), ")".to_owned())
}

/// The header of region `id`, and a fresh id for the same header (an item
/// the expander replays prints its regions under new ids).
pub(crate) fn region(id: usize) -> Option<(String, String, bool)> {
    REGIONS.with(|regions| regions.borrow().get(&id).cloned())
}

pub(crate) fn renumber_region(header: (String, String, bool)) -> usize {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    REGIONS.with(|regions| regions.borrow_mut().insert(id, header));
    id
}

pub(crate) fn unused_len() -> usize {
    UNUSED.with(|list| list.borrow().len())
}

pub(crate) fn unused_since(from: usize) -> Vec<Unused> {
    UNUSED.with(|list| list.borrow().get(from..).unwrap_or_default().to_vec())
}

pub(crate) fn lets_empty() -> bool {
    LETS.with(|lets| lets.borrow().is_empty())
}

pub(crate) fn hoist_marker(binder: &str) -> String {
    format!("{HOIST}{binder}*/")
}

/// A parameter a `required` region never used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unused {
    pub param: String,
    /// The region's body as printed: where upstream reports SWR4022.
    pub body: String,
}

thread_local! {
    static UNUSED: std::cell::RefCell<Vec<Unused>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    static LETS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// While a `#let` body renders, its name stands for `binder`.
pub(crate) fn push_let(name: &str, binder: &str) {
    LETS.with(|lets| lets.borrow_mut().push((name.to_owned(), binder.to_owned())));
}

pub(crate) fn pop_let() {
    LETS.with(|lets| {
        lets.borrow_mut().pop();
    });
}

/// `text` with every enclosing `#let` name replaced by its binder.
pub(crate) fn let_binder_in(text: &str) -> String {
    LETS.with(|lets| {
        let mut out = text.to_owned();
        for (name, binder) in lets.borrow().iter().rev() {
            out = rename_outside(&out, &[], name, binder).0;
        }
        out
    })
}

pub(crate) fn clear_unused() {
    UNUSED.with(|list| list.borrow_mut().clear());
}

pub(crate) fn note_unused(unused: Vec<Unused>) {
    UNUSED.with(|list| list.borrow_mut().extend(unused));
}

pub(crate) fn take_unused() -> Vec<Unused> {
    UNUSED.with(|list| std::mem::take(&mut *list.borrow_mut()))
}

/// Where the call that opens at `open` (its `(`) closes.
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut close = None;
    scan_code(&text[open..], |at, ch| {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + at);
                    return true;
                }
            }
            _ => {}
        }
        false
    });
    close
}

/// The offset just past the string or character literal that opens at
/// `at`. An escape skips one character, not one byte, so a multi-byte
/// scalar after `\` is never sliced in half.
fn skip_quoted(text: &str, at: usize, quote: char) -> usize {
    let mut chars = text[at + quote.len_utf8()..].chars();
    let mut end = at + quote.len_utf8();
    while let Some(ch) = chars.next() {
        end += ch.len_utf8();
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                end += escaped.len_utf8();
            }
            continue;
        }
        if ch == quote || ch == '\n' {
            break;
        }
    }
    end.min(text.len())
}

/// Call `f` with each code character (not in a string, template text, or
/// comment) and its offset; stop when it returns true.
fn scan_code(text: &str, mut f: impl FnMut(usize, char) -> bool) {
    let mut stack: Vec<(State, usize)> = vec![(State::Code, 0)];
    let mut at = 0;
    while at < text.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        let (state, depth) = *stack.last().unwrap_or(&(State::Code, 0));
        if state == State::Template {
            if ch == '`' {
                stack.pop();
            } else if ch == '\\' {
                at += 1;
            } else if text[at..].starts_with("${") {
                stack.push((State::Code, 0));
                at += 2;
                continue;
            }
            at += text[at..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        if text[at..].starts_with("//") {
            at = text[at..].find('\n').map_or(text.len(), |e| at + e);
            continue;
        }
        if text[at..].starts_with("/*") {
            at = text[at + 2..]
                .find("*/")
                .map_or(text.len(), |e| at + 2 + e + 2);
            continue;
        }
        if ch == '"' || ch == '\'' {
            at = skip_quoted(text, at, ch);
            continue;
        }
        if ch == '`' {
            stack.push((State::Template, 0));
            at += 1;
            continue;
        }
        if ch == '{' {
            if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
        } else if ch == '}' {
            if depth == 0 && stack.len() > 1 {
                stack.pop();
                at += 1;
                continue;
            }
            if let Some(top) = stack.last_mut() {
                top.1 = top.1.saturating_sub(1);
            }
        }
        if f(at, ch) {
            return;
        }
        at += ch.len_utf8();
    }
}

/// Replace every parameter use inside resolved regions, innermost first.
pub(crate) fn resolve_parameters(code: &str) -> (String, Vec<Unused>) {
    let mut code = code.to_owned();
    let mut unused = Vec::new();
    loop {
        // The last marker call: no other region starts inside it.
        let mut chosen = None;
        let mut from = 0;
        while let Some(found) = code[from..].find(OPEN) {
            let at = from + found;
            from = at + OPEN.len();
            if code[..at].chars().next_back().is_some_and(ident_char) {
                continue;
            }
            let digits = code[from..].bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || code.as_bytes().get(from + digits) != Some(&b'(') {
                continue;
            }
            let Ok(id) = code[from..from + digits].parse::<usize>() else {
                continue;
            };
            let Some(header) = REGIONS.with(|regions| regions.borrow().get(&id).cloned()) else {
                continue;
            };
            chosen = Some((at, from + digits, header));
        }
        let Some((at, paren, (param, value, required))) = chosen else {
            break;
        };
        let Some(close) = matching_paren(&code, paren) else {
            break;
        };
        let (replaced, count) = substitute(&code[paren + 1..close], &param, &value);
        if required && count == 0 {
            unused.push(Unused {
                param,
                body: code[paren + 1..close].trim().to_owned(),
            });
        }
        code.replace_range(at..close + 1, &replaced);
    }
    (code, unused)
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Code,
    Template,
}

const OPERATOR_WORDS: &[&str] = &[
    "return",
    "yield",
    "await",
    "typeof",
    "void",
    "delete",
    "new",
    "in",
    "of",
    "case",
    "throw",
    "instanceof",
    "else",
    "do",
];

fn ident_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphanumeric()
}

/// Replace uses of `param` in code positions of `text` with `value`.
pub(crate) fn substitute(text: &str, param: &str, value: &str) -> (String, usize) {
    let (out, uses) = substitute_at(text, param, value);
    (out, uses.len())
}

/// Offsets in `text` where `param` is used as an operand.
pub(crate) fn parameter_uses(text: &str, param: &str) -> Vec<usize> {
    substitute_at(text, param, param).1
}

fn substitute_at(text: &str, param: &str, value: &str) -> (String, Vec<usize>) {
    let word = param.chars().all(ident_char);
    let mut out = String::with_capacity(text.len());
    let mut count = Vec::new();
    let mut stack: Vec<(State, usize)> = vec![(State::Code, 0)];
    // Whether an operand may start here.
    let mut operand_next = true;
    let mut last_dot = false;
    let mut at = 0;
    while at < text.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        let (state, depth) = *stack.last().unwrap_or(&(State::Code, 0));
        if state == State::Template {
            if ch == '`' {
                stack.pop();
                out.push(ch);
                at += 1;
                operand_next = false;
                continue;
            }
            if ch == '\\' {
                let end = (at + 2).min(text.len());
                out.push_str(&text[at..end]);
                at = end;
                continue;
            }
            if text[at..].starts_with("${") {
                stack.push((State::Code, 0));
                out.push_str("${");
                at += 2;
                operand_next = true;
                continue;
            }
            out.push(ch);
            at += ch.len_utf8();
            continue;
        }
        if text[at..].starts_with("//") {
            let end = text[at..].find('\n').map_or(text.len(), |e| at + e);
            out.push_str(&text[at..end]);
            at = end;
            continue;
        }
        if text[at..].starts_with("/*") {
            let end = text[at + 2..]
                .find("*/")
                .map_or(text.len(), |e| at + 2 + e + 2);
            out.push_str(&text[at..end]);
            at = end;
            continue;
        }
        if ch == '"' || ch == '\'' {
            let end = skip_quoted(text, at, ch);
            out.push_str(&text[at..end]);
            at = end;
            operand_next = false;
            continue;
        }
        if ch == '`' {
            stack.push((State::Template, 0));
            out.push(ch);
            at += 1;
            continue;
        }
        if ch.is_whitespace() {
            out.push(ch);
            at += ch.len_utf8();
            continue;
        }
        if ident_char(ch) {
            let end = text[at..]
                .char_indices()
                .find(|(_, c)| !ident_char(*c))
                .map_or(text.len(), |(o, _)| at + o);
            let token = &text[at..end];
            if word && token == param && !last_dot {
                out.push_str(value);
                count.push(at);
            } else {
                out.push_str(token);
            }
            operand_next = OPERATOR_WORDS.contains(&token);
            last_dot = false;
            at = end;
            continue;
        }
        if !word && text[at..].starts_with(param) && operand_next {
            let after = &text[at + param.len()..];
            if !after.starts_with('=') || after.starts_with("==") {
                out.push_str(value);
                count.push(at);
                at += param.len();
                operand_next = false;
                last_dot = false;
                continue;
            }
        }
        match ch {
            '{' => {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                operand_next = true;
            }
            '}' => {
                if depth == 0 && stack.len() > 1 {
                    stack.pop();
                    out.push(ch);
                    at += 1;
                    operand_next = false;
                    continue;
                }
                if let Some(top) = stack.last_mut() {
                    top.1 = top.1.saturating_sub(1);
                }
                operand_next = false;
            }
            ')' | ']' => operand_next = false,
            _ => operand_next = true,
        }
        last_dot = ch == '.';
        out.push(ch);
        at += ch.len_utf8();
    }
    (out, count)
}

/// Rename the template's own `name` to `binder` outside capture spans.
pub(crate) fn rename_outside(
    text: &str,
    spans: &[(usize, usize)],
    name: &str,
    binder: &str,
) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(text.len());
    let mut moved: Vec<(usize, isize)> = Vec::new();
    let mut at = 0;
    let mut delta: isize = 0;
    while at < text.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        if ident_char(ch) {
            let end = text[at..]
                .char_indices()
                .find(|(_, c)| !ident_char(*c))
                .map_or(text.len(), |(o, _)| at + o);
            let token = &text[at..end];
            let inside = spans.iter().any(|(from, to)| *from <= at && end <= *to);
            let member =
                text[..at].trim_end().ends_with('.') && !text[..at].trim_end().ends_with("..");
            if token == name && !inside && !member {
                out.push_str(binder);
                delta += binder.len() as isize - token.len() as isize;
                moved.push((end, delta));
            } else {
                out.push_str(token);
            }
            at = end;
            continue;
        }
        out.push(ch);
        at += ch.len_utf8();
    }
    let shift = |offset: usize| -> usize {
        let mut d = 0;
        for (end, total) in &moved {
            if *end <= offset {
                d = *total;
            }
        }
        (offset as isize + d).max(0) as usize
    };
    let spans = spans
        .iter()
        .map(|(from, to)| (shift(*from), shift(*to)))
        .collect();
    (out, spans)
}

/// Whether `text` awaits or yields outside a nested function.
pub(crate) fn suspends(text: &str) -> bool {
    let (_, awaits) = substitute(text, "await", "await");
    let (_, yields) = substitute(text, "yield", "yield");
    awaits + yields > 0
}

fn function_like(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::FunctionDeclaration
            | SyntaxKind::FunctionExpression
            | SyntaxKind::ArrowFunction
            | SyntaxKind::MethodDeclaration
            | SyntaxKind::GetAccessor
            | SyntaxKind::SetAccessor
            | SyntaxKind::Constructor
    )
}

/// The block body `{` of the innermost function around `at`, or the start of
/// the top-level statement holding it.
fn hoist_site(root: Node, at: usize, code: &str) -> usize {
    let mut site = None;
    let mut statement = 0;
    fn walk(node: Node, at: usize, code: &str, site: &mut Option<usize>) {
        node.for_each_child(|child| {
            if child == node {
                return false;
            }
            let (pos, end) = (child.pos().max(0) as usize, child.end().max(0) as usize);
            if pos <= at && at < end {
                if function_like(child.kind()) {
                    let body = child.body();
                    if !body.is_nil() && body.kind() == SyntaxKind::Block {
                        let from = body.pos().max(0) as usize;
                        if let Some(open) = code[from..].find('{') {
                            *site = Some(from + open + 1);
                        }
                    }
                }
                walk(child, at, code, site);
            }
            false
        });
    }
    for child in root.statements().iter() {
        let (pos, end) = (child.pos().max(0) as usize, child.end().max(0) as usize);
        if pos <= at && at < end {
            let leading = code[pos..end].len() - code[pos..end].trim_start().len();
            statement = pos + leading;
        }
    }
    walk(root, at, code, &mut site);
    site.unwrap_or(statement)
}

/// Declare every `#let` binder that a suspending body assigns, at the top of
/// its function, and drop the markers. Segments move with the text.
pub(crate) fn hoist(code: &str, segments: &mut [Segment], tsx: bool) -> String {
    if !code.contains(HOIST) {
        return code.to_owned();
    }
    let leaked: &'static str = Box::leak(code.to_owned().into_boxed_str());
    let opts = SourceFileParseOptions {
        file_name: if tsx { "/hoist.tsx" } else { "/hoist.ts" }.to_owned(),
        ..SourceFileParseOptions::default()
    };
    let kind = if tsx { ScriptKind::TSX } else { ScriptKind::TS };
    let file = parse_source_file(&opts, leaked, kind);
    let mut sites: Vec<(usize, Vec<String>)> = Vec::new();
    let mut removals: Vec<(usize, usize)> = Vec::new();
    let mut from = 0;
    while let Some(found) = code[from..].find(HOIST) {
        let at = from + found;
        let Some(close) = code[at..].find("*/").map(|c| at + c) else {
            break;
        };
        let binder = code[at + HOIST.len()..close].to_owned();
        let site = hoist_site(file.root, at, code);
        match sites.iter_mut().find(|(s, _)| *s == site) {
            Some((_, names)) => names.push(binder),
            None => sites.push((site, vec![binder])),
        }
        removals.push((at, close + 2));
        from = close + 2;
    }
    let mut edits: Vec<(usize, usize, String)> = removals
        .into_iter()
        .map(|(a, b)| (a, b, String::new()))
        .collect();
    for (site, names) in sites {
        let text = if code[..site].ends_with('{') {
            format!(" let {};", names.join(", "))
        } else {
            format!("let {};\n", names.join(", "))
        };
        edits.push((site, site, text));
    }
    apply_edits(code, edits, segments)
}

/// Apply non-overlapping `(start, end, replacement)` edits and move segment
/// offsets with them.
pub(crate) fn apply_edits(
    code: &str,
    mut edits: Vec<(usize, usize, String)>,
    segments: &mut [Segment],
) -> String {
    edits.sort_by_key(|(start, end, _)| (*start, *end));
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    let mut moved: Vec<(usize, usize, usize)> = Vec::new();
    for (start, end, text) in edits {
        if start < cursor {
            continue;
        }
        out.push_str(&code[cursor..start]);
        out.push_str(&text);
        moved.push((start, end, out.len()));
        cursor = end;
    }
    out.push_str(&code[cursor..]);
    let map = |offset: usize| -> usize {
        let mut result = offset as isize;
        for (start, end, new_end) in &moved {
            if *end <= offset && !(start == end && *start == offset) {
                result = offset as isize + (*new_end as isize - *end as isize);
            } else if *start < offset {
                result = *new_end as isize;
            } else {
                break;
            }
        }
        result.max(0) as usize
    };
    for segment in segments.iter_mut() {
        segment.virtual_start = map(segment.virtual_start as usize) as u32;
        segment.virtual_end = map(segment.virtual_end as usize) as u32;
        // A rewrite inside a verbatim range changed its length: the bytes
        // no longer match one for one, so the range maps as a whole.
        if segment.kind == crate::SegmentKind::Verbatim
            && segment.virtual_end - segment.virtual_start
                != segment.original_end - segment.original_start
        {
            segment.kind = crate::SegmentKind::Atom;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_parameter_is_an_operand_only() {
        assert_eq!(substitute("% % 5", "%", "t"), ("t % 5".to_owned(), 1));
        assert_eq!(
            substitute("[%, % * 10]", "%", "t"),
            ("[t, t * 10]".to_owned(), 2)
        );
        assert_eq!(substitute("`$ ${%}`", "%", "t"), ("`$ ${t}`".to_owned(), 1));
        assert_eq!(substitute("x %= 2", "%", "t").1, 0);
        assert_eq!(substitute("\"%\" + %", "%", "t").0, "\"%\" + t");
    }

    #[test]
    fn inner_regions_resolve_first() {
        let (open_b, close_b) = parameter_markers("%", "B", false);
        let (open_a, close_a) = parameter_markers("%", "A", true);
        let outer = format!("{open_a}f(%, {open_b}%.b{close_b}){close_a}");
        assert_eq!(resolve_parameters(&outer), ("f(A, B.b)".to_owned(), vec![]));
        let strings = format!("{open_a}\")\" + %{close_a}");
        assert_eq!(resolve_parameters(&strings).0, "\")\" + A");
        let (open, close) = parameter_markers("%", "A", true);
        let lonely = format!("{open}1{close}");
        assert_eq!(resolve_parameters(&lonely).1.len(), 1);
    }

    #[test]
    fn hoists_into_the_enclosing_function() {
        let code = format!(
            "function* g() {{\n  const r = (t = 1, (yield t){});\n}}\n",
            hoist_marker("t")
        );
        assert_eq!(
            hoist(&code, &mut [], false),
            "function* g() { let t;\n  const r = (t = 1, (yield t));\n}\n"
        );
    }
}
