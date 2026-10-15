# TypeGPU

`typegpu` is a devDependency of the examples package. `mod` and `vertex` are marked with `"use gpu"`. `value % 2` and `position + d.vec2f(...)` use the vector overloads from that package. `frame` is not marked, so its `+` stays.

```sh
npm run build
```
