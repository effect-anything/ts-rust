//! End to end: a `.sts` file through the transform session, then back from
//! output positions to the author's positions and text through the
//! session's spanmap tuples and its v3 source map.

use ts_ext_syntax::TransformSession;

const SOURCE: &str = "import { helper } from \"./helper.sts\";\nexport syntax pair:expr {\n  rule { pair($a:expr, $b:expr) } => { [$b, $a, $b] }\n}\nexport syntax all:expr {\n  rule { all($($x:expr),*) } => { [$($x + 1),*] }\n}\nconst ünï = \"é\";\nconst a = pair(left, right);\nconst d = all(p, q,\n  r);\nexport const e = helper(a, d, ünï);\n";

fn decode(mappings: &str) -> Vec<(usize, usize, usize, usize)> {
    let mut out = Vec::new();
    let (mut src_line, mut src_col) = (0i64, 0i64);
    for (line, text) in mappings.split(';').enumerate() {
        let mut col = 0i64;
        for segment in text.split(',').filter(|s| !s.is_empty()) {
            let mut fields = Vec::new();
            let (mut value, mut shift) = (0i64, 0);
            for ch in segment.bytes() {
                let digit = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
                    .iter()
                    .position(|&d| d == ch)
                    .unwrap() as i64;
                value |= (digit & 31) << shift;
                shift += 5;
                if digit & 32 == 0 {
                    fields.push(if value & 1 == 1 {
                        -(value >> 1)
                    } else {
                        value >> 1
                    });
                    value = 0;
                    shift = 0;
                }
            }
            col += fields[0];
            src_line += fields[2];
            src_col += fields[3];
            out.push((line, col as usize, src_line as usize, src_col as usize));
        }
    }
    out
}

fn line_col16(text: &str, at: usize) -> (usize, usize) {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    (
        text[..at].matches('\n').count(),
        text[line_start..at].encode_utf16().count(),
    )
}

fn offset_of(text: &str, line: usize, col16: usize) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let mut units = 0;
    for (index, ch) in text[start..].char_indices() {
        if units >= col16 {
            return start + index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn tuples(json: &str) -> Vec<[i64; 5]> {
    json.trim_matches(|c| c == '[' || c == ']')
        .split("],[")
        .filter(|s| !s.is_empty())
        .map(|row| {
            let v: Vec<i64> = row.split(',').map(|n| n.parse().unwrap()).collect();
            [v[0], v[1], v[2], v[3], v[4]]
        })
        .collect()
}

fn word_at(text: &str, at: usize) -> &str {
    let end = text[at..]
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .map_or(text.len(), |n| at + n);
    &text[at..end]
}

#[test]
fn output_positions_round_trip_to_author_text() {
    for session in [TransformSession::build(), TransformSession::check()] {
        let out = session.transform("main.sts", SOURCE);
        assert!(out.changed);
        // The specifier rewrite happened after expansion; the map follows it.
        assert!(out.code.contains("\"./helper\""), "{}", out.code);
        // Every verbatim tuple copies its original bytes exactly.
        for [vs, vl, os, ol, kind] in tuples(&out.mappings) {
            if kind == 0 {
                let (vs, vl, os, ol) = (vs as usize, vl as usize, os as usize, ol as usize);
                assert_eq!(vl, ol);
                assert_eq!(
                    &out.code[vs..vs + vl],
                    &SOURCE[os..os + ol],
                    "{}",
                    out.mappings
                );
            }
        }
        assert!(out.source_map.starts_with("{\"version\":3,"));
        assert!(
            out.source_map
                .contains("\"sourcesContent\":[\"import { helper }")
        );
        let mappings = out.source_map.split("\"mappings\":\"").nth(1).unwrap();
        let points = decode(&mappings[..mappings.find('"').unwrap()]);
        let lookup = |needle: &str, nth: usize| -> (usize, String) {
            let at = out.code.match_indices(needle).nth(nth).unwrap().0;
            let (line, col) = line_col16(&out.code, at);
            let &(_, _, sl, sc) = points
                .iter()
                .find(|p| p.0 == line && p.1 == col)
                .unwrap_or_else(|| panic!("no mapping for {needle}#{nth} in {}", out.code));
            let original = offset_of(SOURCE, sl, sc);
            (original, word_at(SOURCE, original).to_owned())
        };
        // Captures map to their own source tokens, duplicates included.
        let right = SOURCE.find("right").unwrap();
        let left = SOURCE.find("left").unwrap();
        let line = out.code.find("const a").unwrap();
        let nth = out.code[..line].matches("right").count();
        assert_eq!(lookup("right", nth), (right, "right".into()));
        assert_eq!(lookup("left", 0), (left, "left".into()));
        assert_eq!(lookup("right", nth + 1), (right, "right".into()));
        // A repetition row on a later author line.
        assert_eq!(
            lookup("r + 1", 0),
            (SOURCE.find("  r)").unwrap() + 2, "r".into())
        );
        // Columns past non-ASCII text are UTF-16, and the line after the
        // rewritten import is not shifted by the shorter specifier.
        assert_eq!(
            lookup("ünï", 1),
            (SOURCE.rfind("ünï").unwrap(), "ünï".into())
        );
        assert_eq!(
            lookup("helper(", 0),
            (SOURCE.find("helper(").unwrap(), "helper".into())
        );
        // Template text maps to the invocation that produced it.
        let bracket = out.code[line..].find('[').unwrap() + line;
        let (l, c) = line_col16(&out.code, bracket);
        let point = points.iter().find(|p| p.0 == l && p.1 == c).unwrap();
        assert_eq!(
            offset_of(SOURCE, point.2, point.3),
            SOURCE.find("pair(left").unwrap()
        );
    }
}
