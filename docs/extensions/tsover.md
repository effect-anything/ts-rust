# tsover

Operator overloading lives in this compiler. It is the lower layer:
`crates/ts_goport/src/ext/tsover.rs` owns the operator table, the two
directives, the rewrite walk, the prelude and the span-map segments.
`ext::typegpu` is a consumer: it wraps marked functions and records the
tinyest AST of what this layer produced.

Its tests are `crates/ts_ext_syntax/tests/tsover.rs` (the table, both
directives, scope, prelude, span map, nesting). The consumer's are in
`tests/typegpu.rs`. `scripts/ext-bench/tsover-probes/` holds snippets with no
upstream oracle (the pinned TypeGPU plugin has no `"use tsover"`).
 It is not a fork of `tsc`, and
it is not part of TypeGPU. TypeGPU and typenet are consumers. A library that
declares `Symbol.operatorPlus` on its types can use the same path.

The runtime is `tsover-runtime`. It installs `globalThis.__tsover_add` and
the other five calls, then looks up the symbol on either operand. This
compiler does not inject that import.

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

Check and LSP see the calls, not a second language service. The span map
points a diagnostic at the author's operator. A numeric `declare function`
is synthesized for check only, and only on a rewritten file. Build output
does not include those declarations.

This is not yet the upstream type-driven rule. Upstream leaves a primitive
`number + number` alone and rewrites only when an operand type has the
operator symbol. That decision needs the checker. Until it is wired, an
opted-in scope rewrites every operator in the table, so a library does not
have to be known by name.

## Not a content-mapper extension

`"use tsover"` is legal TypeScript. Registering `.ts` as a content mapper
would send every TypeScript file through an external process. The rewrite
runs after parse, before check, inside this compiler. `.sts` and `.stsx`
remain the content-mapper extensions. Their expansion finishes before this
pass.
