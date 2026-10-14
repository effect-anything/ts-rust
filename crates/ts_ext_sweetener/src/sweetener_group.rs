//! Grouping parentheses for spliced expressions.
//!
//! Upstream Sweetener prints a captured expression, and an expression
//! macro's result, as a protected node: the printer wraps it in
//! parentheses unless it is atomic, stands alone in a list slot, or binds
//! at least as tightly as the operator around it
//! (`packages/printer/src/printed-file.ts`, `markBoundOperands`; the
//! precedences are `packages/enforestation/src/core-operators.ts`). This
//! expander splices text, so the same decision is made from the text on
//! either side of the splice.
//!
//! One deliberate difference: upstream treats a parameter-list arrow as
//! atomic and prints `p && () => 1`, which does not parse. An arrow is
//! grouped here like any other low-precedence expression.

/// Precedence of an operand that holds no operator: nothing re-associates it.
const ATOM: u32 = 1000;
const UNARY: u32 = 160;
const CONDITIONAL: u32 = 30;
const ARROW: u32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// A word: identifier, keyword, or number.
    Word(String),
    /// A string, template, or a whole bracket group.
    Atom,
    Punct(&'static str),
    Open(char),
    Close(char),
}

const PUNCTS: [&str; 52] = [
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "**", "+=", "-=", "*=", "/=", "%=", "&=", "|=",
    "^=", "<<", ">>", "=", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "!", "~", "?", ":",
    ",", ";", ".", "@",
];

/// Raw tokens of `text`, brackets included as `Open` / `Close`.
fn lex(text: &str) -> Vec<Tok> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        if ch.is_whitespace() {
            at += ch.len_utf8();
        } else if text[at..].starts_with("//") {
            at = text[at..].find('\n').map_or(bytes.len(), |end| at + end);
        } else if text[at..].starts_with("/*") {
            at = text[at + 2..].find("*/").map_or(bytes.len(), |end| at + 2 + end + 2);
        } else if matches!(ch, '"' | '\'' | '`') {
            at = string_end(text, at);
            out.push(Tok::Atom);
        } else if matches!(ch, '(' | '[' | '{') {
            out.push(Tok::Open(ch));
            at += 1;
        } else if matches!(ch, ')' | ']' | '}') {
            out.push(Tok::Close(ch));
            at += 1;
        } else if ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '#' {
            let start = at;
            at += ch.len_utf8();
            while let Some(next) = text[at..].chars().next() {
                if next.is_alphanumeric() || next == '_' || next == '$' {
                    at += next.len_utf8();
                } else if next == '.' && text[start..at].chars().all(|c| c.is_ascii_digit()) {
                    at += 1;
                } else {
                    break;
                }
            }
            out.push(Tok::Word(text[start..at].to_owned()));
        } else if let Some(punct) = PUNCTS.iter().find(|punct| text[at..].starts_with(**punct)) {
            out.push(Tok::Punct(punct));
            at += punct.len();
        } else {
            out.push(Tok::Atom);
            at += ch.len_utf8();
        }
    }
    out
}

/// The byte after the string or template that opens at `start`.
fn string_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let quote = bytes[start];
    let mut at = start + 1;
    let mut holes = 0usize;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 1,
            b'$' if quote == b'`' && bytes.get(at + 1) == Some(&b'{') => {
                holes += 1;
                at += 1;
            }
            b'}' if holes > 0 => holes -= 1,
            byte if byte == quote && holes == 0 => return at + 1,
            _ => {}
        }
        at += 1;
    }
    bytes.len()
}

/// The tokens of `text` at its own top level, each bracket group one atom.
fn top_level(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for tok in lex(text) {
        match tok {
            Tok::Open(_) => {
                if depth == 0 {
                    out.push(Tok::Atom);
                }
                depth += 1;
            }
            Tok::Close(_) => depth = depth.saturating_sub(1),
            other if depth == 0 => out.push(other),
            _ => {}
        }
    }
    out
}

fn is_prefix_word(word: &str) -> bool {
    matches!(word, "typeof" | "void" | "delete" | "await" | "new" | "yield")
}

fn infix_word(word: &str) -> Option<u32> {
    matches!(word, "in" | "instanceof" | "as" | "satisfies").then_some(110)
}

/// Precedence and associativity (true: right) of an infix spelling.
fn infix(punct: &str) -> Option<(u32, bool)> {
    Some(match punct {
        "**" => (150, true),
        "*" | "/" | "%" => (140, false),
        "+" | "-" => (130, false),
        "<<" | ">>" | ">>>" => (120, false),
        "<" | "<=" | ">" | ">=" => (110, false),
        "==" | "!=" | "===" | "!==" => (100, false),
        "&" => (90, false),
        "^" => (80, false),
        "|" => (70, false),
        "&&" => (60, false),
        "||" => (50, false),
        "??" => (40, false),
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "<<=" | ">>=" | ">>>=" | "&=" | "^="
        | "|=" | "&&=" | "||=" | "??=" => (20, true),
        "," => (10, false),
        _ => return None,
    })
}

/// Whether a token ends an operand, so a `+` / `-` / `<` after it is infix.
fn ends_operand(tok: Option<&Tok>) -> bool {
    match tok {
        Some(Tok::Word(word)) => !is_prefix_word(word) && infix_word(word).is_none() && !is_statement_word(word),
        Some(Tok::Atom | Tok::Close(_)) => true,
        Some(Tok::Punct(punct)) => matches!(*punct, "++" | "--"),
        _ => false,
    }
}

fn is_statement_word(word: &str) -> bool {
    matches!(
        word,
        "return" | "throw" | "case" | "else" | "do" | "export" | "default" | "const" | "let" | "var"
    )
}

/// What an expression's text says about its grouping.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    /// No operator of its own: a name, call, member chain, literal, group.
    atomic: bool,
    /// The loosest operator at its top level.
    precedence: u32,
    /// It starts with a prefix operator (a unary base of `**`).
    unary_start: bool,
}

fn shape(text: &str) -> Shape {
    let toks = top_level(text);
    let mut precedence = ATOM;
    let mut atomic = true;
    let mut unary_start = false;
    for (index, tok) in toks.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|at| toks.get(at));
        let operand_before = index > 0 && ends_operand(previous);
        let found = match tok {
            Tok::Word(word) if is_prefix_word(word) && !operand_before => {
                Some(if word == "yield" { ARROW } else { UNARY })
            }
            Tok::Word(word) if operand_before => infix_word(word),
            Tok::Punct("=>") => Some(ARROW),
            Tok::Punct("?") => Some(CONDITIONAL),
            Tok::Punct("++" | "--") if operand_before => Some(170),
            Tok::Punct("+" | "-" | "~" | "++" | "--") if !operand_before => Some(UNARY),
            // `!` never binds against a neighbour upstream (`nonBinding`).
            Tok::Punct("!") => None,
            Tok::Punct(punct) => infix(punct).map(|(prec, _)| prec),
            _ => None,
        };
        if let Some(found) = found {
            atomic = false;
            precedence = precedence.min(found);
            if index == 0 && found == UNARY {
                unary_start = true;
            }
        }
    }
    Shape {
        atomic,
        precedence,
        unary_start,
    }
}

/// What stands on one side of a splice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// The edge of a template: whatever surrounds the whole decides.
    Neutral,
    /// `(` / `[` / `)` / `]`: a list slot's edge.
    ListEdge,
    /// `,` or `;`.
    Separator,
    /// An operator the expression is an operand of: the least precedence
    /// it must have to hold together without parentheses, and its spelling.
    Operand { minimum: u32, spelling: &'static str },
    /// A member access, call, index or tagged template: only an atomic
    /// expression holds together before it.
    Member,
    /// An arrow body that runs to the end of its slot.
    ArrowBody,
    /// No expression parent: a declaration, `return`, a property value.
    Nothing,
}

fn left_side(before: &str, neutral: bool) -> Side {
    let toks = lex(before);
    let Some(last) = toks.last() else {
        return if neutral { Side::Neutral } else { Side::Nothing };
    };
    let previous = toks.len().checked_sub(2).and_then(|at| toks.get(at));
    match last {
        Tok::Open('(' | '[') => Side::ListEdge,
        Tok::Punct("," | ";") => Side::Separator,
        Tok::Punct("=>") => Side::ArrowBody,
        Tok::Punct("?") => Side::Operand { minimum: 11, spelling: "?" },
        Tok::Punct(":") if open_conditional(&toks) => Side::Operand { minimum: ARROW, spelling: ":" },
        Tok::Punct("+" | "-" | "~" | "!" | "++" | "--") if !ends_operand(previous) => Side::Operand {
            minimum: UNARY + 1,
            spelling: "prefix",
        },
        Tok::Word(word) if is_prefix_word(word) => Side::Operand {
            minimum: if word == "yield" { ARROW + 1 } else { UNARY + 1 },
            spelling: "prefix",
        },
        Tok::Word(word) if infix_word(word).is_some() => Side::Operand { minimum: 111, spelling: "in" },
        Tok::Punct(punct) => match infix(punct) {
            Some((20, _)) if declares(&toks) => Side::Nothing,
            Some((prec, right)) => Side::Operand {
                minimum: if right { prec } else { prec + 1 },
                spelling: punct,
            },
            None => Side::Nothing,
        },
        _ => Side::Nothing,
    }
}

fn right_side(after: &str, neutral: bool, conditional: bool) -> Side {
    let toks = lex(after);
    let Some(first) = toks.first() else {
        return if neutral { Side::Neutral } else { Side::Nothing };
    };
    match first {
        Tok::Close(')' | ']') => Side::ListEdge,
        Tok::Punct("," | ";") => Side::Separator,
        Tok::Punct("." | "?." | "!") | Tok::Open('(' | '[') | Tok::Atom => Side::Member,
        Tok::Punct("?") => Side::Operand { minimum: 40, spelling: "?" },
        Tok::Punct(":") if conditional => Side::Operand { minimum: 11, spelling: ":" },
        Tok::Punct("++" | "--") => Side::Operand { minimum: 171, spelling: "postfix" },
        Tok::Word(word) if infix_word(word).is_some() => Side::Operand { minimum: 110, spelling: "in" },
        Tok::Punct(punct) => match infix(punct) {
            Some((prec, right)) => Side::Operand {
                minimum: if right { prec + 1 } else { prec },
                spelling: punct,
            },
            None => Side::Nothing,
        },
        _ => Side::Nothing,
    }
}

/// Whether the statement `toks` ends in has a `?` still waiting for its `:`.
fn open_conditional(toks: &[Tok]) -> bool {
    let mut depth = 0i32;
    let mut open = 0i32;
    for tok in toks[..toks.len() - 1].iter().rev() {
        match tok {
            Tok::Close(_) => depth += 1,
            Tok::Open(_) if depth == 0 => break,
            Tok::Open(_) => depth -= 1,
            Tok::Punct(";" | ",") if depth == 0 => break,
            Tok::Punct("?") if depth == 0 => open += 1,
            Tok::Punct(":") if depth == 0 => open -= 1,
            _ => {}
        }
    }
    open > 0
}

/// Whether the `=` that ends `toks` initializes a declaration or a
/// parameter default rather than assigning.
fn declares(toks: &[Tok]) -> bool {
    let mut depth = 0i32;
    for tok in toks[..toks.len() - 1].iter().rev() {
        match tok {
            Tok::Close(_) => depth += 1,
            Tok::Open('(') if depth == 0 => return true,
            Tok::Open(_) if depth == 0 => return false,
            Tok::Open(_) => depth -= 1,
            Tok::Punct(";") if depth == 0 => return false,
            Tok::Word(word) if depth == 0 && matches!(word.as_str(), "const" | "let" | "var" | "using") => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Whether `expr`, spliced between `before` and `after`, needs grouping
/// parentheses to keep its meaning. `neutral` says the edges of `before`
/// and `after` are a template's edges, not the file's.
pub(crate) fn needs_group(before: &str, expr: &str, after: &str, neutral: bool) -> bool {
    let expr = expr.trim();
    if expr.is_empty() || (expr.starts_with('(') && closes_at_end(expr)) {
        return false;
    }
    let shape = shape(expr);
    if shape.atomic {
        return false;
    }
    let left = left_side(before, neutral);
    let conditional = matches!(left, Side::Operand { spelling: "?", .. });
    let right = right_side(after, neutral, conditional);
    let edge = |side: Side| matches!(side, Side::ListEdge | Side::Separator | Side::Neutral);
    if right == Side::Member {
        return true;
    }
    if edge(left) && edge(right) {
        return shape.precedence <= 10 && !(left == Side::Neutral && right == Side::Neutral);
    }
    if left == Side::ArrowBody && (edge(right) || right == Side::Nothing) {
        return false;
    }
    let bound = |side: Side, as_left_operand: bool| match side {
        Side::Operand { minimum, spelling } => {
            let logical = matches!(shape.precedence, 50 | 60);
            if spelling == "??" && logical {
                return false;
            }
            if as_left_operand && spelling == "**" && shape.unary_start {
                return false;
            }
            shape.precedence >= minimum
        }
        _ => false,
    };
    // The tighter of the two operators owns the operand.
    let parent_is_left = match (left, right) {
        (Side::Operand { minimum: l, .. }, Side::Operand { minimum: r, .. }) => l >= r,
        (Side::Operand { .. }, _) => true,
        _ => false,
    };
    let owner = if parent_is_left { left } else { right };
    !bound(owner, !parent_is_left)
}

/// The end of `text` the left side needs: its last statement or so.
pub(crate) fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(2048);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// The start of `text` the right side needs: its first token.
pub(crate) fn head(text: &str) -> &str {
    let mut end = text.len().min(256);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Whether the `(` that opens `text` closes at its very end.
fn closes_at_end(text: &str) -> bool {
    let mut depth = 0usize;
    let toks = lex(text);
    for (index, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index == toks.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Wrap each span of `text` that [`needs_group`] says so. `spans` are
/// byte ranges, possibly overlapping or out of order; the wrapped text and
/// the spans moved with it come back.
pub(crate) fn group_spans(text: &str, spans: &[(usize, usize)], eligible: impl Fn(&str) -> bool) -> (String, Vec<(usize, usize)>) {
    let mut wrap: Vec<(usize, usize)> = spans
        .iter()
        .copied()
        .filter(|&(from, to)| from < to && to <= text.len() && text.is_char_boundary(from) && text.is_char_boundary(to))
        .filter(|&(from, to)| eligible(text[from..to].trim()))
        .filter(|&(from, to)| needs_group(&text[..from], &text[from..to], &text[to..], true))
        .collect();
    wrap.sort_unstable();
    wrap.dedup();
    // Nested spans: the outer one decides; an inner one is in its own slot.
    let mut kept: Vec<(usize, usize)> = Vec::new();
    for span in wrap {
        if kept.last().is_some_and(|last| span.0 < last.1) {
            continue;
        }
        kept.push(span);
    }
    if kept.is_empty() {
        return (text.to_owned(), spans.to_vec());
    }
    let shift = |at: usize, closing: bool| {
        let mut moved = at;
        for &(from, to) in &kept {
            if from < at || (from == at && closing) {
                moved += 1;
            }
            if to < at || (to == at && closing) {
                moved += 1;
            }
        }
        moved
    };
    let mut out = String::with_capacity(text.len() + kept.len() * 2);
    let mut cursor = 0;
    for &(from, to) in &kept {
        out.push_str(&text[cursor..from]);
        out.push('(');
        out.push_str(&text[from..to]);
        out.push(')');
        cursor = to;
    }
    out.push_str(&text[cursor..]);
    let moved = spans
        .iter()
        .map(|&(from, to)| {
            if kept.contains(&(from, to)) {
                (shift(from, false) + 1, shift(to, false) + 1)
            } else {
                (shift(from, false), shift(to, true))
            }
        })
        .collect();
    (out, moved)
}

#[cfg(test)]
mod tests {
    use super::needs_group;

    fn group(before: &str, expr: &str, after: &str) -> bool {
        needs_group(before, expr, after, false)
    }

    #[test]
    fn matches_upstream_printing() {
        assert!(group("const a = ", "p && q", ";"));
        assert!(!group("x = ", "p && q", ";"));
        assert!(!group("f(", "p && q", ")"));
        assert!(!group("[", "p && q", ", 1]"));
        assert!(group("function g() { return ", "p && q", "; }"));
        assert!(!group("const i = ", "p && q", " ? 1 : 2;"));
        assert!(group("!", "p && q", ";"));
        assert!(group("const a = ", "x + 1", " * 2;"));
        assert!(!group("const c = ", "-(x * y)", " * 2;"));
        assert!(group("const j = ", "-(a + b)", " ** 2;"));
        assert!(group("{ k: ", "a + b", " }"));
        assert!(!group("const e = ", "g(a + b)", ";"));
        assert!(needs_group("", "a + b", ".length", true));
        assert!(needs_group("p && ", "q || r", "", true));
        assert!(!needs_group("p && ", "typeof q", "", true));
        assert!(!needs_group("p && ", "x as T", "", true));
        assert!(needs_group("p && ", "q?.r ?? s", "", true));
        assert!(!needs_group("", "a * b", " && c", true));
        assert!(needs_group("", "a || b", " && c", true));
        assert!(!needs_group("", "p && q", "", true));
        assert!(needs_group("-", "-x", "", true));
        assert!(needs_group("p && ", "() => 1", "", true));
        assert!(!needs_group("", "(v) => v", "", true));
    }
}
