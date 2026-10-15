      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';
      const root = await tgpu.init();

      class MyController {
        myBuffer = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createUniform(d.u32), "myBuffer");
      }
