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
#[must_use]
pub fn apply(opts: &SourceFileParseOptions, file: ParsedSourceFile) -> ParsedSourceFile {
    crate::ext::typegpu::rewrite_after_parse(opts, file)
}

/// Runtime wrappers for emitted JavaScript. Check does not call this.
#[must_use]
pub fn emit_javascript(filename: &str, code: &str) -> String {
    crate::ext::typegpu::embed_metadata_js(filename, code)
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
    content_mapper == crate::ext::typegpu::MAPPER_NAME
}
