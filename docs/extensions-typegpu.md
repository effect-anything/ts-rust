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
