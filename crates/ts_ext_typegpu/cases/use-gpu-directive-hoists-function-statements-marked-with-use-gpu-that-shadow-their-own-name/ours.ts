const miter = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function miter(a) {
      'use gpu';
      const miter = __tsover_mul(a, 2);
      return miter;
    }, {
  v: 2,
  name: "miter",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }],
    body: [0, [[13, "miter", [1, "a", "*", [5, "2"]]], [10, "miter"]]]
  },
  externals: {}
}) && $.f)({});
    console.log(miter);

