//! Where an tsc-ext transform spends its time, per stage.
//!
//! `cargo run --release -p ts_ext_syntax --example transform_stages -- [--runs N] <file>...`
//!
//! Mapper path (`protocol::transform_file`, what `tsgo` and the LSP call):
//! expand (cold, caches cleared), span-map segments, mappings JSON,
//! expansion regions; then the whole call again with the expansion cached.
//! Plugin path (`TransformSession::transform`, what vite calls): build-phase
//! expand plus `TypeGPU`, specifier rewrite, retarget and the v3 source map.

use std::time::{Duration, Instant};

use ts_ext_sweetener::clear_caches;
use ts_ext_syntax::{ExpandRequest, Phase, TransformSession, expand, protocol};

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

fn time<T>(runs: usize, mut before: impl FnMut(), mut f: impl FnMut() -> T) -> Duration {
    median(
        (0..runs)
            .map(|_| {
                before();
                let start = Instant::now();
                std::hint::black_box(f());
                start.elapsed()
            })
            .collect(),
    )
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut runs = 5;
    if let Some(at) = args.iter().position(|a| a == "--runs") {
        runs = args[at + 1].parse().expect("--runs N");
        args.drain(at..at + 2);
    }
    println!(
        "| file | expand cold | segments | mappings json | regions | mapper total cold | mapper total cached | plugin (build) cold | plugin cached |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for file in &args {
        let path = std::fs::canonicalize(file).expect("file");
        let name = path.display().to_string();
        let code = std::fs::read_to_string(&path).expect("read");
        let run_expand = || {
            expand(ExpandRequest {
                filename: &name,
                code: &code,
                phase: Phase::Check,
            })
        };
        let expand_cold = time(runs, clear_caches, run_expand);
        let expanded = run_expand();
        let segments = time(runs, || {}, || expanded.span_map_segments());
        let segs = expanded.span_map_segments();
        let json = time(runs, || {}, || ts_ext_syntax::session::mappings_json(&segs));
        let regions = time(runs, || {}, || protocol::expansion_regions(&expanded));
        let mapper_cold = time(runs, clear_caches, || {
            protocol::transform_file(&name, &code)
        });
        let mapper_cached = time(runs, || {}, || protocol::transform_file(&name, &code));
        let session = TransformSession::new();
        let plugin_cold = time(runs, clear_caches, || session.transform(&name, &code));
        let plugin_cached = time(runs, || {}, || session.transform(&name, &code));
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            path.file_name().unwrap().to_string_lossy(),
            ms(expand_cold),
            ms(segments),
            ms(json),
            ms(regions),
            ms(mapper_cold),
            ms(mapper_cached),
            ms(plugin_cold),
            ms(plugin_cached)
        );
    }
}
