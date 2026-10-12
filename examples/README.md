# Examples

One project per extension, then `tour`, which uses all of them. Later examples can go deeper. This pass only fixes the layout.

```text
src
  -> TransformSession.build()
  -> generated/
  -> tsc
  -> dist/
```

Install once, from this directory:

```sh
npm install
npm run build
```

`npm run build` inside one project builds that project only. `ext-syntax` is the local native package. `typescript` is the local stub at `npm/typescript`: its `tsc` runs `target/goport-release/bin/tsgo` from `crates/ts_goport/scripts/build-release.sh`. Set `TSGO` to point that stub at another binary.

| project | what it shows |
| --- | --- |
| `sweetener` | `.sts` expands before parse. `unless` becomes an `if`. |
| `tsover` | `"use tsover"` in a `.ts` file. Operators become `__tsover_*` calls. |
| `typegpu` | `"use gpu"` on a vertex function and a method. `frame` is not marked, so it stays. |
| `tour` | `.sts`, `.stsx`, `.ts`, and `.tsx` in one program. |

`.ts` and `.tsx` are not content-mapper extensions. Only `.sts` and `.stsx` are. `"use tsover"` and `"use gpu"` are rewritten after parse. A stock `tsc` typechecks the author text and does not do that rewrite. `npm run build` is the plugin path. `tsgo` is the compiler path.

`generated/` and `dist/` are build output.
