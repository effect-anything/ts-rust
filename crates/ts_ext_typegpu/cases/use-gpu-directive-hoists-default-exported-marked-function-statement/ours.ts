    /** ADD */
    const add = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function add(a, b) {
      'use gpu';
      return __tsover_add(a, b);
    }, {
  v: 2,
  name: "add",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }, {
      type: "i",
      name: "b"
    }],
    body: [0, [[10, [1, "a", "+", "b"]]]]
  },
  externals: {}
}) && $.f)({});
    console.log(add);


export default add;
