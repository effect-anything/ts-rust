//! Plugin pipeline. This is not the `tsc` path.
//!
//! `tsc` rewrites `"use gpu"` itself, after content mapping, and does not
//! call this session. A bundler has no `tsc`, so the plugin does both
//! steps when the file needs them:
//!
//! - Sweetener source: macro expansion,
//!   then [`ts_ext_typegpu::TypegpuSession`] on the expanded text.
//! - Any other file: that session only. Check leaves operators alone.
//!   Build rewrites them to `__tsover_*` and then wraps marked functions.

use crate::{ExpandRequest, Phase, expand, sweetener_matches};

/// What the caller will do with the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformPhase {
    /// Typecheck and LSP. No `__TYPEGPU_META__`.
    #[default]
    Check,
    /// Final JavaScript input. GPU functions are wrapped for the runtime.
    Build,
}

/// Reusable plugin transform. It holds no compiler and no tsconfig.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransformSession {
    phase: TransformPhase,
    /// Upstream's `autoNamingEnabled`. The plugin defaults it on; the plugin's
    /// test helper defaults it off, so a caller that compares against those
    /// snapshots sets it per file.
    auto_naming: bool,
    /// Upstream `earlyPruning`. Off: this session is the visitor the snapshots
    /// call. The compiler emit hook prunes on its own.
    early_pruning: bool,
    /// Upstream `forceTgpuAlias`.
    force_tgpu_alias: Option<String>,
    /// Upstream default include regex.
    filter_id: bool,
    /// Upstream `exclude`.
    exclude: Option<String>,
    /// Rewrite `.sts`/`.stsx` module specifiers to extensionless ones (on by
    /// default here; `sweetener.rewriteImports` of the tsc-ext options).
    /// Off, specifiers keep the extension the author wrote, as upstream
    /// Sweetener prints them.
    rewrite_imports: bool,
}

impl Default for TransformSession {
    fn default() -> Self {
        Self::check()
    }
}

/// Virtual TypeScript for the bundler, plus the map back to the author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transformed {
    pub code: String,
    /// `.ts` or `.tsx`.
    pub extension: &'static str,
    /// Spanmap tuples:
    /// `[virtualStart, virtualLength, originalStart, originalLength, kind]`.
    pub mappings: String,
    /// Source map from `code` back to the author's file. A later pipeline
    /// composes its own map through this one.
    pub source_map: String,
    pub diagnostics: Vec<SessionDiagnostic>,
    /// Files the expansion read (macro modules). A bundler watches them.
    pub dependencies: Vec<String>,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionDiagnostic {
    pub message: String,
    pub start: u32,
    pub end: u32,
    pub severity: crate::Severity,
    /// Secondary locations in the original file (where the reader stopped).
    pub related: Vec<crate::RelatedLocation>,
}

impl TransformSession {
    #[must_use]
    pub fn new() -> Self {
        Self::check()
    }

    #[must_use]
    pub fn check() -> Self {
        Self {
            phase: TransformPhase::Check,
            auto_naming: true,
            early_pruning: false,
            force_tgpu_alias: None,
            filter_id: false,
            exclude: None,
            rewrite_imports: true,
        }
    }

    #[must_use]
    pub fn build() -> Self {
        Self {
            phase: TransformPhase::Build,
            auto_naming: true,
            early_pruning: false,
            force_tgpu_alias: None,
            filter_id: false,
            exclude: None,
            rewrite_imports: true,
        }
    }

    /// [`Self::build`] with upstream's `autoNamingEnabled` set.
    #[must_use]
    pub fn build_with_auto_naming(auto_naming: bool) -> Self {
        Self {
            phase: TransformPhase::Build,
            auto_naming,
            early_pruning: false,
            force_tgpu_alias: None,
            filter_id: false,
            exclude: None,
            rewrite_imports: true,
        }
    }

    #[must_use]
    pub fn with_auto_naming(mut self, enabled: bool) -> Self {
        self.auto_naming = enabled;
        self
    }

    #[must_use]
    pub fn early_pruning(mut self, enabled: bool) -> Self {
        self.early_pruning = enabled;
        self
    }

    #[must_use]
    pub fn force_tgpu_alias(mut self, alias: impl Into<String>) -> Self {
        self.force_tgpu_alias = Some(alias.into());
        self
    }

    #[must_use]
    pub fn filter_id(mut self, enabled: bool) -> Self {
        self.filter_id = enabled;
        self
    }

    /// See the `rewrite_imports` field.
    #[must_use]
    pub fn rewrite_imports(mut self, enabled: bool) -> Self {
        self.rewrite_imports = enabled;
        self
    }

    #[must_use]
    pub fn exclude(mut self, text: impl Into<String>) -> Self {
        self.exclude = Some(text.into());
        self
    }

    /// The bundler pipeline under tsc-ext options: build phase (`TypeGPU`
    /// functions are wrapped with `__TYPEGPU_META__`), `TypeGPU` switches from
    /// `options.typegpu`.
    #[must_use]
    pub fn from_options(options: &crate::config::Options) -> Self {
        let session = Self::build_with_auto_naming(options.typegpu.auto_naming())
            .early_pruning(options.typegpu.early_pruning())
            .rewrite_imports(options.sweetener.rewrite_imports());
        match &options.typegpu.force_tgpu_alias {
            Some(alias) => session.force_tgpu_alias(alias.clone()),
            None => session,
        }
    }

    /// Plugin entry. See the module docs for which passes run.
    #[must_use]
    pub fn transform(&self, path: &str, code: &str) -> Transformed {
        if sweetener_matches(path) {
            return sweetener_then_typegpu(self, path, code);
        }
        typegpu_only(self, path, code)
    }
}

fn sweetener_then_typegpu(session: &TransformSession, path: &str, code: &str) -> Transformed {
    let expanded = expand(ExpandRequest {
        filename: path,
        code,
        phase: Phase::Build,
    });
    apply_typegpu(
        session,
        session.phase,
        path,
        code,
        &expanded.code,
        &expanded,
        code != expanded.code,
    )
}

fn typegpu_only(session: &TransformSession, path: &str, code: &str) -> Transformed {
    apply_typegpu(
        session,
        session.phase,
        path,
        code,
        code,
        &crate::unchanged(code),
        false,
    )
}

fn apply_typegpu(
    session: &TransformSession,
    phase: TransformPhase,
    path: &str,
    source: &str,
    code: &str,
    first: &crate::Expansion,
    already_changed: bool,
) -> Transformed {
    let extension = virtual_extension(path);
    let gpu = match phase {
        TransformPhase::Build => {
            let mut gpu =
                ts_ext_typegpu::TypegpuSession::build_with_auto_naming(session.auto_naming)
                    .early_pruning(session.early_pruning)
                    .filter_id(session.filter_id);
            if let Some(alias) = session.force_tgpu_alias.clone() {
                gpu = gpu.force_tgpu_alias(alias);
            }
            if let Some(exclude) = session.exclude.clone() {
                gpu = gpu.exclude(exclude);
            }
            gpu
        }
        TransformPhase::Check => ts_ext_typegpu::TypegpuSession::check(),
    }
    .transform(path, code);
    if phase == TransformPhase::Build {
        let output = rewrite_specifiers(session.rewrite_imports, source, &gpu.code);
        let segments = crate::retarget_segments(code, &gpu.code, &first.span_map_segments());
        let segments = crate::retarget_segments(&gpu.code, &output, &segments);
        let mappings = mappings_json(&segments);
        let source_map = v3_source_map(path, source, &output, &segments);
        let mut diagnostics = diagnostics_of(first);
        diagnostics.extend(author_diagnostics(first, gpu.diagnostics));
        return Transformed {
            changed: already_changed || gpu.changed || output != gpu.code,
            extension,
            dependencies: first.dependencies.clone(),
            source_map,
            mappings,
            diagnostics,
            code: output,
        };
    }
    let Some(rewritten) = gpu.rewrite else {
        let changed = already_changed || !first.diagnostics.is_empty() || gpu.code != code;
        let output = rewrite_specifiers(session.rewrite_imports, source, &gpu.code);
        let segments = crate::retarget_segments(code, &gpu.code, &first.span_map_segments());
        let segments = crate::retarget_segments(&gpu.code, &output, &segments);
        let mappings = mappings_json(&segments);
        let source_map = v3_source_map(path, source, &output, &segments);
        let mut diagnostics = diagnostics_of(first);
        diagnostics.extend(author_diagnostics(first, gpu.diagnostics));
        return Transformed {
            changed: changed || output != gpu.code,
            extension,
            dependencies: first.dependencies.clone(),
            source_map,
            mappings,
            diagnostics,
            code: output,
        };
    };
    let mut segments = compose(first, &rewritten);
    // Check output prepends the operator declarations. The rewrite's
    // virtual offsets do not include them, and the declarations are not
    // author text.
    let shift = gpu
        .code
        .strip_suffix(&rewritten.code)
        .map(|prefix| i32::try_from(prefix.len()).unwrap_or(0))
        .unwrap_or(0);
    for segment in &mut segments {
        segment.0 = segment.0.wrapping_add(shift);
        segment.1 = segment.1.wrapping_add(shift);
    }
    let output = rewrite_specifiers(session.rewrite_imports, source, &gpu.code);
    let segments = crate::retarget_segments(&gpu.code, &output, &segments);
    let mappings = mappings_json(&segments);
    let source_map = v3_source_map(path, source, &output, &segments);
    Transformed {
        changed: true,
        extension,
        dependencies: first.dependencies.clone(),
        source_map,
        mappings,
        diagnostics: {
            let mut diagnostics = diagnostics_of(first);
            diagnostics.extend(rewritten.diagnostics.into_iter().filter_map(|item| {
                let mapped = map_through(first, item.start, item.end)?;
                Some(SessionDiagnostic {
                    message: item.message,
                    start: mapped.start,
                    end: mapped.end,
                    severity: crate::Severity::Error,
                    related: Vec::new(),
                })
            }));
            diagnostics
        },
        code: output,
    }
}

/// Sweetener imports name the source extension. After expansion the file is
/// TypeScript, so a module specifier that names a `.sts` or `.stsx` file
/// must name the virtual extension instead. A later bundler then resolves
/// `logic.ts`, not `logic.sts`.
///
/// Only module specifiers change: `import` and `import type` declarations,
/// `export ... from` declarations, dynamic `import()` and `import.source()`
/// / `import.defer()` calls, and `import("...")` types. A string, a template
/// that is not itself the specifier, or a comment that merely contains
/// `.sts` stays. The scan follows statement and expression structure, so a
/// `.sts` inside an ordinary literal is never a specifier just because it
/// sits on the same line as an import.
fn rewrite_specifiers(rewrite: bool, source: &str, code: &str) -> String {
    // Sweetener expansion strips `.sts` from every string, specifier or
    // not. Put the author's literal back when this text still matches it
    // apart from that strip, then remove the extension from specifiers only.
    let code = restore_unstripped_literals(source, code);
    if !rewrite {
        return keep_specifiers(source, &code);
    }
    apply_specifier_spans(&code)
}

/// Sweetener's printer drops `.sts`/`.stsx` from strings. With import
/// rewriting off, every quoted literal of the author that named such a file
/// gets its extension back where the output has the stripped spelling, so
/// `import "./logic.sts"` stays as written (upstream Sweetener's output).
fn keep_specifiers(source: &str, code: &str) -> String {
    let bytes = source.as_bytes();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for ext in [".stsx", ".sts"] {
        for (at, _) in source.match_indices(ext) {
            let end = at + ext.len();
            let Some(&quote) = bytes.get(end) else {
                continue;
            };
            if quote != b'"' && quote != b'\'' {
                continue;
            }
            let Some(open) = source[..at].rfind(|ch| ch == quote as char || ch == '\n') else {
                continue;
            };
            if bytes[open] != quote {
                continue;
            }
            let quote = quote as char;
            let original = format!("{quote}{}{quote}", &source[open + 1..end]);
            let stripped = format!("{quote}{}{quote}", &source[open + 1..at]);
            if !pairs.iter().any(|(from, _)| *from == stripped) {
                pairs.push((stripped, original));
            }
        }
    }
    let mut out = code.to_owned();
    for (stripped, original) in pairs {
        if out.contains(&stripped) {
            out = out.replace(&stripped, &original);
        }
    }
    out
}

fn apply_specifier_spans(code: &str) -> String {
    let code = rewrite_from_clauses(code);
    let spans = sts_specifier_spans(&code);
    if spans.is_empty() {
        return code.to_owned();
    }
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0usize;
    for (start, end, rewritten) in spans {
        if start < cursor || end > code.len() || &code[start..end] == rewritten {
            continue;
        }
        out.push_str(&code[cursor..start]);
        out.push_str(&rewritten);
        cursor = end;
    }
    out.push_str(&code[cursor..]);
    out
}

/// Sweetener's printer drops a trailing `.sts` / `.stsx` from every
/// string. Where `code` is `source` with only those drops, put the
/// author's literal back. A later pass then strips the extension from
/// module specifiers alone, so a string or a comment keeps `.sts`.
fn restore_unstripped_literals(source: &str, code: &str) -> String {
    if source == code || !source.contains(".sts") {
        return code.to_owned();
    }
    let source_bytes = source.as_bytes();
    let code_bytes = code.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut si = 0usize;
    let mut ci = 0usize;
    while si < source_bytes.len() && ci < code_bytes.len() {
        if source_bytes[si] == code_bytes[ci] {
            out.push(source_bytes[si] as char);
            si += 1;
            ci += 1;
            continue;
        }
        let rest = &source[si..];
        let ext = [".stsx", ".sts"]
            .into_iter()
            .find(|ext| rest.starts_with(ext));
        let Some(ext) = ext else {
            return code.to_owned();
        };
        // The drop is only undone inside a quoted literal. A comment or
        // other byte that differs is not this printer's rewrite.
        if !inside_quote(source_bytes, si) {
            return code.to_owned();
        }
        out.push_str(ext);
        si += ext.len();
    }
    if si == source_bytes.len() && ci == code_bytes.len() {
        out
    } else {
        code.to_owned()
    }
}

fn inside_quote(bytes: &[u8], at: usize) -> bool {
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < at && i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && quote.is_some() {
            i += 2;
            continue;
        }
        if matches!(ch, b'"' | b'\'' | b'`') {
            quote = match quote {
                Some(open) if open == ch => None,
                Some(_) => quote,
                None => Some(ch),
            };
        }
        i += 1;
    }
    quote.is_some()
}

/// Byte ranges of quoted module specifiers whose path ends in `.sts` or
/// `.stsx`, in source order.
///
/// A specifier is the quoted literal after `from`, after `require(`, or
/// after `import(` (`import.source(` and `import.defer(` included). The
/// walk is one token scan. A string, a template, a comment, or a regexp is
/// one token, so `.sts` inside it is not a keyword and not a specifier
/// unless that token is itself the specifier.
/// `from "....sts"` and `from '....sts'`, including a newline between
/// the keyword and the string. `from:` is a property and is left alone.
fn rewrite_from_clauses(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0usize;
    let mut at = 0usize;
    while at + 4 <= bytes.len() {
        if &bytes[at..at + 4] == b"from"
            && (at == 0 || !is_ident_cont(bytes[at - 1]))
            && !is_ident_cont(bytes.get(at + 4).copied().unwrap_or(b' '))
        {
            let mut next = at + 4;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if matches!(bytes.get(next), Some(b'"' | b'\'')) {
                let end = skip_quoted(bytes, next);
                if let Some(rewritten) = strip_sts_extension(&code[next..end]) {
                    out.push_str(&code[cursor..next]);
                    out.push_str(&rewritten);
                    cursor = end;
                    at = end;
                    continue;
                }
            }
        }
        at += 1;
    }
    if cursor == 0 {
        return code.to_owned();
    }
    out.push_str(&code[cursor..]);
    out
}

fn sts_specifier_spans(code: &str) -> Vec<(usize, usize, String)> {
    let bytes = code.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0usize;
    let mut expect = Expect::None;
    while at < bytes.len() {
        let ch = bytes[at];
        if ch.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if ch == b'/' && bytes.get(at + 1) == Some(&b'/') {
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if ch == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                at += 1;
            }
            at = (at + 2).min(bytes.len());
            continue;
        }
        if ch == b'/' && slash_starts_regexp(bytes, at) {
            at = skip_regexp(bytes, at);
            expect = Expect::None;
            continue;
        }
        if matches!(ch, b'"' | b'\'') {
            let start = at;
            at = skip_quoted(bytes, at);
            if expect == Expect::Specifier {
                push_sts(&code[start..at], start, at, &mut spans);
            }
            expect = Expect::None;
            continue;
        }
        if ch == b'`' {
            at = scan_template(code, bytes, at, expect == Expect::Specifier, &mut spans);
            expect = Expect::None;
            continue;
        }
        if is_ident_start(ch) {
            let start = at;
            at += 1;
            while at < bytes.len() && is_ident_cont(bytes[at]) {
                at += 1;
            }
            let word = &code[start..at];
            expect = if expect == Expect::Import {
                // `import.source` and `import.defer` are still the call.
                Expect::Import
            } else {
                match word {
                    "import" => Expect::Import,
                    "require" => Expect::Require,
                    "from" => Expect::From,
                    _ => Expect::None,
                }
            };
            continue;
        }
        match (expect, ch) {
            (Expect::Import | Expect::Require, b'(') => expect = Expect::Specifier,
            (Expect::From, b'"' | b'\'' | b'`') => {
                let start = at;
                at = if ch == b'`' {
                    scan_template(code, bytes, at, true, &mut spans)
                } else {
                    let end = skip_quoted(bytes, at);
                    push_sts(&code[start..end], start, end, &mut spans);
                    end
                };
                expect = Expect::None;
                continue;
            }
            (Expect::Import, b'.') => {}
            (Expect::Import, b'<') => {
                at = skip_type_args(bytes, at);
                expect = Expect::Import;
                continue;
            }
            // `from:` is a property. `:` is not trivia between `from` and
            // its string, so the word was not a clause.
            (Expect::From, b':') => expect = Expect::None,
            (_, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b',' | b';') => {}
            _ => {
                expect = Expect::None;
            }
        }
        at += 1;
    }
    spans
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    None,
    /// `import` or `import.source` / `import.defer`, waiting for `(`.
    Import,
    /// `require`, waiting for `(`.
    Require,
    /// `from`, waiting for a string. `from:` is a property, not a clause.
    From,
    /// The next quoted literal is a module specifier.
    Specifier,
}

fn push_sts(literal: &str, start: usize, end: usize, spans: &mut Vec<(usize, usize, String)>) {
    if let Some(rewritten) = strip_sts_extension(literal) {
        spans.push((start, end, rewritten));
    }
}

fn scan_template(
    code: &str,
    bytes: &[u8],
    mut at: usize,
    is_specifier: bool,
    spans: &mut Vec<(usize, usize, String)>,
) -> usize {
    let start = at;
    at += 1;
    while at < bytes.len() {
        let ch = bytes[at];
        if ch == b'\\' {
            at = (at + 2).min(bytes.len());
            continue;
        }
        if ch == b'`' {
            at += 1;
            if is_specifier {
                push_sts(&code[start..at], start, at, spans);
            }
            return at;
        }
        if ch == b'$' && bytes.get(at + 1) == Some(&b'{') {
            // A template with a substitution is not a specifier literal.
            at += 2;
            at = scan_nested(code, bytes, at, spans);
            continue;
        }
        at += 1;
    }
    at
}

/// Tokens inside `${ ... }`, stopped at the `}` that closes this hole.
fn scan_nested(
    code: &str,
    bytes: &[u8],
    mut at: usize,
    spans: &mut Vec<(usize, usize, String)>,
) -> usize {
    let mut depth = 1i32;
    let mut expect = Expect::None;
    while at < bytes.len() && depth > 0 {
        let ch = bytes[at];
        if ch.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if matches!(ch, b'"' | b'\'') {
            let start = at;
            at = skip_quoted(bytes, at);
            if depth == 1 && expect == Expect::Specifier {
                push_sts(&code[start..at], start, at, spans);
            }
            expect = Expect::None;
            continue;
        }
        if ch == b'`' {
            at = scan_template(
                code,
                bytes,
                at,
                depth == 1 && expect == Expect::Specifier,
                spans,
            );
            expect = Expect::None;
            continue;
        }
        if ch == b'{' {
            depth += 1;
            at += 1;
            continue;
        }
        if ch == b'}' {
            depth -= 1;
            at += 1;
            if depth == 0 {
                break;
            }
            continue;
        }
        if depth == 1 && is_ident_start(ch) {
            let start = at;
            at += 1;
            while at < bytes.len() && is_ident_cont(bytes[at]) {
                at += 1;
            }
            let word = &code[start..at];
            expect = if expect == Expect::Import {
                Expect::Import
            } else {
                match word {
                    "import" => Expect::Import,
                    "require" => Expect::Require,
                    "from" => Expect::From,
                    _ => Expect::None,
                }
            };
            continue;
        }
        if depth == 1 {
            match (expect, ch) {
                (Expect::Import | Expect::Require, b'(') => expect = Expect::Specifier,
                (Expect::From, b'"' | b'\'' | b'`') => {
                    let start = at;
                    at = if ch == b'`' {
                        scan_template(code, bytes, at, true, spans)
                    } else {
                        let end = skip_quoted(bytes, at);
                        push_sts(&code[start..end], start, end, spans);
                        end
                    };
                    expect = Expect::None;
                    continue;
                }
                (Expect::Import, b'.') => {}
                (_, b'(' | b')' | b'[' | b']' | b',' | b';' | b':') => {}
                _ => expect = Expect::None,
            }
        }
        at += 1;
    }
    at
}

fn skip_quoted(bytes: &[u8], mut at: usize) -> usize {
    let quote = bytes[at];
    at += 1;
    while at < bytes.len() {
        let ch = bytes[at];
        at += 1;
        if ch == b'\\' {
            if at < bytes.len() {
                at += 1;
            }
            continue;
        }
        if ch == quote {
            break;
        }
    }
    at
}

fn skip_type_args(bytes: &[u8], mut at: usize) -> usize {
    let mut depth = 0i32;
    while at < bytes.len() {
        let ch = bytes[at];
        if matches!(ch, b'"' | b'\'' | b'`') {
            at = skip_quoted(bytes, at);
            continue;
        }
        if ch == b'<' {
            depth += 1;
        } else if ch == b'>' {
            depth -= 1;
            at += 1;
            if depth == 0 {
                break;
            }
            continue;
        }
        at += 1;
    }
    at
}

/// `/` after a value is division. Anywhere else it opens a regexp.
fn slash_starts_regexp(bytes: &[u8], at: usize) -> bool {
    let mut prev = at;
    while prev > 0 && bytes[prev - 1].is_ascii_whitespace() {
        prev -= 1;
    }
    if prev == 0 {
        return true;
    }
    let ch = bytes[prev - 1];
    !(ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'$' | b')' | b']' | b'}'))
}

fn skip_regexp(bytes: &[u8], mut at: usize) -> usize {
    at += 1;
    let mut class = false;
    while at < bytes.len() {
        let ch = bytes[at];
        if ch == b'\\' {
            at = (at + 2).min(bytes.len());
            continue;
        }
        at += 1;
        if ch == b'[' {
            class = true;
        } else if ch == b']' {
            class = false;
        } else if ch == b'/' && !class {
            while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                at += 1;
            }
            break;
        } else if ch == b'\n' {
            break;
        }
    }
    at
}

fn is_ident_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, b'_' | b'$')
}

fn is_ident_cont(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'$')
}

/// The quoted specifier with a trailing `.sts` or `.stsx` removed, query
/// and hash included. `None` when this literal does not name such a file.
fn strip_sts_extension(literal: &str) -> Option<String> {
    let bytes = literal.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = *bytes.first()?;
    if quote != b'"' && quote != b'\'' && quote != b'`' {
        return None;
    }
    if *bytes.last()? != quote {
        return None;
    }
    let body = &literal[1..literal.len() - 1];
    let (path, suffix) = split_specifier_suffix(body);
    let stripped = path
        .strip_suffix(".stsx")
        .or_else(|| path.strip_suffix(".sts"))?;
    if stripped.is_empty() || stripped.ends_with('.') {
        return None;
    }
    Some(format!("{q}{stripped}{suffix}{q}", q = quote as char))
}

/// `file.sts?x=1` and `file.sts#hash` keep the suffix; the extension is
/// the end of the path.
fn split_specifier_suffix(body: &str) -> (&str, &str) {
    match body.find(['?', '#']) {
        Some(at) => (&body[..at], &body[at..]),
        None => (body, ""),
    }
}

fn diagnostics_of(expansion: &crate::Expansion) -> Vec<SessionDiagnostic> {
    expansion
        .diagnostics
        .iter()
        .map(|item| SessionDiagnostic {
            message: item.message.clone(),
            start: item.start,
            end: item.end,
            severity: item.severity,
            related: item.related.clone(),
        })
        .collect()
}

fn compose(
    first: &crate::Expansion,
    second: &ts_goport::ext::tsover::Rewrite,
) -> Vec<(i32, i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for segment in &second.segments {
        if segment.kind == ts_goport::spanmap::Kind::VERBATIM {
            split_verbatim(first, segment, &mut out);
        } else if let Some(mapped) = map_through(
            first,
            segment.original_start as u32,
            segment.original_end as u32,
        ) {
            out.push((
                segment.virtual_start,
                segment.virtual_end,
                i32::try_from(mapped.start).unwrap_or(0),
                i32::try_from(mapped.end).unwrap_or(0),
                1,
            ));
        }
    }
    out
}

fn split_verbatim(
    first: &crate::Expansion,
    segment: &ts_goport::spanmap::Segment,
    out: &mut Vec<(i32, i32, i32, i32, i32)>,
) {
    let origin = segment.original_start as u32;
    let end = segment.original_end as u32;
    let mut cursor = origin;
    for prev in &first.segments {
        if prev.virtual_end <= cursor {
            continue;
        }
        if prev.virtual_start >= end {
            break;
        }
        if prev.virtual_start > cursor {
            cursor = prev.virtual_start;
        }
        let from = cursor.max(prev.virtual_start);
        let to = end.min(prev.virtual_end);
        if to <= from {
            continue;
        }
        let shift = i32::try_from(from - origin).unwrap_or(0);
        let len = i32::try_from(to - from).unwrap_or(0);
        let (original_start, original_end, kind) = match prev.kind {
            crate::SegmentKind::Verbatim => {
                let start = prev.original_start + (from - prev.virtual_start);
                (start, start + (to - from), 0)
            }
            crate::SegmentKind::Atom => (prev.original_start, prev.original_end, 1),
        };
        out.push((
            segment.virtual_start.wrapping_add(shift),
            segment.virtual_start.wrapping_add(shift).wrapping_add(len),
            i32::try_from(original_start).unwrap_or(0),
            i32::try_from(original_end).unwrap_or(0),
            kind,
        ));
        cursor = to;
        if cursor >= end {
            break;
        }
    }
}

struct MappedRange {
    start: u32,
    end: u32,
}

fn author_diagnostics(
    first: &crate::Expansion,
    diagnostics: Vec<ts_ext_typegpu::Diagnostic>,
) -> Vec<SessionDiagnostic> {
    diagnostics
        .into_iter()
        .map(|item| {
            let (start, end) = map_through(first, item.start, item.end)
                .map(|range| (range.start, range.end))
                .unwrap_or((item.start, item.end));
            SessionDiagnostic {
                message: item.message,
                start,
                end,
                severity: crate::Severity::Error,
                related: Vec::new(),
            }
        })
        .collect()
}

/// Map a span of the first expansion's virtual text back to the author.
/// A macro atom contributes its whole author span. A gap contributes nothing.
fn map_through(first: &crate::Expansion, start: u32, end: u32) -> Option<MappedRange> {
    if end < start {
        return None;
    }
    if start == end {
        let segment = segment_at(first, start)?;
        return Some(match segment.kind {
            crate::SegmentKind::Verbatim => {
                let at = segment
                    .original_start
                    .saturating_add(start - segment.virtual_start);
                MappedRange { start: at, end: at }
            }
            crate::SegmentKind::Atom => MappedRange {
                start: segment.original_start,
                end: segment.original_end,
            },
        });
    }
    let start_at = segment_at(first, start)?;
    let end_at = segment_at(first, end - 1)?;
    if std::ptr::eq(start_at, end_at) {
        return Some(match start_at.kind {
            crate::SegmentKind::Verbatim => MappedRange {
                start: start_at
                    .original_start
                    .saturating_add(start - start_at.virtual_start),
                end: start_at
                    .original_start
                    .saturating_add(end - start_at.virtual_start),
            },
            crate::SegmentKind::Atom => MappedRange {
                start: start_at.original_start,
                end: start_at.original_end,
            },
        });
    }
    // A range across segments is the union of what each covered segment
    // maps to: macro output can put operands in a different order than the
    // author wrote them, so the two endpoints alone may cover too little.
    let mut low = u32::MAX;
    let mut high = 0;
    for segment in &first.segments {
        let lo = start.max(segment.virtual_start);
        let hi = end.min(segment.virtual_end);
        if lo >= hi {
            continue;
        }
        let (s, e) = match segment.kind {
            crate::SegmentKind::Verbatim => (
                segment
                    .original_start
                    .saturating_add(lo - segment.virtual_start),
                segment
                    .original_start
                    .saturating_add(hi - segment.virtual_start),
            ),
            crate::SegmentKind::Atom => (segment.original_start, segment.original_end),
        };
        low = low.min(s);
        high = high.max(e);
    }
    (low <= high).then_some(MappedRange {
        start: low,
        end: high,
    })
}

fn segment_at(first: &crate::Expansion, pos: u32) -> Option<&crate::Segment> {
    first.segments.iter().find(|segment| {
        pos >= segment.virtual_start && pos < segment.virtual_end
            || (pos == segment.virtual_end
                && first
                    .segments
                    .last()
                    .is_some_and(|last| pos == last.virtual_end))
    })
}

/// The error for a file kind tsc-ext does not take: `.sjs`/`.sjsx`
/// (Sweetener over JavaScript) are not supported. `None` for any other path.
#[must_use]
pub fn unsupported_source(path: &str) -> Option<String> {
    let bare = path.split(['?', '#']).next().unwrap_or(path);
    [".sjs", ".sjsx"]
        .iter()
        .find(|ext| bare.ends_with(*ext))
        .map(|ext| {
            format!(
                "{path}: {ext} (Sweetener over JavaScript) is not supported; write .sts or .stsx (TypeScript)"
            )
        })
}

pub fn virtual_extension(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext {
        "tsx" | "stsx" => ".tsx",
        _ => ".ts",
    }
}

/// A source map whose segments are the spanmap tuples. The next tool
/// reads `segments` and composes them; it does not replace them.
pub fn source_map_json(file: &str, mappings: &str) -> String {
    format!(
        r#"{{"version":3,"file":{},"sources":[{}],"segments":{mappings}}}"#,
        json_string(file),
        json_string(file),
    )
}

/// A Source Map v3 from `output` back to `source`, with real VLQ
/// `mappings` so any consumer (bundlers, devtools, `source-map`) can read
/// it. A verbatim range maps every token start, plus every line start
/// inside it, to its exact original position. An atom maps its start (and
/// its line starts) to the start of the original range that produced it.
/// The raw spanmap tuples stay under `segments` for composing tools.
#[must_use]
pub fn v3_source_map(
    file: &str,
    source: &str,
    output: &str,
    segments: &[(i32, i32, i32, i32, i32)],
) -> String {
    let segment_json = mappings_json(segments);
    let source_lines = line_starts(source);
    let output_lines = line_starts(output);
    let position = |lines: &[usize], offset: usize| -> (usize, usize) {
        let line = lines
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        (line, offset - lines[line])
    };
    // (output offset, original offset), sorted by output.
    let mut points: Vec<(usize, usize)> = Vec::new();
    for &(vs, ve, os, _, kind) in segments {
        let (Ok(vs), Ok(ve), Ok(os)) = (
            usize::try_from(vs),
            usize::try_from(ve),
            usize::try_from(os),
        ) else {
            continue;
        };
        if ve > output.len() || vs >= ve {
            continue;
        }
        let bytes = output.as_bytes();
        let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$';
        for at in vs..ve {
            let byte = bytes[at];
            if byte.is_ascii_whitespace() || !output.is_char_boundary(at) {
                continue;
            }
            let starts_line = at == 0 || bytes[at - 1] == b'\n';
            let starts_token = at == vs || !(is_word(byte) && is_word(bytes[at - 1]));
            let point = if kind == 0 {
                starts_token || starts_line
            } else {
                at == vs || starts_line
            };
            if !point {
                continue;
            }
            let original = if kind == 0 { os + (at - vs) } else { os };
            if original <= source.len() {
                points.push((at, original));
            }
        }
    }
    points.sort_unstable();
    points.dedup_by_key(|point| point.0);
    let mut mappings = String::new();
    let mut line = 0usize;
    let (mut prev_col, mut prev_src_line, mut prev_src_col) = (0i64, 0i64, 0i64);
    let mut first_on_line = true;
    for (out_at, original) in points {
        let (out_line, _) = position(&output_lines, out_at);
        while line < out_line {
            mappings.push(';');
            line += 1;
            prev_col = 0;
            first_on_line = true;
        }
        if !first_on_line {
            mappings.push(',');
        }
        first_on_line = false;
        let (src_line, _) = position(&source_lines, original);
        let col = utf16_column(output, output_lines[out_line], out_at);
        let src_col_utf16 = utf16_column(source, source_lines[src_line], original);
        vlq(&mut mappings, col - prev_col);
        vlq(&mut mappings, 0);
        vlq(&mut mappings, src_line as i64 - prev_src_line);
        vlq(&mut mappings, src_col_utf16 - prev_src_col);
        prev_col = col;
        prev_src_line = src_line as i64;
        prev_src_col = src_col_utf16;
    }
    format!(
        r#"{{"version":3,"file":{},"sources":[{}],"sourcesContent":[{}],"names":[],"mappings":"{mappings}","segments":{segment_json}}}"#,
        json_string(file),
        json_string(file),
        json_string(source),
    )
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    starts
}

/// Source map columns count UTF-16 code units.
fn utf16_column(text: &str, line_start: usize, at: usize) -> i64 {
    let mut end = at.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[line_start..end].encode_utf16().count() as i64
}

fn vlq(out: &mut String, value: i64) {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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
        out.push(DIGITS[digit as usize] as char);
        if rest == 0 {
            break;
        }
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub fn mappings_json(segments: &[(i32, i32, i32, i32, i32)]) -> String {
    let mut out = String::from("[");
    for (index, (v0, v1, o0, o1, kind)) in segments.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "[{v0},{},{o0},{},{kind}]",
            v1.wrapping_sub(*v0),
            o1.wrapping_sub(*o0),
        ));
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::{TransformSession, compose, map_through};

    #[test]
    fn import_specifiers_keep_the_extension_unless_rewriting() {
        let code = "import { b } from \"./b.sts\";\nexport syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\nexport const a = twice(b);\n";
        let kept = TransformSession::from_options(&crate::config::Options::default())
            .transform("/p/a.sts", code);
        assert!(kept.code.contains("from \"./b.sts\""), "{}", kept.code);
        assert!(kept.code.contains("[b, b]"), "{}", kept.code);
        let mut options = crate::config::Options::default();
        options.sweetener.rewrite_imports = Some(true);
        let rewritten = TransformSession::from_options(&options).transform("/p/a.sts", code);
        assert!(
            rewritten.code.contains("from \"./b\""),
            "{}",
            rewritten.code
        );
    }

    #[test]
    fn ranges_across_reordered_segments_cover_every_operand() {
        let verbatim = |vs, ve, os| crate::Segment {
            virtual_start: vs,
            virtual_end: ve,
            original_start: os,
            original_end: os + (ve - vs),
            kind: crate::SegmentKind::Verbatim,
        };
        // `[b, a]` printed from `pair(a, b)` (a at 5, b at 8).
        let first = crate::Expansion {
            segments: vec![verbatim(1, 2, 8), verbatim(4, 5, 5)],
            ..Default::default()
        };
        let mapped = map_through(&first, 1, 5).unwrap();
        assert_eq!((mapped.start, mapped.end), (5, 9));
    }

    #[test]
    fn ordinary_file_runs_typegpu_only() {
        let code = "function f(a, b) { \"use gpu\"; return a + b }";
        let out = TransformSession::new().transform("a.ts", code);
        assert!(!out.changed);
        assert_eq!(out.code, code);
        assert!(!out.code.contains("__tsover_add"), "{}", out.code);
    }

    #[test]
    fn check_keeps_the_operator_and_build_emits_the_call() {
        let code = "function add(a, b) { \"use gpu\"; return a + b }";
        let check = TransformSession::check().transform("a.ts", code);
        assert_eq!(check.code, code);
        assert!(!check.code.contains("__TYPEGPU_META__"), "{}", check.code);
        let build = TransformSession::build().transform("a.ts", code);
        assert!(build.code.contains("__tsover_add"), "{}", build.code);
        assert!(build.code.contains("__TYPEGPU_META__"), "{}", build.code);
        assert!(build.code.contains("v: 2"), "{}", build.code);
    }

    #[test]
    fn plain_ts_is_unchanged() {
        let code = "export const n = 1\n";
        let out = TransformSession::new().transform("a.ts", code);
        assert!(!out.changed);
        assert_eq!(out.code, code);
    }

    #[test]
    fn sweetener_file_takes_the_macro_path() {
        let out = TransformSession::new().transform("a.sts", "export const n = 1\n");
        assert_eq!(out.extension, ".ts");
    }

    #[test]
    fn check_keeps_the_author_operator_in_place() {
        let code = "function step(n: number) {\n  \"use gpu\";\n  return n + 1;\n}\n";
        let out = TransformSession::check().transform("a.ts", code);
        assert_eq!(out.code, code);
        let plus = code.find('+').expect("plus") as i32;
        let covering = tuples(&out.mappings)
            .into_iter()
            .find(|item| item[0] <= plus && plus < item[0] + item[1])
            .expect("segment");
        assert_eq!(covering[4], 0, "check does not rewrite the operator");
        assert_eq!(covering[2] + (plus - covering[0]), plus);
        let name = out.code.find("function step").expect("name") as i32;
        let name_seg = tuples(&out.mappings)
            .into_iter()
            .find(|item| item[0] <= name && name < item[0] + item[1])
            .expect("name segment");
        assert_eq!(name_seg[4], 0);
        let from = name_seg[2] as usize;
        let to = from + name_seg[3] as usize;
        let virtual_from = name_seg[0] as usize;
        let virtual_to = virtual_from + name_seg[1] as usize;
        assert_eq!(&code[from..to], &out.code[virtual_from..virtual_to]);
    }

    #[test]
    fn macro_atom_composes_onto_the_author_span() {
        use crate::{Expansion, Segment, SegmentKind};
        let head = "function step(n: number) {\n  \"use gpu\";\n  ";
        let author_macro = "unless (n < 0) { return n + 1 }";
        let virtual_macro = "if (!(n < 0)) { return n + 1 }";
        let tail = "\n  return kept;\n}\n";
        let author = format!("{head}{author_macro}{tail}");
        let virtual_text = format!("{head}{virtual_macro}{tail}");
        let head_len = head.len() as u32;
        let virtual_end = head_len + virtual_macro.len() as u32;
        let author_end = head_len + author_macro.len() as u32;
        let expansion = Expansion {
            code: virtual_text.clone(),
            segments: vec![
                Segment {
                    virtual_start: 0,
                    virtual_end: head_len,
                    original_start: 0,
                    original_end: head_len,
                    kind: SegmentKind::Verbatim,
                },
                Segment {
                    virtual_start: head_len,
                    virtual_end: virtual_end,
                    original_start: head_len,
                    original_end: author_end,
                    kind: SegmentKind::Atom,
                },
                Segment {
                    virtual_start: virtual_end,
                    virtual_end: virtual_text.len() as u32,
                    original_start: author_end,
                    original_end: author.len() as u32,
                    kind: SegmentKind::Verbatim,
                },
            ],
            diagnostics: Vec::new(),
            dependencies: Vec::new(),
            ..Default::default()
        };
        let rewritten = ts_goport::ext::tsover::rewrite_text("a.ts", &virtual_text).unwrap();
        let segments = compose(&expansion, &rewritten);
        let call = rewritten.code.find("__tsover_add").unwrap() as i32;
        let covering = segments
            .iter()
            .find(|item| item.0 <= call && call < item.1)
            .unwrap();
        assert_eq!(covering.4, 1);
        assert_eq!(covering.2, head_len as i32);
        assert_eq!(covering.3, author_end as i32);
        assert_eq!(
            &author[covering.2 as usize..covering.3 as usize],
            author_macro
        );
        let kept = rewritten.code.find("return kept").unwrap() as i32;
        let kept_seg = segments
            .iter()
            .find(|item| item.0 <= kept && kept < item.1)
            .unwrap();
        assert_eq!(kept_seg.4, 0);
        assert_eq!(
            &author[kept_seg.2 as usize..kept_seg.3 as usize],
            &rewritten.code[kept_seg.0 as usize..kept_seg.1 as usize]
        );
    }

    #[test]
    fn sts_specifiers_rewrite_imports_only() {
        let code = concat!(
            "import { a } from \"./logic.sts\";\n",
            "import type { T } from './types.stsx';\n",
            "import value = require(\"./value.sts\");\n",
            "export { b } from \"./panel.stsx\";\n",
            "export * from \"./all.sts\";\n",
            "const loaded = import(\"./dyn.sts\");\n",
            "const phase = import.source(\"./source.stsx\");\n",
            "type Box = import(\"./box.sts\").Box;\n",
            "const text = \"keep ./note.sts in a string\";\n",
            "const tick = `keep ./note.sts in a template`;\n",
            "const mixed = `head ${import(\"./inner.sts\")} tail ./note.sts`;\n",
            "// comment ./comment.sts must stay\n",
            "const query = import(\"./q.sts?x=1\");\n",
            "const plain = import(\"./already.ts\");\n",
            "const mid = \"not.sts.a.module\";\n",
            "const obj = { from: \"./obj.sts\", note: \"./note.sts\" };\n",
            "function nested() { import { z } from \"./inside.sts\"; return import(\"./also.sts\"); }\n",
            "const arrow = () => { import { y } from \"./arrow.sts\"; };\n",
        );
        let mut out = TransformSession::check().transform("main.sts", code);
        let has = |needle: &str| {
            assert!(
                out.code.contains(needle),
                "{needle} missing in {}",
                out.code
            )
        };
        has("from \"./logic\"");
        has("from './types'");
        has("require(\"./value\")");
        has("from \"./panel\"");
        has("from \"./all\"");
        has("import(\"./dyn\")");
        has("import.source(\"./source\")");
        has("import(\"./box\").Box");
        has("import(\"./inner\")");
        has("import(\"./q?x=1\")");
        has("import(\"./already.ts\")");
        has("\"keep ./note.sts in a string\"");
        has("`keep ./note.sts in a template`");
        has("tail ./note.sts");
        has("// comment ./comment.sts must stay");
        has("\"not.sts.a.module\"");
        has("from \"./inside\"");
        has("import(\"./also\")");
        has("from \"./arrow\"");
        let obj = out.code.find("const obj").expect("obj");
        let line = &out.code[obj..out.code[obj..]
            .find('\n')
            .map(|n| obj + n)
            .unwrap_or(out.code.len())];
        assert_eq!(
            line, "const obj = { from: \"./obj.sts\", note: \"./note.sts\" };",
            "{line:?}"
        );
        has("./note.sts");
        has("not.sts.a.module");
        // Verbatim pieces still copy the author's bytes. The shortened
        // specifier is an atom over the original literal.
        for [vs, vl, os, ol, kind] in tuples(&out.mappings) {
            let (vs, vl, os, ol) = (vs as usize, vl as usize, os as usize, ol as usize);
            if kind == 0 {
                assert_eq!(&out.code[vs..vs + vl], &code[os..os + ol]);
            }
        }
        let logic = out.code.find("./logic\"").unwrap();
        let covering = tuples(&out.mappings)
            .into_iter()
            .find(|item| item[0] as usize <= logic && logic < item[0] as usize + item[1] as usize)
            .expect("logic specifier segment");
        assert_eq!(covering[4], 1, "rewritten specifier is an atom");
        let original = &code[covering[2] as usize..covering[2] as usize + covering[3] as usize];
        assert!(original.contains("./logic.sts"), "{original}");
        // Text after the shorter specifier still maps to itself.
        let comment = out.code.find("comment ./comment.sts").unwrap();
        let comment_seg = tuples(&out.mappings)
            .into_iter()
            .find(|item| {
                item[0] as usize <= comment && comment < item[0] as usize + item[1] as usize
            })
            .expect("comment segment");
        assert_eq!(comment_seg[4], 0);
        assert_eq!(
            &out.code[comment_seg[0] as usize..comment_seg[0] as usize + comment_seg[1] as usize],
            &code[comment_seg[2] as usize..comment_seg[2] as usize + comment_seg[3] as usize]
        );

        // The same rewrite on a file that is not Sweetener source.
        out = TransformSession::check().transform("main.ts", code);
        assert!(out.code.contains("from \"./logic\""), "{}", out.code);
        assert!(
            out.code.contains("\"keep ./note.sts in a string\""),
            "{}",
            out.code
        );
    }

    fn tuples(mappings: &str) -> Vec<[i32; 5]> {
        let mut out = Vec::new();
        let mut number = String::new();
        let mut values = Vec::new();
        for ch in mappings.chars() {
            if ch.is_ascii_digit() || ch == '-' {
                number.push(ch);
                continue;
            }
            if !number.is_empty() {
                values.push(number.parse::<i32>().unwrap_or(0));
                number.clear();
            }
            if ch == ']' && values.len() == 5 {
                out.push([values[0], values[1], values[2], values[3], values[4]]);
                values.clear();
            }
        }
        out
    }
}
