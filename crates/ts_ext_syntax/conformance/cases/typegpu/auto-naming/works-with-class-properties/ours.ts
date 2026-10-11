import { tgpu } from 'typegpu';
import * as d from 'typegpu/data';
const root = await tgpu.init();

class MyController {
  myBuffer = root.createUniform(d.u32);
}
