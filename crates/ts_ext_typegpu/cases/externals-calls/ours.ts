    import tgpu, { d } from 'typegpu';

    const fn = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
      'use gpu';
      const a = ext.comptime().x;
      const b = ext.runtime().y;
    }, {
  v: 2,
  name: "fn",
  ast: {
    params: [],
    body: [0, [[13, "a", [7, [6, "ext.comptime", []], "x"]], [13, "b", [7, [6, "ext.runtime", []], "y"]]]]
  },
  externals: {
    "ext.comptime": () => ext.comptime,
    "ext.runtime": () => ext.runtime
  }
}) && $.f)({});

    const ext = {
      comptime: tgpu.comptime(() => {
        return d.vec4f();
      }),
      runtime:
/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
        'use gpu';
        return d.vec4f();
      }, {
  v: 2,
  name: "runtime",
  ast: {
    params: [],
    body: [0, [[10, [6, "d.vec4f", []]]]]
  },
  externals: {
    "d.vec4f": () => d.vec4f
  }
}) && $.f)({}),
    };

    console.log(fn);
