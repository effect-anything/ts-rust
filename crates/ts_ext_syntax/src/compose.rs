//! Compose a later transform's source map with the tsc-ext span map.
//!
//! A bundler runs other transforms on the virtual TypeScript a
//! [`crate::Transformed`] returns. Their map points into that text. This
//! module rewrites every mapping of such a map through the span map (the
//! byte-exact `[virtualStart, virtualLength, originalStart, originalLength,
//! kind]` tuples), so the result points at the `.sts` the author wrote:
//!
//! - a position in a verbatim range maps to the same character of the
//!   author text, exactly;
//! - a position in a macro atom maps to the start of the author range that
//!   produced it (the invocation, or the captured operand);
//! - a position no range covers (text the expansion introduced) stays in
//!   the map as an unmapped segment, so it is not credited to its neighbour.
//!
//! Columns are UTF-16 code units, as Source Map v3 counts them.

use serde_json::{Value, json};

/// One span-map tuple: virtual start, virtual end, original start, original
/// end (bytes), kind (0 verbatim).
type Span = (usize, usize, usize, usize, i64);

/// Parse `mappings_json` output.
///
/// # Errors
/// When the text is not that JSON.
pub fn parse_spans(text: &str) -> Result<Vec<Span>, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let rows = value.as_array().ok_or("span map: not an array")?;
    let mut spans = Vec::with_capacity(rows.len());
    for row in rows {
        let field = |index: usize| row.get(index).and_then(Value::as_i64).unwrap_or(-1);
        let (vs, vl, os, ol, kind) = (field(0), field(1), field(2), field(3), field(4));
        if vs < 0 || vl < 0 || os < 0 || ol < 0 {
            continue;
        }
        let as_usize = |value: i64| usize::try_from(value).unwrap_or(0);
        spans.push((
            as_usize(vs),
            as_usize(vs + vl),
            as_usize(os),
            as_usize(os + ol),
            kind,
        ));
    }
    spans.sort_unstable();
    Ok(spans)
}

/// The author offset of byte `at` of the virtual text, if a span covers it.
fn through(spans: &[Span], at: usize) -> Option<usize> {
    let index = spans.partition_point(|span| span.0 <= at);
    // Span-map ranges are disjoint; a few earlier ones are checked in case
    // an atom nests a range. The latest-starting one wins.
    for span in spans[..index].iter().rev().take(8) {
        if at < span.1 || (at == span.1 && span.0 == span.1) {
            return Some(if span.4 == 0 {
                (span.2 + (at - span.0)).min(span.3.max(span.2))
            } else {
                span.2
            });
        }
    }
    None
}

struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self { text, starts }
    }

    /// Byte offset of (line, UTF-16 column). A column past the line end is
    /// the line end.
    fn offset(&self, line: usize, column: usize) -> Option<usize> {
        let start = *self.starts.get(line)?;
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1);
        let mut units = 0;
        for (at, ch) in self.text[start..end].char_indices() {
            if units >= column {
                return Some(start + at);
            }
            units += ch.len_utf16();
        }
        Some(end)
    }

    /// (line, UTF-16 column) of byte `at`.
    fn position(&self, at: usize) -> (usize, usize) {
        let at = at.min(self.text.len());
        let line = self.starts.partition_point(|&start| start <= at) - 1;
        let mut end = at;
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        let column = self.text[self.starts[line]..end].encode_utf16().count();
        (line, column)
    }
}

/// The base64 alphabet of VLQ fields and inline maps.
pub(crate) const DIGITS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn decode(mappings: &str) -> Result<Vec<Vec<Vec<i64>>>, String> {
    let mut lines = Vec::new();
    for line in mappings.split(';') {
        let mut segments = Vec::new();
        for segment in line.split(',').filter(|segment| !segment.is_empty()) {
            let mut fields = Vec::new();
            let (mut value, mut shift) = (0i64, 0);
            for byte in segment.bytes() {
                let digit = DIGITS
                    .iter()
                    .position(|&d| d == byte)
                    .ok_or_else(|| format!("invalid mappings character {:?}", byte as char))?;
                let digit = i64::try_from(digit).unwrap_or(0);
                value += (digit & 31) << shift;
                if digit & 32 == 0 {
                    fields.push(if value & 1 == 1 {
                        -(value >> 1)
                    } else {
                        value >> 1
                    });
                    value = 0;
                    shift = 0;
                } else {
                    shift += 5;
                }
            }
            segments.push(fields);
        }
        lines.push(segments);
    }
    Ok(lines)
}

/// Append `value` as one base64 VLQ field of a v3 source map.
pub(crate) fn vlq(out: &mut String, value: i64) {
    let mut rest = if value < 0 {
        ((-value) << 1) | 1
    } else {
        value << 1
    };
    loop {
        let mut digit = rest & 31;
        rest >>= 5;
        if rest > 0 {
            digit |= 32;
        }
        out.push(char::from(
            DIGITS[usize::try_from(digit).unwrap_or_default()],
        ));
        if rest == 0 {
            break;
        }
    }
}

/// Is `name` (a `sources` entry of the later map) the virtual file?
fn names_virtual(name: &str, host_path: &str, path: &str) -> bool {
    let base = |text: &str| text.rsplit(['/', '\\']).next().unwrap_or(text).to_owned();
    let name = name.split(['?', '#']).next().unwrap_or(name);
    name == host_path
        || name == path
        || name.ends_with(&format!("/{}", base(host_path)))
        || name == base(host_path)
        || name.ends_with(&format!("/{}", base(path)))
        || name == base(path)
}

/// `later` (a v3 map whose sources include the virtual text `code`) composed
/// through `spans` onto `source`, the author file at `path`. Sources other
/// than the virtual file are kept as they are. When `later` has a single
/// source it is taken to be the virtual file, whatever its name.
///
/// # Errors
/// When `later` is not a v3 map.
#[allow(clippy::too_many_lines)] // one decode, map and encode loop
pub fn compose(
    path: &str,
    host_path: &str,
    source: &str,
    code: &str,
    spans: &[Span],
    later: &Value,
) -> Result<Value, String> {
    let mappings = later
        .get("mappings")
        .and_then(Value::as_str)
        .ok_or("map has no `mappings` string")?;
    let sources: Vec<String> = later
        .get("sources")
        .and_then(Value::as_array)
        .ok_or("map has no `sources` array")?
        .iter()
        .map(|item| item.as_str().unwrap_or("").to_owned())
        .collect();
    let contents = later.get("sourcesContent").and_then(Value::as_array);
    let virtual_index: Vec<bool> = if sources.len() == 1 {
        vec![true]
    } else {
        sources
            .iter()
            .map(|name| names_virtual(name, host_path, path))
            .collect()
    };
    // Output sources: the author file first, then the others in order.
    let mut out_sources = vec![Value::String(path.to_owned())];
    let mut out_contents = vec![Value::String(source.to_owned())];
    let mut remap = Vec::with_capacity(sources.len());
    for (index, name) in sources.iter().enumerate() {
        if virtual_index[index] {
            remap.push(0usize);
        } else {
            remap.push(out_sources.len());
            out_sources.push(Value::String(name.clone()));
            out_contents.push(
                contents
                    .and_then(|items| items.get(index))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
    }
    let virtual_lines = Lines::new(code);
    let source_lines = Lines::new(source);
    let mut encoded = String::new();
    let (mut src, mut src_line, mut src_col, mut name) = (0i64, 0i64, 0i64, 0i64);
    let (mut in_src, mut in_line, mut in_col, mut in_name) = (0i64, 0i64, 0i64, 0i64);
    for (line_index, line) in decode(mappings)?.iter().enumerate() {
        if line_index > 0 {
            encoded.push(';');
        }
        let mut column = 0i64;
        let mut out_column = 0i64;
        let mut first = true;
        for fields in line {
            if fields.is_empty() {
                continue;
            }
            column += fields[0];
            let mut target: Option<(i64, i64, i64, Option<i64>)> = None;
            if fields.len() >= 4 {
                in_src += fields[1];
                in_line += fields[2];
                in_col += fields[3];
                let name_field = if fields.len() >= 5 {
                    in_name += fields[4];
                    Some(in_name)
                } else {
                    None
                };
                let index = usize::try_from(in_src).unwrap_or(usize::MAX);
                if virtual_index.get(index).copied().unwrap_or(false) {
                    let at = virtual_lines.offset(
                        usize::try_from(in_line).unwrap_or(usize::MAX),
                        usize::try_from(in_col).unwrap_or(0),
                    );
                    if let Some(original) = at.and_then(|at| through(spans, at)) {
                        let (line, col) = source_lines.position(original);
                        target = Some((
                            0,
                            i64::try_from(line).unwrap_or(0),
                            i64::try_from(col).unwrap_or(0),
                            name_field,
                        ));
                    }
                } else if let Some(&mapped) = remap.get(index) {
                    target = Some((
                        i64::try_from(mapped).unwrap_or(0),
                        in_line,
                        in_col,
                        name_field,
                    ));
                }
            }
            if !first {
                encoded.push(',');
            }
            first = false;
            vlq(&mut encoded, column - out_column);
            out_column = column;
            if let Some((s, l, c, n)) = target {
                vlq(&mut encoded, s - src);
                vlq(&mut encoded, l - src_line);
                vlq(&mut encoded, c - src_col);
                src = s;
                src_line = l;
                src_col = c;
                if let Some(n) = n {
                    vlq(&mut encoded, n - name);
                    name = n;
                }
            }
        }
    }
    let mut map = json!({
        "version": 3,
        "sources": out_sources,
        "sourcesContent": out_contents,
        "names": later.get("names").cloned().unwrap_or_else(|| json!([])),
        "mappings": encoded,
    });
    if let Some(file) = later.get("file") {
        map["file"] = file.clone();
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::{compose, decode, parse_spans};
    use serde_json::json;

    #[test]
    fn verbatim_positions_map_exactly_and_introduced_text_is_unmapped() {
        // virtual "let a = 1;\nX;\nlet b = 2;" from source "let a = 1;\nlet b = 2;"
        let source = "let a = 1;\nlet b = 2;";
        let code = "let a = 1;\nX;\nlet b = 2;";
        let spans = parse_spans("[[0,11,0,11,0],[14,10,11,10,0]]").unwrap();
        // identity map of `code`: one segment per line start and at col 4.
        let later = json!({"version":3,"sources":["a.ts"],"names":[],
            "mappings":"AAAA,IAAI;AACJ;AACA,IAAI"});
        let map = compose("a.sts", "a.ts", source, code, &spans, &later).unwrap();
        assert_eq!(map["sources"], json!(["a.sts"]));
        let lines = decode(map["mappings"].as_str().unwrap()).unwrap();
        // line 0: (0 -> 0:0), (4 -> 0:4); line 1: unmapped; line 2: -> 1:0, 1:4
        assert_eq!(lines[0], vec![vec![0, 0, 0, 0], vec![4, 0, 0, 4]]);
        assert_eq!(lines[1], vec![vec![0]]);
        assert_eq!(lines[2], vec![vec![0, 0, 1, -4], vec![4, 0, 0, 4]]);
    }
}
