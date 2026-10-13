# Expansion bench

Times for `fixture/`. Not a case comparison.

## Tool

`target/debug/ext-syntax transform`, three runs, debug build. RSS is the maximum resident set from `/usr/bin/time -l`.

| input | bytes | median | max RSS | unchanged |
| --- | ---: | ---: | ---: | --- |
| `plain.ts` | 85,850 | 0.0084 s | 8.4 MB | yes |
| `mentioned.ts` | 23,070 | 0.0078 s | 8.2 MB | yes |
| `bench.stsx` | 73,938 | 2.55 s | 12.6 MB | no, expanded |

`plain.ts` does not enter the expander. `mentioned.ts` names `tgpu.fn` and comes back unchanged: this command is the Sweetener process, and TypeGPU does not run without `"use gpu"`.

## Compiler

`target/debug/tsgo --noEmit`, three runs. The second project adds one `"use gpu"` file and no `.sts` file.

| project | median | max RSS | exit |
| --- | ---: | ---: | ---: |
| plain `tsgo` | 1.32 s | 109 MB | 0 |
| plain plus one `"use gpu"` file | 1.38 s | 109 MB | 0 |

The added file is inside the noise of this debug build. `tsgo` does not expand `bench.stsx` unless a content mapper is registered, so that file is timed with `ext-syntax`, not with `tsc`.

## Release bench (`ext/bench/run.sh`), 2026-10-13

`run.sh [copies]` builds release binaries, then times in process with
`examples/expand_bench` (median of 5): `cold` clears the caches; `modules warm`
edits the file so only macro-module reads are cached; `memo hit` repeats the
same file; `build` is the build phase, cold. `big.stsx` is `bench.stsx`'s
imports plus four copies of its body (11,625 lines). Apple Silicon, mostly idle
for the first table; the A/B below ran under load average ~50 and is
interleaved.

| file | lines | cold ms | modules warm ms | memo hit ms | build cold ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| bench.stsx, before | 2,913 | 456 | 460 | 0.23 | 456 |
| big.stsx, before | 11,625 | 2,547 | 2,554 | 0.52 | 2,529 |
| bench.stsx, after | 2,913 | 387 | 389 | 0.20 | 390 |
| big.stsx, after | 11,625 | 1,629 | 1,623 | 0.52 | 1,633 |

Before, 4× the input cost 5.6× the time: every form head rescanned the whole
file for its `for syntax` imports. They are now read once per file
(`sweetener_forms::with_file_imports`), and 4× costs 4.2×. Interleaved A/B
under load, big.stsx cold, three rounds: before 3,735 / 3,461 / 2,964 ms,
after 2,065 / 2,352 / 2,691 ms (median −32%).

Macro-module reads are not a measurable share (modules warm ≈ cold). The
remaining profile is allocation: capture maps cloned while the matcher
backtracks.

The largest Sweetener cases are 48–155 lines and expand in 0.2–4 ms cold;
a memo hit is under 0.1 ms.

`ext-syntax transform` (process, release, protocol and v3 map included,
under load): bench.stsx 0.53 s, big.stsx 2.38 s.

`npx vite build` of `ext/examples/vite`: 1.59 s median of 3 (idle), 4.25 s
under load.
