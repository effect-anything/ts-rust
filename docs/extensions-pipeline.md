# Pipeline work on this compiler

It will delete its own Sweetener process
and stop using oxlint as the type checker once the items here exist. The
published surface is `tsc` and `TransformSession` in `crates/ext_syntax_node`.

## Order

A file can contain both extensions. The only order is:

```
author text
  -> Sweetener expansion          .sts / .stsx only
  -> TypeGPU rewrite              "use gpu" only
  -> check                        no __TYPEGPU_META__
  -> emit or TransformSession     wrapper, then the host erases types
```

Three maps become one. A diagnostic or a breakpoint refers to the author
file, including a `.sts` line that a macro produced. Reversing the order
misses a `"use gpu"` that a macro emitted.

## Cache

Check, LSP, and build must not expand a macro again when nothing changed.
The content-mapper cache is the place. The key is the source text, the text
of every `import … for syntax` file, and a transform version. Bumping the
version drops results after we change the expander or the TypeGPU rewrite.
A macro edit or an LSP invalidation drops every file that imported that
macro. Vite does not watch macro files.

## Check

`tsc -p --noEmit` is the check. There is no second checker command.

A macro error is reported before TypeScript parse, in tsc's format:

```
file.sts(line,col): error SWxxxx: message
```

The file is not parsed. Other files in the program still check. After a
successful expansion, type errors are ordinary tsc diagnostics. The span
map sends them to the author line.

## Declaration emit

`.d.ts` keeps the author's signatures. It does not contain `__tsover_add`
or `__TYPEGPU_META__`. The synthesized `declare function __tsover_*` block
exists only in the check text.

## Session API

`crates/ext_syntax_node/index.d.ts` needs to grow these. Names can change.
The fields cannot.

```ts
session.check({ filename, source });
session.build({ filename, source });
```

`check` rewrites operators and does not wrap. `build` wraps. Both return:

```ts
{
  code: string
  sourceMap: string
  diagnostics: { file, message, code, line, column, start, end }[]
  dependencies: string[]   // macro files this expansion read
  changed: boolean
}
```

`filename` selects the pass and is the source-map name. `source` is the
text the host already has. The session does not read the file. Vite has
unsaved text and a query suffix. The content mapper inside `tsc` is the
path that reads from disk.

`dependencies` is how macro invalidation stays inside tsc. The host records
them for its own watcher only when it is not using `tsc -w`.

## One compile per job

`tsc --noEmit` checks. `TransformSession.build()` produces the text a
bundler erases and lowers. The bundler must not also run `tsc` emit, or the
wrapper is applied twice.

## LSP

The first LSP cut is diagnostics only: file, line, column, and the same
message shape as `tsc`. Jump-to-definition inside a macro template, and
rename of a hygienic name such as `tmp_1`, are out of this cut.

## Concurrency

Vite transforms modules in parallel. One session call owns its state. The
cache is safe to read from those calls. No process-global expansion result.

## Source maps

Generated text is not a breakpoint target. The prelude `declare function`
and the `__TYPEGPU_META__` wrapper map through to the author's statement.

## Runtime import

This transform does not add `import "typegpu"` or `import "tsover-runtime"`.
A `"use gpu"` function called from JavaScript needs that import already,
because `__tsover_*` is assigned when the package loads. Shader printing
does not need it. It reads the AST. This is a decision to keep, not an
omission, until a file with a marked function and no TypeGPU import shows
up in a real program.

## Host stages after the session

`build` returns TypeScript, not JavaScript. Returning JavaScript would
throw away the type table. The host appends the session map in front of the
two it already has. For `.stsx` the chain is session, erasure, framework.
Two maps put breakpoints on the expanded text.

The host picks the framework from the suffix. `.stsx` is not `.tsx`, so it
would stop after erasure and leave JSX in the bundle. `extension` on the
result has to say `.tsx` for `.stsx` and `.ts` for `.sts`, which is what
the host's pipeline switch reads.
