//! Time Sweetener expansion of real files.
//!
//! `cargo run --release -p ts_ext_sweetener --example expand_bench -- [--runs N] <file>...`
//!
//! Per file, each a median of N runs:
//! - `cold`: caches cleared, so macro modules are read and the file expanded.
//! - `modules warm`: the file changed (a trailing comment), modules cached.
//! - `memo hit`: the same file again, validated against its probes.
//! - `build`: the build phase, cold.
//!
//! `--fingerprint <file>...` prints one hash of each file's whole expansion
//! (code, segments, origins, traces, diagnostics) in both phases instead,
//! to compare two builds of the expander for identical output.
//!
//! `--edits <file>...` times the edit of one file in an editor: the file is
//! expanded once, then each run expands it with a new comment inserted at
//! the start of a line near its start, middle or end (modules and the
//! previous version cached). Set `SWEETENER_EDIT_PROFILE=<n>` to repeat the
//! middle edit n times (for a sampling profiler).

use std::time::{Duration, Instant};

use ts_ext_sweetener::{ExpandRequest, Phase, clear_caches, expand};

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

fn time(runs: usize, mut before: impl FnMut(), mut f: impl FnMut()) -> Duration {
    median(
        (0..runs)
            .map(|_| {
                before();
                let start = Instant::now();
                f();
                start.elapsed()
            })
            .collect(),
    )
}

fn ms(d: Duration) -> String {
    format!("{:.2}", d.as_secs_f64() * 1000.0)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut runs = 5;
    if let Some(at) = args.iter().position(|a| a == "--runs") {
        runs = args[at + 1].parse().expect("--runs N");
        args.drain(at..at + 2);
    }
    if let Some(at) = args.iter().position(|a| a == "--fingerprint") {
        args.remove(at);
        for file in &args {
            let code = std::fs::read_to_string(file).expect("read");
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for phase in [Phase::Check, Phase::Build] {
                clear_caches();
                let out = expand(ExpandRequest {
                    filename: file,
                    code: &code,
                    phase,
                });
                std::hash::Hash::hash(&format!("{out:?}"), &mut hasher);
            }
            println!("{:016x} {file}", std::hash::Hasher::finish(&hasher));
        }
        return;
    }
    if let Some(at) = args.iter().position(|a| a == "--edits") {
        args.remove(at);
        edits(runs, &args);
        return;
    }
    println!(
        "| file | lines | bytes | cold ms | modules warm ms | memo hit ms | build cold ms | output bytes | diagnostics |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for file in &args {
        let path = std::fs::canonicalize(file).expect("file");
        let filename = path.display().to_string();
        let code = std::fs::read_to_string(&path).expect("read");
        let request = |code: &str, phase| {
            expand(ExpandRequest {
                filename: &filename,
                code,
                phase,
            })
        };
        let first = request(&code, Phase::Check);
        let cold = time(runs, clear_caches, || {
            request(&code, Phase::Check);
        });
        let mut edit = 0usize;
        let warm = time(
            runs,
            || {
                clear_caches();
                request(&code, Phase::Check);
            },
            || {
                edit += 1;
                request(&format!("{code}\n// edit {edit}\n"), Phase::Check);
            },
        );
        request(&code, Phase::Check);
        let hit = time(
            runs,
            || {},
            || {
                request(&code, Phase::Check);
            },
        );
        let build = time(runs, clear_caches, || {
            request(&code, Phase::Build);
        });
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            path.parent()
                .and_then(|dir| dir.file_name())
                .filter(|_| path.file_stem().is_some_and(|stem| stem == "input"))
                .map_or_else(
                    || path.file_name().unwrap().to_string_lossy().into_owned(),
                    |dir| dir.to_string_lossy().into_owned(),
                ),
            code.lines().count(),
            code.len(),
            ms(cold),
            ms(warm),
            ms(hit),
            ms(build),
            first.code.len(),
            first.diagnostics.len()
        );
    }
}

/// The byte offset of the first line start at or after `fraction` of `code`
/// whose line is not blank.
fn line_near(code: &str, fraction: f64) -> usize {
    let target = (code.len() as f64 * fraction) as usize;
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        if offset >= target && !line.trim().is_empty() {
            return offset;
        }
        offset += line.len();
    }
    code.rfind('\n').map_or(0, |at| at + 1)
}

fn edits(runs: usize, files: &[String]) {
    println!("| file | edit at | ms (median of {runs}) |");
    println!("| --- | --- | ---: |");
    let mut serial = 0usize;
    for file in files {
        let path = std::fs::canonicalize(file).expect("file");
        let filename = path.display().to_string();
        let code = std::fs::read_to_string(&path).expect("read");
        let request = |code: &str| {
            expand(ExpandRequest {
                filename: &filename,
                code,
                phase: Phase::Check,
            })
        };
        if let Ok(n) = std::env::var("SWEETENER_EDIT_PROFILE") {
            let at = line_near(&code, 0.5);
            request(&code);
            for _ in 0..n.parse::<usize>().expect("n") {
                serial += 1;
                request(&format!("{}/*e{serial}*/ {}", &code[..at], &code[at..]));
            }
            return;
        }
        for (name, fraction) in [("start", 0.0), ("middle", 0.5), ("end", 0.97)] {
            let at = line_near(&code, fraction);
            clear_caches();
            request(&code);
            let took = time(
                runs,
                || {},
                || {
                    serial += 1;
                    request(&format!("{}/*e{serial}*/ {}", &code[..at], &code[at..]));
                },
            );
            println!(
                "| {} | {name} | {} |",
                path.file_name().unwrap().to_string_lossy(),
                ms(took)
            );
        }
    }
}
