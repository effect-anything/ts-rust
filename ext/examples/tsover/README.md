# tsover

`"use tsover"` is legal TypeScript. `Vec2f` implements `Operator.plus` and `Operator.star` from `tsover-runtime`. `a + b`, `a * 2`, and `2 * b` use those overloads. A `number + number` inside the method body does not.

`tsover-runtime` is a devDependency of the examples package. Check uses the symbols on `Vec2f`. Emit rewrites the operators to `__tsover_*`.

```sh
npm run build
```
