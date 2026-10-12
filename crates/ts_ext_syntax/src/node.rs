//! Node-API for the plugin session.
//!
//! `napi build` writes `index.js` and `index.d.ts` beside this crate.
//! The content-mapper process is
//! `ext-syntax` and does not go through this module.

use napi_derive::napi;

use crate::{SessionDiagnostic, TransformSession, Transformed};

#[napi(object, js_name = "Diagnostic")]
pub struct NodeDiagnostic {
    pub message: String,
    pub start: u32,
    pub end: u32,
}

#[napi(object, js_name = "Transformed")]
pub struct NodeTransform {
    pub code: String,
    pub extension: String,
    pub mappings: String,
    #[napi(js_name = "sourceMap")]
    pub source_map: String,
    pub diagnostics: Vec<NodeDiagnostic>,
    pub changed: bool,
}

#[napi(js_name = "TransformSession")]
pub struct NodeSession {
    inner: TransformSession,
}

#[napi]
impl NodeSession {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: TransformSession::check(),
        }
    }

    /// Build output. GPU functions are wrapped with `__TYPEGPU_META__`.
    #[napi(factory)]
    pub fn build() -> Self {
        Self {
            inner: TransformSession::build(),
        }
    }

    /// Sweetener source expands, then TypeGPU runs. Any other file runs
    /// TypeGPU only. `sourceMap` maps `code` back to the author's file.
    #[napi]
    pub fn transform(&self, path: String, code: String) -> NodeTransform {
        to_node(self.inner.transform(&path, &code))
    }
}

fn to_node(value: Transformed) -> NodeTransform {
    NodeTransform {
        code: value.code,
        extension: value.extension.to_owned(),
        mappings: value.mappings,
        source_map: value.source_map,
        diagnostics: value.diagnostics.into_iter().map(to_diagnostic).collect(),
        changed: value.changed,
    }
}

fn to_diagnostic(value: SessionDiagnostic) -> NodeDiagnostic {
    NodeDiagnostic {
        message: value.message,
        start: value.start,
        end: value.end,
    }
}
