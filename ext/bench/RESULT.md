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
