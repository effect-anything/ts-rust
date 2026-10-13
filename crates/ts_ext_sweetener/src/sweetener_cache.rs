//! Incremental expansion caching.
//!
//! Two layers, both process-wide because every expansion runs on its own
//! thread (see [`super::sweetener_expand::expand`]):
//!
//! - **Macro modules.** A module's token tree is read once per distinct
//!   (text, operator spellings) pair and shared, so N files importing the
//!   same macro module do not re-read it. Spellings are part of the key
//!   because a file's operator definitions change how text is read.
//! - **Whole files.** An expansion is memoized by (filename, phase,
//!   budget, code) and reused only while every macro module it read still
//!   has the content it had then. An expansion that failed to resolve a
//!   module is validated the same way: every read and `canonicalize` it
//!   made, misses included, must observe the same result again, so a
//!   created, removed or retargeted file invalidates it.
//!
//! Hits return exactly what a fresh expansion returns; the cache tests
//! compare them field by field.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use super::sweetener_reader::{self, ReadSyntax};
use crate::{Expansion, Phase, ResourceBudget};

/// Entries kept per layer before the layer is cleared.
const CAPACITY: usize = 256;

fn hash_text(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

type ModuleKey = (String, Vec<String>);

fn modules() -> &'static Mutex<HashMap<ModuleKey, Arc<ReadSyntax>>> {
    static MODULES: OnceLock<Mutex<HashMap<ModuleKey, Arc<ReadSyntax>>>> = OnceLock::new();
    MODULES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One filesystem observation an expansion depended on. Misses count: a
/// file that did not exist then and does now changes resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// `read_to_string(path)`: hash of the text, `None` when it failed.
    Read(std::path::PathBuf, Option<u64>),
    /// `canonicalize(path)`: the target, `None` when it failed.
    Canonical(std::path::PathBuf, Option<std::path::PathBuf>),
}

impl Probe {
    fn observe_read(path: &std::path::Path) -> (Self, Option<String>) {
        let text = std::fs::read_to_string(path).ok();
        (
            Self::Read(path.to_path_buf(), text.as_deref().map(hash_text)),
            text,
        )
    }

    fn observe_canonical(path: &std::path::Path) -> Self {
        Self::Canonical(path.to_path_buf(), path.canonicalize().ok())
    }

    fn still_holds(&self) -> bool {
        match self {
            Self::Read(path, _) => Self::observe_read(path).0 == *self,
            Self::Canonical(path, _) => Self::observe_canonical(path) == *self,
        }
    }
}

thread_local! {
    /// Filesystem observations this thread's expansion made.
    static PROBES: std::cell::RefCell<Vec<Probe>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn record(probe: Probe) {
    PROBES.with(|probes| {
        let mut probes = probes.borrow_mut();
        if !probes.contains(&probe) {
            probes.push(probe);
        }
    });
}

/// `std::fs::read_to_string`, recorded (hit or miss) for memo validation.
pub fn read_file(path: &std::path::Path) -> Option<String> {
    let (probe, text) = Probe::observe_read(path);
    record(probe);
    text
}

/// `Path::canonicalize`, recorded for memo validation: a symlink retarget
/// changes cycle keys, dependency names and messages.
pub fn canonicalize(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let probe = Probe::observe_canonical(path);
    record(probe.clone());
    match probe {
        Probe::Canonical(_, Some(target)) => Ok(target),
        _ => path.canonicalize(),
    }
}

/// Read a macro module at `path`, reusing an earlier read of the same text
/// under the same operator spellings.
pub fn read_module(text: &str) -> Arc<ReadSyntax> {
    let key = (text.to_owned(), sweetener_reader::extra_operators());
    if let Some(hit) = modules().lock().ok().and_then(|map| map.get(&key).cloned()) {
        return hit;
    }
    let read = Arc::new(sweetener_reader::read_syntax(text));
    if let Ok(mut map) = modules().lock() {
        if map.len() >= CAPACITY {
            map.clear();
        }
        map.insert(key, read.clone());
    }
    read
}

/// Clear this thread's recorded filesystem observations, returning them.
pub fn take_probes() -> Vec<Probe> {
    PROBES.with(|probes| std::mem::take(&mut *probes.borrow_mut()))
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct FileKey {
    filename: String,
    phase: String,
    budget: String,
    code: String,
}

struct FileEntry {
    expansion: Expansion,
    probes: Vec<Probe>,
}

fn files() -> &'static Mutex<HashMap<FileKey, FileEntry>> {
    static FILES: OnceLock<Mutex<HashMap<FileKey, FileEntry>>> = OnceLock::new();
    FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_key(filename: &str, code: &str, phase: Phase, budget: &ResourceBudget) -> FileKey {
    FileKey {
        filename: filename.to_owned(),
        phase: format!("{phase:?}"),
        budget: format!("{budget:?}"),
        code: code.to_owned(),
    }
}

fn still_current(probes: &[Probe]) -> bool {
    probes.iter().all(Probe::still_holds)
}

/// A memoized expansion of this file, if every module it read is unchanged.
pub fn lookup_file(
    filename: &str,
    code: &str,
    phase: Phase,
    budget: &ResourceBudget,
) -> Option<Expansion> {
    let key = file_key(filename, code, phase, budget);
    let mut map = files().lock().ok()?;
    let entry = map.get(&key)?;
    if still_current(&entry.probes) {
        return Some(entry.expansion.clone());
    }
    map.remove(&key);
    None
}

/// Memoize `expansion`, given every filesystem observation it made.
pub fn store_file(
    filename: &str,
    code: &str,
    phase: Phase,
    budget: &ResourceBudget,
    expansion: &Expansion,
    probes: Vec<Probe>,
) {
    let Ok(mut map) = files().lock() else {
        return;
    };
    if map.len() >= CAPACITY {
        map.clear();
    }
    map.insert(
        file_key(filename, code, phase, budget),
        FileEntry {
            expansion: expansion.clone(),
            probes,
        },
    );
}

/// Forget every cached module read and file expansion.
pub fn clear() {
    if let Ok(mut map) = modules().lock() {
        map.clear();
    }
    if let Ok(mut map) = files().lock() {
        map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_reads_are_shared_per_text_and_operator_spellings() {
        let text = "export syntax one:expr {\n  rule { one() } => { 1 @@ 2 }\n}\n";
        let first = read_module(text);
        let again = read_module(text);
        assert!(Arc::ptr_eq(&first, &again));
        let with_op = sweetener_reader::with_operators(&["@@".to_owned()], || read_module(text));
        assert!(!Arc::ptr_eq(&first, &with_op));
        assert_eq!(
            *with_op,
            sweetener_reader::with_operators(&["@@".to_owned()], || {
                sweetener_reader::read_syntax(text)
            })
        );
        assert_eq!(*first, sweetener_reader::read_syntax(text));
        assert!(take_probes().is_empty());
    }
}
