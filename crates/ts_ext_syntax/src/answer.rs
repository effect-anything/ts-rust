//! The JSON answer of one transform, shared by the Node API
//! (`transformNative`) and the `Project`.

use serde_json::{Value, json};

use crate::SessionDiagnostic;

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

/// The JSON answer for `result`, the transform of `code` at `path` (see
/// [`transform_native`]). The `Project` answer adds `moduleType`.
///
/// # Errors
/// The file is longer than `i32::MAX` bytes, or the session's source map is
/// not JSON.
pub fn result_json(
    path: &str,
    code: &str,
    result: &crate::Transformed,
    tsconfig: Option<&str>,
) -> Result<Value, String> {
    let positions = Positions::new(code);
    let diagnostics: Vec<Value> = result
        .diagnostics
        .iter()
        .map(|item| diagnostic_json(path, &positions, item))
        .collect();
    // Unchanged: one verbatim span, so `map` is the identity map and
    // `composeMap` renames the later map's source to `path` like any other.
    let (source_map, spans) = if result.changed {
        (result.source_map.clone(), result.mappings.clone())
    } else {
        let len = i32::try_from(code.len()).map_err(|err| err.to_string())?;
        let identity = [(0, len, 0, len, 0)];
        (
            crate::v3_source_map(path, code, code, &identity),
            crate::session::mappings_json(&identity),
        )
    };
    let mut map: Value = serde_json::from_str(&source_map).map_err(|err| err.to_string())?;
    if let Some(object) = map.as_object_mut() {
        object.remove("segments");
    }
    Ok(json!({
        "code": result.code,
        "hostPath": host_path(path, result.extension),
        "dependencies": result.dependencies,
        "diagnostics": diagnostics,
        "map": map,
        "changed": result.changed,
        "spans": spans,
        "tsconfig": tsconfig,
    }))
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
