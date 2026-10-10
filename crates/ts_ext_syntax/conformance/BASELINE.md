# Reference-program baselines

Pins: TypeGPU `fc85681e` (`origin/main`). Sweetener `2e78489` (`origin/main`). Captured 2026-10-10. See `pins.json`.

Vendor Sweetener is not that pin (checkout `fix/session-project-expansion`). The operators case therefore copies pinned blobs instead of a vendor symlink. `input.sts` blob `903a6f11a8f76ae1d40b3eb3476a804a64510cec`. Deps (`declarative.sts` `8b802bb98b6011791ed9c9292e4106e0e583e370`, `case.json` `7cbcfadd673162609ef6642e85e89274a2407b5f`, `expected.ts` `9384173c9a3dcd82e85daa94461cdf0bb41fe6b7`) are in `deps/` and `deps.blob`. `input.pin` is `2e78489`.

Sweetener `theirs.ts` is a live `session.transform` of pinned `acceptance.sts` (compiler built in a detached worktree at `2e78489`, not the vendor checkout). Diagnostics empty. It differs from `deps/expected.ts` by two leading newlines; that is recorded in `theirs.ts.note`. The fixture oracle is stale relative to the program.

TypeGPU `input.test.ts` symlinks `vendor/TypeGPU/.../use-gpu-directive.test.ts` (pin `fc85681e`). The parent `theirs.ts` is a live `babelTransform` of the first test whose snapshot contains `__tsover_add`: title `"use gpu" marked arrow function, assigned to a const` (`test.title`). Live output matches that inline snapshot.

Subcases under `use-gpu-directive/<slug>/` are the other `describe` blocks in that file. Each `input.ts` is the `const code` string passed to `babelTransform` (no separate fixture file), with a first-line extraction comment. Each `theirs.ts` is a live `babelTransform` from `test/transform.ts` (not a copied snapshot). `typescript-syntax/as-type` is the one `__tsover_` rewrite in `typescript-syntax.test.ts`, captured the same way. Rollup snapshots in the same describes were not re-run; babel is the operator-rewrite oracle, matching the parent case.

Operators present in those sources: `+`, `*`, `%`, `+=`. No test in the captured files uses `-`, `/`, `-=`, `*=`, `/=`, or `%=`. Those were not invented.

Skipped (not the operator-rewrite baseline): `obfuscation.test.ts`, `auto-naming.test.ts`, `tgsl-transpiling.test.ts`, `bun.test.ts`, `regex.test.ts`. Also not captured: `aliasing.test.ts`, `embedded-metadata.test.ts`, `nested-externals.test.ts`, `parser-options.test.ts` (no `__tsover_` rewrites).

`ours.ts` is intentionally absent. This capture does not run `ts_ext_syntax::expand`.

## Sweetener corpus (pin `2e78489`)

Unit tests are not the baseline. Pin `2e78489` has 172 `*.test.ts` files. Those are their unit tests, not expansion oracles, and they are not snapshotted. The baseline is fixtures + language-tour + macro-suite (plus the small macro programs below).

Sources were copied from commit `2e78489` (`git show 2e78489:<path>`), not from the vendor checkout (`fix/session-project-expansion`). `input.pin` is `2e78489`. `deps.blob` records `git rev-parse 2e78489:<path>` for the entry and each copied dep.

Live `theirs.ts` is the pin compiler, built in detached worktree `target/sweetener-baseline-2e78489` (`pnpm install --frozen-lockfile` and `tsc6 -b packages/compiler packages/cli`). Not `vendor/sweetener/packages/compiler/dist`. Command: `sweetener expand <entry> -p <generated tsconfig>` whose `files` are the entry plus imported macro modules (`declarative.sts` / `macros.sts` and transitive imports). Output is CLI stdout plus a trailing newline when the compiler omitted one. Diagnostics on stderr abort the case.

| set | cases | live `theirs.ts` | `theirs.ts.missing` |
| --- | --- | --- | --- |
| `fixtures/**/case.json` | 17 | 17 | 0 |
| examples (below) | 58 | 56 | 2 |
| `samples/external` macro programs | 1 | 1 | 0 |

Fixture breakdown: playground 12 (`adt`, `core-rewrites`, `csp`, `currying`, `do-notation`, `implicit-return`, `multi-part-methods`, `new-language`, `operators`, `protocols`, `rewritten-if`, `threading`), real-world 4 (`debug`, `jsx-control-flow`, `match`, `signals`), conformance 1 (`hygiene/semantic-suite`). Case id `sweetener/fixtures/<path-under-fixtures>`. The earlier `sweetener/playground/operators` directory was left in place and not rewritten.

Examples captured: language-tour `recovered/*/main` 21, language-tour `new/*/main` 25, `library-macros` 4 (`drizzle-query`, `drizzle-schema`, `effect-do`, `effect-service`), `macro-suite/showcase` 1, `react-memoization/src` 3 (`fine-jsx-implementation`, `function-shadow`, `memoized`), `vite-react/src` 2 (`main`, `react-hooks`). `react-memoization` and `vite-react` are included only as those small `.sts` / `.stsx` macro demos, not as app shells.

Empty expansion (not a compiler crash): `sweetener/examples/macro-suite/macros` and `sweetener/examples/react-memoization/src/fine-jsx`. Both are macro-definition modules. CLI `expand` and `session.transformSync` returned empty code and no diagnostics. `explain` says "This position produced no generated TypeScript." Status stays `pending`; reason is in `theirs.ts.missing`. `macro-suite/showcase` (the program that imports those macros) did expand.

Playground fixture `theirs.ts` is not byte-identical to `deps/expected.ts` (formatting, and operators differs by two leading newlines). Real-world fixtures differ from `expected.ts` / `expected.tsx` by a single trailing newline. `theirs.ts` is the program. Operators records that in `theirs.ts.note`.

Skipped full apps (no small standalone macro entry captured): `examples/astro`, `examples/bun`, `examples/deno`, `examples/next`, `examples/nuxt`, `examples/solid-start`, `examples/sveltekit`, `examples/tanstack-start`, `examples/vite-preact`, `examples/vite-solid`, `examples/vite-svelte`, `examples/vite-vanilla`, `examples/vite-vue`. `examples/vite-react` app shell skipped except `src/main.stsx` and `src/react-hooks.sts`. `examples/react-memoization` HTML/vite shell skipped except the four `src/*.stsx` files.

`samples/external`: `default-project` has `.sts` (`main.sts`, `macros.sts`, `workflow.sts`) and is captured as `sweetener/samples/external/default-project/src/main`. `javascript-project` is plain `.js`, not `.sts` / `.stsx`, skipped. `macro-editor` and `project-graph` are editor/tooling runners without macro source, skipped.

