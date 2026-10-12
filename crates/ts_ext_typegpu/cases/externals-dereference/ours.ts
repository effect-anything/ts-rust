import tgpu, { d } from 'typegpu';
const root = await tgpu.init();
const buffer = root.createMutable(d.vec2u);
const fn = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
  'use gpu';

  const a = buffer.$.x;
}, {
  v: 2,
  name: "fn",
  ast: {
    params: [],
    body: [0, [[13, "a", [7, [7, "buffer", "$"], "x"]]]]
  },
  externals: {
    "buffer": () => buffer
  }
}) && $.f)({});
console.log(fn);
