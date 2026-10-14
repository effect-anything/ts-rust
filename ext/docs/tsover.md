# tsover

Operator overloading lives in this compiler. It is the lower layer:
`crates/ts_goport/src/ext/tsover.rs` owns the operator table, the two
directives, the rewrite walk, the prelude and the span-map segments.
`ext::typegpu` is a consumer: it wraps marked functions and records the
tinyest AST of what this layer produced.

Its tests are `crates/ts_ext_typegpu/tests/tsover.rs` (the table, both
directives, scope, prelude, span map, nesting). The consumer's are in
`tests/typegpu.rs`. The pinned TypeGPU plugin has no `"use tsover"`, so
there is no upstream snapshot for that directive.
It is not a fork of `tsc`, and
it is not part of TypeGPU. TypeGPU and typenet are consumers. A library that
declares `Symbol.operatorPlus` on its types can use the same path.

The runtime is `tsover-runtime`. It installs `globalThis.__tsover_add` and
the other five calls, then looks up the symbol on either operand. This
compiler does not inject that import, matching upstream and `tsc` emit:
`typegpu` imports it itself (`data/assignInfixOperators.js`), and an app
that uses tsover without typegpu imports `tsover-runtime` once at its entry.

## Opt-in

A file is rewritten only when it contains `"use tsover"` or `"use gpu"`.
Both directives enable the same calls. The GPU wrapper, metadata, numeric
AST, externals, hoist, and shell run only for `"use gpu"`. A file with
neither directive is unchanged.

`"use tsover"` is file-scoped here. `"use gpu"` still rewrites only the
function that carries it. Upstream's plugin skips `"use gpu"` because the
TypeGPU plugin used to own that scope. This compiler owns both, so it does
not skip.

## Calls

| author | checked and built text |
| --- | --- |
| `a + b` | `__tsover_add(a, b)` |
| `a - b` | `__tsover_sub(a, b)` |
| `a * b` | `__tsover_mul(a, b)` |
| `a / b` | `__tsover_div(a, b)` |
| `a % b` | `__tsover_mod(a, b)` |
| `a ** b` | `__tsover_pow(a, b)` |
| `a += b` | `a = __tsover_add(a, b)` |

The assignment forms of `- * / % **` follow the same pattern.

## Check, LSP, build

Check and LSP see the author's `+`. In a `"use gpu"` or `"use tsover"`
scope, the checker uses `[Symbol.operatorPlus]` on the left operand, then
the right operand, and the expression type is that signature's return type.
`Symbol.deferOperation` moves to the other operand. `number + number` has
no symbol, so it stays `number`. A pair with no matching signature is a
normal operator error (`TS2365`), not `any`. Hover on the operator shows
that expression type. Emit rewrites the operator to `__tsover_add` for the
runtime. Build output does not include a numeric declaration.

The directive has to be a real prologue: the first statement of the file,
or the first statement of the function. An import before `"use tsover"`
means the directive is not in scope, and `Vec2f + Vec2f` is then an error.
`tests/fixtures/overload` locks the check.

## Not a content-mapper extension

`"use tsover"` is legal TypeScript. Registering `.ts` as a content mapper
would send every TypeScript file through an external process. The rewrite
runs after parse, before check, inside this compiler. `.sts` and `.stsx`
remain the content-mapper extensions. Their expansion finishes before this
pass.
