    const fn = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
      'use gpu';
    }, {
  v: 2,
  name: "fn",
  ast: {
    params: [],
    body: [0, []]
  },
  externals: {}
}) && $.f)({});

    console.log(fn);
