# tsc-ext options

Options live in the `options` object of the `@effect-x/tsc` entry in
`contentMappers`. Every field is optional; unknown keys and wrong types are
errors (exit 3 from `tsc-ext expand`).

```json
{
  "compilerOptions": { "allowArbitraryExtensions": true },
  "contentMappers": [{
    "package": "@effect-x/tsc",
    "extensions": [".sts", ".stsx"],
    "options": {
      "sweetener": { "rewriteImports": false },
      "typegpu": { "autoNaming": true, "earlyPruning": true }
    }
  }]
}
```

Run tsc with `--runExternalCode` so it may start the mapper.

Who reads them: tsc sends them to the mapper; `tsc-ext expand` and `eval`
read `--tsconfig`, else the nearest `tsconfig.json` above the file; the Node
API reads its `tsconfig` option, and explicit API options override single
fields.

## sweetener

| option | type | default | effect |
| --- | --- | --- | --- |
| `rewriteImports` | boolean | `false` | Drop `.sts`/`.stsx` from module specifiers (`./logic.sts` becomes `./logic`), for a bundler that resolves the virtual `.ts` by extension search. Only the Node API (bundle) output uses it; tsc resolves `.sts` imports itself. |

## typegpu

These apply to the bundle stage (Node API, bundler plugin, `--stage bundle`).

| option | type | default | effect |
| --- | --- | --- | --- |
| `autoNaming` | boolean | `true` | Name TypeGPU resources after the binding they are assigned to. |
| `earlyPruning` | boolean | `true` | Skip a file that mentions none of `"use gpu"`, `tgpu`, `typegpu`. |
| `forceTgpuAlias` | string | none | The name `tgpu` is imported as, when the import cannot be found. |
