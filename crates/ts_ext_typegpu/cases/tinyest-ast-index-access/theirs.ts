const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number[], i: number, o: any) {
  "use gpu";

  const x = a[i];
  const y = o["k"];
  return __tsover_add(x, y);
}, {
  v: 2,
  name: "f",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }, {
      type: "i",
      name: "i"
    }, {
      type: "i",
      name: "o"
    }],
    body: [0, [[13, "x", [8, "a", "i"]], [13, "y", [8, "o", [103, "k"]]], [10, [1, "x", "+", "y"]]]]
  },
  externals: {}
}) && $.f)({});
