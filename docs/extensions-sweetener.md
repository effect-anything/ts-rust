# Sweetener

Sweetener source is not legal TypeScript. The parser must not see `unless`,
`doSteps`, or a `for syntax` import. Those files match by extension and
expand before parse. That is a content mapper, not `precheck`.

## What the author writes

```ts
import { unless } from "./macros.sts" for syntax;
import { shade } from "./view";

export function step(n: number): number {
  unless (n < 0) {
    return n + 1;
  }
  return shade(0);
}
```

`step` may call a `.ts` or `.tsx` file, and those files may call `step`.
The extension is how the host chooses the mapper. The directive inside is
not.

## Expansion

The mapper reads a token tree, expands the macros it knows, and returns
ordinary TypeScript plus a span map. A `for syntax` import is removed. An
ordinary `.sts` import is rewritten to an extensionless specifier, so a
later `tsc` resolves `./logic` to `logic.ts`.

`unless (n < 0) { return n + 1; }` becomes `if (!(n < 0)) { return n + 1; }`.
The replacement is an atom on `unless`, so a diagnostic points at the macro
name. Text that is not a macro form is copied.

A macro error is reported by the mapper, at the macro token, before
TypeScript parses the file. `unless n` is a Sweetener diagnostic. It is not
a TypeScript parse error. An unmatched `)` is a reader diagnostic at that
byte.

The host parses the returned text under the original file name and attaches
the span map. The author still sees one file.

## Check, emit, runtime

Check sees the expanded TypeScript. It does not see `unless`. Hover and
diagnostics use the mapper span map.

This compiler does not emit JavaScript for a content-mapped file. The
mapper, or a plugin in front of another compiler, owns that output. The
plugin path is:

```text
.sts / .stsx
  -> expand macros
  -> write .ts / .tsx
  -> ordinary tsc
  -> .js
```

`TransformSession` expands the file and writes ordinary TypeScript. It does
not add a runtime wrapper.

Macro diagnostics stay in the expansion result. They are not delayed until
`tsc`, where the same mistake would look like a TypeScript error in
generated code.

## What is still open

The expander matches the reference compiler on a minority of the captured
cases. Forms that still differ include hygiene renaming, `#syntax` and
`#join`, JSX `when` / `else`, `match`, `doSteps`, and threading `->`.
Layout-only differences also remain. A case is done only when its emitted
TypeScript matches the captured reference byte for byte.

## What this does not do

- It does not expand `.ts` or `.tsx`.
- It does not register those extensions as content-mapper extensions.
- It does not run a second expansion inside `precheck`.
- It does not change the parser, binder, or checker.
- It does not make this compiler emit `.sts.js`.
