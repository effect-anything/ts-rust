//! Node-API binding of the `Transformer` (js/index.cjs).
//!
//! The JavaScript wrapper owns the public shape (class, option merging,
//! `composeMap`, `formatDiagnostics`); this module does the work and answers
//! in JSON, so the wrapper can be renamed or reshaped without touching Rust.
//! The content mapper is the `tsc-ext` binary and does not go through here.

use napi_derive::napi;
use serde_json::{Value, json};

use crate::TransformSession;
use crate::config::{self, Options};

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
    Ok(
        crate::answer::result_json(&path, &code, &result, config.as_deref())
            .map_err(error)?
            .to_string(),
    )
}

// ---- Project -------------------------------------------------------------

type Handles = std::sync::Mutex<
    std::collections::HashMap<u32, (std::sync::Arc<crate::project::Project>, String)>,
>;

fn handles() -> &'static Handles {
    static HANDLES: std::sync::OnceLock<Handles> = std::sync::OnceLock::new();
    HANDLES.get_or_init(Default::default)
}

fn handle(id: u32) -> napi::Result<std::sync::Arc<crate::project::Project>> {
    handles()
        .lock()
        .map_err(|err| error(err.to_string()))?
        .get(&id)
        .map(|(project, _)| project.clone())
        .ok_or_else(|| error(format!("Project {id} is disposed")))
}

/// Open (or join) the project of `tsconfig` under `options` (JSON). Answers
/// `{ id, tsconfig, files, extensions, configErrors }`.
///
/// # Errors
/// Invalid options, or a tsconfig that cannot be read.
#[napi(js_name = "projectOpen")]
#[allow(clippy::needless_pass_by_value)] // napi passes owned strings
pub fn project_open(tsconfig: String, options: Option<String>) -> napi::Result<String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let key = options.clone().unwrap_or_else(|| "{}".to_owned());
    let explicit = match options.as_deref() {
        Some(text) => {
            let value: Value = serde_json::from_str(text).map_err(|err| error(err.to_string()))?;
            let (options, errors) = Options::from_json(&value);
            if let Some(first) = errors.first() {
                return Err(error(format!("Project {first}")));
            }
            options
        }
        None => Options::default(),
    };
    let project =
        crate::project::open(std::path::Path::new(&tsconfig), explicit, &key).map_err(error)?;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let answer = json!({
        "id": id,
        "tsconfig": project.tsconfig,
        "files": project.files(),
        "extensions": project.extensions(),
        "configErrors": project.config_errors(),
    });
    handles()
        .lock()
        .map_err(|err| error(err.to_string()))?
        .insert(id, (project, key));
    Ok(answer.to_string())
}

/// Whether `path` is a root file of project `id`.
///
/// # Errors
/// A disposed project.
#[napi(js_name = "projectOwns")]
#[allow(clippy::needless_pass_by_value)]
pub fn project_owns(id: u32, path: String) -> napi::Result<bool> {
    Ok(handle(id)?.owns(&path))
}

/// The owned files of project `id`.
///
/// # Errors
/// A disposed project.
#[napi(js_name = "projectFiles")]
pub fn project_files(id: u32) -> napi::Result<Vec<String>> {
    Ok(handle(id)?.files())
}

/// The rewrite of `path` (JSON; `transformNative`'s shape plus `moduleType`).
///
/// # Errors
/// A disposed project, a file it does not own, or an unreadable file.
#[napi(js_name = "projectGetFile")]
#[allow(clippy::needless_pass_by_value)]
pub fn project_get_file(id: u32, path: String, text: Option<String>) -> napi::Result<String> {
    let project = handle(id)?;
    project
        .get_file(&path, text)
        .map(|json| json.as_ref().clone())
        .map_err(error)
}

/// [`project_get_file`] on a worker thread.
pub struct GetFileTask {
    project: std::sync::Arc<crate::project::Project>,
    path: String,
    text: Option<String>,
}

impl napi::Task for GetFileTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<String> {
        self.project
            .get_file(&self.path, self.text.take())
            .map(|json| json.as_ref().clone())
            .map_err(error)
    }

    fn resolve(&mut self, _env: napi::Env, output: String) -> napi::Result<String> {
        Ok(output)
    }
}

/// [`project_get_file`] off the main thread: a promise of the JSON answer.
///
/// # Errors
/// A disposed project.
#[napi(js_name = "projectGetFileAsync")]
pub fn project_get_file_async(
    id: u32,
    path: String,
    text: Option<String>,
) -> napi::Result<napi::bindgen_prelude::AsyncTask<GetFileTask>> {
    Ok(napi::bindgen_prelude::AsyncTask::new(GetFileTask {
        project: handle(id)?,
        path,
        text,
    }))
}

/// `path` changed: the owned files to load again.
///
/// # Errors
/// A disposed project, or a tsconfig that can no longer be read.
#[napi(js_name = "projectInvalidate")]
#[allow(clippy::needless_pass_by_value)]
pub fn project_invalidate(id: u32, path: String) -> napi::Result<Vec<String>> {
    handle(id)?.invalidate(&path).map_err(error)
}

/// Drop handle `id`. The project closes with its last handle.
///
/// # Errors
/// Never; a second dispose is a no-op.
#[napi(js_name = "projectDispose")]
pub fn project_dispose(id: u32) -> napi::Result<()> {
    let removed = handles()
        .lock()
        .map_err(|err| error(err.to_string()))?
        .remove(&id);
    if let Some((project, key)) = removed {
        crate::project::close(&project, &key);
    }
    Ok(())
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
