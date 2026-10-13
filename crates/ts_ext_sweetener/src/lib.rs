//!
//! Sweetener expansion: ordinary TypeScript plus a span map. This crate
//! does not rewrite `"use gpu"`. `ts_ext_syntax` is the content mapper: it
//! calls [`expand`], then `ts_ext_typegpu`. Cases, `cases.jsonl`, and the
//! discover/sync tools live in this crate. One directory is one program.

pub mod structure;

mod sweetener;
mod sweetener_apps;
mod sweetener_budget;
mod sweetener_context;
mod sweetener_expand;
mod sweetener_forms;
mod sweetener_hygiene;
mod sweetener_reader;
mod sweetener_scoped;
mod sweetener_validate;

/// Why a virtual span exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// Same length on both sides. Interior offsets map 1:1.
    Verbatim,
    /// The whole virtual span maps to the whole original span.
    Atom,
}

/// One mapped region. Offsets are UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub virtual_start: u32,
    pub virtual_end: u32,
    pub original_start: u32,
    pub original_end: u32,
    pub kind: SegmentKind,
}

/// How serious a diagnostic is. Upstream registers each code with one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    #[default]
    Error,
    /// The file still expands; nothing in it was lost (SWR4025).
    Warning,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// A secondary location a diagnostic points at, in the original file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelatedLocation {
    pub message: String,
    pub start: u32,
    pub end: u32,
}

/// A diagnostic in the original file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub start: u32,
    pub end: u32,
    pub severity: Severity,
    /// Upstream's `relatedOrigins`: where the reader stopped, for example.
    pub related: Vec<RelatedLocation>,
}

/// Result of a pre-check expansion.
///
/// `code` is ordinary TypeScript (or the original text when nothing applied).
/// `segments` cover the virtual text that came from the original. Gaps are
/// synthesized and have no source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    pub code: String,
    pub segments: Vec<Segment>,
    pub diagnostics: Vec<Diagnostic>,
    /// Other files this expansion read. The build hook watches them.
    pub dependencies: Vec<String>,
}

impl Expansion {
    /// Segments in the shape `ts_goport`'s `spanmap::Segment` stores:
    /// `i32` half-open offsets, kind `0` verbatim and `1` atom, features all.
    ///
    /// The language service already maps diagnostics through a `SpanMap`
    /// attached by `set_content_mapper_info`. This is that attachment's
    /// input. A `u32` that does not fit in `i32` is dropped.
    #[must_use]
    pub fn span_map_segments(&self) -> Vec<(i32, i32, i32, i32, i32)> {
        self.segments
            .iter()
            .filter_map(|segment| {
                Some((
                    i32::try_from(segment.virtual_start).ok()?,
                    i32::try_from(segment.virtual_end).ok()?,
                    i32::try_from(segment.original_start).ok()?,
                    i32::try_from(segment.original_end).ok()?,
                    match segment.kind {
                        SegmentKind::Verbatim => 0,
                        SegmentKind::Atom => 1,
                    },
                ))
            })
            .collect()
    }

    /// Map a byte offset in [`Self::code`] back to the original file.
    /// A gap (synthesized text) returns `None`.
    #[must_use]
    pub fn original_offset(&self, virtual_offset: u32) -> Option<u32> {
        for segment in &self.segments {
            if virtual_offset < segment.virtual_start || virtual_offset >= segment.virtual_end {
                continue;
            }
            return Some(match segment.kind {
                SegmentKind::Verbatim => segment
                    .original_start
                    .saturating_add(virtual_offset - segment.virtual_start),
                SegmentKind::Atom => segment.original_start,
            });
        }
        None
    }
}

/// Which front end is asking. Both get the same `code`. A build tool may
/// later run a post step that check must not see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Check,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandRequest<'a> {
    pub filename: &'a str,
    pub code: &'a str,
    pub phase: Phase,
}

/// True when `filename` is a Sweetener source. TypeGPU is not matched here.
#[must_use]
pub fn sweetener_matches(filename: &str) -> bool {
    sweetener::matches(filename)
}

/// Expand macros. Input may be extension syntax. Output `code` is
/// TypeScript, ready for the normal parse, check, LSP and emit path.
///
/// A non-macro file is unchanged, including a `.ts` file that contains
/// `"use gpu"`. That rewrite happens after this function, on `code`.
#[must_use]
pub fn expand(request: ExpandRequest<'_>) -> Expansion {
    if sweetener::matches(request.filename) {
        return sweetener::expand(request);
    }
    unchanged(request.code)
}

/// [`expand`] under explicit resource limits instead of upstream's
/// defaults. A non-macro file is unchanged.
#[must_use]
pub fn expand_with_budget(request: ExpandRequest<'_>, budget: ResourceBudget) -> Expansion {
    sweetener_budget::with_budget(budget, || expand(request))
}

pub use sweetener_budget::ResourceBudget;

#[must_use]
pub fn unchanged(code: &str) -> Expansion {
    let end = u32::try_from(code.len()).unwrap_or(u32::MAX);
    Expansion {
        code: code.to_owned(),
        segments: vec![Segment {
            virtual_start: 0,
            virtual_end: end,
            original_start: 0,
            original_end: end,
            kind: SegmentKind::Verbatim,
        }],
        diagnostics: Vec::new(),
        dependencies: Vec::new(),
    }
}
