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
//! Register another pass in [`apply`]. Do not special-case its name in the
//! emitter.

use crate::frontend::parser::{ParsedSourceFile, SourceFileParseOptions};

/// Run the internal rewrites. Content mapping has already happened.
///
/// Operator overloads are not rewritten here. Check and LSP type the
/// author's `+` from `Symbol.operatorPlus`. Emit rewrites the calls.
#[must_use]
pub fn apply(_opts: &SourceFileParseOptions, file: ParsedSourceFile) -> ParsedSourceFile {
    crate::ext::tsover::report_nesting(&file);
    file
}

/// Runtime wrappers for emitted JavaScript. Check does not call this.
///
/// The caller is the loading thread, after the emit workers have returned.
/// Parsing here is the same as the parse cache: one thread, then
/// `publish_parsed_files`.
#[must_use]
pub fn emit_javascript(filename: &str, code: &str) -> String {
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
    let Some(rewrite) = crate::ext::tsover::rewrite_text(filename, code) else {
        return EmittedOutput {
            code: code.to_owned(),
            segments: Vec::new(),
        };
    };
    // The operator rewrite is the only emit change that records positions.
    // TypeGPU metadata is inserted without a span map, so it cannot move
    // these segments.
    let code = crate::ext::typegpu::embed_metadata_js(filename, &rewrite.code);
    let segments = if code == rewrite.code {
        rewrite.segments
    } else {
        Vec::new()
    };
    EmittedOutput { code, segments }
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
            return text.to_owned();
        }
        return compose_source_map(text, javascript, &output.code, &output.segments)
            .unwrap_or_else(|| text.to_owned());
    }
    emit_javascript(file_name, text)
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
    let mut last_position = -1i32;
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
        if moved < last_position {
            continue;
        }
        last_position = moved;
        moved_mappings.push((moved, mapping));
    }

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
fn original_offset_to_rewritten(segments: &[crate::spanmap::Segment], offset: i32) -> Option<i32> {
    let mut fallback = None;
    for segment in segments {
        if segment.kind == crate::spanmap::Kind::VERBATIM {
            if offset >= segment.original_start && offset <= segment.original_end {
                return Some(segment.virtual_start + (offset - segment.original_start));
            }
        } else if offset >= segment.original_start && offset <= segment.original_end {
            fallback = Some(segment.virtual_start);
        }
    }
    fallback
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
        let rewritten = emit_javascript("a.js", javascript);
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
        let (call_line, call_column) = line_column_of(&rewritten, call);
        assert_eq!(
            generated_at(&mappings, 1, 22),
            (call_line, call_column),
            "the author's + is the start of __tsover_add, map {map}"
        );
        let (tail_line, tail_column) = line_column_of(&rewritten, rewritten.rfind(';').unwrap());
        assert_eq!(
            generated_at(&mappings, 1, 30),
            (tail_line, tail_column),
            "the tail shifts by the inserted call, map {map}"
        );
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
}
