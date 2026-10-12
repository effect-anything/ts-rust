//! The TypeGPU plugin session.
//!
//! The rewrite and the tinyest payload live in `ts_goport::ext`, because
//! they use the compiler's parser and `tsgo` calls them from its own parse
//! hook. This crate is the session a bundler calls when there is no `tsc`:
//! check rewrites operators, build wraps the rewritten text. It does not
//! expand Sweetener. `ts_ext_syntax` does that first, then calls
//! [`TypegpuSession`].

use ts_goport::ext::tsover::{self, Rewrite};
use ts_goport::ext::typegpu;

pub mod structure;

/// What the caller will do with the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Typecheck and LSP. Operators become calls. No `__TYPEGPU_META__`.
    #[default]
    Check,
    /// Final JavaScript input. Marked functions are wrapped for the runtime.
    Build,
}

/// A diagnostic in the input text. Offsets are UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub start: u32,
    pub end: u32,
}

/// One TypeGPU pass over text that is already TypeScript.
#[derive(Clone, Debug)]
pub struct TypegpuSession {
    phase: Phase,
    /// Upstream's `autoNamingEnabled`. The plugin defaults it on. The
    /// plugin's test helper defaults it off.
    auto_naming: bool,
}

/// Text after one TypeGPU pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transformed {
    pub code: String,
    pub diagnostics: Vec<Diagnostic>,
    pub changed: bool,
    /// Set when check rewrote operators. Segment and diagnostic offsets are
    /// bytes of the input. Build does not set it: the metadata wrapper is
    /// not a span-mapped rewrite.
    pub rewrite: Option<Rewrite>,
}

impl Default for TypegpuSession {
    fn default() -> Self {
        Self::check()
    }
}

impl TypegpuSession {
    #[must_use]
    pub fn check() -> Self {
        Self {
            phase: Phase::Check,
            auto_naming: true,
        }
    }

    #[must_use]
    pub fn build() -> Self {
        Self {
            phase: Phase::Build,
            auto_naming: true,
        }
    }

    /// [`Self::build`] with upstream's `autoNamingEnabled` set.
    #[must_use]
    pub fn build_with_auto_naming(auto_naming: bool) -> Self {
        Self {
            phase: Phase::Build,
            auto_naming,
        }
    }

    /// Run the pass. `path` is only a file name for the parser; Sweetener
    /// extensions are not special here.
    #[must_use]
    pub fn transform(&self, path: &str, code: &str) -> Transformed {
        match self.phase {
            Phase::Build => build(path, code, self.auto_naming),
            Phase::Check => check(path, code),
        }
    }
}

fn build(path: &str, code: &str, auto_naming: bool) -> Transformed {
    // Operators are rewritten once. `embed_metadata` parses that text and
    // slices the calls; it does not rewrite a second time. The two parses
    // each take this thread's one detached store, so the rewrite finishes
    // and drops its store before the metadata parse starts.
    let rewritten = {
        let rewrite = tsover::rewrite_text(path, code);
        rewrite.map_or_else(|| code.to_owned(), |rewrite| rewrite.code)
    };
    let (embedded, unsupported) = typegpu::embed_metadata_reports(path, &rewritten, auto_naming);
    let end = u32::try_from(code.len()).unwrap_or(u32::MAX);
    Transformed {
        changed: embedded != code,
        diagnostics: unsupported
            .into_iter()
            .map(|message| Diagnostic {
                message,
                start: 0,
                end,
            })
            .collect(),
        code: embedded,
        rewrite: None,
    }
}

fn check(path: &str, code: &str) -> Transformed {
    // Check and LSP keep the author's operators. The compiler types `+`
    // from `Symbol.operatorPlus`. Build emits `__tsover_*`.
    let _ = (path, code);
    Transformed {
        code: code.to_owned(),
        diagnostics: Vec::new(),
        changed: false,
        rewrite: None,
    }
}
