# TypeGPU

This extension replaces the TypeGPU bundler plugin. A user does not install
that plugin and does not add another pipeline step. The processed result
comes from this compiler's emit, or from `TransformSession` in the Node
package.

One example covers the plugin. It is a marked vertex function that exports,
calls a marked method, closes over a uniform, and uses vector members. An
unmarked function in the same file is left alone.

## Author

```ts
import tgpu, { d } from "typegpu";

const angle = { $: 0 };
const obj = {
  /** MOD */
  mod: (a, b) => {
    "use gpu";
    return a % b;
  },
};

export const vertex = tgpu.vertexFn({
  in: { position: d.vec2f, color: d.vec3f },
  out: { position: d.builtin.position, color: d.vec3f },
})(({ position, color }) => {
  "use gpu";
  const turned = d.vec2f(position.x + position.y, position.y + angle.$);
  if (obj.mod(position.x, 2) === 0) {
    return { position: d.vec4f(turned, 0, 1), color };
  }
  return { position: d.vec4f(position, 0, 1), color };
});

export function frame(a: number, b: number) {
  return a + b;
}
```

`frame` stays `a + b`. `vertex` does not.

## Check

Check sees calls, not the runtime wrapper. Inside the marked function:

| author                    | checked text                           |
| ------------------------- | -------------------------------------- |
| `position.x + position.y` | `__tsover_add(position.x, position.y)` |
| `a % b` in `mod`          | `__tsover_mod(a, b)`                   |

`- * /` and `+= -= *= /= %=` follow the same table. A nested `"use gpu"` is
a diagnostic on that directive. The span map points the error at the
operator. Numeric declarations are synthesized on this file only. Vector
overloads come from the imported `typegpu` package.

## Build

This is the plugin's final JavaScript, the same shape as the captured
`theirs.ts` files. `mod` is wrapped where it sits. `vertex`'s implementation
is wrapped, named, and keeps the `export`. `angle` is an external. `frame`
is not wrapped.

```js
import tgpu, { d } from "typegpu";

const angle = { $: 0 };
const obj = {
  /** MOD */
  mod: /*#__PURE__*/ (($) =>
    (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set(
      ($.f = (a, b) => {
        "use gpu";
        return __tsover_mod(a, b);
      }),
      {
        v: 2,
        name: "mod",
        ast: {
          params: [
            { type: "i", name: "a" },
            { type: "i", name: "b" },
          ],
          body: [0, [[10, [1, "a", "%", "b"]]]],
        },
        externals: {},
      },
    ) && $.f)({}),
};

export const vertex = tgpu.vertexFn({
  in: { position: d.vec2f, color: d.vec3f },
  out: { position: d.builtin.position, color: d.vec3f },
})(
  /*#__PURE__*/ (($) =>
    (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set(
      ($.f = ({ position, color }) => {
        "use gpu";
        const turned = d.vec2f(
          __tsover_add(position.x, position.y),
          __tsover_add(position.y, angle.$),
        );
        if (obj.mod(position.x, 2) === 0) {
          return { position: d.vec4f(turned, 0, 1), color };
        }
        return { position: d.vec4f(position, 0, 1), color };
      }),
      {
        v: 2,
        name: "vertex",
        ast: {
          params: [
            { type: "i", name: "position" },
            { type: "i", name: "color" },
          ],
          body: [0, [[10, [1, "position.x", "+", "position.y"]]]],
        },
        externals: { angle: () => angle },
      },
    ) && $.f)({}),
);

export function frame(a, b) {
  return a + b;
}
```

The AST node `[1, "position.x", "+", "position.y"]` is the addition. The
JavaScript call `__tsover_add` is only for typechecking and numeric runtime.
The shader printer reads the `+` in the AST.

The wrapper is what `typegpu` looks up with `WeakMap.get(function)`. A
second build must not wrap a function that already has this call. A `tgpu`
shell whose function has no directive still gets this metadata. An unnamed
`tgpu.*` constructor gets an automatic name.

## Shader

`typegpu` prints the shader when the vertex function is resolved. This
transform does not print it. From the payload above, the runtime emits:

```wgsl
struct VertexIn {
  @location(0) position: vec2f,
  @location(1) color: vec3f,
}
struct VertexOut {
  @builtin(position) position: vec4f,
  @location(0) color: vec3f,
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
  let turned = vec2f(in.position.x + in.position.y, in.position.y + angle);
  if (mod(in.position.x, 2) == 0) {
    return VertexOut(vec4f(turned, 0, 1), in.color);
  }
  return VertexOut(vec4f(in.position, 0, 1), in.color);
}
```

A fragment function gets `@fragment`. A compute function gets `@compute
@workgroup_size(...)`. `angle` is substituted from `externals` while
printing. An empty `ast` means this shader cannot be printed.

Check uses `TransformSession.check()` and does not contain
`__TYPEGPU_META__`. Build uses `TransformSession.build()` or this
compiler's emit, and the result is the JavaScript above. The comparison
oracle is `conformance/cases/typegpu/use-gpu-directive/*/theirs.ts`.

## What this does not do

It does not change the parser, binder, or checker. It does not register
`.ts` as a content-mapper extension. It does not replace `wgslGenerator.ts`.
A missing payload is a failed plugin, even when the operators were rewritten.

## Upstream coverage

`sync-cases.py` copies the upstream snippet, then copies the babel
`toMatchInlineSnapshot` already written in that test to `theirs.ts`. The
plugin is not built or run. `capture_ours` compares after that. Timing inputs
live in `ext/bench` and are not part of this.

Where each behaviour is implemented, and the snippet that shows it:

The operator rewrite itself is the `tsover` layer
(`crates/ts_goport/src/ext/tsover.rs`; tests in `tests/tsover.rs`). This file
is the consumer: it decides _where_ a `"use gpu"` scope is, wraps it, and
records the tinyest AST.

| pinned plugin (`unplugin-typegpu`)                                                                                        | this port                                                                                  | snippet                                                      |
| ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `tgpu` aliases come from an `import ... from 'typegpu'` only: default, `{ tgpu as x }`, `* as x` giving `x.tgpu`          | `gather_aliases`                                                                           | `no-import-shell.ts`, `other-source-import.ts`, `alias-*.ts` |
| `isTgpu` walks the member path and skips a `["~unstable"]` step                                                           | `is_tgpu_alias`                                                                            | `unstable-access.ts`                                         |
| naming positions: `VariableDeclarator`, `ObjectProperty`, `ClassProperty`, `ClassPrivateProperty`, `AssignmentExpression` | `labelled`                                                                                 | `auto-naming/*` cases                                        |
| a shell implementation is a named resource whatever the tgpu object is called                                             | `contains_resource`                                                                        | `aliased-shell.ts`, `shell-unmarked.ts`                      |
| `$name(...)` ends the resource chain                                                                                      | `contains_resource`                                                                        | `dollar-name.ts`                                             |
| the directive must be the first statement of the body                                                                     | `function_marked`                                                                          | `directive-not-first.ts`                                     |
| operators `+ - * / %` and their assignments, inside a `"use gpu"` scope                                                   | `Walk` with `tsover::name`                                                                 | `pow-*.ts`, `bitwise.ts`                                     |
| `**` is not in the plugin's table, so it stays `a ** b` (the pin's runtime does install `__tsover_pow`)                   | `"use gpu"` keeps `**`; only `"use tsover"` rewrites it (`tsover::name_with_pow`)          | `pow-operator.ts`, `pow-assign.ts`                           |
| tinyest node kinds: 1 binary, 2 assignment, 3 logical, and the operator text                                              | `operator_text`, `is_assignment`, `is_logical`                                             | `assign-kinds`, `bitwise.ts`                                 |
| hoisting: a named declaration moves to the top, `export { name }` (or `export default name;`) stays behind                | `hoist_edit`, `apply_wraps`                                                                | `hoists-*` cases                                             |
| `earlyPruneRegex` (on by default) skips a file that mentions none of `"use gpu"`, `tgpu`                                  | `pruned` in the emit hook; a bundler session uses `embed_metadata_unpruned`                | `works-with-properties` case                                 |
| `autoNamingEnabled` is an option: the plugin defaults it on, its test helper off                                          | `TransformSession::build_with_auto_naming`; the corpus passes the setting its capture used | `aliasing/*` (off), `auto-naming/*` (on)                     |

The tinyest AST is what the runtime's WGSL printer reads, so its node kinds
are part of the plugin's contract. `NodeTypeCatalog`
(`tinyest/src/nodes.ts`) has 24 kinds and this port emits all of them.
The kinds the pinned plugin prints are captured as
`crates/ts_ext_typegpu/cases/tinyest-*`. `theirs.ts` is a run of the pin's
`babelTransform`, not one of its test files.
A numeric literal is recorded as its value, not the author's spelling
(`0x10` is `[5, "16"]`), because the runtime prints that text into WGSL.

Where the plugin throws, there is no `theirs.ts` to capture. This port
refuses the same way and says so, and `tests/typegpu.rs` locks the message:
`Unsupported JS functionality: <Kind>`, `Arrow functions are not supported
inside TGSL.`, `` `var` declarations are not supported. ``, `Prefix update
expressions are not supported in WGSL.`, `Spread elements are not supported
in TGSL.`. A template literal, a spread, a `do/while`, `for...in` and a
prefix update all end in an unprintable body (`body: [0, []]`, no externals).
Those refusals are locked in `tests/typegpu.rs`.

`crates/ts_ext_typegpu` is the plugin session. `cases/` there is flat, one directory per `cases.jsonl` id, and compares with `structure::same`:
the syntax tree, so whitespace, a trailing comma, a redundant pair of
parentheses, a comment and a numeric literal form are shape, while a node
kind, a name, an operator or a statement order is not.

## Open differences

- A nested function inside a `"use gpu"` body: the plugin throws
  (`Arrow functions are not supported inside TGSL.`, `Unsupported JS
  functionality: FunctionDeclaration`). This port keeps the wrapper, marks the
  body unprintable (`ast` body `[0, []]`, no externals), and reports the same
  text in `Transformed::diagnostics` with the whole file as the span, because
  the payload carries no positions. The compiler emit hook returns text only,
  so it marks the body unprintable and reports nothing.
  Locked in `tests/typegpu.rs`.
- `unstable_obfuscate` is not implemented. The pin's default is off, and it
  refuses to run with auto-naming on.
