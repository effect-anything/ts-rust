const f = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function f(a: number) {
  "use gpu";

  let s = 0;
  for (let i = 0; i < a; i++) {
    if (i === 2) {
      continue;
    }
    if (i === 5) {
      break;
    }
    s = __tsover_add(s, i);
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
    body: [0, [[12, "s", [5, "0"]], [14, [12, "i", [5, "0"]], [1, "i", "<", "a"], [102, "++", "i"], [0, [[11, [1, "i", "===", [5, "2"]], [0, [[16]]]], [11, [1, "i", "===", [5, "5"]], [0, [[17]]]], [2, "s", "+=", "i"]]]], [10, "s"]]]
  },
  externals: {}
}) && $.f)({});
