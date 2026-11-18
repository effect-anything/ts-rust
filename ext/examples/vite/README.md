# Vite

`main.sts` mounts the page. `App.stsx` keeps the flower in React state. `canvas.tsx` draws it. `@typegpu/react` (`useRootWithStatus`, `useConfigureContext`, `useFrame`) owns the device, the canvas context, and the frame loop. There is no `useMemo` and no vertex buffer.

Petal length is millimetres. `measure.ts` divides it by the 160 mm frame to get clip space. The uniform carries that scale, the animated angle, the petal count, a point amount, the time, and the picked tint. The vertex function builds the petal. Point moves the outline from round to sharp. The fragment shader keeps a three-stop gradient and rolls it with time, so the colours travel around the flower even when speed is 0. Petals start at 3.

Macros live under `macros/`, one category per file, the same split as the Sweetener playground examples:

- `pipe.sts` — `value |> name`
- `unless.sts` — run the body when the condition is false
- `jsx.sts` — `{each}` and `{when}`
- `thread.sts` — `(-> value, next(args))`
- `repeat.sts` — counted loop
- `fallback.sts` — `value ?? backup`
- `effect-do.sts` — `gen`, `handle`, and `effect name(...) { }` (Sweetener effect-do case)
- `effect-service.sts` — `error` and `service` (Sweetener effect-service case). The built-in form still prints Effect 3 `Context.Tag`.
- `atom.sts` — `atom count = 1` is `Atom.make`. Inside a component, `atom count` is `useAtom(countAtom)` and `atomValue count` is `useAtomValue`. No custom atom runtime.

`effect` and `@effect/atom-react` are installed. The flower still uses React state. Wiring atoms into `src` comes after this macro.

`vertex` and `fragment` in `flower.sts` are `"use gpu"` functions.

The config uses `@effect-x/tsc/unplugin/vite`. That package's native addon is a build output, not a committed file. From the repo root, build it before `npm install` in `ext/examples` (the workspace depends on `file:../../crates/ts_ext_syntax`, and `index.js` loads `./tsc-ext.<platform>.node`):

```sh
cd crates/ts_ext_syntax && npm install && npx napi build --platform --output-dir .
cd ../../ext/examples && npm install
cd vite && npm run dev
npm run build
```
