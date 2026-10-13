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
    /// Upstream's `earlyPruning`. Off here: the babel visitor the captured
    /// snapshots call does not prune. The compiler emit hook prunes on its
    /// own.
    early_pruning: bool,
    /// Upstream's `forceTgpuAlias`.
    force_tgpu_alias: Option<String>,
    /// When set, skip a path the upstream default include regex would skip.
    filter_id: bool,
    /// Upstream `exclude`: a path containing this text is left unchanged.
    exclude: Option<String>,
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
            early_pruning: false,
            force_tgpu_alias: None,
            filter_id: false,
            exclude: None,
        }
    }

    #[must_use]
    pub fn build() -> Self {
        Self {
            phase: Phase::Build,
            auto_naming: true,
            early_pruning: false,
            force_tgpu_alias: None,
            filter_id: false,
            exclude: None,
        }
    }

    /// [`Self::build`] with upstream's `autoNamingEnabled` set.
    #[must_use]
    pub fn build_with_auto_naming(auto_naming: bool) -> Self {
        Self {
            phase: Phase::Build,
            auto_naming,
            early_pruning: false,
            force_tgpu_alias: None,
            filter_id: false,
            exclude: None,
        }
    }

    /// Upstream `earlyPruning`. `true` matches the unplugin default and the
    /// compiler emit hook. `false` matches the babel visitor the snapshots
    /// call, and the bun test's `earlyPruning: false`.
    #[must_use]
    pub fn early_pruning(mut self, enabled: bool) -> Self {
        self.early_pruning = enabled;
        self
    }

    /// Upstream `forceTgpuAlias`.
    #[must_use]
    pub fn force_tgpu_alias(mut self, alias: impl Into<String>) -> Self {
        self.force_tgpu_alias = Some(alias.into());
        self
    }

    /// Apply upstream's default include regex (`/\.m?[jt]sx?(?:\?.*)?$/`).
    #[must_use]
    pub fn filter_id(mut self, enabled: bool) -> Self {
        self.filter_id = enabled;
        self
    }

    /// Skip a path that contains `text`. Upstream `exclude`.
    #[must_use]
    pub fn exclude(mut self, text: impl Into<String>) -> Self {
        self.exclude = Some(text.into());
        self
    }

    /// Run the pass. `path` is only a file name for the parser; Sweetener
    /// extensions are not special here.
    #[must_use]
    pub fn transform(&self, path: &str, code: &str) -> Transformed {
        if !id_selected(path, self.filter_id, self.exclude.as_deref()) {
            return Transformed {
                code: code.to_owned(),
                diagnostics: Vec::new(),
                changed: false,
                rewrite: None,
            };
        }
        match self.phase {
            Phase::Build => build(
                path,
                code,
                self.auto_naming,
                self.early_pruning,
                self.force_tgpu_alias.as_deref(),
            ),
            Phase::Check => check(path, code),
        }
    }
}

/// Upstream `defaultOptions.include`: `/\.m?[jt]sx?(?:\?.*)?$/`.
#[must_use]
pub fn default_include_matches(id: &str) -> bool {
    let id = id.replace('\\', "/");
    let path = id.split('?').next().unwrap_or(id.as_str());
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext,
        "ts" | "tsx" | "mts" | "mtsx" | "js" | "jsx" | "mjs" | "mjsx"
    )
}

fn id_selected(path: &str, filter_id: bool, exclude: Option<&str>) -> bool {
    if exclude.is_some_and(|text| !text.is_empty() && path.contains(text)) {
        return false;
    }
    !filter_id || default_include_matches(path)
}

fn build(
    path: &str,
    code: &str,
    auto_naming: bool,
    early_pruning: bool,
    force_tgpu_alias: Option<&str>,
) -> Transformed {
    // Operators are rewritten once. `embed_metadata` parses that text and
    // slices the calls; it does not rewrite a second time. The two parses
    // each take this thread's one detached store, so the rewrite finishes
    // and drops its store before the metadata parse starts.
    let rewritten = {
        let rewrite = tsover::rewrite_text(path, code);
        rewrite.map_or_else(|| code.to_owned(), |rewrite| rewrite.code)
    };
    let (embedded, unsupported) = typegpu::embed_metadata_with(
        path,
        &rewritten,
        &typegpu::EmbedFlags {
            auto_naming,
            early_pruning,
            force_tgpu_alias,
        },
    );
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
