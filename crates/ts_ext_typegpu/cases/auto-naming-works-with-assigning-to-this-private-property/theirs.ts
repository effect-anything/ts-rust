import { tgpu } from 'typegpu';
import * as d from 'typegpu/data';
const root = await tgpu.init();
class MyController {
  #myBuffer;
  constructor() {
    this.#myBuffer = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createUniform(d.u32), "myBuffer");
  }
  get myBuffer() {
    return this.#myBuffer;
  }
}
console.log(MyController);
