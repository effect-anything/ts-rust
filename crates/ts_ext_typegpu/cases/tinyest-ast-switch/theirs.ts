const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number) {
  "use gpu";

  switch (a) {
    case 1:
      return 10;
    case 2:
      return 20;
    default:
      return 0;
  }
}, {
  v: 2,
  name: "f",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }],
    body: [0, [[19, "a", [[[5, "1"], [[10, [5, "10"]]]], [[5, "2"], [[10, [5, "20"]]]], [null, [[10, [5, "0"]]]]]]]]
  },
  externals: {}
}) && $.f)({});
