//! Content mapper. This crate does not expand macros itself.
//!
//! A `.sts` file is expanded by [`ts_ext_sweetener`], then
//! [`ts_ext_typegpu::TypegpuSession`] rewrites operators and, on build,
//! wraps marked functions. `ext-syntax` and the Node API stay here.
//! `tsgo` does not link this crate. It calls `ts_goport::ext` from its
//! own parse hook.
//!
//! `"use gpu"` is not a Sweetener expansion. [`expand`] leaves that text
//! alone. WGSL metadata (`__TYPEGPU_META__`) is a build post-step.

pub mod session;

pub use ts_ext_sweetener::{
    Diagnostic, ExpandRequest, Expansion, Phase, RelatedLocation, Segment, SegmentKind, Severity,
    expand, sweetener_matches, unchanged,
};
pub use ts_ext_typegpu::structure;

pub mod protocol;
pub use session::{SessionDiagnostic, TransformSession, Transformed};

mod node;
