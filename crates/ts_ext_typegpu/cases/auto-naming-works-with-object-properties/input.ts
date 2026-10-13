      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';
      const root = await tgpu.init();

      const items: { myBuffer: unknown } = { myBuffer: undefined };

      items.myBuffer = root.createUniform(d.u32);
    
