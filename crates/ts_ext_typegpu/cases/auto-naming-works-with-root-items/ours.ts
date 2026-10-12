      import { tgpu } from 'typegpu';

      const root = await tgpu.init();
      const myBuffer = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createBuffer(d.u32, 2), "myBuffer");

      console.log(myBuffer);
