const hello = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = (a: number, b: number | undefined) => {
  'use gpu';

  return __tsover_add(a, (b as number));
}, {
  v: 2,
  name: "hello",
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
}) && $.f)({});
