//! `tsc --watch` with the real `tsc-ext` mapper: an edit of a `for syntax`
//! macro module expands and checks again the files that import it.
//!
//! Two layouts: the macro module outside the program (`files` names only
//! the importer), which the watcher must still watch, and inside it. In
//! both, the importer's text does not change, so its incremental version
//! must cover its expansion, not only its text.
//!
//! The watch backend is a stub that routes the events the test sends, and
//! each cycle runs on `do_cycle`. One test: the working directory and the
//! watch backend are per process / thread.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rustc_hash::FxHashMap;
use ts_goport::emitter::program_emit::{EmitResult, WriteFile};
use ts_goport::execute::execute_tsc::{TscCompilationHooks, command_line};
use ts_goport::execute::incremental::program::Program;
use ts_goport::execute::tsc::compile::{CommandLineTesting, Writer, new_os_system};
use ts_goport::execute::watcher::set_test_watch_backend;
use ts_goport::execute::watchmanager::{WatchBackend, WatchDirectoryRequest};
use ts_goport::frontend::compiler::TraceFn;
use ts_goport::frontend::tspath;
use ts_goport::fswatch::{self, Event, EventKind};
use ts_goport::gostd::{GoError, context};
use ts_goport::locale::Locale;

#[derive(Default)]
struct Backend {
    watches: Mutex<Vec<(String, bool, fswatch::WatchCallback)>>,
}

struct Closer;

impl fswatch::Watch for Closer {
    fn close(&self) -> Result<(), GoError> {
        Ok(())
    }
    fn unexported(&self) {}
}

impl WatchBackend for Backend {
    fn watch_directory(
        &self,
        dir: &str,
        callback: fswatch::WatchCallback,
        recursive: bool,
        ignore: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
    ) -> Result<Box<dyn fswatch::Watch>, GoError> {
        let mut closers = self.watch_directories(vec![WatchDirectoryRequest {
            dir: dir.to_owned(),
            callback,
            recursive,
            ignore,
        }])?;
        Ok(closers.remove(0))
    }

    fn watch_directories(
        &self,
        requests: Vec<WatchDirectoryRequest>,
    ) -> Result<Vec<Box<dyn fswatch::Watch>>, GoError> {
        let mut watches = self.watches.lock().unwrap();
        Ok(requests
            .into_iter()
            .map(|request| {
                watches.push((request.dir, request.recursive, request.callback));
                Box::new(Closer) as Box<dyn fswatch::Watch>
            })
            .collect())
    }
}

impl Backend {
    /// An update of `file`, through every watch of a directory above it.
    /// Returns whether any watch covered it.
    fn update(&self, file: &Path) -> bool {
        let path = file.to_string_lossy().into_owned();
        let parent = file.parent().unwrap().to_string_lossy().into_owned();
        let targets: Vec<fswatch::WatchCallback> = self
            .watches
            .lock()
            .unwrap()
            .iter()
            .filter(|(dir, recursive, _)| {
                parent == *dir || (*recursive && parent.starts_with(&format!("{dir}/")))
            })
            .map(|(_, _, callback)| callback.clone())
            .collect();
        for callback in &targets {
            callback(
                vec![Event {
                    kind: EventKind::Update,
                    path: path.clone(),
                }],
                None,
            );
        }
        !targets.is_empty()
    }
}

struct Quiet;

impl CommandLineTesting for Quiet {
    fn on_emitted_files(
        &self,
        _result: &EmitResult,
        _m_times_cache: Option<&Mutex<FxHashMap<tspath::Path, Option<SystemTime>>>>,
    ) {
    }
    fn on_list_files_start(&self, _w: &Writer) {}
    fn on_list_files_end(&self, _w: &Writer) {}
    fn on_statistics_start(&self, _w: &Writer) {}
    fn on_statistics_end(&self, _w: &Writer) {}
    fn on_build_status_report_start(&self, _w: &Writer) {}
    fn on_build_status_report_end(&self, _w: &Writer) {}
    fn on_watch_status_report_start(&self) {}
    fn on_watch_status_report_end(&self) {}
    fn get_trace(&self, _w: Writer, _locale: Locale) -> TraceFn {
        Rc::new(|_, _| {})
    }
    fn on_program(&self, _program: &Program) {}
}

struct Hooks;

impl TscCompilationHooks for Hooks {
    fn write_file(&self) -> Option<WriteFile> {
        None
    }
    fn testing(&self) -> Option<Rc<dyn CommandLineTesting>> {
        Some(Rc::new(Quiet))
    }
}

const PAIR: &str = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n";
const TRIPLE: &str = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x, $x] }\n}\n";
const APP: &str = "import { twice } from \"./macros/twice.sts\" for syntax;\nexport const pair: [number, number] = twice(1);\n";

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Sets up a project in `root`, runs `tsc --watch`, edits the macro module
/// and returns the output of the first build and of the cycle after the edit.
fn run(root: &Path, files: Option<&str>) -> (String, String, String) {
    let files = files.map_or_else(String::new, |files| format!("\"files\": {files},"));
    write(
        root,
        "tsconfig.json",
        &format!(
            r#"{{ {files} "compilerOptions": {{ "noEmit": true, "strict": true, "types": [] }},
  "contentMappers": [ {{ "package": "mapper", "extensions": [".sts"] }} ] }}"#
        ),
    );
    write(
        root,
        "node_modules/mapper/package.json",
        &format!(
            r#"{{ "name": "mapper", "version": "1.0.0", "typescript": {{ "contentMapper": {{ "exec": [{:?}] }} }} }}"#,
            env!("CARGO_BIN_EXE_tsc-ext")
        ),
    );
    write(root, "macros/twice.sts", PAIR);
    write(root, "app.sts", APP);
    std::env::set_current_dir(root).unwrap();

    let output: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
    let take = || String::from_utf8(std::mem::take(&mut *output.borrow_mut())).unwrap();
    let sys = Rc::new(new_os_system().expect("an OS system").with_writer(output.clone()));
    let backend = Rc::new(Backend::default());
    set_test_watch_backend(backend.clone());
    let (ctx, cancel) = context::with_cancel(&context::background());
    let args: Vec<String> = ["--watch", "--runExternalCode", "--preserveWatchOutput"]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();
    let result = command_line(&ctx, sys, &args, &Hooks);
    let mut watcher = result.watcher.expect("a watcher");
    let first = take();

    write(root, "macros/twice.sts", TRIPLE);
    assert!(
        backend.update(&root.join("macros/twice.sts")),
        "no watch covers the macro module"
    );
    watcher.do_cycle();
    let edited = take();

    write(root, "macros/twice.sts", PAIR);
    backend.update(&root.join("macros/twice.sts"));
    watcher.do_cycle();
    let restored = take();
    cancel();
    drop(watcher);
    (first, edited, restored)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ts_ext_watch_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

#[test]
fn an_edited_macro_module_rechecks_its_importers() {
    for (name, files) in [("outside", Some(r#"["app.sts"]"#)), ("inside", None)] {
        let root = scratch(name);
        let (first, edited, restored) = run(&root, files);
        assert!(!first.contains("error TS"), "{name}: first build:\n{first}");
        assert!(
            edited.contains("app.sts") && edited.contains("error TS2322"),
            "{name}: after the edit, app.sts must fail:\n{edited}"
        );
        assert!(
            !restored.contains("error TS"),
            "{name}: after the revert:\n{restored}"
        );
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }
}
