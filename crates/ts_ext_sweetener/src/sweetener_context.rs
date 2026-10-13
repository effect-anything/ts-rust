//! The syntactic contexts a position stands in: whether `yield` and `await`
//! are expressions there. Upstream's expander tracks the same two facts while
//! it walks (`withYield` / `withAwait`); a `context generator;` or
//! `context async;` clause selects a rule only where they hold.
//!
//! A function's own header decides: a generator body admits `yield`, an async
//! body `await`. An arrow is never a generator. Parameter lists, class field
//! initializers and static blocks admit neither. The module top level admits
//! `await`; a namespace body admits neither.

use super::sweetener_reader::{Delimiter, Syntax};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Contexts {
    pub generator: bool,
    pub async_: bool,
}

impl Contexts {
    pub(crate) fn has(self, name: &str) -> bool {
        match name {
            "generator" => self.generator,
            "async" => self.async_,
            _ => false,
        }
    }
}

const TOP: Contexts = Contexts {
    generator: false,
    async_: true,
};
const NEITHER: Contexts = Contexts {
    generator: false,
    async_: false,
};

/// A half-open byte range and the contexts inside it. The smallest range
/// containing an offset is the innermost.
pub(crate) type Region = (usize, usize, Contexts);

/// The module top level: `await` is an expression, `yield` is not.
pub(crate) const MODULE: Contexts = TOP;

/// Index a token tree whose top level stands in `base`.
pub(crate) fn regions(nodes: &[Syntax], base: Contexts) -> Vec<Region> {
    let mut regions = Vec::new();
    walk(nodes, base, false, &mut regions);
    regions
}

/// The contexts at `offset`, given the regions of its tree.
pub(crate) fn lookup(regions: &[Region], offset: usize, base: Contexts) -> Contexts {
    regions
        .iter()
        .filter(|(start, end, _)| *start <= offset && offset < *end)
        .min_by_key(|(start, end, _)| end - start)
        .map_or(base, |(_, _, contexts)| *contexts)
}

fn raw(node: Option<&Syntax>) -> Option<&str> {
    match node? {
        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => Some(raw.as_str()),
        _ => None,
    }
}

fn start(node: &Syntax) -> usize {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => span.start as usize,
    }
}

fn end(node: &Syntax) -> usize {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => span.end as usize,
    }
}

fn is_paren(node: Option<&Syntax>) -> bool {
    matches!(
        node,
        Some(Syntax::Group {
            delimiter: Delimiter::Paren,
            ..
        })
    )
}

/// What a `{` group at `index` opens, read from the tokens before it.
enum Opens {
    /// A function body with its own contexts.
    Function(Contexts),
    /// A class body: members are functions, fields admit neither.
    Class,
    /// A namespace body.
    Namespace,
    /// A block or object literal: the enclosing contexts.
    Inherit,
}

/// The index of the parameter list a body at `brace` belongs to, skipping a
/// return type annotation (`(): Promise<T> {`).
fn params_before(nodes: &[Syntax], brace: usize) -> Option<usize> {
    let mut at = brace;
    // `<` / `>` nesting of a return type (`Generator<Effect, number>`).
    let mut angle = 0i32;
    while at > 0 {
        at -= 1;
        match &nodes[at] {
            Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            } if angle == 0 => return Some(at),
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => match raw.as_str() {
                ">" => angle += 1,
                "<" => angle -= 1,
                ";" | "=" | "{" | "}" => return None,
                "," | "=>" if angle == 0 => return None,
                _ => {}
            },
            _ => {}
        }
        if brace - at > 48 {
            return None;
        }
    }
    None
}

fn opens(nodes: &[Syntax], brace: usize, in_class: bool) -> Opens {
    let prev = raw(brace.checked_sub(1).and_then(|at| nodes.get(at)));
    if prev == Some("=>") {
        return Opens::Function(Contexts {
            generator: false,
            async_: arrow_is_async(nodes, brace - 1),
        });
    }
    // `class Name extends Base<T> implements I {`
    let mut at = brace;
    while at > 0 && brace - at < 16 {
        at -= 1;
        match raw(nodes.get(at)) {
            Some("class") => return Opens::Class,
            Some("namespace" | "module") if at + 2 == brace || at + 1 < brace => {
                if matches!(raw(nodes.get(at + 1)), Some(name) if name != ".")
                    && !is_paren(nodes.get(at + 1))
                    && raw(nodes.get(at.wrapping_sub(1))) != Some(".")
                {
                    let only_names = (at + 1..brace).all(|i| {
                        matches!(raw(nodes.get(i)), Some(word) if word == "." || word.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
                    });
                    if only_names {
                        return Opens::Namespace;
                    }
                }
                break;
            }
            Some(";" | "}" | "=" | "," | "=>") => break,
            // A preceding block or body ends this header.
            None if matches!(
                nodes.get(at),
                Some(Syntax::Group {
                    delimiter: Delimiter::Brace,
                    ..
                })
            ) =>
            {
                break;
            }
            _ => {}
        }
    }
    let Some(params) = params_before(nodes, brace) else {
        return Opens::Inherit;
    };
    // Look at the header words before the parameter list.
    let mut words = Vec::new();
    let mut at = params;
    while at > 0 && words.len() < 5 {
        at -= 1;
        match raw(nodes.get(at)) {
            Some(word) if matches!(word, ";" | "{" | "}" | "," | "=" | ":" | "(" | "?") => break,
            Some(word) => words.push(word),
            None => {
                if !matches!(nodes.get(at), Some(Syntax::Comment { .. })) {
                    break;
                }
            }
        }
    }
    let has = |word: &str| words.contains(&word);
    if matches!(
        words.first().copied(),
        Some("if" | "for" | "while" | "switch" | "catch" | "with")
    ) {
        return Opens::Inherit;
    }
    let generator = has("*");
    let async_ = has("async");
    if has("function") || in_class || generator || async_ {
        return Opens::Function(Contexts { generator, async_ });
    }
    // A method in an object literal: `name(...) {`, `get name() {`.
    if matches!(words.first(), Some(word) if word.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
    {
        return Opens::Function(NEITHER);
    }
    Opens::Inherit
}

/// Whether the arrow whose `=>` is at `arrow` is written `async`.
fn arrow_is_async(nodes: &[Syntax], arrow: usize) -> bool {
    let mut at = arrow;
    // `(params): T =>` or `x =>`
    while at > 0 {
        at -= 1;
        if is_paren(nodes.get(at))
            || matches!(raw(nodes.get(at)), Some(w) if w.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
        {
            return raw(at.checked_sub(1).and_then(|i| nodes.get(i))) == Some("async");
        }
        if matches!(raw(nodes.get(at)), Some(";" | "," | "=")) || arrow - at > 12 {
            return false;
        }
    }
    false
}

fn walk(nodes: &[Syntax], here: Contexts, class_body: bool, regions: &mut Vec<Region>) {
    let mut index = 0;
    // A class member's initializer, up to `;`, admits neither.
    let mut field_from: Option<usize> = None;
    while index < nodes.len() {
        let node = &nodes[index];
        if class_body {
            match raw(Some(node)) {
                Some("=") if field_from.is_none() => field_from = Some(end(node)),
                Some(";") => {
                    if let Some(from) = field_from.take() {
                        regions.push((from, start(node), NEITHER));
                    }
                }
                _ => {}
            }
        }
        // An expression-bodied arrow: its body runs to the next `,` or `;`
        // at this level, or the end of the group.
        if raw(Some(node)) == Some("=>")
            && !matches!(
                nodes.get(index + 1),
                Some(Syntax::Group {
                    delimiter: Delimiter::Brace,
                    ..
                })
            )
        {
            let contexts = Contexts {
                generator: false,
                async_: arrow_is_async(nodes, index),
            };
            let mut stop = index + 1;
            while stop < nodes.len() && !matches!(raw(nodes.get(stop)), Some("," | ";")) {
                stop += 1;
            }
            let body_end = if stop < nodes.len() {
                start(&nodes[stop])
            } else {
                nodes.last().map_or(end(node), end)
            };
            regions.push((end(node), body_end, contexts));
            walk(&nodes[index + 1..stop], contexts, false, regions);
            index = stop;
            continue;
        }
        if let Syntax::Group {
            delimiter,
            children,
            span,
            ..
        } = node
        {
            let range = (span.start as usize, span.end as usize);
            match delimiter {
                Delimiter::Brace => match opens(nodes, index, class_body) {
                    Opens::Function(inner) => {
                        regions.push((range.0, range.1, inner));
                        walk(children, inner, false, regions);
                    }
                    Opens::Class => {
                        regions.push((range.0, range.1, NEITHER));
                        walk(children, NEITHER, true, regions);
                    }
                    Opens::Namespace => {
                        regions.push((range.0, range.1, NEITHER));
                        walk(children, NEITHER, false, regions);
                    }
                    Opens::Inherit => walk(children, here, false, regions),
                },
                Delimiter::Paren
                    if matches!(
                        nodes.get(index + 1),
                        Some(Syntax::Group {
                            delimiter: Delimiter::Brace,
                            ..
                        })
                    ) && matches!(opens(nodes, index + 1, class_body), Opens::Function(_)) =>
                {
                    // Parameter defaults are evaluated as their own function.
                    regions.push((range.0, range.1, NEITHER));
                    walk(children, NEITHER, false, regions);
                }
                _ => walk(children, here, false, regions),
            }
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweetener_reader::read_syntax;

    fn at(code: &str, marker: &str) -> Contexts {
        let regions = regions(&read_syntax(code).children, MODULE);
        lookup(&regions, code.find(marker).expect("marker"), MODULE)
    }

    #[test]
    fn function_headers_decide_yield_and_await() {
        let code = "function* g() { A; const f = () => B; }\nconst c = C;\nasync function k(x = D) { E }\nclass K { *m() { F } x = G; async n() { H } }\nnamespace N { I }\nasync function* ag() { const q = async () => J; function inner() { K2 } }\nclass Z {}\nfunction* ty(): Generator<A, B, C> { P; }\nasync function* ab() { N; const q = async () => O; }\nconst o = { m() { L } };\nasync function k() { return w(M); }\n";
        let generator = Contexts {
            generator: true,
            async_: false,
        };
        let async_only = Contexts {
            generator: false,
            async_: true,
        };
        for (marker, expected) in [
            ("A;", generator),
            ("B;", NEITHER),
            ("C;", TOP),
            ("D)", NEITHER),
            ("E }", async_only),
            ("F }", generator),
            ("G;", NEITHER),
            ("H }", async_only),
            ("I }", NEITHER),
            ("J;", async_only),
            ("K2", NEITHER),
            ("L }", NEITHER),
            ("M)", async_only),
            (
                "N;",
                Contexts {
                    generator: true,
                    async_: true,
                },
            ),
            ("O;", async_only),
            ("P;", generator),
        ] {
            assert_eq!(at(code, marker), expected, "{marker}");
        }
    }
}
