//! Lossless token trees for Sweetener source.
//!
//! Modeled on `vendor/sweetener` `packages/reader`: scan text, keep balanced
//! delimiter groups, and report an unmatched delimiter at its source span.
//! This is not a TypeScript parse. Macro structure is decided here so a bad
//! form is a Sweetener diagnostic, not a later TypeScript error.

use crate::{Diagnostic, Severity};

/// A byte range in the original file. Half-open, UTF-8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// One node in a lossless token tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// An identifier, keyword, number, or other non-delimiter atom.
    Token { span: Span, raw: String },
    /// `//` or `/* */`, including the markers.
    Comment { span: Span, raw: String },
    /// A quoted string, template, or character run, including delimiters.
    String { span: Span, raw: String },
    /// A single operator or punctuation character that is not a group delimiter.
    Operator { span: Span, raw: String },
    /// `(...)`, `{...}`, or `[...]`. Children do not include the delimiters.
    Group {
        delimiter: Delimiter,
        span: Span,
        open: Span,
        close: Option<Span>,
        children: Vec<Syntax>,
    },
}

/// Which balanced pair a [`Syntax::Group`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimiter {
    Paren,
    Brace,
    Bracket,
}

impl Delimiter {
    fn close(self) -> char {
        match self {
            Self::Paren => ')',
            Self::Brace => '}',
            Self::Bracket => ']',
        }
    }

    fn from_open(ch: char) -> Option<Self> {
        match ch {
            '(' => Some(Self::Paren),
            '{' => Some(Self::Brace),
            '[' => Some(Self::Bracket),
            _ => None,
        }
    }

    fn from_close(ch: char) -> Option<Self> {
        match ch {
            ')' => Some(Self::Paren),
            '}' => Some(Self::Brace),
            ']' => Some(Self::Bracket),
            _ => None,
        }
    }
}

/// A file read into a token tree, plus reader diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadSyntax {
    pub children: Vec<Syntax>,
    pub diagnostics: Vec<Diagnostic>,
}

thread_local! {
    /// Operator spellings the file's operator definitions add, longest first.
    static EXTRA_OPERATORS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Spellings the reader already reads as one operator.
const BUILTIN_SPELLINGS: &[&str] = &["|>", "==", "=>", "**", "->", "<-", "::"];

/// Whether the reader reads `spelling` as one token now.
#[must_use]
pub fn reads_as_one(spelling: &str) -> bool {
    BUILTIN_SPELLINGS.contains(&spelling)
        || EXTRA_OPERATORS.with(|extra| extra.borrow().iter().any(|have| have == spelling))
}

/// Run `f` with the reader also reading each of `spellings` as one operator
/// token, as upstream's reader does for the operators a file defines or
/// imports (`@@`, `~~`).
pub fn with_operators<T>(spellings: &[String], f: impl FnOnce() -> T) -> T {
    let saved = EXTRA_OPERATORS.with(|extra| {
        let mut extra = extra.borrow_mut();
        let saved = extra.clone();
        for spelling in spellings {
            if !extra.contains(spelling) {
                extra.push(spelling.clone());
            }
        }
        extra.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
        saved
    });
    let out = f();
    EXTRA_OPERATORS.with(|extra| *extra.borrow_mut() = saved);
    out
}

/// Read `source` into a token tree. Unmatched delimiters are diagnostics
/// whose message names this as a Sweetener reader error, not a TypeScript
/// parse error. Codes follow the reference reader (`SWR1002`, `SWR1003`).
#[must_use]
pub fn read_syntax(source: &str) -> ReadSyntax {
    let mut reader = Reader {
        source,
        bytes: source.as_bytes(),
        index: 0,
        diagnostics: Vec::new(),
    };
    let children = reader.read_until(None);
    ReadSyntax {
        children,
        diagnostics: reader.diagnostics,
    }
}

struct Reader<'a> {
    source: &'a str,
    bytes: &'a [u8],
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Reader<'a> {
    fn read_until(&mut self, stop: Option<Delimiter>) -> Vec<Syntax> {
        let mut children = Vec::new();
        while self.index < self.bytes.len() {
            self.skip_whitespace();
            if self.index >= self.bytes.len() {
                break;
            }
            let ch = self.bytes[self.index] as char;
            if let Some(delimiter) = Delimiter::from_close(ch) {
                if stop == Some(delimiter) {
                    break;
                }
                let start = self.index;
                self.index += 1;
                self.diagnostics.push(Diagnostic {
                    message: format!(
                        "SWR1002: Sweetener reader error, not a TypeScript parse error: unexpected closing delimiter '{}'",
                        delimiter.close()
                    ),
                    start: u32_at(start),
                    end: u32_at(self.index),
                    severity: Severity::Error,
                    related: Vec::new(),
                });
                children.push(Syntax::Operator {
                    span: span(start, self.index),
                    raw: delimiter.close().to_string(),
                });
                continue;
            }
            if let Some(delimiter) = Delimiter::from_open(ch) {
                let group = self.read_group(delimiter);
                children.push(group);
                continue;
            }
            children.push(self.read_atom());
        }
        children
    }

    fn read_group(&mut self, delimiter: Delimiter) -> Syntax {
        let open_at = self.index;
        self.index += 1;
        let mut children = self.read_until(Some(delimiter));
        let close = if self.index < self.bytes.len()
            && self.bytes[self.index] as char == delimiter.close()
        {
            let start = self.index;
            self.index += 1;
            Some(span(start, self.index))
        } else {
            self.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR1003: Sweetener reader error, not a TypeScript parse error: missing closing delimiter '{}'",
                    delimiter.close()
                ),
                start: u32_at(open_at),
                end: u32_at(open_at + 1),
                severity: Severity::Error,
                related: Vec::new(),
            });
            None
        };
        // A missing closer used to own every token through the end of the
        // file. Upstream closes the open frame and keeps reading, so a
        // statement on the next line is still its own item. The line break
        // in front of a later child is that boundary.
        let mut end = close.map_or(self.index, |close| close.end as usize);
        if close.is_none() {
            let mut prev_end = open_at + 1;
            let mut split_at = None;
            for (at, child) in children.iter().enumerate() {
                let start = node_span_start(child);
                if self
                    .source
                    .get(prev_end..start)
                    .is_some_and(|gap| gap.contains('\n'))
                {
                    split_at = Some((at, prev_end, start));
                    break;
                }
                prev_end = node_span_end(child);
            }
            if let Some((at, prev_end, start)) = split_at {
                children.truncate(at);
                // The break belongs to the next statement. Ending the group
                // on the following line made that statement look like it
                // continued this one.
                end = prev_end;
                let _ = start;
                self.index = self.source[prev_end..]
                    .find('\n')
                    .map_or(prev_end, |rel| prev_end + rel + 1);
            } else if let Some(last) = children.last() {
                end = node_span_end(last);
            } else {
                end = open_at + 1;
            }
        }
        Syntax::Group {
            delimiter,
            span: span(open_at, end),
            open: span(open_at, open_at + 1),
            close,
            children,
        }
    }

    fn read_atom(&mut self) -> Syntax {
        let start = self.index;
        let ch = self.bytes[self.index] as char;
        if ch == '/' && self.peek(1) == Some('/') {
            return self.read_line_comment(start);
        }
        if ch == '/' && self.peek(1) == Some('*') {
            return self.read_block_comment(start);
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            return self.read_string(start, ch);
        }
        if ch == '$' {
            // `$$inferSelect` is one literal `$` and the name. A hole is a
            // single `$`. Reading the pair as one token keeps the name out
            // of the hole.
            let doubled = self.peek(1) == Some('$');
            self.index += if doubled { 2 } else { 1 };
            return Syntax::Operator {
                span: span(start, self.index),
                raw: if doubled { "$$" } else { "$" }.to_owned(),
            };
        }
        if is_ident_start(ch) {
            self.index += 1;
            while self.index < self.bytes.len() && is_ident_continue(self.bytes[self.index] as char)
            {
                self.index += 1;
            }
            return Syntax::Token {
                span: span(start, self.index),
                raw: self.source[start..self.index].to_owned(),
            };
        }
        if ch.is_ascii_digit() {
            self.index += 1;
            while self.index < self.bytes.len() {
                let next = self.bytes[self.index] as char;
                if next.is_ascii_digit() || next == '.' || next == '_' {
                    self.index += 1;
                } else {
                    break;
                }
            }
            return Syntax::Token {
                span: span(start, self.index),
                raw: self.source[start..self.index].to_owned(),
            };
        }
        let extra = EXTRA_OPERATORS.with(|extra| {
            extra
                .borrow()
                .iter()
                .find(|spelling| self.source[start..].starts_with(spelling.as_str()))
                .map(String::len)
        });
        if let Some(width) = extra {
            self.index = start + width;
            return Syntax::Operator {
                span: span(start, self.index),
                raw: self.source[start..self.index].to_owned(),
            };
        }
        self.index += 1;
        // Multi-character operator spellings Sweetener macros import.
        // Comparisons stay single characters so `$left:expr` can include them.
        let spelling = match ch {
            '|' if self.peek_at(start, 1) == Some('>') => Some(2),
            '=' if self.peek_at(start, 1) == Some('=') => Some(2),
            '=' if self.peek_at(start, 1) == Some('>') => Some(2),
            '*' if self.peek_at(start, 1) == Some('*') => Some(2),
            '-' if self.peek_at(start, 1) == Some('>') => Some(2),
            '<' if self.peek_at(start, 1) == Some('-') => Some(2),
            ':' if self.peek_at(start, 1) == Some(':') => Some(2),
            _ => None,
        };
        if let Some(width) = spelling {
            self.index = start + width;
        }
        Syntax::Operator {
            span: span(start, self.index),
            raw: self.source[start..self.index].to_owned(),
        }
    }

    fn read_line_comment(&mut self, start: usize) -> Syntax {
        self.index += 2;
        while self.index < self.bytes.len() && self.bytes[self.index] != b'\n' {
            self.index += 1;
        }
        Syntax::Comment {
            span: span(start, self.index),
            raw: self.source[start..self.index].to_owned(),
        }
    }

    fn read_block_comment(&mut self, start: usize) -> Syntax {
        self.index += 2;
        while self.index + 1 < self.bytes.len() {
            if self.bytes[self.index] == b'*' && self.bytes[self.index + 1] == b'/' {
                self.index += 2;
                break;
            }
            self.index += 1;
        }
        Syntax::Comment {
            span: span(start, self.index),
            raw: self.source[start..self.index].to_owned(),
        }
    }

    /// SWR1001, the scanner's unterminated literal. Only where a literal
    /// can start: after an operator, an opener, or line start. A quote after
    /// `/` may sit in a regular expression this reader does not scan.
    fn unterminated(&mut self, start: usize, at: usize, what: &str) {
        let before = self.source[..start].trim_end();
        let operand_start = before.is_empty()
            || before.ends_with(|ch: char| "=(,:[{;!&|?+-*%<>".contains(ch))
            || before.ends_with("return");
        if !operand_start {
            return;
        }
        let at = at.min(self.bytes.len());
        self.diagnostics.push(Diagnostic {
            message: format!("SWR1001: Unterminated {what} literal."),
            start: u32_at(at),
            end: u32_at(at),
            severity: Severity::Error,
            related: Vec::new(),
        });
    }

    fn read_string(&mut self, start: usize, quote: char) -> Syntax {
        self.index += 1;
        // A template literal is one string. `${...}` may hold quotes, and a
        // macro template writes `` `${#text($name)}` ``, whose `#` is not an
        // interpolation the scanner would otherwise enter. Stopping at the
        // first inner backtick splits that into a fragment later re-quoted.
        let mut brace_depth = 0i32;
        while self.index < self.bytes.len() {
            let ch = self.bytes[self.index] as char;
            if ch == '\\' {
                self.index += 1;
                if self.index < self.bytes.len() {
                    self.index += 1;
                }
                continue;
            }
            if quote == '`' && ch == '$' && self.peek(1) == Some('{') {
                brace_depth += 1;
                self.index += 2;
                continue;
            }
            if quote == '`' && ch == '#' && brace_depth == 0 {
                // `#text($name)` inside a template is still inside it.
                let rest = &self.source[self.index..];
                if rest.starts_with("#text")
                    || rest.starts_with("#core")
                    || rest.starts_with("#join")
                {
                    self.index += 1;
                    continue;
                }
            }
            if quote == '`' && brace_depth > 0 {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                } else if ch == '"' || ch == '\'' || ch == '`' {
                    let _ = self.read_string(self.index, ch);
                    continue;
                }
                self.index += 1;
                continue;
            }
            if ch == '\n' && quote != '`' {
                self.unterminated(start, self.index, "string");
                self.index += 1;
                break;
            }
            self.index += 1;
            if ch == quote {
                break;
            }
            if self.index >= self.bytes.len() {
                let what = if quote == '`' { "template" } else { "string" };
                self.unterminated(start, self.index, what);
            }
        }
        Syntax::String {
            span: span(start, self.index),
            raw: self.source[start..self.index].to_owned(),
        }
    }

    fn skip_whitespace(&mut self) {
        while self.index < self.bytes.len() && (self.bytes[self.index] as char).is_whitespace() {
            self.index += 1;
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.peek_at(self.index, offset)
    }

    fn peek_at(&self, at: usize, offset: usize) -> Option<char> {
        self.bytes.get(at + offset).map(|byte| *byte as char)
    }
}

fn node_span_start(node: &Syntax) -> usize {
    node_span_of(node).start as usize
}

fn node_span_end(node: &Syntax) -> usize {
    node_span_of(node).end as usize
}

fn node_span_of(node: &Syntax) -> Span {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => *span,
    }
}

fn span(start: usize, end: usize) -> Span {
    Span {
        start: u32_at(start),
        end: u32_at(end),
    }
}

fn u32_at(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

fn is_ident_start(ch: char) -> bool {
    // `$` starts a metavariable, not an identifier. `$name` is `$` then `name`.
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || ch.is_ascii_digit()
}

/// A half-open source slice. Out-of-range bounds yield an empty string.
#[must_use]
pub fn slice_range(source: &str, start: usize, end: usize) -> &str {
    if start > end || end > source.len() {
        return "";
    }
    &source[start..end]
}

/// Text of a node, taken from the original source so interior spacing survives.
#[must_use]
pub fn slice<'a>(source: &'a str, node: &Syntax) -> &'a str {
    let (start, end) = match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => (span.start as usize, span.end as usize),
    };
    source.get(start..end).unwrap_or("")
}
