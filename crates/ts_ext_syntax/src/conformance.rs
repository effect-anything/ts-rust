//! Snapshot layout for reference-program comparison.
//!
//! The runner that fills `theirs.ts` and `ours.ts` is not here yet. This
//! module only names the files so both passes write the same shape.

use std::path::{Path, PathBuf};

/// Files of one case. `root` is `conformance/cases/<id>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CasePaths {
    pub root: PathBuf,
}

impl CasePaths {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn input(&self) -> PathBuf {
        self.root.join("input")
    }

    #[must_use]
    pub fn theirs(&self) -> PathBuf {
        self.root.join("theirs.ts")
    }

    #[must_use]
    pub fn ours(&self) -> PathBuf {
        self.root.join("ours.ts")
    }

    #[must_use]
    pub fn diff(&self) -> PathBuf {
        self.root.join("diff.txt")
    }

    /// Their program could not be run. Not a pass.
    #[must_use]
    pub fn theirs_missing(&self) -> PathBuf {
        self.root.join("theirs.ts.missing")
    }

    #[must_use]
    pub fn exists_captured(&self) -> bool {
        self.input().is_symlink() && self.theirs().is_file() && self.ours().is_file()
    }
}

/// `conformance/cases` next to this crate's manifest.
#[must_use]
pub fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance/cases")
}
