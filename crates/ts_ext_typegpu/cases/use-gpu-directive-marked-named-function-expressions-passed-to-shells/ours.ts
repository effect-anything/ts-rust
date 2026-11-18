    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function addGPU(a, b) {
      'use gpu';
      return __tsover_add(a, b);
    }, {
  v: 2,
  name: "addGPU",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }, {
      type: "i",
      name: "b"
    }],
    body: [0, [[10, [1, "a", "+", "b"]]]]
  },
  externals: {}
}) && $.f)({}))

    shell(function addCPU(a, b) {
      return a + b;
    })
