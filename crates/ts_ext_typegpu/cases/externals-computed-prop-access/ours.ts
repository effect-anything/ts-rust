const ext = {
  n: 1,
};
const fn = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
  'use gpu';

  const a = ext['n'];
}, {
  v: 2,
  name: "fn",
  ast: {
    params: [],
    body: [0, [[13, "a", "ext"]]]
  },
  externals: {
    "ext": () => ext
  }
}) && $.f)({});
console.log(fn);
