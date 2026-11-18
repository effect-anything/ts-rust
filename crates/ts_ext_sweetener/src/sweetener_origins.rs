//! Token-level origins of an expansion
//!
//! Upstream keeps an origin on every output token (`OriginStore`): a token
//! the user wrote keeps its source location, a token a template introduced
//! points at the invocation. Here the printer records segments; this pass
//! refines them to tokens. The printer carries each capture's source offset
//! through the template, the hygiene renames and nested expansions, so a
//! token the user wrote is in a verbatim segment and maps exactly. A token
//! left in an atom segment was written by a template and gets the span of
//! the invocation that printed it.

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
            at += ch.len_utf8();
            while at < text.len() {
                let next = text[at..].chars().next().unwrap_or('\n');
                if next == ch || next == '\n' {
                    break;
                }
                at += next.len_utf8();
                if next == '\\'
                    && let Some(escaped) = text[at..].chars().next()
                {
                    at += escaped.len_utf8();
                }
            }
            if at < text.len() {
                at += text[at..].chars().next().map_or(0, char::len_utf8);
            }
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
pub(crate) fn token_origins(code: &str, segments: &[Segment]) -> Vec<TokenOrigin> {
    let mut out = Vec::new();
    let mut segment_index = 0;
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
            SegmentKind::Atom => out.push(TokenOrigin {
                virtual_start: u32_of(start),
                virtual_end: u32_of(end),
                original_start: segment.original_start,
                original_end: segment.original_end,
                introduced: true,
            }),
        }
    }
    out
}

/// Index pairs of one longest common subsequence of two token sequences.
///
/// Myers' O((N+M)D) shortest edit script. The forward trace is the same
/// greedy walk the quadratic DP used: on a tie between deleting an old
/// token and inserting a new one, delete. Two LCS of equal length therefore
/// pick the same pairs the table produced, including when a token repeats.
fn lcs_pairs(
    old: &str,
    new: &str,
    old_tokens: &[(usize, usize)],
    new_tokens: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    let same = |i: usize, j: usize| {
        let (a0, a1) = old_tokens[i];
        let (b0, b1) = new_tokens[j];
        old[a0..a1] == new[b0..b1]
    };
    let (n, m) = (old_tokens.len(), new_tokens.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // Myers: `v[k]` is the furthest `i` reached on diagonal k = i - j.
    // The index of diagonal k is `k + offset`. `trace[d]` is `v` before
    // edit distance d. `v[1]` starts at 0, which is the sentinel the d = 0
    // step reads; every other value it reads was written by an earlier
    // distance. On a tie (`v[k - 1] == v[k + 1]`) the step deletes an old
    // token, the same choice as the quadratic walk's `>=` test.
    let max_d = n + m;
    let offset = max_d as isize;
    let idx = |k: isize| (k + offset) as usize;
    let mut v = vec![0isize; max_d * 2 + 1];
    let mut trace: Vec<Vec<isize>> = Vec::with_capacity(max_d + 1);
    let mut found = None;
    for d in 0..=max_d {
        trace.push(v.clone());
        let d_i = d as isize;
        let mut k = -d_i;
        while k <= d_i {
            let mut i = if k == -d_i || (k != d_i && v[idx(k - 1)] < v[idx(k + 1)]) {
                v[idx(k + 1)]
            } else {
                v[idx(k - 1)] + 1
            };
            let mut j = i - k;
            while i < n as isize && j < m as isize && same(i as usize, j as usize) {
                i += 1;
                j += 1;
            }
            v[idx(k)] = i;
            if i >= n as isize && j >= m as isize {
                found = Some(d);
                break;
            }
            k += 2;
        }
        if found.is_some() {
            break;
        }
    }
    let found = found.unwrap_or(max_d);

    // Reverse the script. `trace[d]` is the frontier before distance d, so
    // the snake at d runs from one step off that frontier to where `v`
    // ended. What remains after distance 1 is the d = 0 snake.
    let mut edits_rev: Vec<u8> = Vec::new();
    let (mut i, mut j) = (n as isize, m as isize);
    for d in (1..=found).rev() {
        let prev = &trace[d];
        let d_i = d as isize;
        let k = i - j;
        let down = k == -d_i || (k != d_i && prev[idx(k - 1)] < prev[idx(k + 1)]);
        let (prev_i, prev_j) = if down {
            let prev_i = prev[idx(k + 1)];
            (prev_i, prev_i - (k + 1))
        } else {
            let prev_i = prev[idx(k - 1)];
            (prev_i, prev_i - (k - 1))
        };
        while i > prev_i && j > prev_j {
            i -= 1;
            j -= 1;
            edits_rev.push(0);
        }
        if down {
            edits_rev.push(2);
            j -= 1;
        } else {
            edits_rev.push(1);
            i -= 1;
        }
    }
    while i > 0 && j > 0 {
        i -= 1;
        j -= 1;
        edits_rev.push(0);
    }
    edits_rev.reverse();

    let mut pairs = Vec::new();
    let (mut oi, mut ni) = (0usize, 0usize);
    for edit in edits_rev {
        match edit {
            0 => {
                pairs.push((oi, ni));
                oi += 1;
                ni += 1;
            }
            1 => oi += 1,
            _ => ni += 1,
        }
    }
    pairs
}

/// Carry `pieces` (ranges of `old` that copy the source) to `new`, which a
/// rewrite produced from `old`. Tokens the rewrite left alone keep their
/// origin; a token it changed or inserted has none. The tokens are aligned
/// by the rewrite's own diff (common prefix and suffix, then Myers'
/// shortest edit script, which is a longest common subsequence), not by
/// searching the source for a spelling.
pub(crate) fn carry(
    old: &str,
    new: &str,
    pieces: &[crate::sweetener_expand::Piece],
) -> Vec<crate::sweetener_expand::Piece> {
    if pieces.is_empty() {
        return Vec::new();
    }
    if old == new {
        return pieces.to_vec();
    }
    // A pure trim or a removed prefix/suffix only moves offsets.
    if let Some(shift) = old.find(new).filter(|_| new.len() * 2 >= old.len()) {
        let end = shift + new.len();
        return pieces
            .iter()
            .filter_map(|&(start, stop, at)| {
                let from = start.max(shift);
                let to = stop.min(end);
                (from < to).then(|| (from - shift, to - shift, at + u32_of(from - start)))
            })
            .collect();
    }
    let old_tokens = tokens(old);
    let new_tokens = tokens(new);
    let same = |a: (usize, usize), b: (usize, usize)| old[a.0..a.1] == new[b.0..b.1];
    let mut prefix = 0;
    while prefix < old_tokens.len()
        && prefix < new_tokens.len()
        && same(old_tokens[prefix], new_tokens[prefix])
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_tokens.len() - prefix
        && suffix < new_tokens.len() - prefix
        && same(
            old_tokens[old_tokens.len() - 1 - suffix],
            new_tokens[new_tokens.len() - 1 - suffix],
        )
    {
        suffix += 1;
    }
    let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|i| (i, i)).collect();
    let old_mid = &old_tokens[prefix..old_tokens.len() - suffix];
    let new_mid = &new_tokens[prefix..new_tokens.len() - suffix];
    if !old_mid.is_empty() && !new_mid.is_empty() {
        for (oi, ni) in lcs_pairs(old, new, old_mid, new_mid) {
            pairs.push((prefix + oi, prefix + ni));
        }
    }
    for k in 0..suffix {
        pairs.push((old_tokens.len() - suffix + k, new_tokens.len() - suffix + k));
    }
    let pairs = anchored_pairs(old, &old_tokens, pairs);
    let mut out: Vec<crate::sweetener_expand::Piece> = Vec::new();
    let mut piece = 0;
    for (oi, ni) in pairs {
        let (os, oe) = old_tokens[oi];
        while pieces.get(piece).is_some_and(|p| p.1 <= os) {
            piece += 1;
        }
        let Some(&(ps, pe, at)) = pieces.get(piece) else {
            break;
        };
        if os < ps || oe > pe {
            continue;
        }
        let (ns, ne) = new_tokens[ni];
        let src = at + u32_of(os - ps);
        // Join with the previous token when the text between them is the
        // same copy on both sides.
        if let Some(last) = out.last_mut() {
            let gap_new = &new[last.1..ns];
            let last_src_end = last.2 as usize + (last.1 - last.0);
            if ns >= last.1
                && src as usize >= last_src_end
                && src as usize - last_src_end == gap_new.len()
                && os >= ps + gap_new.len()
                && old.get(os - gap_new.len()..os) == Some(gap_new)
            {
                last.1 = ne;
                continue;
            }
        }
        out.push((ns, ne, src));
    }
    out
}

/// Drop a punctuation pair the alignment made on its own. A `(` or `,`
/// matched with no matched token next to it on both sides is any of the
/// rewrite's punctuation, not the one the author wrote, so it keeps no
/// origin. Words, and punctuation inside a run that agrees on both sides,
/// stay.
fn anchored_pairs(
    old: &str,
    old_tokens: &[(usize, usize)],
    pairs: Vec<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let word = |oi: usize| {
        let (a, b) = old_tokens[oi];
        old[a..b].chars().any(|c| {
            c.is_alphanumeric() || c == '_' || c == '$' || c == '"' || c == '\'' || c == '`'
        })
    };
    let set: std::collections::HashSet<(usize, usize)> = pairs.iter().copied().collect();
    pairs
        .iter()
        .copied()
        .filter(|&(oi, ni)| {
            word(oi)
                || (oi > 0 && ni > 0 && set.contains(&(oi - 1, ni - 1)))
                || set.contains(&(oi + 1, ni + 1))
        })
        .collect()
}

/// Compose `outer` (ranges of a text copying a middle text) with `inner`
/// (ranges of the middle text copying the source).
pub(crate) fn compose(
    outer: &[crate::sweetener_expand::Piece],
    inner: &[crate::sweetener_expand::Piece],
) -> Vec<crate::sweetener_expand::Piece> {
    let mut out = Vec::new();
    for &(start, end, mid) in outer {
        let mid = mid as usize;
        let mid_end = mid + (end - start);
        for &(a, b, src) in inner {
            let from = mid.max(a);
            let to = mid_end.min(b);
            if from < to {
                out.push((
                    start + (from - mid),
                    start + (to - mid),
                    src + u32_of(from - a),
                ));
            }
        }
    }
    out.sort_unstable();
    out
}

/// Split each atom (one invocation) at the ranges inside it that copy the
/// source: those become verbatim segments, the rest stays the invocation.
pub(crate) fn refine_segments(segments: &[Segment], fine: &[(u32, u32, u32)]) -> Vec<Segment> {
    if fine.is_empty() {
        return segments.to_vec();
    }
    let mut fine = fine.to_vec();
    fine.sort_unstable();
    let mut out = Vec::with_capacity(segments.len() + fine.len() * 2);
    let mut next = 0;
    for segment in segments {
        if segment.kind != SegmentKind::Atom {
            out.push(segment.clone());
            continue;
        }
        while fine
            .get(next)
            .is_some_and(|piece| piece.0 < segment.virtual_start)
        {
            next += 1;
        }
        let mut cursor = segment.virtual_start;
        while let Some(&(start, end, at)) = fine.get(next) {
            if end > segment.virtual_end {
                break;
            }
            next += 1;
            if start < cursor {
                continue;
            }
            if start > cursor {
                out.push(Segment {
                    virtual_start: cursor,
                    virtual_end: start,
                    ..segment.clone()
                });
            }
            out.push(Segment {
                virtual_start: start,
                virtual_end: end,
                original_start: at,
                original_end: at + (end - start),
                kind: SegmentKind::Verbatim,
            });
            cursor = end;
        }
        if cursor < segment.virtual_end || cursor == segment.virtual_start {
            out.push(Segment {
                virtual_start: cursor,
                virtual_end: segment.virtual_end,
                ..segment.clone()
            });
        }
    }
    out
}

/// Move span-map tuples `[virtual start, virtual end, original start,
/// original end, kind]` (kind `0` verbatim, `1` atom) from `old` to `new`,
/// a later rewrite of the same text. A verbatim range keeps exact bytes
/// wherever the rewrite left its tokens alone. Text the rewrite changed
/// becomes an atom over the original ranges the old text there mapped to,
/// so no offset is left pointing at the wrong place.
#[must_use]
pub fn retarget_segments(
    old: &str,
    new: &str,
    segments: &[(i32, i32, i32, i32, i32)],
) -> Vec<(i32, i32, i32, i32, i32)> {
    if old == new {
        return segments.to_vec();
    }
    let to_usize = |value: i32| usize::try_from(value).unwrap_or(0);
    let to_i32 = |value: usize| i32::try_from(value).unwrap_or(i32::MAX);
    // Identity copy of the whole old text, carried into the new text: an
    // old-to-new correspondence for every byte the rewrite kept.
    let kept = carry(old, new, &[(0, old.len(), 0)]);
    let mut out = Vec::new();
    // Old virtual range -> original range of what was there.
    let original_of = |from: usize, to: usize| -> Option<(i32, i32, bool)> {
        let mut low: Option<i32> = None;
        let mut high = 0;
        let mut exact = true;
        for &(vs, ve, os, oe, kind) in segments {
            let (vs, ve) = (to_usize(vs), to_usize(ve));
            let lo = from.max(vs);
            let hi = to.min(ve);
            if lo >= hi && !(from == to && vs <= from && from < ve) {
                continue;
            }
            let (s, e) = if kind == 0 {
                (os + to_i32(lo - vs), os + to_i32(hi.max(lo) - vs))
            } else {
                exact = false;
                (os, oe)
            };
            low = Some(low.map_or(s, |low: i32| low.min(s)));
            high = high.max(e);
        }
        low.map(|low| (low, high.max(low), exact))
    };
    let mut new_cursor = 0usize;
    let mut old_cursor = 0usize;
    let push_gap =
        |out: &mut Vec<_>, new_from: usize, new_to: usize, old_from: usize, old_to: usize| {
            if new_from >= new_to {
                return;
            }
            if let Some((s, e, _)) = original_of(old_from, old_to) {
                out.push((to_i32(new_from), to_i32(new_to), s, e, 1));
            }
        };
    for &(ns, ne, os) in &kept {
        let os = os as usize;
        let oe = os + (ne - ns);
        push_gap(&mut out, new_cursor, ns, old_cursor, os);
        // The kept run, split where the old segments split it.
        for &(vs, ve, gs, ge, kind) in segments {
            let (vs, ve) = (to_usize(vs), to_usize(ve));
            let lo = os.max(vs);
            let hi = oe.min(ve);
            if lo >= hi {
                continue;
            }
            let virtual_from = to_i32(ns + (lo - os));
            let virtual_to = to_i32(ns + (hi - os));
            if kind == 0 {
                out.push((
                    virtual_from,
                    virtual_to,
                    gs + to_i32(lo - vs),
                    gs + to_i32(hi - vs),
                    0,
                ));
            } else {
                out.push((virtual_from, virtual_to, gs, ge, kind));
            }
        }
        new_cursor = ne;
        old_cursor = oe;
    }
    push_gap(&mut out, new_cursor, new.len(), old_cursor, old.len());
    out.sort_unstable();
    // Adjacent pieces of one atom are one atom again.
    let mut merged: Vec<(i32, i32, i32, i32, i32)> = Vec::with_capacity(out.len());
    for piece in out {
        if let Some(last) = merged.last_mut()
            && last.1 == piece.0
            && last.4 == piece.4
            && ((piece.4 == 1 && last.2 == piece.2 && last.3 == piece.3)
                || (piece.4 == 0 && last.3 == piece.2))
        {
            last.1 = piece.1;
            last.3 = piece.3.max(last.3);
            continue;
        }
        merged.push(piece);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{carry, lcs_pairs, retarget_segments, tokens};

    /// The quadratic DP this replaced: common-subsequence lengths filled
    /// from the end, then a forward walk that deletes an old token whenever
    /// that score ties an insertion. Tests compare Myers against it.
    fn dp_pairs(old: &[String], new: &[String]) -> Vec<(usize, usize)> {
        let (n, m) = (old.len(), new.len());
        let mut table = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * (m + 1) + j] = if old[i] == new[j] {
                    table[(i + 1) * (m + 1) + j + 1] + 1
                } else {
                    table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
                };
            }
        }
        let mut pairs = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if old[i] == new[j] {
                pairs.push((i, j));
                i += 1;
                j += 1;
            } else if table[(i + 1) * (m + 1) + j] >= table[i * (m + 1) + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
        pairs
    }

    fn spellings(text: &str) -> Vec<String> {
        tokens(text)
            .into_iter()
            .map(|(a, b)| text[a..b].to_owned())
            .collect()
    }

    fn assert_matches_dp(old: &str, new: &str) {
        let old_tokens = tokens(old);
        let new_tokens = tokens(new);
        let pairs = lcs_pairs(old, new, &old_tokens, &new_tokens);
        let want = dp_pairs(&spellings(old), &spellings(new));
        assert_eq!(pairs, want, "myers {pairs:?} != dp {want:?}");
        for &(i, j) in &pairs {
            let (a0, a1) = old_tokens[i];
            let (b0, b1) = new_tokens[j];
            assert_eq!(&old[a0..a1], &new[b0..b1]);
        }
        // Strictly increasing on both sides: a real alignment, not a bag.
        for w in pairs.windows(2) {
            assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1);
        }
    }

    #[test]
    fn diff_edges_match_the_quadratic_walk() {
        // Empty on either side, and both empty.
        assert_matches_dp("", "");
        assert_matches_dp("", "a b");
        assert_matches_dp("a b", "");
        // Identical, including one token and whitespace-only.
        assert_matches_dp("a b c", "a b c");
        assert_matches_dp("only", "only");
        assert_matches_dp("  \n", " \t");
        // Fully disjoint: no token survives.
        assert_matches_dp("a b c", "x y z");
        assert_matches_dp("foo", "bar baz");
        // A repeated token has more than one LCS. The walk must pick the
        // same one the table did: delete the old token on a tie.
        assert_matches_dp("a a", "a");
        assert_matches_dp("a", "a a");
        assert_matches_dp("a b a", "a a");
        assert_matches_dp("a x a y a", "a a a");
        assert_matches_dp("a b a b a", "b a b");
        // A change in the middle, with a shared prefix and suffix.
        assert_matches_dp("a b c d", "a x c d");
        assert_matches_dp("a b c", "a b c d e");
        assert_matches_dp("x y a b", "a b");
    }

    #[test]
    fn repeated_tokens_keep_the_dp_origin() {
        // `a` occurs three times. The first and the last are the ones the
        // DP keeps; the middle `a` is the insertion, so it gets no origin.
        let kept = carry("a b a", "a a a", &[(0, 5, 10)]);
        assert_eq!(kept, vec![(0, 1, 10), (4, 5, 14)]);
    }

    #[test]
    fn retarget_edges() {
        // Unchanged text is unchanged segments.
        let same = [(0, 5, 0, 5, 0)];
        assert_eq!(retarget_segments("a b c", "a b c", &same), same);
        // A disjoint rewrite drops the verbatim copy and keeps an atom
        // over the original range.
        assert_eq!(
            retarget_segments("a b", "x y", &[(0, 3, 7, 10, 0)]),
            vec![(0, 3, 7, 10, 1)]
        );
        // Nothing to retarget.
        assert!(retarget_segments("a", "b", &[]).is_empty());
        // Empty rewrite.
        assert!(retarget_segments("a b", "", &[(0, 3, 0, 3, 0)]).is_empty());
    }

    /// Large inputs. `cargo test -- --ignored` runs it; a normal test run
    /// only checks that it compiles. A few hundred tokens with an edit in
    /// every one used to allocate an N*M table; Myers keeps a row per edit
    /// distance and finishes it in well under a second.
    #[test]
    #[ignore = "large-input timing of the Myers token diff"]
    fn myers_large_input_is_fast() {
        let n = 600;
        // One repeated token, so the common prefix and suffix are one each
        // and the middle is a long run of equal tokens with a few edits.
        // The edit distance stays small while the sequence does not.
        let mut old = "a ".repeat(n);
        for i in (0..n).step_by(n / 20) {
            let at = i * 2;
            old.replace_range(at..at + 1, "b");
        }
        let mut new_text = "a ".repeat(n);
        for i in (10..n).step_by(n / 20) {
            let at = i * 2;
            new_text.replace_range(at..at + 1, "c");
        }
        let start = std::time::Instant::now();
        let kept = carry(&old, &new_text, &[(0, old.len(), 0)]);
        let elapsed = start.elapsed();
        // Adjacent copies of `a` join into one piece, so the kept tokens
        // come back as a handful of runs rather than one entry each. What
        // matters is that the runs cover most of the text.
        let covered: usize = kept.iter().map(|(s, e, _)| e - s).sum();
        assert!(
            covered > old.len() / 2,
            "covered {covered} of {}",
            old.len()
        );
        assert!(
            elapsed.as_millis() < 1_000,
            "myers took {elapsed:?} on {n} tokens"
        );
    }

    /// A lone `(` the alignment paired between unrelated words is not the
    /// author's paren; punctuation inside a matching run still is.
    #[test]
    fn isolated_punctuation_pairs_keep_no_origin() {
        let old = "alpha ( beta";
        let new = "gamma ( delta";
        let pieces = carry(old, new, &[(0, old.len(), 100)]);
        assert!(pieces.is_empty(), "{pieces:?}");
        let old = "f(a, b)";
        let new = "g(a, b)";
        let pieces = carry(old, new, &[(0, old.len(), 100)]);
        assert_eq!(pieces, vec![(1, 7, 101)]);
    }
}
