const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f() {
  "use gpu";
  const a = 1;
  const b = 1.5;
  const c = 0x10;
  const d = 1e3;
  const e = true;
  const g = false;
  const h = null;
  return __tsover_add(__tsover_add(__tsover_add(a, b), c), d);
}, {
  v: 2,
  name: "f",
  ast: {
    params: [],
    body: [0, [[13, "a", [5, "1"]], [13, "b", [5, "1.5"]], [13, "c", [5, "16"]], [13, "d", [5, "1000"]], [13, "e", true], [13, "g", false], [13, "h", [106]], [10, [1, [1, [1, "a", "+", "b"], "+", "c"], "+", "d"]]]]
  },
  externals: {}
}) && $.f)({});
