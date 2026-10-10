//! Run TypeGPU build embed against one conformance case and write ours/diff.
//! Does not depend on ts_ext_syntax.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let arg = std::env::args().nth(1).expect("case path");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ts_ext_syntax/conformance/cases")
        .join(arg);
    let input = ["input.ts", "input.tsx", "input.sts"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
        .expect("input");
    let code = fs::read_to_string(&input).unwrap();
    let filename = input.to_string_lossy();
    let built = ts_goport::ext::typegpu::embed_metadata(&filename, &code);
    fs::write(dir.join("ours.ts"), &built).unwrap();
    let reference = fs::read_to_string(dir.join("theirs.ts")).unwrap();
    let left = payload(&reference);
    let right = payload(&built);
    if left == right {
        let _ = fs::remove_file(dir.join("diff.txt"));
        println!("shape");
    } else {
        fs::write(
            dir.join("diff.txt"),
            format!("--- theirs payload\n{left}\n--- ours payload\n{right}\n"),
        )
        .unwrap();
        println!("differ");
    }
}

fn payload(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("name:")
            || line.starts_with("body:")
            || line.starts_with("externals:")
            || line.contains("() =>")
        {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
