const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number, b: boolean) {
  "use gpu";

  const x = -a;
  const y = !b;
  const z = ~a;
  const w = +a;
  return __tsover_add(__tsover_add(__tsover_add(x, (y ? 1 : 0)), z), w);
}, {
  v: 2,
  name: "f",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }, {
      type: "i",
      name: "b"
    }],
    body: [0, [[13, "x", [4, "-", "a"]], [13, "y", [4, "!", "b"]], [13, "z", [4, "~", "a"]], [13, "w", [4, "+", "a"]], [10, [1, [1, [1, "x", "+", [105, "y", [5, "1"], [5, "0"]]], "+", "z"], "+", "w"]]]]
  },
  externals: {}
}) && $.f)({});
