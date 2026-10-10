A normal project. `src` is what you write. `npm run build` is what you run.

```text
src/*.sts, *.stsx, *.ts, *.tsx
  -> ext-syntax TransformSession
  -> generated/*.ts, *.tsx
  -> tsc
  -> dist/*.js
```

`TransformSession` expands Sweetener, then rewrites `"use gpu"`. It also
rewrites `.sts` imports to extensionless specifiers, so `tsc` can resolve
them. Each generated file has a `.map` back to `src`.

The editor uses the same package as a content mapper
(`contentMappers` points at `ext-syntax`). The build does not: it calls
the API, then `tsc`.
