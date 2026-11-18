      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';

      const root = await tgpu.init();

      class MyController {
        #myBuffer;

        constructor() {
          this.#myBuffer = root.createUniform(d.u32);
        }

        get myBuffer() {
          return this.#myBuffer;
        }
      }

      console.log(MyController);
    
