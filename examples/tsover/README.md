# tsover

`"use tsover"` is legal TypeScript. This file is not a content mapper. The whole file is rewritten: `+`, `+=`, and `**` become `__tsover_*` calls.

Build output does not include the numeric declarations. `examples/runtime.d.ts` is added so `tsc` can check the generated file. Check inside `tsgo` synthesizes those declarations itself.

```sh
npm run build
```
