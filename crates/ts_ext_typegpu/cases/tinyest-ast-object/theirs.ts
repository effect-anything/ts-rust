const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number) {
  "use gpu";

  const o = {
    x: a,
    y: 2
  };
  return __tsover_add(o.x, o.y);
}, {
  v: 2,
  name: "f",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }],
    body: [0, [[13, "o", [104, {
      x: "a",
      y: [5, "2"]
    }]], [10, [1, [7, "o", "x"], "+", [7, "o", "y"]]]]]
  },
  externals: {}
}) && $.f)({});
