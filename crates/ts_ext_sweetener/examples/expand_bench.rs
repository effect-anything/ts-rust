//! Time Sweetener expansion of real files.
//!
//! `cargo run --release -p ts_ext_sweetener --example expand_bench -- [--runs N] <file>...`
//!
//! Per file, each a median of N runs:
//! - `cold`: caches cleared, so macro modules are read and the file expanded.
//! - `modules warm`: the file changed (a trailing comment), modules cached.
//! - `memo hit`: the same file again, validated against its probes.
//! - `build`: the build phase, cold.

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
