//! PORT: ts-rust extension (no Go counterpart). A macro-expanding mapper:
//! `twice(x)` becomes `[x, x]`, and the response reports the expansion so
//! language features can say where macro output came from.

use super::prelude::*;

pub(super) struct ExpandingHandler;

impl MapperHandler for ExpandingHandler {
    fn handle_request(&self, _ctx: &Context, method: &str, params: JsonValue) -> HandlerResult {
        match method {
            contentmapper::METHOD_INITIALIZE => reply(initialize_result("mapper")),
            contentmapper::METHOD_TRANSFORM => {
                let p: TransformParams = unmarshal_params(&params)?;
                reply(TransformResult {
                    mapped_output: expand_twice(&p.content)?,
                    ..TransformResult::default()
                })
            }
            _ => Err(unexpected_method(method)),
        }
    }
}

/// The first `twice(x)` in `content`, expanded.
fn expand_twice(content: &str) -> Result<MappedOutput, GoError> {
    let Some(at) = content.find("twice(") else {
        return identity_mapped_output(content);
    };
    let open = at + "twice(".len();
    let close = open + content[open..].find(')').unwrap_or(0);
    let arg = &content[open..close];
    let expansion = format!("[{arg}, {arg}]");
    let text = format!("{}{expansion}{}", &content[..at], &content[close + 1..]);
    let at = at as i32;
    let open = open as i32;
    let close = close as i32;
    let width = expansion.len() as i32;
    let arg_len = arg.len() as i32;
    let segment = |vs: i32, ve: i32, os: i32, oe: i32, kind: spanmap::Kind| spanmap::Segment {
        virtual_start: vs,
        virtual_end: ve,
        original_start: os,
        original_end: oe,
        kind,
        features: spanmap::Feature::ALL,
    };
    let tail = content.len() as i32 - (close + 1);
    let mappings = spanmap::new(&[
        segment(0, at, 0, at, spanmap::Kind::VERBATIM),
        segment(at, at + 1, at, close + 1, spanmap::Kind::ATOM),
        segment(
            at + 1,
            at + 1 + arg_len,
            open,
            close,
            spanmap::Kind::VERBATIM,
        ),
        segment(
            at + 1 + arg_len,
            at + width,
            at,
            close + 1,
            spanmap::Kind::ATOM,
        ),
        segment(
            at + width,
            at + width + tail,
            close + 1,
            close + 1 + tail,
            spanmap::Kind::VERBATIM,
        ),
    ])
    .marshal()?;
    Ok(MappedOutput {
        text,
        extension: ".ts".to_string(),
        mappings: JsonValue(mappings),
        expansions: vec![contentmapper::ExpansionInformation {
            virtual_start: at,
            virtual_length: width,
            original_start: at,
            original_length: close + 1 - at,
            label: "macro `twice`".to_string(),
        }],
        ..MappedOutput::default()
    })
}
