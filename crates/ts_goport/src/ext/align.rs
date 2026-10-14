//! Token alignment between a text and a later rewrite of it. Not part of the
//! typescript-go port.
//!
//! The `TypeGPU` wrap and the operator rewrite keep the author's tokens but
//! insert wrapper text around them, collapse whitespace and hoist marked
//! function statements to the top of their scope. A source map through such
//! a rewrite needs every kept token at its new place, including tokens that
//! moved. [`align`] finds them: a recursive unique-token diff (monotonic
//! runs), then a pass that pairs the runs a hoist moved.
//!
//! Segments are `(new start, new end, old start, old end, kind)` in bytes;
//! kind `0` copies bytes one for one, kind `1` is generated text standing
//! for the whole old range (an operator that became a call).

use std::collections::HashMap;

/// One span-map tuple: new start, new end, old start, old end, kind.
pub type Tuple = (i32, i32, i32, i32, i32);

/// Byte ranges of the tokens of `text`. Whitespace separates tokens and is
/// not one. A comment, a string and a template are each one token.
#[must_use]
pub fn tokens(text: &str) -> Vec<(usize, usize)> {
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
        } else if ch == '"' || ch == '\'' || ch == '`' {
            at += 1;
            while at < bytes.len() {
                let next = bytes[at];
                if next == b'\\' {
                    at += 2;
                    continue;
                }
                at += 1;
                if next == ch as u8 || (ch != '`' && next == b'\n') {
                    break;
                }
            }
            at = at.min(bytes.len());
        } else if text[at..].starts_with("//") {
            at = text[at..].find('\n').map_or(bytes.len(), |end| at + end);
        } else if text[at..].starts_with("/*") {
            at = text[at + 2..]
                .find("*/")
                .map_or(bytes.len(), |end| at + 2 + end + 2);
        } else {
            at += len;
        }
        while !text.is_char_boundary(at) {
            at += 1;
        }
        out.push((start, at));
    }
    out
}

struct Side<'a> {
    text: &'a str,
    tokens: Vec<(usize, usize)>,
}

impl Side<'_> {
    fn tok(&self, index: usize) -> &str {
        let (start, end) = self.tokens[index];
        &self.text[start..end]
    }
    fn is_word(&self, index: usize) -> bool {
        self.tok(index).chars().any(|c| {
            c.is_alphanumeric()
                || c == '_'
                || c == '$'
                || c == '"'
                || c == '\''
                || c == '`'
                || c == '/'
        })
    }
}

/// Segments that carry `old` into `new`, see the module docs.
#[must_use]
pub fn align(old: &str, new: &str) -> Vec<Tuple> {
    let old = Side {
        text: old,
        tokens: tokens(old),
    };
    let new = Side {
        text: new,
        tokens: tokens(new),
    };
    let (n, m) = (old.tokens.len(), new.tokens.len());
    // new index -> old index.
    let mut pair: Vec<Option<usize>> = vec![None; m];
    let mut used = vec![false; n];
    seed_runs(&old, &new, &mut pair, &mut used);
    fill_gaps(&old, &new, &mut pair, &mut used);
    moved_runs(&old, &new, &mut pair, &mut used);
    drop_lone_punctuation(&new, &mut pair, &mut used);
    segments(&old, &new, &pair)
}

/// Words that occur once on each side are the same token. Each one grows
/// into the longest run of equal tokens around it, and the runs are taken
/// longest first. This is what follows a hoisted function.
fn seed_runs(old: &Side<'_>, new: &Side<'_>, pair: &mut [Option<usize>], used: &mut [bool]) {
    let mut counts: HashMap<&str, (u32, usize, u32, usize)> = HashMap::new();
    for i in 0..old.tokens.len() {
        if old.is_word(i) {
            let entry = counts.entry(old.tok(i)).or_insert((0, i, 0, 0));
            entry.0 += 1;
            entry.1 = i;
        }
    }
    for j in 0..new.tokens.len() {
        if let Some(entry) = counts.get_mut(new.tok(j)) {
            entry.2 += 1;
            entry.3 = j;
        }
    }
    // (length, old start, new start, seed old, seed new)
    let mut runs: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
    for &(oc, i, nc, j) in counts.values() {
        if oc != 1 || nc != 1 {
            continue;
        }
        let (mut i0, mut j0) = (i, j);
        while i0 > 0 && j0 > 0 && old.tok(i0 - 1) == new.tok(j0 - 1) {
            i0 -= 1;
            j0 -= 1;
        }
        let mut len = i - i0 + 1;
        while i0 + len < old.tokens.len()
            && j0 + len < new.tokens.len()
            && old.tok(i0 + len) == new.tok(j0 + len)
        {
            len += 1;
        }
        runs.push((len, i0, j0, i, j));
    }
    runs.sort_unstable_by(|a, b| b.cmp(a));
    for (len, i0, j0, i, j) in runs {
        if used[i] || pair[j].is_some() {
            continue;
        }
        // A longer run may already own the edge of this one (`);` before a
        // hoisted comment). Keep the free part around the seed.
        let free = |k: usize| !used[i0 + k] && pair[j0 + k].is_none();
        let seed = i - i0;
        let mut lo = seed;
        while lo > 0 && free(lo - 1) {
            lo -= 1;
        }
        let mut hi = seed + 1;
        while hi < len && free(hi) {
            hi += 1;
        }
        for k in lo..hi {
            pair[j0 + k] = Some(i0 + k);
            used[i0 + k] = true;
        }
    }
}

/// Each run of unpaired new tokens between two paired tokens that stay in
/// order is diffed against the unpaired old tokens between their partners.
fn fill_gaps(old: &Side<'_>, new: &Side<'_>, pair: &mut [Option<usize>], used: &mut [bool]) {
    let m = pair.len();
    let mut j = 0;
    while j <= m {
        if j < m && pair[j].is_some() {
            j += 1;
            continue;
        }
        let b0 = j;
        while j < m && pair[j].is_none() {
            j += 1;
        }
        let b1 = j;
        let a0 = if b0 == 0 {
            0
        } else {
            pair[b0 - 1].map_or(0, |i| i + 1)
        };
        let a1 = if b1 == m {
            old.tokens.len()
        } else {
            pair[b1].unwrap_or(0)
        };
        if b0 < b1 {
            if a0 < a1 && (a0..a1).all(|i| !used[i]) {
                diff(old, new, a0, a1, b0, b1, pair, used);
            } else {
                // The right partner moved (a hoist). The old tokens after
                // the left partner, up to the next paired one, are the
                // candidates.
                let end = (a0..old.tokens.len())
                    .find(|&i| used[i])
                    .unwrap_or(old.tokens.len());
                if a0 < end {
                    diff(old, new, a0, end, b0, b1, pair, used);
                }
            }
        }
        j += 1;
    }
}

/// Recursive unique-token diff of `old[a0..a1]` and `new[b0..b1]`: common
/// prefix and suffix, then the longest increasing run of tokens unique on
/// both sides as anchors, then the gaps between anchors.
#[allow(clippy::too_many_arguments)]
fn diff(
    old: &Side<'_>,
    new: &Side<'_>,
    mut a0: usize,
    mut a1: usize,
    mut b0: usize,
    mut b1: usize,
    pair: &mut [Option<usize>],
    used: &mut [bool],
) {
    while a0 < a1 && b0 < b1 && old.tok(a0) == new.tok(b0) {
        pair[b0] = Some(a0);
        used[a0] = true;
        a0 += 1;
        b0 += 1;
    }
    while a0 < a1 && b0 < b1 && old.tok(a1 - 1) == new.tok(b1 - 1) {
        pair[b1 - 1] = Some(a1 - 1);
        used[a1 - 1] = true;
        a1 -= 1;
        b1 -= 1;
    }
    if a0 >= a1 || b0 >= b1 {
        return;
    }
    let mut counts: HashMap<&str, (u32, usize, u32, usize)> = HashMap::new();
    for i in a0..a1 {
        let entry = counts.entry(old.tok(i)).or_insert((0, i, 0, 0));
        entry.0 += 1;
        entry.1 = i;
    }
    for j in b0..b1 {
        if let Some(entry) = counts.get_mut(new.tok(j)) {
            entry.2 += 1;
            entry.3 = j;
        }
    }
    // Unique pairs in new order, then the longest run increasing in old.
    let mut uniques: Vec<(usize, usize)> = counts
        .values()
        .filter(|entry| entry.0 == 1 && entry.2 == 1)
        .map(|entry| (entry.3, entry.1))
        .collect();
    if uniques.is_empty() {
        // No anchor: walk both sides, pairing the next equal token a few
        // tokens ahead (`a + b;` against `__tsover_add(a, b);`).
        let (mut i, mut j) = (a0, b0);
        while i < a1 && j < b1 {
            if let Some(k) = (j..b1.min(j + 8)).find(|&k| new.tok(k) == old.tok(i)) {
                pair[k] = Some(i);
                used[i] = true;
                j = k + 1;
            }
            i += 1;
        }
        return;
    }
    uniques.sort_unstable();
    let anchors = longest_increasing(&uniques);
    let (mut pa, mut pb) = (a0, b0);
    for (j, i) in anchors {
        diff(old, new, pa, i, pb, j, pair, used);
        pair[j] = Some(i);
        used[i] = true;
        pa = i + 1;
        pb = j + 1;
    }
    diff(old, new, pa, a1, pb, b1, pair, used);
}

/// Longest subsequence of `items` (sorted by `.0`) increasing in `.1`.
fn longest_increasing(items: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; items.len()];
    for (index, item) in items.iter().enumerate() {
        let at = tails.partition_point(|&tail| items[tail].1 < item.1);
        if at > 0 {
            prev[index] = tails[at - 1];
        }
        if at == tails.len() {
            tails.push(index);
        } else {
            tails[at] = index;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        out.push(items[at]);
        at = prev[at];
    }
    out.reverse();
    out
}

/// Pair the runs the monotonic diff could not: a hoisted function is the
/// same tokens at another place. A run counts when it is at least three
/// tokens long and holds a word.
fn moved_runs(old: &Side<'_>, new: &Side<'_>, pair: &mut [Option<usize>], used: &mut [bool]) {
    let mut free: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, taken) in used.iter().enumerate() {
        if !taken {
            free.entry(old.tok(i)).or_default().push(i);
        }
    }
    let mut j = 0;
    while j < pair.len() {
        if pair[j].is_some() {
            j += 1;
            continue;
        }
        let mut best = (0usize, 0usize);
        if let Some(candidates) = free.get(new.tok(j)) {
            for &i in candidates.iter().take(256) {
                if used[i] {
                    continue;
                }
                let mut len = 0;
                while j + len < pair.len()
                    && i + len < used.len()
                    && pair[j + len].is_none()
                    && !used[i + len]
                    && old.tok(i + len) == new.tok(j + len)
                {
                    len += 1;
                }
                if len > best.1 {
                    best = (i, len);
                }
            }
        }
        let (i, len) = best;
        if len >= 3 && (0..len).any(|k| new.is_word(j + k)) {
            for k in 0..len {
                pair[j + k] = Some(i + k);
                used[i + k] = true;
            }
            j += len;
        } else {
            j += 1;
        }
    }
}

/// A `(` or `,` paired with nothing paired next to it is any punctuation of
/// the rewrite, not the author's.
fn drop_lone_punctuation(new: &Side<'_>, pair: &mut [Option<usize>], used: &mut [bool]) {
    let snapshot = pair.to_vec();
    for (j, slot) in pair.iter_mut().enumerate() {
        let Some(i) = *slot else { continue };
        if new.is_word(j) {
            continue;
        }
        let before = j > 0 && i > 0 && snapshot[j - 1] == Some(i - 1);
        let after = snapshot.get(j + 1).copied().flatten() == Some(i + 1);
        if !before && !after {
            *slot = None;
            used[i] = false;
        }
    }
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Runs of paired tokens become verbatim segments. The text between two
/// runs that stay in order and replaced an operator becomes an atom.
fn segments(old: &Side<'_>, new: &Side<'_>, pair: &[Option<usize>]) -> Vec<Tuple> {
    // (new start, new end, old start, old end) of each verbatim run.
    let mut runs: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (j, slot) in pair.iter().enumerate() {
        let Some(i) = *slot else { continue };
        let (ns, ne) = new.tokens[j];
        let (os, oe) = old.tokens[i];
        if let Some(last) = runs.last_mut() {
            let gap_new = &new.text[last.1..ns];
            if os >= last.3 && old.text.get(last.3..os) == Some(gap_new) {
                last.1 = ne;
                last.3 = oe;
                continue;
            }
        }
        runs.push((ns, ne, os, oe));
    }
    let mut out = Vec::with_capacity(runs.len() * 2);
    for (index, run) in runs.iter().enumerate() {
        if index > 0 {
            let prev = runs[index - 1];
            let new_gap = new.text[prev.1..run.0].trim();
            let in_order = prev.3 <= run.2;
            if in_order && !new_gap.is_empty() {
                let old_gap = &old.text[prev.3..run.2];
                let trimmed = old_gap.trim_start();
                let lead = old_gap.len() - trimmed.len();
                let trimmed = trimmed.trim_end();
                // Only an operator the rewrite replaced (`+` → `, `) is an
                // atom. Other generated text (a hoist's `const name =`) is
                // not the author's and keeps no origin.
                let operator = !trimmed.is_empty()
                    && trimmed.len() <= 3
                    && !trimmed
                        .chars()
                        .any(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    && new_gap.len() <= 32;
                if operator {
                    let new_lead =
                        new.text[prev.1..run.0].len() - new.text[prev.1..run.0].trim_start().len();
                    out.push((
                        to_i32(prev.1 + new_lead),
                        to_i32(prev.1 + new_lead + new_gap.len()),
                        to_i32(prev.3 + lead),
                        to_i32(prev.3 + lead + trimmed.len()),
                        1,
                    ));
                }
            }
        }
        out.push((
            to_i32(run.0),
            to_i32(run.1),
            to_i32(run.2),
            to_i32(run.3),
            0,
        ));
    }
    out
}

/// Carry `segments` (old text → its source) through [`align`] of `old` and
/// `new`: the result maps `new` to that same source.
#[must_use]
pub fn retarget(old: &str, new: &str, segments: &[Tuple]) -> Vec<Tuple> {
    if old == new {
        return segments.to_vec();
    }
    let mut out = Vec::new();
    for (ns, ne, os, oe, kind) in align(old, new) {
        if kind == 0 {
            for &(vs, ve, gs, ge, inner) in segments {
                let lo = os.max(vs);
                let hi = oe.min(ve);
                if lo >= hi {
                    continue;
                }
                let from = ns + (lo - os);
                let to = ns + (hi - os);
                if inner == 0 {
                    out.push((from, to, gs + (lo - vs), gs + (hi - vs), 0));
                } else {
                    out.push((from, to, gs, ge, inner));
                }
            }
        } else {
            let mut low: Option<i32> = None;
            let mut high = 0;
            for &(vs, ve, gs, ge, inner) in segments {
                let lo = os.max(vs);
                let hi = oe.min(ve);
                if lo >= hi {
                    continue;
                }
                let (s, e) = if inner == 0 {
                    (gs + (lo - vs), gs + (hi - vs))
                } else {
                    (gs, ge)
                };
                low = Some(low.map_or(s, |low| low.min(s)));
                high = high.max(e);
            }
            if let Some(low) = low {
                out.push((ns, ne, low, high.max(low), 1));
            }
        }
    }
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::{align, retarget};

    fn mapped<'a>(old: &'a str, new: &str, at: &str) -> Option<&'a str> {
        let pos = i32::try_from(new.find(at)?).ok()?;
        align(old, new)
            .into_iter()
            .find(|seg| seg.4 == 0 && seg.0 <= pos && pos < seg.1)
            .map(|seg| {
                let start = usize::try_from(seg.2 + (pos - seg.0)).unwrap_or(0);
                &old[start..start + at.len()]
            })
    }

    #[test]
    fn a_hoisted_function_keeps_its_tokens() {
        let old = "console.log(add);\nfunction add(a, b) {\n  'use gpu';\n  return a + b;\n}\n";
        let new = "const add = ($ => set($.f = function add(a, b) {\n  'use gpu';\n  return __tsover_add(a, b);\n}, {v: 2}) && $.f)({});\nconsole.log(add);\n";
        assert_eq!(mapped(old, new, "'use gpu'"), Some("'use gpu'"));
        assert_eq!(mapped(old, new, "return"), Some("return"));
        assert_eq!(mapped(old, new, "console"), Some("console"));
        let segs = align(old, new);
        // The comma between the operands stands for the `+` it replaced.
        let call = i32::try_from(new.rfind("a, b)").unwrap() + 1).unwrap();
        assert!(
            segs.iter().any(|s| s.4 == 1
                && s.0 <= call
                && call < s.1
                && &old[s.2 as usize..s.3 as usize] == "+"),
            "{segs:?}"
        );
    }

    #[test]
    fn retarget_composes() {
        let source = "let x = 1 |> f;";
        let old = "let x = f(1);";
        let first = vec![(0, 8, 0, 8, 0), (8, 12, 8, 14, 1), (12, 13, 14, 15, 0)];
        let new = "let x = f(1);\n";
        assert_eq!(
            retarget(old, new, &first),
            vec![(0, 8, 0, 8, 0), (8, 12, 8, 14, 1), (12, 13, 14, 15, 0)]
        );
        let _ = source;
    }
}
