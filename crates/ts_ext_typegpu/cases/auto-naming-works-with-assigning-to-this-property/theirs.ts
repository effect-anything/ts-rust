import { tgpu, type TgpuUniform } from 'typegpu';
import * as d from 'typegpu/data';
const root = await tgpu.init();
class MyController {
  myBuffer: TgpuUniform<d.U32>;
  constructor() {
    this.myBuffer = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createUniform(d.u32), "myBuffer");
  }
}
