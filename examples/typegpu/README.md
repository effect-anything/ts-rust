# TypeGPU

One file. `mod` and `vertex` are marked with `"use gpu"`. `frame` is not, so its `+` stays.

Build output wraps the marked functions with `__TYPEGPU_META__`. Check output does not. This script writes the build output. `stubs/typegpu.d.ts` stands in for the `typegpu` package.

```sh
npm run build
```
