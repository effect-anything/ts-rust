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
    let rewritten = crate::ext::tsover::rewrite_text(filename, code)
        .map(|rewrite| rewrite.code)
        .unwrap_or_else(|| code.to_owned());
    crate::ext::typegpu::embed_metadata_js(filename, &rewritten)
}

/// Rewrites JavaScript the emit workers already wrote. Source maps and
/// declaration files are left alone. A missing file was captured in memory
/// by the API and is rewritten there instead.
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
            let next = emit_javascript(path, &text);
            if next != text {
                let _ = fs.write_file(path, &next);
            }
        }
    }
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
