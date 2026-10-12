    const ext = {
      value: 7,
      config: {
        multiplier: 1,
        zero: 0,
      }
    };
    const fn = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
      'use gpu';
      const a = ext.value;
      const b = ext.config.multiplier;
      const c = ext.config.zero;
      const d = ext.config.multiplier;
    }, {
  v: 2,
  name: "fn",
  ast: {
    params: [],
    body: [0, [[13, "a", "ext.value"], [13, "b", "ext.config.multiplier"], [13, "c", "ext.config.zero"], [13, "d", "ext.config.multiplier"]]]
  },
  externals: {
    "ext.value": () => ext.value,
    "ext.config.multiplier": () => ext.config.multiplier,
    "ext.config.zero": () => ext.config.zero
  }
}) && $.f)({});
    console.log(fn);
