//! Incremental expansion: an edited file expanded with the item log of its
//! previous version equals a fresh expansion of the edited text, on every
//! case and under random edits.

use ts_ext_sweetener::{ExpandRequest, Expansion, Phase, expand_uncached, incremental_stats};

fn run(filename: &str, code: &str, incremental: bool) -> Expansion {
    expand_uncached(
        ExpandRequest {
            filename,
            code,
            phase: Phase::Check,
        },
        incremental,
    )
}

/// A small deterministic generator, so a failure names its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn line_starts(code: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(code.match_indices('\n').map(|(at, _)| at + 1))
        .filter(|at| *at < code.len())
        .collect()
}

/// One random edit of `code`, the kind an editor makes, and what it was.
fn edit(code: &str, rng: &mut Rng, serial: usize) -> (String, String) {
    let starts = line_starts(code);
    if starts.is_empty() {
        return (format!("{code}\n// e{serial}\n"), "append".to_owned());
    }
    let line = starts[rng.below(starts.len())];
    let line_end = code[line..]
        .find('\n')
        .map_or(code.len(), |at| line + at + 1);
    match rng.below(7) {
        0 => (
            format!("{}/*e{serial}*/ {}", &code[..line], &code[line..]),
            format!("comment at {line}"),
        ),
        1 => (
            format!("{}\n{}", &code[..line], &code[line..]),
            format!("blank line at {line}"),
        ),
        2 => (
            format!("{}{}", &code[..line], &code[line_end..]),
            format!("delete line at {line}"),
        ),
        3 => (
            format!("{}{}", &code[..line_end], &code[line..]),
            format!("duplicate line at {line}"),
        ),
        4 => {
            // Change one letter of a word.
            let letters: Vec<usize> = code
                .char_indices()
                .filter(|(_, ch)| ch.is_ascii_lowercase())
                .map(|(at, _)| at)
                .collect();
            if letters.is_empty() {
                return (format!("{code}\n"), "newline".to_owned());
            }
            let at = letters[rng.below(letters.len())];
            let ch = (b'a' + rng.below(26) as u8) as char;
            (
                format!("{}{ch}{}", &code[..at], &code[at + 1..]),
                format!("letter {ch} at {at}"),
            )
        }
        5 => (
            format!(
                "{}const e{serial} = {serial};\n{}",
                &code[..line],
                &code[line..]
            ),
            format!("statement at {line}"),
        ),
        _ => {
            // Type a character in the middle of a line.
            let at = line + rng.below(line_end - line);
            let at = (0..=at)
                .rev()
                .find(|at| code.is_char_boundary(*at))
                .unwrap_or(0);
            let ch = [' ', ';', '(', ')', 'x', '{', '}', '<', '>', '1'][rng.below(10)];
            (
                format!("{}{ch}{}", &code[..at], &code[at..]),
                format!("type {ch:?} at {at}"),
            )
        }
    }
}

fn first_difference(a: &str, b: &str) -> String {
    let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let from = at.saturating_sub(80);
    format!(
        "at byte {at}:\nincremental: {:?}\nfull:        {:?}",
        a.get(from..(at + 80).min(a.len())).unwrap_or(""),
        b.get(from..(at + 80).min(b.len())).unwrap_or("")
    )
}

/// Expand `code`, then `edits` random edits of it one after another; each
/// edited version, expanded with the log of the version before, must equal
/// a fresh expansion.
fn check_edits(filename: &str, code: &str, seed: u64, edits: usize) {
    let mut rng = Rng(seed);
    let mut code = code.to_owned();
    assert_eq!(
        run(filename, &code, true),
        run(filename, &code, false),
        "{filename}: unedited"
    );
    for serial in 0..edits {
        let (edited, what) = edit(&code, &mut rng, serial);
        let incremental = run(filename, &edited, true);
        let full = run(filename, &edited, false);
        if incremental != full {
            let dump = std::env::temp_dir().join("sweetener-incremental-failure");
            let _ = std::fs::create_dir_all(&dump);
            let _ = std::fs::write(dump.join("before.sts"), &code);
            let _ = std::fs::write(dump.join("after.sts"), &edited);
            panic!(
                "{filename} seed {seed} edit {serial} ({what}): incremental differs; code {}\n\
                 segments equal: {}, diagnostics equal: {}, traces equal: {}, origins equal: {}\n\
                 diagnostics incremental {:?}\nfull {:?}\nfirst trace difference {:?}",
                first_difference(&incremental.code, &full.code),
                incremental.segments == full.segments,
                incremental.diagnostics == full.diagnostics,
                incremental.traces == full.traces,
                incremental.origins == full.origins,
                incremental.diagnostics,
                full.diagnostics,
                incremental
                    .traces
                    .iter()
                    .zip(&full.traces)
                    .find(|(a, b)| a != b)
                    .map_or_else(
                        || format!("lengths {} {}", incremental.traces.len(), full.traces.len()),
                        |pair| format!("{pair:?}")
                    ),
            );
        }
        code = edited;
    }
}

fn cases() -> Vec<(String, String)> {
    let dir = format!("{}/cases", env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        for name in ["input.sts", "input.stsx"] {
            let file = path.join(name);
            if let Ok(code) = std::fs::read_to_string(&file) {
                out.push((file.display().to_string(), code));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_case_expands_the_same_incrementally_after_random_edits() {
    let cases = cases();
    assert!(cases.len() > 50, "{}", cases.len());
    // `SWEETENER_FUZZ_EDITS=<n>` runs longer chains.
    let edits = std::env::var("SWEETENER_FUZZ_EDITS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(6);
    let seed: u64 = std::env::var("SWEETENER_FUZZ_SEED")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0x5eed);
    for (index, (filename, code)) in cases.iter().enumerate() {
        check_edits(filename, code, seed + index as u64, edits);
    }
}

fn bench_file() -> (String, String) {
    let file = format!(
        "{}/../../ext/bench/fixture/bench.stsx",
        env!("CARGO_MANIFEST_DIR")
    );
    let code = std::fs::read_to_string(&file).unwrap();
    (file, code)
}

#[test]
fn an_edit_of_bench_replays_the_items_it_did_not_touch() {
    let (file, code) = bench_file();
    run(&file, &code, true);
    let (hits, _) = incremental_stats();
    let at = code.len() / 2;
    let at = code[at..].find('\n').map_or(at, |n| at + n + 1);
    let edited = format!("{}/*edit*/ {}", &code[..at], &code[at..]);
    assert_eq!(run(&file, &edited, true), run(&file, &edited, false));
    let (after, printed) = incremental_stats();
    assert!(after - hits > 0, "no item replayed ({printed} printed)");
}

/// The long run: many random edits of the benchmark file. `cargo test
/// --release -p ts_ext_sweetener --test sweetener_incremental -- --ignored`
/// (`SWEETENER_FUZZ_SEEDS=<n>` chains of 12 edits).
#[test]
#[ignore = "slow; run in release"]
fn random_edits_of_bench_expand_the_same_incrementally() {
    // `SWEETENER_FUZZ_FILE=<path>` fuzzes another file the same way.
    let (file, code) = match std::env::var("SWEETENER_FUZZ_FILE") {
        Ok(file) => {
            let code = std::fs::read_to_string(&file).unwrap();
            (file, code)
        }
        Err(_) => bench_file(),
    };
    let seeds: u64 = std::env::var("SWEETENER_FUZZ_SEEDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(4);
    for seed in 0..seeds {
        check_edits(&file, &code, 0xbe7c + seed, 12);
    }
}
