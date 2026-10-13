//! Token-level origins of an expansion
//!
//! Upstream keeps an origin on every output token (`OriginStore`): a token
//! the user wrote keeps its source location, a token a template introduced
//! points at the invocation. Here the printer records segments; this pass
//! refines them to tokens. A verbatim segment maps each token exactly. In
//! an atom segment (one invocation), a token spelled the same as a source
//! token of that invocation, in order, came from the user; any other token
//! was introduced by the template and gets the invocation's span.

use crate::{Segment, SegmentKind, TokenOrigin};

/// Byte ranges of the tokens of `text`: identifiers and numbers, string
/// literals, and every other non-space character on its own.
pub(crate) fn tokens(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let ch = text[at..].chars().next().unwrap_or(' ');
        let len = ch.len_utf8();
        if ch.is_whitespace() {
            at += len;
            continue;
        }
        let start = at;
        if ch == '_' || ch == '$' || ch.is_alphanumeric() {
            while let Some(next) = text[at..].chars().next() {
                if next == '_' || next == '$' || next.is_alphanumeric() {
                    at += next.len_utf8();
                } else {
                    break;
                }
            }
        } else if ch == '"' || ch == '\'' {
            at += 1;
            while at < bytes.len() && bytes[at] != ch as u8 && bytes[at] != b'\n' {
                at += if bytes[at] == b'\\' { 2 } else { 1 };
            }
            at = (at + 1).min(bytes.len());
        } else {
            at += len;
        }
        while !text.is_char_boundary(at.min(text.len())) {
            at += 1;
        }
        at = at.min(text.len());
        out.push((start, at));
    }
    out
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// One origin per token of `code`, in order. Tokens outside every segment
/// (synthesized imports, hoisted helpers) are introduced with no span.
pub(crate) fn token_origins(code: &str, source: &str, segments: &[Segment]) -> Vec<TokenOrigin> {
    let mut out = Vec::new();
    let mut segment_index = 0;
    // Source tokens of the current atom, and how far alignment has used them.
    let mut atom: Option<(usize, Vec<(usize, usize)>, usize)> = None;
    for (start, end) in tokens(code) {
        while segments
            .get(segment_index)
            .is_some_and(|segment| (segment.virtual_end as usize) <= start)
        {
            segment_index += 1;
        }
        let segment = segments.get(segment_index).filter(|segment| {
            (segment.virtual_start as usize) <= start && end <= segment.virtual_end as usize
        });
        let Some(segment) = segment else {
            out.push(TokenOrigin {
                virtual_start: u32_of(start),
                virtual_end: u32_of(end),
                original_start: 0,
                original_end: 0,
                introduced: true,
            });
            continue;
        };
        match segment.kind {
            SegmentKind::Verbatim => {
                let delta = start - segment.virtual_start as usize;
                out.push(TokenOrigin {
                    virtual_start: u32_of(start),
                    virtual_end: u32_of(end),
                    original_start: segment.original_start + u32_of(delta),
                    original_end: segment.original_start + u32_of(delta + end - start),
                    introduced: false,
                });
            }
            SegmentKind::Atom => {
                if atom
                    .as_ref()
                    .is_none_or(|(index, _, _)| *index != segment_index)
                {
                    let from = segment.original_start as usize;
                    let text = source
                        .get(from..segment.original_end as usize)
                        .unwrap_or_default();
                    let found = tokens(text)
                        .into_iter()
                        .map(|(s, e)| (s + from, e + from))
                        .collect();
                    atom = Some((segment_index, found, 0));
                }
                let (_, found, used) = atom.as_mut().expect("atom tokens");
                let spelling = &code[start..end];
                let hit = found[*used..]
                    .iter()
                    .position(|(s, e)| source.get(*s..*e) == Some(spelling));
                // A capture the template prints twice is the same source
                // token both times.
                let again = || {
                    found[..*used]
                        .iter()
                        .rposition(|(s, e)| source.get(*s..*e) == Some(spelling))
                };
                let hit = hit.map(|offset| *used + offset).or_else(again);
                if let Some(index) = hit {
                    let (s, e) = found[index];
                    *used = (*used).max(index + 1);
                    out.push(TokenOrigin {
                        virtual_start: u32_of(start),
                        virtual_end: u32_of(end),
                        original_start: u32_of(s),
                        original_end: u32_of(e),
                        introduced: false,
                    });
                } else {
                    out.push(TokenOrigin {
                        virtual_start: u32_of(start),
                        virtual_end: u32_of(end),
                        original_start: segment.original_start,
                        original_end: segment.original_end,
                        introduced: true,
                    });
                }
            }
        }
    }
    out
}
