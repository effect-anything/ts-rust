
      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';

      const root = await tgpu.init();

      class MyController {
        myBuffer = root.createUniform(d.u32);
        myFn = tgpu.fn([], d.u32)(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
          return this.myBuffer.$;
        }, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[10, [7, "this.myBuffer", "$"]]]]
  },
  externals: {
    "this.myBuffer": () => this.myBuffer
  }
}) && $.f)({}));
      }

      const myController = new MyController();

      console.log(tgpu.resolve([myController.myFn]));
