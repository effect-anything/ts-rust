//! Passes that run after parse and content mapping, before check.
//!
//! The name is the phase, not "after emit". A later emit wrapper is a
//! different hook and does not belong here.
//!
//! A content mapper owns the runtime JS of a foreign file. These passes do
//! not. They rewrite ordinary TypeScript into other ordinary TypeScript and
//! attach a span map so diagnostics land on the author's text. Emit still
//! belongs to this compiler.
//!
//! Register another pass in [`checked`]. Do not special-case its name in the
//! emitter.

use crate::frontend::parser::ParsedSourceFile;
use std::rc::Rc;

/// The program's parse hook: run the internal passes on a file the loader
/// just parsed (content mapping has already happened) and hand it back.
///
/// Operator overloads are not rewritten here. Check and LSP type the
/// author's `+` from `Symbol.operatorPlus`. Emit rewrites the calls.
/// The passes only report: the file and its `Rc` stay the same.
#[must_use]
pub fn checked(file: Option<Rc<ParsedSourceFile>>) -> Option<Rc<ParsedSourceFile>> {
    if let Some(file) = &file {
        crate::ext::tsover::report_nesting(file);
        crate::ext::typegpu::report_refusals(file);
    }
    file
}

/// Point the related information of a mapper diagnostic at the mapped file,
/// as its primary location is.
pub fn retarget_related(diagnostic: &mut crate::core::Diagnostic, file: crate::core::Node) {
    if diagnostic.related_information().is_empty() {
        return;
    }
    let related = diagnostic
        .related_information()
        .iter()
        .cloned()
        .map(|mut item| {
            item.set_file(file);
            item
        })
        .collect();
    diagnostic.set_related_info(related);
}

/// Runtime wrappers for emitted JavaScript. Check does not call this.
///
/// The caller is the loading thread, after the emit workers have returned.
/// Each file is parsed into a store that the transform drops. Nothing is
/// written to the process parse store.
#[must_use]
pub fn emit_javascript(filename: &str, code: &str) -> String {
    // `emit_output` runs the operator rewrite to completion before the
    // metadata wrap. Each of those parses takes this thread's one detached
    // store, so the first has to be dropped before the second starts.
    emit_output(filename, code).code
}

/// The rewritten JavaScript and the span segments that produced it.
///
/// `segments` is empty when the text is unchanged. A source map of the
/// original emit composes through them (`compose_source_map`) so a column
/// in the rewritten file still names the author's location.
struct EmittedOutput {
    code: String,
    /// Empty when `code` is `emit_javascript` of the input with no recorded
    /// shift. The input text is then its own line map.
    segments: Vec<crate::spanmap::Segment>,
}

fn emit_output(filename: &str, code: &str) -> EmittedOutput {
    // The rewrite owns this thread's detached store until it returns, so
    // its text and segments are taken out before the metadata parse, which
    // needs that same store. A file without a directive still goes through
    // the metadata pass: a shell callback (`tgpu.fn(...)(() => ...)`) and
    // auto-naming need no directive, as in unplugin-typegpu.
    let (rewritten, segments) = match crate::ext::tsover::rewrite_text(filename, code) {
        Some(rewrite) => (rewrite.code, to_tuples(&rewrite.segments)),
        None => (
            code.to_owned(),
            vec![(
                0,
                i32::try_from(code.len()).unwrap_or(i32::MAX),
                0,
                i32::try_from(code.len()).unwrap_or(i32::MAX),
                0,
            )],
        ),
    };
    let wrapped = crate::ext::typegpu::embed_metadata_js(filename, &rewritten);
    if wrapped == code {
        return EmittedOutput {
            code: wrapped,
            segments: Vec::new(),
        };
    }
    // The wrap inserts text, collapses whitespace and hoists marked
    // function statements. Token alignment carries every kept token,
    // including a hoisted one, so the composed map stays exact.
    let segments = crate::ext::align::retarget(&rewritten, &wrapped, &segments);
    EmittedOutput {
        code: wrapped,
        segments: from_tuples(&segments),
    }
}

fn to_tuples(segments: &[crate::spanmap::Segment]) -> Vec<crate::ext::align::Tuple> {
    segments
        .iter()
        .map(|segment| {
            (
                segment.virtual_start,
                segment.virtual_end,
                segment.original_start,
                segment.original_end,
                i32::from(segment.kind != crate::spanmap::Kind::VERBATIM),
            )
        })
        .collect()
}

fn from_tuples(tuples: &[crate::ext::align::Tuple]) -> Vec<crate::spanmap::Segment> {
    tuples
        .iter()
        .map(
            |&(virtual_start, virtual_end, original_start, original_end, kind)| {
                crate::spanmap::Segment {
                    virtual_start,
                    virtual_end,
                    original_start,
                    original_end,
                    kind: if kind == 0 {
                        crate::spanmap::Kind::VERBATIM
                    } else {
                        crate::spanmap::Kind::ATOM
                    },
                    features: crate::spanmap::Feature::ALL,
                }
            },
        )
        .collect()
}

/// Rewrites JavaScript the emit workers already wrote, then composes each
/// matching `.js.map` through that rewrite. A declaration map is not
/// JavaScript and is left alone. A missing file was captured in memory by
/// the API and is rewritten there instead.
pub fn rewrite_emitted_javascript(results: &[crate::emitter::program_emit::EmitResult]) {
    let fs = crate::frontend::vfs::osvfs_fs();
    for result in results {
        for path in &result.emitted_files {
            if !is_javascript_output(path) {
                continue;
            }
            let (text, ok) = fs.read_file(path);
            if !ok {
                continue;
            }
            let next = emit_output(path, &text);
            if next.code == text {
                continue;
            }
            let _ = fs.write_file(path, &next.code);
            if next.segments.is_empty() {
                // The wrapper reprinted the calls, so the old columns would
                // land inside generated text. Drop the mappings rather than
                // leave them pointing at the wrong place.
                let map_path = format!("{path}.map");
                let (map_text, ok) = fs.read_file(&map_path);
                if ok {
                    if let Some(cleared) = clear_source_map(&map_text) {
                        let _ = fs.write_file(&map_path, &cleared);
                    }
                }
                continue;
            }
            let map_path = format!("{path}.map");
            let (map_text, ok) = fs.read_file(&map_path);
            if !ok {
                continue;
            }
            if let Some(composed) = compose_source_map(&map_text, &text, &next.code, &next.segments)
            {
                let _ = fs.write_file(&map_path, &composed);
            }
        }
    }
}

/// Rewrites one captured emit output. A `.map` file is composed through the
/// rewrite of the JavaScript it belongs to (`javascript`), which the caller
/// already rewrote. Other files, including declaration maps, pass through.
///
/// `javascript` is the text the emitter wrote, before this rewrite. The
/// caller passes it only for a `.map` file, and only after the matching
/// JavaScript has been seen, so the two agree.
#[must_use]
pub fn emit_output_file(file_name: &str, text: &str, javascript: Option<&str>) -> String {
    if is_source_map(file_name) {
        let Some(javascript) = javascript else {
            return text.to_owned();
        };
        let output = emit_output(file_name.trim_end_matches(".map"), javascript);
        if output.segments.is_empty() {
            if output.code == javascript {
                return text.to_owned();
            }
            return clear_source_map(text).unwrap_or_else(|| text.to_owned());
        }
        return compose_source_map(text, javascript, &output.code, &output.segments)
            .unwrap_or_else(|| text.to_owned());
    }
    emit_javascript(file_name, text)
}

fn clear_source_map(text: &str) -> Option<String> {
    let mut map = crate::sourcemap::generator::RawSourceMap::default();
    crate::frontend::json::json_unmarshal(text.as_bytes(), &mut map, &[]).ok()?;
    if map.version != 3 || map.mappings.is_empty() {
        return None;
    }
    map.mappings.clear();
    Some(map.to_json())
}

fn is_source_map(path: &str) -> bool {
    let path = path.rsplit(['/', '\\']).next().unwrap_or(path);
    path.ends_with(".js.map")
        || path.ends_with(".jsx.map")
        || path.ends_with(".mjs.map")
        || path.ends_with(".cjs.map")
}

/// Moves every generated position in `map_json` from the original emit text
/// into the rewritten text. The rewrite records the emitted JavaScript as
/// its original and the rewritten JavaScript as its virtual text, so a
/// generated offset maps original-to-virtual. A mapping that lands inside a
/// rewritten call keeps the column of that call. Source positions are
/// unchanged: they already name the author file. `None` when `map_json` is
/// not a source map.
fn compose_source_map(
    map_json: &str,
    original_js: &str,
    rewritten_js: &str,
    segments: &[crate::spanmap::Segment],
) -> Option<String> {
    let mut map = crate::sourcemap::generator::RawSourceMap::default();
    crate::frontend::json::json_unmarshal(map_json.as_bytes(), &mut map, &[]).ok()?;
    if map.version != 3 {
        return None;
    }
    let original_lines = crate::scanner_util::compute_ecma_line_starts(original_js);
    let mut decoder = crate::sourcemap::decode_mappings(&map.mappings);
    let decoded = decoder.values().collect::<Vec<_>>();
    if decoder.error().is_some() {
        return None;
    }
    // The generator only accepts generated positions in order. An atom maps
    // its whole original range to the start of the rewritten call, so a
    // column inside that range would move backward. It is dropped: the
    // mapping at the start of the call already names the author token.
    let mut moved_mappings = Vec::with_capacity(decoded.len());
    for mapping in decoded {
        let generated = line_character_to_offset(
            &original_lines,
            original_js,
            mapping.generated_line,
            mapping.generated_character,
        )?;
        let Some(moved) = original_offset_to_rewritten(segments, generated) else {
            continue;
        };
        let moved = operator_point(segments, moved);
        moved_mappings.push((moved, mapping));
    }
    // A hoisted function moves its mappings ahead of the code that was
    // above it. The generator takes positions in order, and one mapping per
    // position: adjacent atoms (the call name and the comma) share one, and
    // the first already names the author token.
    moved_mappings.sort_by_key(|(moved, _)| *moved);
    moved_mappings.dedup_by_key(|(moved, _)| *moved);

    let rewritten_lines = crate::scanner_util::compute_ecma_line_starts(rewritten_js);
    let mut generator = crate::sourcemap::generator::new_generator(
        &map.file,
        &map.source_root,
        "",
        crate::frontend::tspath::ComparePathsOptions::default(),
    );
    // `add_source` stores the path relative to an empty directory, which is
    // the path itself, so `sources` stays as the map wrote it.
    let mut sources = Vec::with_capacity(map.sources.len());
    for source in &map.sources {
        sources.push(generator.add_source(source));
    }
    let mut names = Vec::with_capacity(map.names.len());
    for name in &map.names {
        names.push(generator.add_name(name));
    }
    for (moved, mapping) in moved_mappings {
        let (line, column) = offset_to_line_character(&rewritten_lines, rewritten_js, moved);
        if mapping.is_source_mapping() {
            let source = *sources.get(mapping.source_index as usize)?;
            if mapping.name_index >= 0 {
                let name = *names.get(mapping.name_index as usize)?;
                generator
                    .add_named_source_mapping(
                        line,
                        column,
                        source,
                        mapping.source_line,
                        mapping.source_character,
                        name,
                    )
                    .ok()?;
            } else {
                generator
                    .add_source_mapping(
                        line,
                        column,
                        source,
                        mapping.source_line,
                        mapping.source_character,
                    )
                    .ok()?;
            }
        } else {
            generator.add_generated_mapping(line, column).ok()?;
        }
    }
    if let Some(contents) = &map.sources_content {
        for (index, content) in contents.iter().enumerate() {
            if let Some(content) = content {
                generator.set_source_content(index as i32, content).ok()?;
            }
        }
    }
    Some(generator.string())
}

/// Where `offset` of the original emit lands in the rewritten text.
///
/// A verbatim slice keeps its relative offset. An atom stands in for the
/// expression it replaced, so every offset of that expression, including
/// its end, is the start of the atom: the rewritten call. The segments are
/// not ordered by original offset (an atom's original range is only the
/// operator), so this walks them instead of using the span map's index.
/// A source-map point that lands inside one operator's generated pieces
/// is reported at the first of those pieces: the call name.
///
/// A verbatim slice is not an operator, so its column stays. The span map
/// itself still names each operand for a position query.
fn operator_point(segments: &[crate::spanmap::Segment], generated: i32) -> i32 {
    let Some(index) = segments
        .iter()
        .position(|segment| generated >= segment.virtual_start && generated < segment.virtual_end)
    else {
        return generated;
    };
    if segments[index].kind != crate::spanmap::Kind::ATOM {
        return generated;
    }
    let mut start = index;
    while start > 0
        && segments[start - 1].kind == crate::spanmap::Kind::ATOM
        && segments[start - 1].virtual_end == segments[start].virtual_start
        && segments[start - 1].original_start == segments[start].original_start
    {
        start -= 1;
    }
    segments[start].virtual_start
}

fn original_offset_to_rewritten(segments: &[crate::spanmap::Segment], offset: i32) -> Option<i32> {
    let mut fallback = None;
    let mut inside = None;
    for segment in segments {
        let covers = offset >= segment.original_start && offset < segment.original_end;
        let boundary = offset == segment.original_end;
        if segment.kind == crate::spanmap::Kind::VERBATIM {
            if covers || (boundary && fallback.is_none() && inside.is_none()) {
                inside = Some(segment.virtual_start + (offset - segment.original_start));
            }
        } else if covers && fallback.is_none() {
            // The first piece of this operator is the call name. A later
            // piece (the comma, the closing parenthesis) names the same
            // token and must not take the source-map point.
            fallback = Some(segment.virtual_start);
        }
    }
    fallback.or(inside)
}

/// A generated mapping is a line and a UTF-16 column. The span map is a
/// byte offset of the same text.
fn line_character_to_offset(
    line_starts: &[i32],
    text: &str,
    line: i32,
    character: i32,
) -> Option<i32> {
    let start = usize::try_from(*line_starts.get(line as usize)?).ok()?;
    let next = line_starts
        .get(line as usize + 1)
        .map(|&next| next as usize)
        .unwrap_or(text.len());
    let line_text = text.get(start..next.min(text.len()))?;
    let mut seen = 0i32;
    for (index, ch) in line_text.char_indices() {
        if seen >= character {
            return i32::try_from(start + index).ok();
        }
        seen += ch.len_utf16() as i32;
    }
    i32::try_from(start + line_text.len()).ok()
}

fn offset_to_line_character(line_starts: &[i32], text: &str, offset: i32) -> (i32, i32) {
    let offset = usize::try_from(offset).unwrap_or(0).min(text.len());
    let line = line_starts.partition_point(|&start| (start as usize) <= offset) as i32 - 1;
    let line = line.max(0);
    let start = line_starts.get(line as usize).copied().unwrap_or(0) as usize;
    let column = crate::scanner_util::utf16_len(text.get(start..offset).unwrap_or(""));
    (line, column)
}

fn is_javascript_output(path: &str) -> bool {
    let path = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if path.ends_with(".map")
        || path.ends_with(".d.ts")
        || path.ends_with(".d.mts")
        || path.ends_with(".d.cts")
    {
        return false;
    }
    path.ends_with(".js")
        || path.ends_with(".jsx")
        || path.ends_with(".mjs")
        || path.ends_with(".cjs")
}

/// True when this compiler still emits JavaScript for the file.
///
/// An external content mapper owns its runtime output. An internal post-pass
/// uses the same marker only for the span map, so its name is not an owner.
#[must_use]
pub fn emits_javascript(content_mapper: &str) -> bool {
    content_mapper.is_empty() || is_internal(content_mapper)
}

fn is_internal(content_mapper: &str) -> bool {
    content_mapper == crate::ext::MAPPER_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The emit rewrite turns `a + b` into `__tsover_add(a, b)` and records
    /// the whole call as one atom on the author's `+`. A map written for the
    /// original emit still has the old columns: the `+` mapping would land
    /// inside `__tsover_add`, and everything after the call would be early by
    /// the inserted length. The composed map moves the `+` to the start of
    /// the call and shifts the tail, and it keeps each source position.
    #[test]
    fn rewritten_javascript_map_keeps_the_author_column() {
        let javascript = "\"use tsover\";\nexport const value = a + b;\n";
        let output = emit_output("a.js", javascript);
        let rewritten = &output.code;
        let call = rewritten.find("__tsover_add(").expect("rewritten call");
        let plus = javascript.find(" + ").unwrap() + 1;
        let tail = javascript.rfind(';').unwrap();
        assert!(
            rewritten.len() > javascript.len(),
            "rewrite {rewritten:?} did not grow {javascript:?}"
        );

        let mut generator = crate::sourcemap::generator::new_generator(
            "a.js",
            "",
            "",
            crate::frontend::tspath::ComparePathsOptions::default(),
        );
        let source = generator.add_source("a.ts");
        // Author columns are fixed sentinels. The assertion is that the
        // composed map still reports them from the rewritten columns.
        let before = line_column_of(javascript, javascript.find("export").unwrap());
        generator
            .add_source_mapping(before.0, before.1, source, 1, 7)
            .unwrap();
        let operator = line_column_of(javascript, plus);
        generator
            .add_source_mapping(operator.0, operator.1, source, 1, 22)
            .unwrap();
        let after = line_column_of(javascript, tail);
        generator
            .add_source_mapping(after.0, after.1, source, 1, 30)
            .unwrap();
        let stale = generator.string();

        let map = emit_output_file("a.js.map", &stale, Some(javascript));
        assert_ne!(map, stale, "the rewrite must move generated columns");
        let mut decoded = crate::sourcemap::generator::RawSourceMap::default();
        crate::frontend::json::json_unmarshal(map.as_bytes(), &mut decoded, &[]).unwrap();
        assert_eq!(decoded.sources, ["a.ts"]);
        let mut mappings = crate::sourcemap::decode_mappings(&decoded.mappings);
        let mappings = mappings.values().collect::<Vec<_>>();

        let (export_line, export_column) =
            line_column_of(&rewritten, rewritten.find("export").unwrap());
        assert_eq!(
            generated_at(&mappings, 1, 7),
            (export_line, export_column),
            "text before the call does not move"
        );
        let (call_line, call_column) = line_column_of(rewritten, call);
        assert_eq!(
            generated_at(&mappings, 1, 22),
            (call_line, call_column),
            "the author's + is the start of __tsover_add, map {map}"
        );
        let (tail_line, tail_column) = line_column_of(rewritten, rewritten.rfind(';').unwrap());
        assert_eq!(
            generated_at(&mappings, 1, 30),
            (tail_line, tail_column),
            "the tail shifts by the inserted call, map {map}"
        );
    }

    /// A marked function is wrapped after the operator rewrite. The wrapper
    /// is inserted text, so the author's `+` and its operands move. They
    /// still name the same author columns: the call is not the wrapper, and
    /// `b` is not the `+`.
    #[test]
    fn wrapped_call_keeps_the_operand_columns() {
        let javascript = "function add(a, b) {\n  \"use gpu\";\n  return a + b;\n}\n";
        let output = emit_output("a.js", javascript);
        let call = output.code.rfind("__tsover_add(").expect(&output.code);
        let left = call + "__tsover_add(".len();
        let right = output.code[call..].find(", b)").expect(&output.code) + call + 2;
        let plus = javascript.find('+').unwrap();
        let author_left = javascript.rfind("a +").unwrap();
        let author_right = javascript.rfind("b;").unwrap();
        assert_eq!(
            original_of(&output.segments, call),
            plus as i32,
            "wrapper moved the operator off its token\n{}",
            output.code
        );
        assert_eq!(original_of(&output.segments, left), author_left as i32);
        assert_eq!(original_of(&output.segments, right), author_right as i32);
    }

    fn original_of(segments: &[crate::spanmap::Segment], generated: usize) -> i32 {
        let offset = i32::try_from(generated).unwrap_or(0);
        let segment = segments
            .iter()
            .find(|segment| offset >= segment.virtual_start && offset < segment.virtual_end)
            .unwrap_or_else(|| panic!("no segment at {generated}"));
        if segment.kind == crate::spanmap::Kind::VERBATIM {
            segment.original_start + (offset - segment.virtual_start)
        } else {
            segment.original_start
        }
    }

    fn generated_at(
        mappings: &[crate::sourcemap::Mapping],
        source_line: i32,
        source_column: i32,
    ) -> (i32, i32) {
        let mapping = mappings
            .iter()
            .find(|mapping| {
                mapping.is_source_mapping()
                    && mapping.source_line == source_line
                    && mapping.source_character == source_column
            })
            .unwrap_or_else(|| panic!("no mapping for {source_line}:{source_column}"));
        (mapping.generated_line, mapping.generated_character)
    }

    fn line_column_of(text: &str, offset: usize) -> (i32, i32) {
        let lines = crate::scanner_util::compute_ecma_line_starts(text);
        let offset = i32::try_from(offset).unwrap_or(0);
        offset_to_line_character(&lines, text, offset)
    }

    #[test]
    fn an_unrewritten_file_keeps_its_map() {
        let map = r#"{"version":3,"file":"a.js","sources":["a.ts"],"names":[],"mappings":"AAAA"}"#;
        let out = emit_output_file("a.js.map", map, Some("export const n = 1;\n"));
        assert_eq!(out, map);
    }
}
