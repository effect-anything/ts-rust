      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';

      let nothing, accessor = tgpu['~unstable'].accessor(d.u32);
      const hello = tgpu.comptime(() => 1 + 2);

      console.log(accessor, shell, fn, cst);
    
