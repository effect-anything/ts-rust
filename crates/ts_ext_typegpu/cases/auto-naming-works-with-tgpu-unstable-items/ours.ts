      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';

      let nothing, accessor = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu['~unstable'].accessor(d.u32), "accessor");
      const hello = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.comptime(() => 1 + 2), "hello");

      console.log(accessor, shell, fn, cst);
