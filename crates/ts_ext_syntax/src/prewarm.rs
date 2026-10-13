//! Expand a project's Sweetener files before the host asks for them.
//!
//! The content mapper process lives as long as the language server or the
//! `tsgo` run. On `openProject` it starts this in the background: every
//! `.sts`/`.stsx` under the config's directory is read and expanded on a
//! few worker threads, which fills the expansion memo (and the macro-module
//! reads behind it). A later `transform` of the same file and content is a
//! memo hit, or waits for the worker already expanding it. The memo checks
//! every module a result read, so warming can never serve stale output;
//! a file edited since is simply expanded again.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Directories never walked: dependencies, outputs, and dot-directories.
const SKIP: &[&str] = &["node_modules", "dist", "build", "target", "out"];

/// Files past this size are left to the request that needs them.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Walked files at most, so a misconfigured root cannot run away.
const MAX_FILES: usize = 5000;

/// The Sweetener files under `root`, sorted.
#[must_use]
pub fn project_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !name.starts_with('.') && !SKIP.contains(&name.as_ref()) {
                    stack.push(path);
                }
            } else if kind.is_file()
                && crate::sweetener_matches(&name)
                && entry.metadata().is_ok_and(|meta| meta.len() <= MAX_BYTES)
            {
                out.push(path);
                if out.len() >= MAX_FILES {
                    out.sort();
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

/// Expand every file of `files` (check phase) on up to `threads` workers.
/// Returns how many were expanded.
pub fn prewarm(files: Vec<PathBuf>, threads: usize) -> usize {
    let queue = Mutex::new(files.into_iter());
    let done = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().ok().and_then(|mut queue| queue.next());
                    let Some(path) = next else {
                        break;
                    };
                    let Ok(code) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    let filename = path.display().to_string();
                    let _ = crate::expand(crate::ExpandRequest {
                        filename: &filename,
                        code: &code,
                        phase: crate::Phase::Check,
                    });
                    done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    done.into_inner()
}

/// Worker count for a background prewarm: half the cores, at most four, so
/// the requests the host is making keep a core.
#[must_use]
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |cores| cores.get() / 2)
        .clamp(1, 4)
}

/// Start [`prewarm`] of the project whose config is `config_file` (or the
/// directory itself) on a background thread.
pub fn prewarm_in_background(config_file: &str) -> Option<std::thread::JoinHandle<usize>> {
    if std::env::var_os("TSC_EXT_NO_PREWARM").is_some() || config_file.is_empty() {
        return None;
    }
    let path = Path::new(config_file);
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    std::thread::Builder::new()
        .name("tsc-ext-prewarm".to_owned())
        .spawn(move || prewarm(project_files(&root), default_threads()))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::{prewarm, project_files};

    #[test]
    fn project_files_skip_dependencies_and_outputs() {
        let root = std::env::temp_dir().join(format!("prewarm-files-{}", std::process::id()));
        for dir in ["src/deep", "node_modules/pkg", "dist", ".cache"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "src/a.sts",
            "src/deep/b.stsx",
            "src/c.ts",
            "node_modules/pkg/d.sts",
            "dist/e.sts",
            ".cache/f.sts",
        ] {
            std::fs::write(root.join(file), "const x = 1;\n").unwrap();
        }
        let found: Vec<String> = project_files(&root)
            .iter()
            .map(|path| path.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        assert_eq!(found, vec!["src/a.sts", "src/deep/b.stsx"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A warmed file answers from the memo with exactly what a fresh
    /// expansion gives; an edit after warming is expanded again.
    #[test]
    fn prewarmed_expansion_equals_a_fresh_one() {
        let root = std::env::temp_dir().join(format!("prewarm-hit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("m.sts"),
            "export syntax one:expr {\n  rule { one() } => { 1 }\n}\n",
        )
        .unwrap();
        let main = root.join("main.sts");
        let code = "import { one } from \"./m.sts\" for syntax;\nconst a = one();\n";
        std::fs::write(&main, code).unwrap();
        assert_eq!(prewarm(project_files(&root), 2), 2);
        let filename = main.display().to_string();
        let request = |code: &str| {
            crate::expand(crate::ExpandRequest {
                filename: &filename,
                code,
                phase: crate::Phase::Check,
            })
        };
        let warmed = request(code);
        assert!(warmed.code.contains("const a = 1;"), "{}", warmed.code);
        ts_ext_sweetener::clear_caches();
        assert_eq!(warmed, request(code));
        let edited = request("import { one } from \"./m.sts\" for syntax;\nconst b = one();\n");
        assert!(edited.code.contains("const b = 1;"), "{}", edited.code);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
