const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number, b: number, c: number) {
  "use gpu";

  return (a && b) || (c ?? a);
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
    }, {
      type: "i",
      name: "c"
    }],
    body: [0, [[10, [3, [3, "a", "&&", "b"], "||", [3, "c", "??", "a"]]]]]
  },
  externals: {}
}) && $.f)({});
