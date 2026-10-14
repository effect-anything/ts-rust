//! Node-API binding of the `Transformer` (js/index.cjs).
//!
//! The JavaScript wrapper owns the public shape (class, option merging,
//! `composeMap`, `formatDiagnostics`); this module does the work and answers
//! in JSON, so the wrapper can be renamed or reshaped without touching Rust.
//! The content mapper is the `tsc-ext` binary and does not go through here.

use napi_derive::napi;
use serde_json::{Value, json};

use crate::config::{self, Options};
use crate::{SessionDiagnostic, TransformSession};

fn error(message: impl Into<String>) -> napi::Error {
    napi::Error::from_reason(message.into())
}

/// Options for `path`: the tsconfig's (`tsconfig`, else the nearest one when
/// `search` is set), with `explicit` (an options object as JSON) over them.
fn options_for(
    path: &str,
    tsconfig: Option<&str>,
    search: bool,
    explicit: Option<&str>,
) -> napi::Result<(Options, Option<String>)> {
    let explicit = match explicit {
        Some(text) => {
            let value: Value = serde_json::from_str(text).map_err(|err| error(err.to_string()))?;
            let (options, errors) = Options::from_json(&value);
            if let Some(first) = errors.first() {
                return Err(error(format!("Transformer {first}")));
            }
            options
        }
        None => Options::default(),
    };
    let config = match tsconfig {
        Some(file) => Some(std::path::PathBuf::from(file)),
        None if search => config::find_tsconfig(std::path::Path::new(path)),
        None => None,
    };
    let base = match &config {
        Some(file) => config::read_tsconfig_options(file).map_err(error)?,
        None => Options::default(),
    };
    Ok((
        base.merged(&explicit),
        config.map(|file| file.display().to_string()),
    ))
}

/// `a.sts` → `a.ts`, `a.stsx` → `a.tsx`; any other path is unchanged.
#[must_use]
pub fn host_path(path: &str, extension: &str) -> String {
    for suffix in [".stsx", ".sts"] {
        if let Some(stem) = path.strip_suffix(suffix) {
            return format!("{stem}{extension}");
        }
    }
    path.to_owned()
}

/// Line starts of `text`, for 1-based line and UTF-16 column lookups.
pub(crate) struct Positions<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Positions<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self { text, starts }
    }

    fn floor(&self, at: usize) -> usize {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// 1-based line and 1-based UTF-16 column of byte `at`.
    pub(crate) fn line_column(&self, at: usize) -> (usize, usize) {
        let at = self.floor(at);
        let line = self.starts.partition_point(|&start| start <= at) - 1;
        let column = self.text[self.starts[line]..at].encode_utf16().count();
        (line + 1, column + 1)
    }

    /// UTF-16 offset of byte `at` (JavaScript string index).
    fn utf16(&self, at: usize) -> usize {
        self.text[..self.floor(at)].encode_utf16().count()
    }
}

/// `SWR4025: text` → (4025, "text"). A message with no code is code 0.
#[must_use]
pub fn split_code(message: &str) -> (u32, &str) {
    for prefix in ["SWR", "TS"] {
        if let Some(rest) = message.strip_prefix(prefix) {
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            if digits > 0
                && let Ok(code) = rest[..digits].parse()
            {
                let text = rest[digits..].strip_prefix(':').unwrap_or(&rest[digits..]);
                return (code, text.trim_start());
            }
        }
    }
    (0, message)
}

fn diagnostic_json(path: &str, positions: &Positions<'_>, item: &SessionDiagnostic) -> Value {
    let (code, message) = split_code(&item.message);
    let (line, column) = positions.line_column(item.start as usize);
    let related: Vec<Value> = item
        .related
        .iter()
        .map(|related| {
            let (line, column) = positions.line_column(related.start as usize);
            json!({
                "message": related.message,
                "file": path,
                "line": line,
                "column": column,
                "start": positions.utf16(related.start as usize),
                "end": positions.utf16(related.end as usize),
            })
        })
        .collect();
    json!({
        "code": code,
        "category": item.severity.as_str(),
        "message": message,
        "file": path,
        "line": line,
        "column": column,
        "start": positions.utf16(item.start as usize),
        "end": positions.utf16(item.end as usize),
        "related": related,
    })
}

/// Transform one file. Answers a JSON object: `code`, `hostPath`,
/// `dependencies`, `diagnostics`, `map` (v3; the identity map when
/// unchanged), `changed`, `spans` (span-map tuples for `composeNative`; one
/// verbatim span when unchanged) and `tsconfig`
/// (the config the options came from, or null).
///
/// # Errors
/// Invalid explicit options, or a tsconfig that cannot be read or holds
/// invalid tsc-ext options.
#[napi(js_name = "transformNative")]
#[allow(clippy::needless_pass_by_value)] // napi passes owned strings
pub fn transform_native(
    code: String,
    path: String,
    tsconfig: Option<String>,
    search_tsconfig: bool,
    options: Option<String>,
) -> napi::Result<String> {
    if let Some(message) = crate::unsupported_source(&path) {
        return Err(error(message));
    }
    let (options, config) = options_for(
        &path,
        tsconfig.as_deref(),
        search_tsconfig,
        options.as_deref(),
    )?;
    let result = TransformSession::from_options(&options).transform(&path, &code);
    let positions = Positions::new(&code);
    let diagnostics: Vec<Value> = result
        .diagnostics
        .iter()
        .map(|item| diagnostic_json(&path, &positions, item))
        .collect();
    // Unchanged: one verbatim span, so `map` is the identity map and
    // `composeMap` renames the later map's source to `path` like any other.
    let (source_map, spans) = if result.changed {
        (result.source_map.clone(), result.mappings.clone())
    } else {
        let len = i32::try_from(code.len()).map_err(|err| error(err.to_string()))?;
        let identity = [(0, len, 0, len, 0)];
        (
            crate::v3_source_map(&path, &code, &code, &identity),
            crate::session::mappings_json(&identity),
        )
    };
    let mut map: Value = serde_json::from_str(&source_map).map_err(|err| error(err.to_string()))?;
    if let Some(object) = map.as_object_mut() {
        object.remove("segments");
    }
    Ok(json!({
        "code": result.code,
        "hostPath": host_path(&path, result.extension),
        "dependencies": result.dependencies,
        "diagnostics": diagnostics,
        "map": map,
        "changed": result.changed,
        "spans": spans,
        "tsconfig": config,
    })
    .to_string())
}

/// `later` (a v3 map, JSON) of a transform of `code`, composed onto the
/// author file `source` at `path` through `spans`. Answers the map as JSON.
///
/// # Errors
/// When `later` or `spans` is not valid.
#[napi(js_name = "composeNative")]
#[allow(clippy::needless_pass_by_value)] // napi passes owned strings
pub fn compose_native(
    path: String,
    host_path: String,
    source: String,
    code: String,
    spans: String,
    later: String,
) -> napi::Result<String> {
    let spans = crate::compose::parse_spans(&spans).map_err(error)?;
    let later: Value = serde_json::from_str(&later).map_err(|err| error(err.to_string()))?;
    let map = crate::compose::compose(&path, &host_path, &source, &code, &spans, &later)
        .map_err(error)?;
    Ok(map.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Positions, host_path, split_code};

    #[test]
    fn codes_paths_and_positions() {
        assert_eq!(split_code("SWR4025: unread"), (4025, "unread"));
        assert_eq!(split_code("plain"), (0, "plain"));
        assert_eq!(host_path("/a/b.stsx", ".tsx"), "/a/b.tsx");
        assert_eq!(host_path("/a/b.ts", ".ts"), "/a/b.ts");
        let positions = Positions::new("é\nab😀c");
        assert_eq!(positions.line_column(3), (2, 1));
        assert_eq!(positions.line_column(9), (2, 5));
        assert_eq!(positions.utf16(9), 6);
    }
}
