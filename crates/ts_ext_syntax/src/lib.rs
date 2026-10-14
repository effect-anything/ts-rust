//! Content mapper. This crate does not expand macros itself.
//!
//! A `.sts` file is expanded by [`ts_ext_sweetener`], then
//! [`ts_ext_typegpu::TypegpuSession`] rewrites operators and, on build,
//! wraps marked functions. `tsc-ext` and the Node API stay here.
//! `tsgo` does not link this crate. It calls `ts_goport::ext` from its
//! own parse hook.
//!
//! `"use gpu"` is not a Sweetener expansion. [`expand`] leaves that text
//! alone. WGSL metadata (`__TYPEGPU_META__`) is a build post-step.

pub mod session;

pub use ts_ext_sweetener::{
    Diagnostic, ExpandRequest, Expansion, ExpansionTrace, Phase, RelatedLocation, ResourceBudget,
    Segment, SegmentKind, Severity, TokenOrigin, TraceStep, TryResult, expand, expand_traced,
    expand_with_budget, retarget_segments, sweetener_matches, unchanged,
};
pub use ts_ext_typegpu::structure;

pub mod cli;
pub mod compose;
pub mod config;
pub mod prewarm;
pub mod project;
pub mod protocol;
pub use session::{
    SessionDiagnostic, TransformSession, Transformed, unsupported_source, v3_source_map,
};

// napi-derive registers the exports outside `cfg(test)` only.
pub mod answer;
#[cfg_attr(test, allow(dead_code))]
mod node;
