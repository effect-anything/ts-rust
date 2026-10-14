//! tsc-ext options, read from the project's tsconfig.
//!
//! The options live in the `options` object of the tsconfig
//! `contentMappers` entry whose `package` is [`PACKAGE`]:
//!
//! ```json
//! "contentMappers": [{
//!   "package": "@effect-x/tsc",
//!   "extensions": [".sts", ".stsx"],
//!   "options": { "sweetener": {}, "typegpu": { "autoNaming": true } }
//! }]
//! ```
//!
//! Three readers share this module, so they see the same values:
//!
//! - the content mapper (`tsc-ext` with no arguments): `tsc` sends the
//!   entry's `options` with `openProject`, already parsed; `transform` (the
//!   check view) has no option that changes it;
//! - `tsc-ext expand` and `eval`: `--tsconfig`, else the nearest
//!   `tsconfig.json` above the file;
//! - the Node API (`Transformer`): its `tsconfig` option, else the nearest
//!   `tsconfig.json` above each file. Explicit API options override single
//!   fields of the tsconfig's ([`Options::merged`]).
//!
//! Every field is optional. An absent field takes the default documented on
//! it. Unknown keys and wrong types are errors ([`OptionError`]), not
//! silently ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::Value;

/// The npm package of the `contentMappers` entry tsc-ext reads.
pub const PACKAGE: &str = "@effect-x/tsc";

/// All tsc-ext options, by category.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub sweetener: SweetenerOptions,
    pub typegpu: TypegpuOptions,
}

/// Sweetener options. `None` is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweetenerOptions {
    /// `rewriteImports`, default `false`: module specifiers keep the
    /// `.sts`/`.stsx` the author wrote (as upstream Sweetener prints them).
    /// `true` drops the extension (`./logic.sts` → `./logic`), for a
    /// bundler that resolves the virtual `.ts` by extension search. Only the
    /// Node API output uses it; `tsc` resolves `.sts` imports itself.
    pub rewrite_imports: Option<bool>,
}

impl SweetenerOptions {
    #[must_use]
    pub fn rewrite_imports(&self) -> bool {
        self.rewrite_imports.unwrap_or(false)
    }
}

/// `TypeGPU` options, as unplugin-typegpu names them. `None` is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypegpuOptions {
    /// `autoNaming`, default `true`: name `TypeGPU` resources after their binding.
    pub auto_naming: Option<bool>,
    /// `earlyPruning`, default `true` (unplugin-typegpu and the `tsc` emit hook
    /// prune too): skip a file that mentions none of `"use gpu"`, `tgpu`, `typegpu`.
    pub early_pruning: Option<bool>,
    /// `forceTgpuAlias`, default none: the name `tgpu` is imported as.
    pub force_tgpu_alias: Option<String>,
}

impl TypegpuOptions {
    #[must_use]
    pub fn auto_naming(&self) -> bool {
        self.auto_naming.unwrap_or(true)
    }

    #[must_use]
    pub fn early_pruning(&self) -> bool {
        self.early_pruning.unwrap_or(true)
    }
}

/// An invalid option. `path` is relative to the `options` object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionError {
    pub path: Vec<String>,
    pub message: String,
}

impl std::fmt::Display for OptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "options: {}", self.message)
        } else {
            write!(f, "options.{}: {}", self.path.join("."), self.message)
        }
    }
}

impl Options {
    /// `self` with every field that `over` sets replaced by `over`'s.
    #[must_use]
    pub fn merged(&self, over: &Options) -> Options {
        Options {
            sweetener: SweetenerOptions {
                rewrite_imports: over
                    .sweetener
                    .rewrite_imports
                    .or(self.sweetener.rewrite_imports),
            },
            typegpu: TypegpuOptions {
                auto_naming: over.typegpu.auto_naming.or(self.typegpu.auto_naming),
                early_pruning: over.typegpu.early_pruning.or(self.typegpu.early_pruning),
                force_tgpu_alias: over
                    .typegpu
                    .force_tgpu_alias
                    .clone()
                    .or_else(|| self.typegpu.force_tgpu_alias.clone()),
            },
        }
    }

    /// Read an `options` object. `null` (or absent) is all defaults. Valid
    /// fields are kept even when others are reported.
    #[must_use]
    pub fn from_json(value: &Value) -> (Options, Vec<OptionError>) {
        let mut options = Options::default();
        let mut errors = Vec::new();
        let error = |path: &[&str], message: String| OptionError {
            path: path.iter().map(|part| (*part).to_owned()).collect(),
            message,
        };
        let object = match value {
            Value::Null => return (options, errors),
            Value::Object(object) => object,
            _ => {
                errors.push(error(&[], "must be an object".to_owned()));
                return (options, errors);
            }
        };
        for (key, value) in object {
            match key.as_str() {
                "sweetener" => match value {
                    Value::Null => {}
                    Value::Object(fields) => {
                        for (key, value) in fields {
                            let at = ["sweetener", key.as_str()];
                            match (key.as_str(), value) {
                                (_, Value::Null) => {}
                                ("rewriteImports", Value::Bool(on)) => {
                                    options.sweetener.rewrite_imports = Some(*on);
                                }
                                ("rewriteImports", _) => {
                                    errors.push(error(&at, "must be a boolean".to_owned()));
                                }
                                _ => errors.push(error(&at, "unknown option".to_owned())),
                            }
                        }
                    }
                    _ => errors.push(error(&["sweetener"], "must be an object".to_owned())),
                },
                "typegpu" => match value {
                    Value::Null => {}
                    Value::Object(fields) => {
                        for (key, value) in fields {
                            let at = ["typegpu", key.as_str()];
                            match (key.as_str(), value) {
                                (_, Value::Null) => {}
                                ("autoNaming", Value::Bool(on)) => {
                                    options.typegpu.auto_naming = Some(*on);
                                }
                                ("earlyPruning", Value::Bool(on)) => {
                                    options.typegpu.early_pruning = Some(*on);
                                }
                                ("forceTgpuAlias", Value::String(alias)) => {
                                    options.typegpu.force_tgpu_alias = Some(alias.clone());
                                }
                                ("autoNaming" | "earlyPruning", _) => {
                                    errors.push(error(&at, "must be a boolean".to_owned()));
                                }
                                ("forceTgpuAlias", _) => {
                                    errors.push(error(&at, "must be a string".to_owned()));
                                }
                                _ => errors.push(error(&at, "unknown option".to_owned())),
                            }
                        }
                    }
                    _ => errors.push(error(&["typegpu"], "must be an object".to_owned())),
                },
                _ => errors.push(error(&[key], "unknown option category".to_owned())),
            }
        }
        (options, errors)
    }
}

/// The nearest `tsconfig.json` in `file`'s directory or above it.
#[must_use]
pub fn find_tsconfig(file: &Path) -> Option<PathBuf> {
    let file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(file)
    };
    let mut dir = file.parent();
    while let Some(at) = dir {
        let candidate = at.join("tsconfig.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = at.parent();
    }
    None
}

type Cached = (Option<SystemTime>, Result<Options, String>);

fn cache() -> &'static Mutex<HashMap<PathBuf, Cached>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<PathBuf, Cached>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The tsc-ext options of `tsconfig`: the `options` of its [`PACKAGE`]
/// `contentMappers` entry, or of the first config it `extends` (relative
/// paths only) that has `contentMappers`. A config with no such entry gives
/// the defaults. Read errors, JSON errors and invalid options are `Err`.
/// Results are cached by path and modification time.
///
/// # Errors
/// The message names the config and what is wrong.
pub fn read_tsconfig_options(tsconfig: &Path) -> Result<Options, String> {
    let modified = std::fs::metadata(tsconfig)
        .and_then(|meta| meta.modified())
        .ok();
    if let Ok(cache) = cache().lock()
        && let Some((at, result)) = cache.get(tsconfig)
        && *at == modified
        && modified.is_some()
    {
        return result.clone();
    }
    let result = read_uncached(tsconfig, 0);
    if let Ok(mut cache) = cache().lock() {
        cache.insert(tsconfig.to_path_buf(), (modified, result.clone()));
    }
    result
}

/// The extensions `tsconfig` (or a relative config it extends) registers
/// for this package's content mapper.
pub fn own_extensions(tsconfig: &Path) -> Vec<String> {
    own_extensions_at(tsconfig, 0)
}

fn own_extensions_at(tsconfig: &Path, depth: usize) -> Vec<String> {
    let Some(value) = (depth <= 16)
        .then(|| std::fs::read_to_string(tsconfig).ok())
        .flatten()
        .and_then(|text| serde_json::from_str::<Value>(&strip_jsonc(&text)).ok())
    else {
        return Vec::new();
    };
    if let Some(mappers) = value.get("contentMappers") {
        return mappers
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("package").and_then(Value::as_str) == Some(PACKAGE))
            .flat_map(|entry| {
                entry
                    .get("extensions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|ext| ext.as_str().map(str::to_owned))
            .collect();
    }
    let parents: Vec<&str> = match value.get("extends") {
        Some(Value::String(parent)) => vec![parent.as_str()],
        Some(Value::Array(parents)) => parents.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    for parent in parents.iter().rev() {
        if !(parent.starts_with("./") || parent.starts_with("../") || parent.starts_with('/')) {
            continue;
        }
        let mut path = tsconfig.parent().unwrap_or(Path::new(".")).join(parent);
        if path.extension().is_none() {
            path.set_extension("json");
        }
        if path.is_file() {
            return own_extensions_at(&path, depth + 1);
        }
    }
    Vec::new()
}

fn read_uncached(tsconfig: &Path, depth: usize) -> Result<Options, String> {
    if depth > 16 {
        return Err(format!("{}: `extends` is too deep", tsconfig.display()));
    }
    let text = std::fs::read_to_string(tsconfig)
        .map_err(|err| format!("{}: {err}", tsconfig.display()))?;
    let value: Value = serde_json::from_str(&strip_jsonc(&text))
        .map_err(|err| format!("{}: {err}", tsconfig.display()))?;
    if let Some(mappers) = value.get("contentMappers") {
        let entry = mappers.as_array().and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.get("package").and_then(Value::as_str) == Some(PACKAGE))
        });
        let Some(entry) = entry else {
            return Ok(Options::default());
        };
        let (options, errors) = Options::from_json(entry.get("options").unwrap_or(&Value::Null));
        if let Some(first) = errors.first() {
            return Err(format!(
                "{}: contentMappers \"{PACKAGE}\" {first}",
                tsconfig.display()
            ));
        }
        return Ok(options);
    }
    let parents: Vec<&str> = match value.get("extends") {
        Some(Value::String(parent)) => vec![parent.as_str()],
        Some(Value::Array(parents)) => parents.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    // A later `extends` entry wins in tsconfig; read from the last.
    for parent in parents.iter().rev() {
        if !(parent.starts_with("./") || parent.starts_with("../") || parent.starts_with('/')) {
            continue;
        }
        let base = tsconfig.parent().unwrap_or(Path::new("."));
        let mut path = base.join(parent);
        if path.extension().is_none() {
            path.set_extension("json");
        }
        if path.is_file() {
            return read_uncached(&path, depth + 1);
        }
    }
    Ok(Options::default())
}

/// tsconfig is JSONC: drop comments and trailing commas outside strings.
#[must_use]
pub fn strip_jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut in_string = false;
    let mut start = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if in_string {
            if byte == b'\\' {
                at += 2;
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            at += 1;
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                at += 1;
            }
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                out.push_str(&text[start..at]);
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                start = at;
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                out.push_str(&text[start..at]);
                at += 2;
                while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                    at += 1;
                }
                at = (at + 2).min(bytes.len());
                start = at;
            }
            b',' => {
                let next = bytes[at + 1..]
                    .iter()
                    .position(|byte| !byte.is_ascii_whitespace())
                    .map(|offset| bytes[at + 1 + offset]);
                // A comma before a comment and then `}` stays; serde_json
                // reports that rare form.
                if matches!(next, Some(b'}' | b']')) {
                    out.push_str(&text[start..at]);
                    start = at + 1;
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    out.push_str(&text[start.min(text.len())..]);
    out
}

#[cfg(test)]
mod tests {
    use super::{Options, strip_jsonc};
    use serde_json::json;

    #[test]
    fn reads_categories_and_reports_unknown_keys() {
        let (options, errors) = Options::from_json(&json!({
            "sweetener": { "rewriteImports": true },
            "typegpu": { "autoNaming": false, "forceTgpuAlias": "gpu", "exclude": "x" },
            "other": 1
        }));
        assert_eq!(options.typegpu.auto_naming, Some(false));
        assert_eq!(options.sweetener.rewrite_imports, Some(true));
        assert_eq!(options.typegpu.force_tgpu_alias.as_deref(), Some("gpu"));
        let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            [
                "options.other: unknown option category",
                "options.typegpu.exclude: unknown option"
            ]
        );
    }

    #[test]
    fn explicit_fields_override_single_fields() {
        let (base, _) =
            Options::from_json(&json!({"typegpu": {"autoNaming": false, "earlyPruning": true}}));
        let (over, _) = Options::from_json(&json!({"typegpu": {"autoNaming": true}}));
        let merged = base.merged(&over);
        assert_eq!(merged.typegpu.auto_naming, Some(true));
        assert_eq!(merged.typegpu.early_pruning, Some(true));
    }

    #[test]
    fn jsonc_comments_and_trailing_commas() {
        let text = "{\n // c\n \"a\": \"//x,}\", /* b */ \"b\": [1,],\n}";
        let value: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(value, json!({"a": "//x,}", "b": [1]}));
    }

    #[test]
    fn tsconfig_entry_and_extends() {
        let dir = std::env::temp_dir().join(format!("tsc-ext-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("base.json"),
            r#"{ "contentMappers": [{ "package": "@effect-x/tsc", "extensions": [".sts"],
                 "options": { "typegpu": { "earlyPruning": true } } }] }"#,
        )
        .unwrap();
        std::fs::write(dir.join("tsconfig.json"), r#"{ "extends": "./base", }"#).unwrap();
        let options = super::read_tsconfig_options(&dir.join("tsconfig.json")).unwrap();
        assert_eq!(options.typegpu.early_pruning, Some(true));
        assert_eq!(
            super::find_tsconfig(&dir.join("src/a.sts")),
            Some(dir.join("tsconfig.json"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
