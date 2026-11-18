        import { tgpu } from 'typegpu';
        import * as d from 'typegpu/data';

        const counterBuffer = root
            .createBuffer(d.vec3f, d.vec3f(0, 1, 0))
            .$usage('storage');
        const counter = counterBuffer.as('mutable');

        const increment = tgpu
            .computeFn({ in: { num: d.builtin.numWorkgroups }, workgroupSize: [1] })(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = input => {
                const tmp = counter.$.x;
                counter.$.x = counter.$.y;
                counter.$.y += tmp;
                counter.$.z += d.f32(input.num.x);
            }, {
  v: 2,
  name: undefined,
  ast: {
    params: [{
      type: "i",
      name: "input"
    }],
    body: [0, [[13, "tmp", [7, [7, "counter", "$"], "x"]], [2, [7, [7, "counter", "$"], "x"], "=", [7, [7, "counter", "$"], "y"]], [2, [7, [7, "counter", "$"], "y"], "+=", "tmp"], [2, [7, [7, "counter", "$"], "z"], "+=", [6, "d.f32", [[7, [7, "input", "num"], "x"]]]]]]
  },
  externals: {
    "counter": () => counter,
    "d.f32": () => d.f32
  }
}) && $.f)({}));
