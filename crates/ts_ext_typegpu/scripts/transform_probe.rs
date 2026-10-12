//! Build-phase output for a directory of snippets.
//!
//! The counterpart of `scripts/ext-bench/upstream-typegpu.mjs`, so a
//! behaviour of the pinned TypeGPU plugin can be diffed against this port
//! before it becomes a conformance case.
//!
//! usage: cargo run -q --example transform_probe -- <dir>

use std::fs;

use ts_ext_typegpu::TypegpuSession;

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: transform_probe <dir>");
        std::process::exit(2);
    };
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{dir}: {error}"))
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".ts"))
        .collect();
    names.sort();
    for name in names {
        let path = format!("{dir}/{name}");
        let code = fs::read_to_string(&path).unwrap_or_default();
        // Upstream's plugin default. The probe dirs are behaviours, not
        // captured cases, so they are asked with auto-naming on.
        let built = TypegpuSession::build().transform(&path, &code);
        println!("### {name}");
        for diagnostic in &built.diagnostics {
            println!("!!! {}", diagnostic.message);
        }
        println!("{}", built.code);
    }
}
