//! Where macro-module reads go.
//!
//! An expansion that imports a macro module (`import { m } from "./m.sts" for
//! syntax`) reads that module and canonicalizes its path. By default both go to
//! the disk. A host that keeps its sources in memory (a bundler with a virtual
//! filesystem, a browser playground, a test) installs a [`SourceReader`] once,
//! and every read and `canonicalize` the expander makes, including the ones the
//! file memo replays to validate a cached expansion, goes to it instead.
//!
//! The reader is process-wide, not thread-local: an expansion runs on worker
//! threads of its own (`sweetener_expand::expand`), which a thread-local
//! reader would not reach.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The filesystem operations macro-module resolution needs.
pub trait SourceReader: Send + Sync {
    /// The UTF-8 text at `path`, or `None` when there is no such file.
    fn read(&self, path: &Path) -> Option<String>;

    /// The canonical form of `path`. An error means the path does not exist,
    /// exactly as [`Path::canonicalize`] reports it.
    ///
    /// # Errors
    /// When `path` does not name a file the reader has.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
}

/// The default reader: the real filesystem.
struct Disk;

impl SourceReader for Disk {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }
}

fn slot() -> &'static RwLock<Option<Arc<dyn SourceReader>>> {
    static SLOT: RwLock<Option<Arc<dyn SourceReader>>> = RwLock::new(None);
    &SLOT
}

/// Send every macro-module read and `canonicalize` to `reader` until
/// [`clear_source_reader`] or the next call.
///
/// Cached expansions stay correct across a change of reader: the file memo
/// re-observes every read it depended on through the reader current at lookup
/// time, so a module that the new reader does not have, or has with other
/// text, invalidates the entry.
pub fn set_source_reader(reader: Arc<dyn SourceReader>) {
    if let Ok(mut current) = slot().write() {
        *current = Some(reader);
    }
}

/// Return macro-module resolution to the real filesystem.
pub fn clear_source_reader() {
    if let Ok(mut current) = slot().write() {
        *current = None;
    }
}

fn current() -> Option<Arc<dyn SourceReader>> {
    slot().read().ok().and_then(|current| current.clone())
}

pub(crate) fn read(path: &Path) -> Option<String> {
    match current() {
        Some(reader) => reader.read(path),
        None => Disk.read(path),
    }
}

pub(crate) fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    match current() {
        Some(reader) => reader.canonicalize(path),
        None => Disk.canonicalize(path),
    }
}
