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
//!   module (`SWR5002`) or listed a dependency it did not read is not
//!   stored, since a file appearing later could change it.
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

thread_local! {
    /// Content hashes of the modules this thread's expansion read.
    static READ_HASHES: std::cell::RefCell<Vec<(String, u64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Read a macro module at `path`, reusing an earlier read of the same text
/// under the same operator spellings. Records the content hash so a file
/// memo can be validated later.
pub fn read_module(path: &str, text: &str) -> Arc<ReadSyntax> {
    READ_HASHES.with(|read| read.borrow_mut().push((path.to_owned(), hash_text(text))));
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

/// Clear this thread's record of modules read, returning it.
pub fn take_read_hashes() -> Vec<(String, u64)> {
    READ_HASHES.with(|read| std::mem::take(&mut *read.borrow_mut()))
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
    modules: Vec<(String, u64)>,
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

fn still_current(modules: &[(String, u64)]) -> bool {
    modules.iter().all(|(path, hash)| {
        std::fs::read_to_string(path).is_ok_and(|text| hash_text(&text) == *hash)
    })
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
    if still_current(&entry.modules) {
        return Some(entry.expansion.clone());
    }
    map.remove(&key);
    None
}

/// Memoize `expansion`, given the modules (and their hashes) it read.
pub fn store_file(
    filename: &str,
    code: &str,
    phase: Phase,
    budget: &ResourceBudget,
    expansion: &Expansion,
    modules: Vec<(String, u64)>,
) {
    if expansion
        .diagnostics
        .iter()
        .any(|d| d.message.starts_with("SWR5002"))
    {
        return;
    }
    let read_all = expansion
        .dependencies
        .iter()
        .all(|dependency| modules.iter().any(|(path, _)| path == dependency));
    if !read_all {
        return;
    }
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
            modules,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_reads_are_shared_per_text_and_operator_spellings() {
        let text = "export syntax one:expr {\n  rule { one() } => { 1 @@ 2 }\n}\n";
        let first = read_module("/m.sts", text);
        let again = read_module("/m.sts", text);
        assert!(Arc::ptr_eq(&first, &again));
        let with_op =
            sweetener_reader::with_operators(&["@@".to_owned()], || read_module("/m.sts", text));
        assert!(!Arc::ptr_eq(&first, &with_op));
        assert_eq!(
            *with_op,
            sweetener_reader::with_operators(&["@@".to_owned()], || {
                sweetener_reader::read_syntax(text)
            })
        );
        assert_eq!(*first, sweetener_reader::read_syntax(text));
        let hashes = take_read_hashes();
        assert_eq!(hashes.len(), 3);
        assert!(take_read_hashes().is_empty());
    }
}
