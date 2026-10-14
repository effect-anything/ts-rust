# ts_ext_syntax: tsc-ext and the @effect-x/tsc Node API

The published `@effect-x/tsc` npm package has two bins and a Node API:

- `tsc`: the compiler (crates/ts_goport).
- `tsc-ext`: this crate's binary, `src/bin/tsc_ext.rs`.
- `import { Transformer, formatDiagnostics } from "@effect-x/tsc/ext"`: this crate's Node addon
  (`src/node.rs`) behind the JavaScript wrapper in `js/`.

## Options

One shape, read by all three entry points: the `options` object of the tsconfig `contentMappers`
entry for `@effect-x/tsc`.

```json
{
  "contentMappers": [
    {
      "package": "@effect-x/tsc",
      "extensions": [".sts", ".stsx"],
      "options": {
        "sweetener": { "rewriteImports": false },
        "typegpu": { "autoNaming": true, "earlyPruning": true, "forceTgpuAlias": "tgpu" }
      }
    }
  ]
}
```

| option | default | |
| --- | --- | --- |
| `sweetener.rewriteImports` | `false` | Drop `.sts`/`.stsx` from module specifiers in the Node API output. |
| `typegpu.autoNaming` | `true` | Name TypeGPU resources after their binding. |
| `typegpu.earlyPruning` | `true` | Skip a file that mentions none of `"use gpu"`, `tgpu`, `typegpu` (as unplugin-typegpu and the `tsc` emit hook do). |
| `typegpu.forceTgpuAlias` | none | The name `tgpu` is imported as. |

Unknown keys and wrong types are errors. `src/config.rs` reads the nearest `tsconfig.json`
above a file (JSONC; a relative `extends` is followed when the config has no `contentMappers`).

- `tsc` sends the entry's options with `openProject`; the content mapper validates them,
  reports invalid ones as option diagnostics, and keeps them per project handle.

**Options shape the build output only; the check treats files as TypeScript.** This is the
design, not a gap.

One pipeline turns the options into output (`TransformSession::from_options`): the Node API
`Transformer`, the Node API `Project` and `tsc-ext expand --stage bundle` print the same bundler
text for the same options (`tests/options_parity.rs`). The mapper's `transform` (what `tsc`
type-checks) and `tsc-ext expand` (stage `tsc`, the default) are the check view: Sweetener only.
No current option changes it: `tsc` keeps the author's operators in `"use gpu"` code and does not
emit `.sts` files, so `typegpu.*` and `sweetener.rewriteImports` only shape runtime JavaScript.
- `tsc-ext expand` and `eval` read `--tsconfig`, else the nearest tsconfig.json.
- `Transformer` reads its `tsconfig` option, else the nearest tsconfig.json of each file
  (`tsconfig: false` reads none). Explicit options override single fields.

## tsc-ext

```
tsc-ext                       content mapper on stdin/stdout (started by tsc)
tsc-ext expand <file>...      what a file becomes, with its diagnostics
tsc-ext eval [code|-]         the same for code given inline or on stdin
tsc-ext explain <SWRcode>     what a diagnostic means and how to fix it
tsc-ext cheatsheet [topic]    syntax quick reference, options, diagnostics
tsc-ext help [command]        help; every command also takes --help
```

`expand` prints the text of a stage to stdout: `--stage tsc` (default) is what `tsc` type-checks
(Sweetener expansion, check phase); `--stage bundle` is the bundler output (the `Transformer`
result under the tsconfig's options: Sweetener, then tsover, then the TypeGPU wrap). `--only
sweetener|typegpu|tsover` runs one pass. Diagnostics go to stderr with a code frame, related
locations and a `tsc-ext explain` hint (`--format tsc` prints `file(line,col): error SWRxxxx:
message` instead). `--trace` prints every macro use, each rule tried, where and why it stopped,
and the replacement. `--diff` prints a unified diff from input to output, `--json` one object
per file, `--map[=file]` the v3 source map (inline by default, so stdout stays one file that tools
and browsers read). Exit codes: 0 ok (warnings allowed), 1 error diagnostics, 2 usage, 3 I/O or
config. `eval` takes the same flags plus `--jsx`, `--lang ts|tsx` and `--as <path>`.

```
$ tsc-ext expand src/step.sts --trace
trace: src/step.sts (stage tsc)
  1. 6:1 unless `unless done work(); …`
       ✗ rule 1/2 (line 3) stopped at 6:8 `done`, expected `( ... )`
       ✗ rule 2/2 (line 4) stopped at 6:13 `work`, expected `then`
src/step.sts:6:13: error SWR4001: No rule for macro unless accepted this input: 2 rule attempt(s).
  ...
  help: tsc-ext explain SWR4001
```

The cheatsheet, the options reference and the diagnostics catalog are built into the binary
(`ext/docs/{cheatsheet,options,diagnostics}.md`; `--raw` prints the Markdown).
`ext/docs/diagnostics.md` is generated from `ts_ext_sweetener::catalog`
(`TSC_EXT_UPDATE=1 cargo test -p ts_ext_sweetener --test catalog`). Command snapshots are in
`tests/fixtures` (`TSC_EXT_UPDATE=1 cargo test -p ts_ext_syntax --test cli_snapshots`).

`tsc` starts the mapper from the package's `typescript.contentMapper.exec` (`./bin/tsc-ext`), run
in the package directory. When the platform package `@effect-x/tsc-<platform>-<arch>` is
installed, this port's `tsc` starts its native `lib/tsc-ext` instead, with no Node process. That is
a host convention, not a manifest field: for a one-entry `exec` of `./bin/<name>`, `tsc` looks for
`<package>-<platform>-<arch>/lib/<name>` resolved from the mapper package
(`crates/ts_goport/src/frontend/tsoptions/content_mappers.rs`, `native_exec`). `bin/tsc-ext` is a Node
launcher of the same binary, the fallback when the platform package is missing and the command a
shell runs; here (development) `bin/tsc-ext` is a
link to `target/debug/tsc-ext` (`ext/examples/link-debug.mjs`). `tsc` runs content mappers only
with `--runExternalCode`.

## Node API

`import { Transformer, formatDiagnostics } from "@effect-x/tsc/ext"` (the package root is
TypeScript's). See `js/index.d.ts`. One pipeline for dev and build, as a bundler plugin needs it: Sweetener
expansion for `.sts`/`.stsx`, then TypeGPU in its build form (operators rewritten, `"use gpu"`
functions wrapped with `__TYPEGPU_META__`, as unplugin-typegpu does). There is no mode option:
the build form is also what a dev server must serve, since the TypeGPU runtime reads
`__TYPEGPU_META__` to compile shaders. A file not to transform is simply not passed in.

`Result.composeMap(map)` takes a map of a later transform of `result.code` and returns one onto
the `.sts`: exact for copied text (the same character), the start of the author text for macro
output, unmapped for text the expansion introduced (`src/compose.rs`). `map` is never null: an
unchanged file gets the identity map and composes through it, so a composed map always names the
original path. `errors` and `warnings` split `diagnostics` (warnings: every non-error category).

`.sjs` and `.sjsx` (Sweetener over JavaScript) are not supported. `transform` throws, `tsc-ext
expand` exits 1, and the content mapper answers `transform` with an error.

Build the addon here (development):

```sh
npm install
npx napi build --platform --no-js --dts native.d.ts --output-dir . --release
```

It writes `tsc-ext.<platform>-<arch>.node` (not tracked). `js/native.cjs` loads it, or in the
published package `lib/tsc-ext.node` of the platform package. `TSC_EXT_NODE=<path>` overrides.

## Bundler plugin (`@effect-x/tsc/unplugin`)

```js
import tsc from "@effect-x/tsc/unplugin/vite"; // also /rollup, /rolldown, /webpack, /rspack
export default { plugins: [tsc(/* { tsconfig, sweetener, typegpu } */), react()] };
```

The plugin opens a `Project` on the tsconfig and `load`s every file it owns
(tsconfig `include`/`files`, minus node_modules and declarations), so the
bundler's own TS strip sees the rewritten TS/TSX with an exact map back to the
`.sts`/`.ts` source. `.sts`/`.stsx` keep their path; an extensionless relative
import finds them. Macro modules are watched; a change invalidates the files
that use them (HMR in Vite).

- Vite 8 / Rolldown / Rollup: complete. Under Vite the plugin strips `.sts`/`.stsx`
  with Vite's oxc options (vite:oxc picks the language by extension only).
- webpack / rspack: the loaded code is TypeScript. Add a TS loader rule for
  `/\.stsx?$/` (ts-loader, `builtin:swc-loader`).
- esbuild / bun: unsupported for `.sts`/`.stsx` until type erasure; `.ts` files work.
- The tsover runtime is not injected: `typegpu` imports `tsover-runtime`; an app
  using tsover without typegpu imports it once at its entry.

Manual mode: `Transformer` from `@effect-x/tsc/ext` (above).
