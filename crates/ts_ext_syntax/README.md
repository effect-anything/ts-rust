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

One pipeline turns the options into output (`TransformSession::from_options`): the Node API, the
mapper's `plugin` method and `sweetener --expand --build` print the same bundler text for the same
options (`tests/options_parity.rs`). The mapper's `transform` (what `tsc` type-checks) and plain
`sweetener --expand` are the check view: Sweetener only. No current option changes it: `tsc` keeps
the author's operators in `"use gpu"` code and does not emit `.sts` files, so `typegpu.*` and
`sweetener.rewriteImports` only shape runtime JavaScript.
- `tsc-ext sweetener --expand` reads `--tsconfig`, else the nearest tsconfig.json.
- `Transformer` reads its `tsconfig` option, else the nearest tsconfig.json of each file
  (`tsconfig: false` reads none). Explicit options override single fields.

## tsc-ext

```
tsc-ext                       content mapper on stdin/stdout (started by tsc)
tsc-ext sweetener --expand <file> [--tsconfig <path>] [--map] [--build]
tsc-ext help
```

`sweetener --expand` prints the text `tsc` type-checks (Sweetener expansion, check phase) to
stdout. Diagnostics go to stderr as `file(line,col): error SWRxxxx: message` (warnings as
`warning`), related locations indented below; an error exits 1. `--map` appends the v3 source map
as an inline `//# sourceMappingURL=data:application/json;base64,...` comment, so stdout stays one
file that tools and browsers read. `--build` prints the bundler output instead (the
`Transformer` result under the tsconfig's options).

`tsc` starts the mapper from the package's `typescript.contentMapper.exec` (`./bin/tsc-ext`), run
in the package directory. The published package also declares
`"platformExec": "@effect-x/tsc-${platform}-${arch}/lib/tsc-ext"`: `tsc` resolves that platform
package from `@effect-x/tsc` and starts its native `lib/tsc-ext` directly (a PORT addition to the
manifest; `crates/ts_goport/src/frontend/tsoptions/content_mappers.rs`). `bin/tsc-ext` is a Node
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
sweetener --expand` exits 1, and the content mapper answers `transform` with an error.

Build the addon here (development):

```sh
npm install
npx napi build --platform --no-js --dts native.d.ts --output-dir . --release
```

It writes `tsc-ext.<platform>-<arch>.node` (not tracked). `js/native.cjs` loads it, or in the
published package `lib/tsc-ext.node` of the platform package. `TSC_EXT_NODE=<path>` overrides.
