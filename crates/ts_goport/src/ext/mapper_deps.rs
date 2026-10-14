//! PORT: not in Go. Invalidation by a content-mapped file's dependencies.
//!
//! A transform can read files other than its input (Sweetener reads the
//! `for syntax` macro modules), and the mapper reports them as
//! `dependencies`. Go keys a mapped parse by the original text and the
//! transform identity only, so after an edit of a macro module the
//! language server kept serving the old expansion of every file that
//! imports it. Here the parse cache key also folds in the state of those
//! files: [`salt`] is mixed into the transform identity, and [`record`]
//! notes the dependencies after each acquire.
//!
//! The state of a dependency is the hash of its text on disk -- the mapper
//! reads the disk, not the editor's buffer. The first state seen for a
//! file is its baseline and salts nothing, so the key stays Go's until a
//! dependency changes; any other state salts with the hash of every
//! dependency the file has had, which no other state shares.
//!
//! `project/compilerhost.rs` keeps one call to each, and
//! `project/projectcollectionbuilder.rs` one to [`with_dependents`] so a
//! changed macro module marks its importers changed; the watcher and the
//! incremental build use [`program_dependencies`] and [`version_text`].

use crate::spanmap::SpanMap;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};
use xxhash_rust::xxh3::xxh3_128;

#[derive(Default)]
struct Record {
    /// Each dependency's hash when the file was first transformed.
    baseline: BTreeMap<String, u128>,
    /// Every dependency the file has had since.
    seen: BTreeSet<String>,
}

fn records() -> &'static Mutex<HashMap<String, Record>> {
    static RECORDS: OnceLock<Mutex<HashMap<String, Record>>> = OnceLock::new();
    RECORDS.get_or_init(Default::default)
}

/// The hash of a file's text on disk; `0` when it cannot be read.
pub fn disk_state(path: &str) -> u128 {
    let (text, ok) = crate::frontend::vfs::osvfs_fs().read_file(path);
    if ok { xxh3_128(text.as_bytes()) | 1 } else { 0 }
}

/// What to mix into the cache key of the mapped file at `path`. `state`
/// reads a dependency's current state (normally [`disk_state`]).
pub fn salt_with(path: &str, state: &dyn Fn(&str) -> u128) -> u128 {
    let records = records().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(record) = records.get(path) else {
        return 0;
    };
    if record.baseline.iter().all(|(dep, hash)| state(dep) == *hash) {
        return 0;
    }
    let mut buf = Vec::with_capacity(record.seen.len() * 48);
    for dep in &record.seen {
        buf.extend_from_slice(dep.as_bytes());
        buf.push(0);
        buf.extend_from_slice(&state(dep).to_le_bytes());
    }
    xxh3_128(&buf) | 1
}

/// [`salt_with`] the disk.
pub fn salt(path: &str) -> u128 {
    salt_with(path, &disk_state)
}

/// Notes the dependencies the mapper reported for the file at `path`.
pub fn record_with(path: &str, span_map: Option<&SpanMap>, state: &dyn Fn(&str) -> u128) {
    let deps = SpanMap::dependencies(span_map);
    let mut records = records().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if deps.is_empty() && !records.contains_key(path) {
        return;
    }
    let record = records.entry(path.to_owned()).or_default();
    // A dependency that appears after the first transform (a macro
    // module that started to import another) is not baseline: the
    // baseline state did not read it.
    record.seen.extend(deps.iter().cloned());
    if record.baseline.is_empty() {
        for dep in deps {
            record.baseline.insert(dep.clone(), state(dep));
        }
    }
}

/// [`record_with`] the disk, from the canonical file of a mapped parse.
pub fn record(path: &str, files: &crate::contentmapper::SourceFiles) {
    let span_map = files.canonical.as_ref().and_then(|file| file.span_map());
    record_with(path, span_map, &disk_state);
}

/// Every dependency of the mapped files of the current program, for the
/// watcher's seen files: an edit of a macro module outside the program
/// must start a build too.
pub fn program_dependencies() -> Vec<String> {
    let Some(program) = crate::program::go_frontend_program() else {
        return Vec::new();
    };
    let mut out = BTreeSet::new();
    for file in program.files_by_path().values() {
        out.extend(SpanMap::dependencies(file.span_map()).iter().cloned());
    }
    out.into_iter().collect()
}

/// `paths` plus every content-mapped file of `program` that read one of
/// them, for the language server's file change handling: a changed macro
/// module changes the files that import it, though their text did not.
pub fn with_dependents<'a>(
    paths: &'a [crate::frontend::tspath::Path],
    program: Option<&crate::frontend::compiler::NewProgram>,
    to_path: &dyn Fn(&str) -> crate::frontend::tspath::Path,
) -> std::borrow::Cow<'a, [crate::frontend::tspath::Path]> {
    let Some(program) = program else {
        return std::borrow::Cow::Borrowed(paths);
    };
    if paths.is_empty() {
        return std::borrow::Cow::Borrowed(paths);
    }
    let changed: std::collections::HashSet<&crate::frontend::tspath::Path> = paths.iter().collect();
    let mut extra = Vec::new();
    for (path, file) in program.files_by_path() {
        let deps = SpanMap::dependencies(file.span_map());
        if !deps.is_empty() && !changed.contains(path) && deps.iter().any(|dep| changed.contains(&to_path(dep))) {
            extra.push(path.clone());
        }
    }
    if extra.is_empty() {
        return std::borrow::Cow::Borrowed(paths);
    }
    extra.sort();
    let mut all = paths.to_vec();
    all.extend(extra);
    std::borrow::Cow::Owned(all)
}

/// The text an incremental build hashes for a content-mapped file's
/// version. Go hashes the original text and the transform identity, so
/// a file whose expansion changed with a macro module kept its version,
/// and the build reused its old diagnostics and output. The virtual text
/// is what the checker reads; it is part of the version here.
pub fn version_text(file: crate::prelude::Node) -> String {
    use crate::ast::{
        source_file_content_mapper_transform_identity, source_file_original_text,
        source_file_span_map, source_file_text,
    };
    let original = source_file_original_text(file);
    let identity = source_file_content_mapper_transform_identity(file);
    // Upstream (Go incremental/programtosnapshot.go:94) is original text plus
    // transform identity. Only a file whose expansion read other modules can
    // change without either changing, so only it adds the virtual text; every
    // other mapped file keeps the upstream version and buildinfo.
    if source_file_span_map(file).is_some_and(|map| !SpanMap::dependencies(Some(map)).is_empty()) {
        format!("{original}\x00{identity}\x00{}", source_file_text(file))
    } else {
        format!("{original}\x00{identity}")
    }
}

#[cfg(test)]
#[path = "mapper_deps_tests.rs"]
mod tests;
