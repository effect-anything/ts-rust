//! PORT: not in Go. Span map additions: macro expansions the mapper
//! reported, and cross-segment virtual ranges mapped as the union of the
//! original pieces they cover. `spanmap.rs` keeps one field and one call.

use crate::prelude::*;
use crate::spanmap::{Fidelity, Kind, MappedSpan, Segment, SpanMap};

/// One macro invocation in the original and the virtual text it expanded
/// to. `label` names the macro (and its module).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expansion {
    pub virtual_start: i32,
    pub virtual_end: i32,
    pub original_start: i32,
    pub original_end: i32,
    pub label: String,
}

/// The expansions attached to a [`SpanMap`], sorted by virtual start.
#[derive(Clone, Debug, Default)]
pub struct Expansions(Vec<Expansion>);

impl SpanMap {
    /// This map with the mapper's expansions attached.
    #[must_use]
    pub fn with_expansions(mut self, mut expansions: Vec<Expansion>) -> SpanMap {
        expansions.sort_by_key(|item| (item.virtual_start, -item.virtual_end));
        self.expansions = Expansions(expansions);
        self
    }

    /// Every expansion whose virtual text contains the virtual range `r`,
    /// innermost first.
    #[must_use]
    pub fn expansions_containing_virtual(m: Option<&SpanMap>, r: TextRange) -> Vec<&Expansion> {
        let Some(m) = m else {
            return Vec::new();
        };
        let mut found: Vec<&Expansion> = m
            .expansions
            .0
            .iter()
            .filter(|item| item.virtual_start <= r.pos() && r.end() <= item.virtual_end)
            .filter(|item| item.virtual_start < item.virtual_end)
            .collect();
        found.sort_by_key(|item| item.virtual_end - item.virtual_start);
        found
    }

    /// Every expansion whose invocation contains original position `pos`,
    /// innermost first.
    #[must_use]
    pub fn expansions_at_original(m: Option<&SpanMap>, pos: i32) -> Vec<&Expansion> {
        let Some(m) = m else {
            return Vec::new();
        };
        let mut found: Vec<&Expansion> = m
            .expansions
            .0
            .iter()
            .filter(|item| item.original_start <= pos && pos < item.original_end)
            .collect();
        found.sort_by_key(|item| item.original_end - item.original_start);
        found
    }

    /// Every original piece a virtual range covers, in virtual order: a
    /// verbatim segment contributes the exact slice under the range, an atom
    /// its whole original span. Synthesized gaps contribute nothing. Adjacent
    /// verbatim pieces that continue each other in the original are merged.
    #[must_use]
    pub fn virtual_to_original_spans(m: Option<&SpanMap>, r: TextRange) -> Vec<MappedSpan> {
        let Some(m) = m else {
            return vec![MappedSpan {
                span: r,
                fidelity: Fidelity::EXACT,
            }];
        };
        let start = r.pos();
        let end = r.end().max(start);
        if start == end {
            let (position, fidelity) = SpanMap::virtual_to_original_position(Some(m), start);
            if fidelity == Fidelity::NONE {
                return Vec::new();
            }
            return vec![MappedSpan {
                span: TextRange::new(position, position),
                fidelity,
            }];
        }
        covered_pieces(&SpanMap::segments(Some(m)), start, end)
    }
}

/// Go maps a cross-segment range by its two endpoints, which collapses or
/// inverts it when the covered segments' originals are out of order (macro
/// output reorders its operands). The span is the union of every covered
/// piece, and exact when it is one verbatim piece with no synthesized gap.
/// `low`/`high` are Go's endpoint mappings; `inside` is whether both
/// endpoints fall in segments.
#[must_use]
pub fn union_span(
    segments: &[Segment],
    (virtual_start, virtual_end, low, high): (i32, i32, i32, i32),
    inside: bool,
) -> (TextRange, Fidelity) {
    let pieces = covered_pieces(segments, virtual_start, virtual_end);
    let (mut orig_start, mut orig_end) = (low, high);
    if inside && let Some(first) = pieces.first() {
        orig_start = first.span.pos();
        orig_end = first.span.end();
    }
    for piece in &pieces {
        orig_start = orig_start.min(piece.span.pos());
        orig_end = orig_end.max(piece.span.end());
    }
    let exact = inside
        && covered_length(segments, virtual_start, virtual_end) == virtual_end - virtual_start
        && matches!(pieces.as_slice(), [only] if only.fidelity == Fidelity::EXACT);
    let fidelity = if exact {
        Fidelity::EXACT
    } else {
        Fidelity::APPROXIMATE
    };
    (TextRange::new(orig_start, orig_end), fidelity)
}

fn covered_length(segments: &[Segment], start: i32, end: i32) -> i32 {
    let first = segments.partition_point(|segment| segment.virtual_end <= start);
    segments[first..]
        .iter()
        .take_while(|segment| segment.virtual_start < end)
        .map(|segment| (end.min(segment.virtual_end) - start.max(segment.virtual_start)).max(0))
        .sum()
}

fn covered_pieces(segments: &[Segment], start: i32, end: i32) -> Vec<MappedSpan> {
    let first = segments.partition_point(|segment| segment.virtual_end <= start);
    let mut out: Vec<MappedSpan> = Vec::new();
    for segment in &segments[first..] {
        if segment.virtual_start >= end {
            break;
        }
        let lo = start.max(segment.virtual_start);
        let hi = end.min(segment.virtual_end);
        if lo >= hi {
            continue;
        }
        let piece = if segment.kind == Kind::VERBATIM {
            MappedSpan {
                span: TextRange::new(
                    segment.original_start + (lo - segment.virtual_start),
                    segment.original_start + (hi - segment.virtual_start),
                ),
                fidelity: Fidelity::EXACT,
            }
        } else {
            MappedSpan {
                span: TextRange::new(segment.original_start, segment.original_end),
                fidelity: Fidelity::ATOM,
            }
        };
        if let Some(last) = out.last_mut()
            && last.fidelity == Fidelity::EXACT
            && piece.fidelity == Fidelity::EXACT
            && last.span.end() == piece.span.pos()
        {
            last.span = TextRange::new(last.span.pos(), piece.span.end());
            continue;
        }
        out.push(piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spanmap::{Feature, new};

    #[test]
    fn cross_segment_ranges_are_unions_and_exact_when_chained() {
        let seg = |vs, ve, os, oe, kind| Segment {
            virtual_start: vs,
            virtual_end: ve,
            original_start: os,
            original_end: oe,
            kind,
            features: Feature::ALL,
        };
        // Macro output `[b, a]` from `pair(a, b)`: originals out of order.
        let m = new(&[
            seg(0, 1, 0, 9, Kind::ATOM),
            seg(1, 2, 8, 9, Kind::VERBATIM),
            seg(2, 4, 0, 9, Kind::ATOM),
            seg(4, 5, 5, 6, Kind::VERBATIM),
            seg(5, 6, 0, 9, Kind::ATOM),
        ]);
        let (span, fidelity) = SpanMap::virtual_to_original_span(Some(&m), TextRange::new(1, 5));
        assert_eq!((span.pos(), span.end()), (0, 9));
        assert_eq!(fidelity, Fidelity::APPROXIMATE);
        let pieces = SpanMap::virtual_to_original_spans(Some(&m), TextRange::new(1, 5));
        let spans: Vec<_> = pieces
            .iter()
            .map(|p| (p.span.pos(), p.span.end(), p.fidelity))
            .collect();
        assert_eq!(
            spans,
            vec![
                (8, 9, Fidelity::EXACT),
                (0, 9, Fidelity::ATOM),
                (5, 6, Fidelity::EXACT)
            ]
        );
        // Only verbatim, out of order, no atoms: Go gave (8, 6) clamped to (8, 8).
        let m = new(&[
            seg(0, 1, 8, 9, Kind::VERBATIM),
            seg(1, 2, 5, 6, Kind::VERBATIM),
        ]);
        let (span, _) = SpanMap::virtual_to_original_span(Some(&m), TextRange::new(0, 2));
        assert_eq!((span.pos(), span.end()), (5, 9));
        // Two verbatim segments that continue each other are one exact span.
        let m = new(&[
            seg(0, 3, 10, 13, Kind::VERBATIM),
            seg(3, 6, 13, 16, Kind::VERBATIM),
        ]);
        let (span, fidelity) = SpanMap::virtual_to_original_span(Some(&m), TextRange::new(1, 5));
        assert_eq!(
            (span.pos(), span.end(), fidelity),
            (11, 15, Fidelity::EXACT)
        );
    }
}
