# @effect-x/tsc

A Rust port of the TypeScript 7 compiler (`tsc`).

It is a direct port of Microsoft's native TypeScript compiler, which is written in Go. On the
projects we test, it gives the same diagnostics and output as that compiler, and it is faster.

This is an early release. Report problems at https://github.com/effect-anything/tsc/issues.

## Use

```sh
npm install -D @effect-x/tsc
npx tsc -p tsconfig.json
```

`@effect-x/tsc` takes the same options as `tsc`. `tsc --version` prints the TypeScript version that it
ports (7.1.0-dev), not the npm version.

## Effect diagnostics

`@effect-x/tsc` has the [Effect](https://effect.website) language service diagnostics built in (codes
377xxx), so an Effect project needs no second compiler. They come from the same check as the
TypeScript diagnostics, and the language server shows them too. They run only when the tsconfig has
the plugin, as with `@effect/language-service`:

```json
{ "compilerOptions": { "plugins": [{ "name": "@effect/language-service" }] } }
```

The rules, options and `@effect-diagnostics` comments are a port of
[Effect-TS/tsgo](https://github.com/Effect-TS/tsgo) 0.51.1. The language server also has its editor
features: the Effect hover (with the Layer summary), inlay hints, the auto-import style, quick fixes,
document symbols, completions and refactors. All the editor features of 0.51.1 are ported. The lightbulb,
the quick fix menu and the editor's Refactor menu show the Effect refactors: as in 0.51.1, the server lists
`refactor.rewrite` in its code action kinds.

The API stability rules of 0.51.1 read the `@stability` JSDoc tags of Effect 4.
`unstableApiUsage` (TS377136) and `experimentalApiUsage` (TS377135) report each use of an
`unstable` or `experimental` API, as warnings by default. `allowedUnstableApis` and
`allowedExperimentalApis` in the plugin entry allow a module (`effect/cli`) or one export
(`effect/cli/Command#make`). `apiStabilityLeak` (TS377137, off by default) is for library
maintainers. So a project on `effect@4` gets TS377136 warnings for its unstable imports, as with
effect-tsgo 0.51.1. See [the Effect diagnostics doc](https://github.com/effect-anything/tsc/blob/main/docs/effect-diagnostics.md#api-stability-rules).

## VS Code

The TypeScript 7 extension looks for the `typescript` package, so it does not find `@effect-x/tsc` by
itself. Point it at the dir of the `tsc` in the platform package, in `.vscode/settings.json`, and
allow it when VS Code asks:

```json
{ "js/ts.tsdk.path": "node_modules/@effect-x/tsc-linux-x64/lib" }
```

On Linux arm64, use `@effect-x/tsc-linux-arm64`. On macOS, use `@effect-x/tsc-darwin-arm64` (Apple silicon) or
`@effect-x/tsc-darwin-x64` (Intel). On Windows, use `@effect-x/tsc-win32-x64` or `@effect-x/tsc-win32-arm64`.

## Platforms

- Linux x64 (static, any distribution)
- Linux arm64 (static, any distribution)
- macOS arm64 (Apple silicon)
- macOS x64 (Intel)
- Windows x64
- Windows arm64

## Known problems

- In some monorepos, the source files of a workspace package are reachable both through
  `node_modules` and through a direct import. There, `@effect-x/tsc` can write output for more of those
  files than `tsc` does. (The set from `tsc` also changes from run to run there.)
- In `tsc -b`, when one project imports the output of another project without a project reference,
  `@effect-x/tsc` can report TS2307 (cannot find module) where `tsc` happens to build the other project
  first. Add the reference to fix it.
- `tsc -b --watch` can stop with an internal error (exit code 70) after some edits.
- In the editor, memory grows slowly during long edit sessions.

## License

MIT. The port keeps the licenses and notices of TypeScript (Apache-2.0, Copyright (c) Microsoft
Corporation) and Go (BSD-3-Clause). See NOTICE.txt.
