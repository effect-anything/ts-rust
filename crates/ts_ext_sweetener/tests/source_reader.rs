//! Macro-module resolution through an installed `SourceReader`, with no file
//! on disk. One test function: the reader is process-wide, and an integration
//! test binary is its own process, so nothing else in the crate sees it.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ts_ext_sweetener::{
    ExpandRequest, Phase, SourceReader, clear_source_reader, expand, set_source_reader,
};

/// Files by absolute, normalized path.
#[derive(Default)]
struct Memory(Mutex<HashMap<PathBuf, String>>);

impl Memory {
    fn put(&self, path: &str, text: &str) {
        self.0
            .lock()
            .expect("lock")
            .insert(PathBuf::from(path), text.to_owned());
    }
}

impl SourceReader for Memory {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.lock().ok()?.get(path).cloned()
    }

    /// Like `Path::canonicalize` on a tree with no symlinks: `.` and `..` are
    /// resolved lexically, and the result must name a file or a directory.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalize(path);
        let files = self.0.lock().map_err(|_| io::ErrorKind::Other)?;
        if files.contains_key(&path) || files.keys().any(|file| file.starts_with(&path)) {
            Ok(path)
        } else {
            Err(io::ErrorKind::NotFound.into())
        }
    }
}

fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

// The fixture of `cases/examples-language-tour-new-source-aware-debug-main`.
const MACROS: &str = "export syntax debug:expr {\n  rule { debug($value:expr) } => {\n    ((result) => ({ expression: #text($value), result }))($value)\n  }\n}\n";
const MAIN: &str =
    "import { debug } from \"./macros.sts\" for syntax;\n\nexport const inspected = debug(20 + 22);\n";
const MAIN_PATH: &str = "/virtual/app/main.sts";

fn expand_main() -> ts_ext_sweetener::Expansion {
    expand(ExpandRequest {
        filename: MAIN_PATH,
        code: MAIN,
        phase: Phase::Check,
    })
}

#[test]
fn macro_modules_resolve_through_the_installed_reader() {
    assert!(
        !Path::new("/virtual").exists(),
        "the test root must not exist on disk"
    );

    // 1. The module exists only in memory, and the expansion finds it.
    let memory = Arc::new(Memory::default());
    memory.put(MAIN_PATH, MAIN);
    memory.put("/virtual/app/macros.sts", MACROS);
    set_source_reader(memory.clone());

    let expansion = expand_main();
    assert!(
        expansion.diagnostics.is_empty(),
        "{:?}",
        expansion.diagnostics
    );
    assert!(
        expansion.code.contains(
            "((result) => ({ expression: \"20 + 22\", result }))(20 + 22)"
        ),
        "{}",
        expansion.code
    );
    assert_eq!(expansion.dependencies, ["/virtual/app/macros.sts"]);

    // 2. A changed module invalidates the memoized expansion of the same file.
    memory.put(
        "/virtual/app/macros.sts",
        &MACROS.replace("expression: #text($value)", "source: #text($value)"),
    );
    let changed = expand_main();
    assert!(
        changed.code.contains("source: \"20 + 22\""),
        "stale memo hit: {}",
        changed.code
    );

    // 3. A reader that lacks the module reports it missing, through the reader:
    // the expansion does not fall through to the disk.
    set_source_reader(Arc::new(Memory::default()));
    let missing = expand_main();
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.starts_with("SWR5002")),
        "{:?}",
        missing.diagnostics
    );

    // 4. Clearing the reader returns resolution to the disk, where `/virtual`
    // does not exist either.
    clear_source_reader();
    let on_disk = expand_main();
    assert!(
        on_disk
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.starts_with("SWR5002")),
        "{:?}",
        on_disk.diagnostics
    );
}
