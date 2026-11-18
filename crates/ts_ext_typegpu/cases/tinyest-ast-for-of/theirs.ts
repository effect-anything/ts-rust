const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number[]) {
  "use gpu";

  let s = 0;
  for (const x of a) {
    s = __tsover_add(s, x);
  }
  return s;
}, {
  v: 2,
  name: "f",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }],
    body: [0, [[12, "s", [5, "0"]], [18, [13, "x"], "a", [0, [[2, "s", "+=", "x"]]]], [10, "s"]]]
  },
  externals: {}
}) && $.f)({});
