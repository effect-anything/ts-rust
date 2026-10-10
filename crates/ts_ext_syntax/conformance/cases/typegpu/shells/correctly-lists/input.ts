import { tgpu } from 'typegpu';
import * as d from 'typegpu/data';

const root = await tgpu.init();

class MyController {
  myBuffer = root.createUniform(d.u32);
  myFn = tgpu.fn([], d.u32)(() => {
    return this.myBuffer.$;
  });
}

const myController = new MyController();

console.log(tgpu.resolve([myController.myFn]));
