//! A project: the tsconfig, its tsc-ext options, its root files, and the
//! rewritten text of each file. The bundler integration (`load` hook) and the
//! Node API `Project` sit on this.
//!
//! The rewrite of one file needs no other file: Sweetener expansion reads
//! its macro modules (they are the file's `dependencies`), tsover and the
//! `TypeGPU` wrap are per-file syntax. The project holds what is shared:
//! the options, the include list (`files`/`include`/`exclude` as `tsc`
//! reads them, through the port's own config parser), a cache of results
//! keyed by text, and the reverse dependency index that turns a changed
//! macro module into the files to load again. Expansion itself also goes
//! through the process-wide Sweetener memo and incremental replay, which
//! the open warms in the background (`prewarm`).
//!
//! One project per tsconfig and options: [`open`] returns the registered
//! one while any handle to it is alive.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use crate::TransformSession;
use crate::config::{self, Options};

/// One file's answer, in the shape the Node API returns (JSON text).
#[derive(Clone, Debug)]
struct Cached {
    text_hash: u64,
    json: Arc<String>,
    dependencies: Vec<String>,
}

#[derive(Debug, Default)]
struct State {
    options: Options,
    session: Option<TransformSession>,
    files: HashSet<String>,
    extensions: Vec<String>,
    /// tsconfig and the configs it extends (a change reopens the project).
    configs: Vec<String>,
    cache: HashMap<String, Cached>,
    /// dependency path → files whose rewrite read it.
    dependents: HashMap<String, BTreeSet<String>>,
    config_errors: Vec<String>,
}

#[derive(Debug)]
pub struct Project {
    pub tsconfig: String,
    explicit: Options,
    state: Mutex<State>,
}

/// What [`Project::get_file`] answers when the file is not in the project.
pub const NOT_OWNED: &str = "not owned";

fn normalize(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    path.replace('\\', "/")
}

fn hash_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

struct ConfigHost {
    fs: Rc<dyn ts_goport::frontend::vfs::Fs>,
    cwd: String,
    /// This package's own extensions. `tsc` drops a mapper whose package it
    /// cannot resolve, or without `runExternalCode`; the project still owns
    /// the files, as it is the mapper.
    own: Vec<String>,
}

struct PlainHost(Rc<dyn ts_goport::frontend::vfs::Fs>, String);

impl ts_goport::frontend::tsoptions::ParseConfigHost for PlainHost {
    fn fs(&self) -> Rc<dyn ts_goport::frontend::vfs::Fs> {
        self.0.clone()
    }
    fn get_current_directory(&self) -> String {
        self.1.clone()
    }
}

impl ts_goport::frontend::tsoptions::ParseConfigHost for ConfigHost {
    fn fs(&self) -> Rc<dyn ts_goport::frontend::vfs::Fs> {
        self.fs.clone()
    }
    fn get_current_directory(&self) -> String {
        self.cwd.clone()
    }
    fn get_file_names_from_config_specs(
        &self,
        config_file_name: &str,
        config_file_specs: &ts_goport::frontend::tsoptions::ConfigFileSpecs,
        base_path: &str,
        options: Option<&ts_goport::options::CompilerOptions>,
        extra_extensions: &[String],
    ) -> (Vec<String>, i32) {
        let mut extensions = extra_extensions.to_vec();
        for ext in &self.own {
            if !extensions
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
            {
                extensions.push(ext.clone());
            }
        }
        ts_goport::frontend::tsoptions::ParseConfigHost::get_file_names_from_config_specs(
            &PlainHost(self.fs.clone(), self.cwd.clone()),
            config_file_name,
            config_file_specs,
            base_path,
            options,
            &extensions,
        )
    }
}

/// Root files of `tsconfig` as `tsc` computes them, the content-mapper
/// extensions it declares, the configs it extends, and its errors.
fn read_project(tsconfig: &str) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let cwd = Path::new(tsconfig)
        .parent()
        .map_or_else(|| ".".to_owned(), |dir| dir.display().to_string());
    let host = ConfigHost {
        fs: ts_goport::frontend::bundled::wrap_fs(ts_goport::frontend::vfs::osvfs_fs()),
        cwd,
        own: crate::config::own_extensions(Path::new(tsconfig)),
    };
    // The project is the content mapper: its code is already running, so the
    // `--runExternalCode` gate (and its diagnostic) does not apply.
    let options = ts_goport::options::CompilerOptions {
        run_external_code: ts_goport::options::Tristate::True,
        ..Default::default()
    };
    let (parsed, errors) = ts_goport::frontend::tsoptions::get_parsed_command_line_of_config_file(
        tsconfig,
        Some(&options),
        None,
        &host,
        None,
    );
    let mut messages: Vec<String> = errors
        .iter()
        .map(|item| item.localize(&ts_goport::locale::DEFAULT))
        .collect();
    let Some(parsed) = parsed else {
        return (Vec::new(), Vec::new(), vec![tsconfig.to_owned()], messages);
    };
    messages.extend(
        parsed
            .get_config_file_parsing_diagnostics()
            .iter()
            .map(|item| item.localize(&ts_goport::locale::DEFAULT)),
    );
    let files = parsed
        .file_names()
        .iter()
        .map(|name| normalize(name))
        .filter(|name| !is_declaration(name) && !name.contains("/node_modules/"))
        .collect();
    let mut configs = vec![normalize(tsconfig)];
    configs.extend(
        parsed
            .extended_source_files()
            .iter()
            .map(|name| normalize(name)),
    );
    let mut extensions = parsed.content_mapper_extensions();
    for ext in &host.own {
        if !extensions
            .iter()
            .any(|known| known.eq_ignore_ascii_case(ext))
        {
            extensions.push(ext.clone());
        }
    }
    (files, extensions, configs, messages)
}

#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "tsc matches extensions case-sensitively"
)]
fn is_declaration(path: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
        || path.contains(".d.") && path.ends_with(".ts")
}

/// `ts`, `tsx`, `js` or `jsx`: how a bundler parses the rewritten text.
#[must_use]
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "tsc matches extensions case-sensitively"
)]
pub fn module_type(path: &str) -> &'static str {
    let path = normalize(path);
    if path.ends_with(".stsx") || path.ends_with(".tsx") {
        "tsx"
    } else if path.ends_with(".jsx") {
        "jsx"
    } else if path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs") {
        "js"
    } else {
        "ts"
    }
}

impl Project {
    fn new(tsconfig: String, explicit: Options) -> Result<Self, String> {
        let project = Self {
            tsconfig,
            explicit,
            state: Mutex::new(State::default()),
        };
        project.reload()?;
        Ok(project)
    }

    fn reload(&self) -> Result<(), String> {
        let base = config::read_tsconfig_options(Path::new(&self.tsconfig))?;
        let options = base.merged(&self.explicit);
        let (files, extensions, configs, errors) = read_project(&self.tsconfig);
        let mut state = self.state.lock().map_err(|err| err.to_string())?;
        state.session = Some(TransformSession::from_options(&options));
        state.options = options;
        state.files = files.into_iter().collect();
        state.extensions = extensions;
        state.configs = configs;
        state.config_errors = errors;
        state.cache.clear();
        state.dependents.clear();
        Ok(())
    }

    /// Whether `path` is a root file of the project (so the integration
    /// serves it).
    #[must_use]
    pub fn owns(&self, path: &str) -> bool {
        let path = normalize(path);
        self.state
            .lock()
            .is_ok_and(|state| state.files.contains(&path))
    }

    /// The owned files, sorted.
    #[must_use]
    pub fn files(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .state
            .lock()
            .map(|state| state.files.iter().cloned().collect())
            .unwrap_or_default();
        files.sort();
        files
    }

    /// Errors reading the tsconfig (an empty list when it parsed).
    #[must_use]
    pub fn config_errors(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|state| state.config_errors.clone())
            .unwrap_or_default()
    }

    /// The content-mapper extensions the tsconfig declares (`.sts`, …).
    #[must_use]
    pub fn extensions(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|state| state.extensions.clone())
            .unwrap_or_default()
    }

    /// The rewrite of `path` as JSON (see `transformNative` for the shape,
    /// plus `moduleType`). `text` is the file's content, read from disk
    /// when absent. A repeated request with the same text is the cached
    /// answer.
    ///
    /// # Errors
    /// `path` is not owned ([`NOT_OWNED`]), is unsupported, or cannot be read.
    pub fn get_file(&self, path: &str, text: Option<String>) -> Result<Arc<String>, String> {
        let path = normalize(path);
        if let Some(message) = crate::unsupported_source(&path) {
            return Err(message);
        }
        let session = {
            let state = self.state.lock().map_err(|err| err.to_string())?;
            if !state.files.contains(&path) {
                return Err(format!("{path}: {NOT_OWNED} by {}", self.tsconfig));
            }
            state.session.clone().unwrap_or_default()
        };
        let text = match text {
            Some(text) => text,
            None => std::fs::read_to_string(&path).map_err(|err| format!("{path}: {err}"))?,
        };
        let text_hash = hash_text(&text);
        if let Ok(state) = self.state.lock()
            && let Some(cached) = state.cache.get(&path)
            && cached.text_hash == text_hash
        {
            return Ok(cached.json.clone());
        }
        let result = session.transform(&path, &text);
        let mut json = crate::answer::result_json(&path, &text, &result, Some(&self.tsconfig))?;
        if let Some(object) = json.as_object_mut() {
            object.insert("moduleType".to_owned(), module_type(&path).into());
        }
        let json = Arc::new(json.to_string());
        let dependencies: Vec<String> = result
            .dependencies
            .iter()
            .map(|dep| normalize(dep))
            .collect();
        let mut state = self.state.lock().map_err(|err| err.to_string())?;
        if let Some(old) = state.cache.get(&path) {
            for dependency in old.dependencies.clone() {
                if let Some(set) = state.dependents.get_mut(&dependency) {
                    set.remove(&path);
                }
            }
        }
        for dependency in &dependencies {
            state
                .dependents
                .entry(dependency.clone())
                .or_default()
                .insert(path.clone());
        }
        state.cache.insert(
            path,
            Cached {
                text_hash,
                json: json.clone(),
                dependencies,
            },
        );
        Ok(json)
    }

    /// `path` changed, was added or was removed. Answers the owned files
    /// whose rewrite must be loaded again: `path` itself when owned, every
    /// file that read it (a Sweetener macro module), and every file when it
    /// is the tsconfig or a config it extends (options and the include list
    /// are read again). A new file under the include list becomes owned.
    ///
    /// # Errors
    /// The tsconfig cannot be read again.
    pub fn invalidate(&self, path: &str) -> Result<Vec<String>, String> {
        let path = normalize(path);
        let is_config = self
            .state
            .lock()
            .is_ok_and(|state| state.configs.contains(&path));
        if is_config {
            self.reload()?;
            return Ok(self.files());
        }
        let mut affected = BTreeSet::new();
        let mut state = self.state.lock().map_err(|err| err.to_string())?;
        let known = state.files.contains(&path);
        state.cache.remove(&path);
        if known {
            affected.insert(path.clone());
        }
        if let Some(dependents) = state.dependents.get(&path).cloned() {
            for file in dependents {
                state.cache.remove(&file);
                affected.insert(file);
            }
        }
        let candidate = !known
            && !path.contains("/node_modules/")
            && !is_declaration(&path)
            && (Path::new(&path).extension().is_some_and(|ext| {
                matches!(
                    ext.to_str(),
                    Some("ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs")
                )
            }) || state
                .extensions
                .iter()
                .any(|ext| path.ends_with(ext.as_str())));
        drop(state);
        if candidate || (known && !Path::new(&path).exists()) {
            // A created or deleted file: the include list is read again.
            let (files, _, _, _) = read_project(&self.tsconfig);
            let mut state = self.state.lock().map_err(|err| err.to_string())?;
            state.files = files.into_iter().collect();
            if state.files.contains(&path) {
                affected.insert(path);
            }
        }
        Ok(affected.into_iter().collect())
    }
}

type Registry = Mutex<HashMap<String, (Arc<Project>, usize)>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(tsconfig: &str, explicit: &str) -> String {
    format!("{tsconfig}\u{0}{explicit}")
}

/// The project of `tsconfig` under `explicit` options (JSON text, the
/// same object the `Transformer` takes). A second open of the same pair is
/// the same project; [`close`] once per open.
///
/// # Errors
/// The tsconfig cannot be read or holds invalid tsc-ext options.
pub fn open(
    tsconfig: &Path,
    explicit: Options,
    explicit_key: &str,
) -> Result<Arc<Project>, String> {
    let tsconfig = std::fs::canonicalize(tsconfig)
        .unwrap_or_else(|_| PathBuf::from(tsconfig))
        .display()
        .to_string();
    let tsconfig = normalize(&tsconfig);
    let id = key(&tsconfig, explicit_key);
    let mut registry = registry().lock().map_err(|err| err.to_string())?;
    if let Some((project, count)) = registry.get_mut(&id) {
        *count += 1;
        return Ok(project.clone());
    }
    let project = Arc::new(Project::new(tsconfig.clone(), explicit)?);
    registry.insert(id, (project.clone(), 1));
    drop(registry);
    let _ = crate::prewarm::prewarm_in_background(&tsconfig);
    Ok(project)
}

/// Drop one open of `project`. The last one unregisters it.
pub fn close(project: &Arc<Project>, explicit_key: &str) {
    let id = key(&project.tsconfig, explicit_key);
    if let Ok(mut registry) = registry().lock()
        && let Some((_, count)) = registry.get_mut(&id)
    {
        *count -= 1;
        if *count == 0 {
            registry.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{module_type, open};
    use crate::config::Options;

    fn fixture(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("tsc-ext-project-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/macros")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
        std::fs::write(
            root.join("tsconfig.json"),
            r#"{ "compilerOptions": { "strict": true, "allowImportingTsExtensions": true, "noEmit": true },
                 "include": ["src"],
                 "contentMappers": [{ "package": "@effect-x/tsc", "extensions": [".sts", ".stsx"] }] }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/macros/one.sts"),
            "export syntax one:expr {\n  rule { one() } => { 1 }\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.sts"),
            "import { one } from \"./macros/one.sts\" for syntax;\nexport const a = one();\n",
        )
        .unwrap();
        std::fs::write(root.join("src/plain.ts"), "export const b: number = 2;\n").unwrap();
        std::fs::write(root.join("src/types.d.ts"), "export {};\n").unwrap();
        std::fs::write(root.join("node_modules/dep/index.ts"), "export {};\n").unwrap();
        root
    }

    #[test]
    fn owns_reads_and_invalidates() {
        let root = fixture("owns");
        let project = open(&root.join("tsconfig.json"), Options::default(), "{}").unwrap();
        let canon = std::fs::canonicalize(&root).unwrap().display().to_string();
        let main = format!("{canon}/src/main.sts");
        let plain = format!("{canon}/src/plain.ts");
        let macros = format!("{canon}/src/macros/one.sts");
        assert!(project.owns(&main), "{:?}", project.files());
        assert!(project.owns(&plain));
        assert!(!project.owns(&format!("{canon}/src/types.d.ts")));
        assert!(!project.owns(&format!("{canon}/node_modules/dep/index.ts")));
        let answer: serde_json::Value =
            serde_json::from_str(&project.get_file(&main, None).unwrap()).unwrap();
        assert!(
            answer["code"]
                .as_str()
                .unwrap()
                .contains("export const a = 1;"),
            "{answer}"
        );
        assert_eq!(answer["moduleType"], "ts");
        assert!(
            answer["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .any(|dep| dep.as_str().unwrap().ends_with("macros/one.sts")),
            "{answer}"
        );
        // A macro module change reloads the file that read it.
        let affected = project.invalidate(&macros).unwrap();
        assert!(affected.contains(&main), "{affected:?}");
        // A new file under `include` becomes owned.
        let added = format!("{canon}/src/added.ts");
        std::fs::write(&added, "export {};\n").unwrap();
        assert!(!project.owns(&added));
        assert_eq!(project.invalidate(&added).unwrap(), vec![added.clone()]);
        assert!(project.owns(&added));
        // The same tsconfig and options is the same project.
        let again = open(&root.join("tsconfig.json"), Options::default(), "{}").unwrap();
        assert!(std::sync::Arc::ptr_eq(&project, &again));
        assert_eq!(module_type("/a/b.stsx"), "tsx");
        let _ = std::fs::remove_dir_all(&root);
    }
}
