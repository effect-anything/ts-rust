# tsc-ext cheatsheet

Quick reference for what `@effect-x/tsc` understands. See what any file
becomes with `tsc-ext expand <file>`, and why a macro did or did not match
with `tsc-ext expand <file> --trace`.

## macros: define syntax and operators

Macros live in `.sts` (and `.stsx`, with JSX) files. They expand before
TypeScript parses the file; tsc checks the result.

```ts
export syntax unless:stmt {
  rule { unless ($cond:expr) $body:stmt } => { if (!($cond)) $body }
}

unless (n < 0) { return n + 1; }   // => if (!(n < 0)) { return n + 1; }
```

- `syntax name:category { rule { pattern } => { template } ... }`. Rules are
  tried in order; the first that matches wins.
- Categories: `expr` (an expression), `stmt` (a statement), `item` (a
  module-level declaration), `type`.
- `export` makes the macro importable with `for syntax`.
- Operators: `export operator (@@):expr { fixity infix; associativity left;
  precedence 40; rule { $l:expr @@ $r:expr } => { pair($l, $r) } }`.
  `fixity` is `infix`, `prefix` or `postfix`; new spellings (`@@`, `~~`)
  become one token.

## patterns: what a rule matches

| pattern | matches |
| --- | --- |
| `$x:expr` | an expression |
| `$s:stmt` | a statement |
| `$t:type` | a type |
| `$n:ident` | an identifier |
| `$b:binding` | a name or destructuring pattern |
| `$i:item` | a module-level declaration |
| `$m:classElement`, `$m:typeMember` | a class member, a type member |
| `$j:jsx` | a JSX child |
| `$t:tt` | one token, or one `(...)`/`[...]`/`{...}` group |
| `$x:MyClass` | a `syntax class MyClass { ... }` |
| `$( ... ),*` `$( ... )+` `$( ... )?` | repetition (separator optional) |
| `a $x:expr \| b` | alternatives; each must bind the same captures |

Syntax classes name reusable patterns and expose fields:

```ts
syntax class Pair {
  fields { left: expr; right: expr; }
  rule { [$left:expr, $right:expr] }
}
export syntax first:expr { rule { first($p:Pair) } => { $p.left } }
```

## imports: `for syntax` and `.sts` modules

```ts
import { unless } from "./macros.sts" for syntax;   // macros: removed from output
import { shade } from "./view.sts";                  // ordinary import
```

- A `for syntax` import brings macros into scope and disappears.
- An ordinary `.sts` import stays (`rewriteImports` drops the extension for
  bundlers; see `tsc-ext cheatsheet options`).
- A `.ts` file can import a `.sts` file and the other way round.

## templates: what a rule produces, built-in operations

| in a template | gives |
| --- | --- |
| `$x` | the capture |
| `$p.left` | a field of a syntax-class capture |
| `$($x),*` | each element of a repeated capture |
| `#if(present $x) { a } #else { b }` | `a` when the optional capture matched |
| `#text($x)` | the capture's source text as a string literal |
| `#count($x)` | how many times a repeated capture matched |
| `#index` | the index inside the current repetition |
| `#join($n, prefix: "get", casing: "upper-first")` | a new identifier (`getName`); `suffix:` too; casing `preserve`, `upper`, `lower`, `upper-first`, `lower-first` |
| `#fresh("tmp")` | a hygienic identifier that cannot clash |
| `#trim($x)` | the capture without surrounding whitespace |
| `#metavar(...)` | the capture's metavariable |
| `#core { ... }` | template text not expanded again |
| `#let`, `#parameterize`, `#fold` | see `tsc-ext cheatsheet scoping` |
| `#callsite($n)`, `#definition($n)`, `#capture($n)` | the identifier in the call site's, definition's, or no scope |

Identifiers a template introduces are hygienic: they do not capture or
shadow the user's names unless made with `#callsite`.

## scoping: `#let`, `#parameterize`, `#fold`

`#let(name = value) { body }` evaluates `value` once. `name` is hygienic
(renamed if it would clash):

```ts
export syntax twice:expr { rule { twice($x:expr) } => { #let(v = $x) { v + v } } }
const a = twice(f());              // => const a = ((v) => v + v)(f());
```

When the body holds an `await` or `yield` of the enclosing function, the
value is assigned to a variable declared at the top of that function
instead: `let topic; return (topic = id, ...)`.

`#parameterize(p = value) { body }` gives a syntax parameter (declared with
`syntax parameter`) a meaning inside `body`, including in captures the body
splices. `required` makes the body use it:

```ts
export syntax parameter (%):expr;
export operator (|>):expr {
  fixity infix; associativity right; precedence 20;
  rule { $value:expr |> $body:expr } => {
    #let(topic = $value) { #parameterize(required % = topic) { $body } }
  }
}
const a = xs |> %.map(f) |> %.join(" ");
// => ((topic) => ((topic_1) => topic_1.join(" "))(topic.map(f)))(xs)
```

`#fold($items, init: { initial }) { ($acc, $item, $index) => { next } }`
threads an accumulator through a repeated capture; no elements gives
`initial`. `init` must be a `{ ... }` group (else SWR2017):

```ts
export syntax sum:expr {
  rule { sum($($x:expr),*) } => {
    #fold($x, init: { 0 }) { ($acc, $item, $index) => { $acc + $item * $index } }
  }
}
const a = sum(1, 2, 3);            // => ((0 + 1 * 0) + 2 * 1) + 3 * 2
const e = sum();                   // => 0
```

Errors: SWR4019 (`#parameterize` names something that is not a syntax
parameter), SWR4022 (`required` parameter unused), SWR4018 (a parameter used
outside any `#parameterize`), SWR2017 (malformed `#fold`).

Caveat: a spliced capture is not parenthesized. Write `($x)` in a template
when its precedence matters (`$acc && ($item)`).

## gpu: `"use gpu"` functions (TypeGPU)

A function whose body starts with `"use gpu"` is a TypeGPU shader function.
tsc type-checks it as written; the bundle stage wraps it with its WGSL
metadata (`__TYPEGPU_META__`) and names resources (`autoNaming`).

```ts
const add = (a: d.v2f, b: d.v2f) => {
  "use gpu";
  return a + b;          // bundle: __tsover_add(a, b)
};
```

See it with `tsc-ext expand file.ts --stage bundle` (or `--only typegpu`).

## operators: overloading with tsover

Operator overloading (tsover). In a `"use gpu"` function, or in a file with
`"use tsover"`, arithmetic operators become calls the runtime dispatches
on `Symbol.operatorPlus` and friends:

| written | built |
| --- | --- |
| `a + b` | `__tsover_add(a, b)` |
| `a - b` | `__tsover_sub(a, b)` |
| `a * b` | `__tsover_mul(a, b)` |
| `a / b` | `__tsover_div(a, b)` |
| `a % b` | `__tsover_mod(a, b)` |
| `a ** b` | `__tsover_pow(a, b)` |
| `a += b` | `a = __tsover_add(a, b)` |

- tsc checks the operators natively against the declared symbols; only the
  bundle stage rewrites. See it with `--stage bundle --only tsover`.
- Import `tsover-runtime` once at the app entry (typegpu does this itself).
