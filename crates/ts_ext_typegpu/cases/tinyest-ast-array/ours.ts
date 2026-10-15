const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number) {
  "use gpu";
  const v = [a, 2, 3];
  return __tsover_add(__tsover_add(v[0], v[1]), v[2]);
}, {
  v: 2,
  name: "f",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }],
    body: [0, [[13, "v", [100, ["a", [5, "2"], [5, "3"]]]], [10, [1, [1, [8, "v", [5, "0"]], "+", [8, "v", [5, "1"]]], "+", [8, "v", [5, "2"]]]]]]
  },
  externals: {}
}) && $.f)({});
