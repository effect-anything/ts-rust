export const scope = () => {
  /** MUL */
  // another comment
  const mul = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function mul(a, b) {
    'use gpu';

    return __tsover_mul(a, b);
  }, {
    v: 2,
    name: "mul",
    ast: {
      params: [{
        type: "i",
        name: "a"
      }, {
        type: "i",
        name: "b"
      }],
      body: [0, [[10, [1, "a", "*", "b"]]]]
    },
    externals: {}
  }) && $.f)({});
  /** ADD */
  // another comment
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
  console.log(add, mul);
};
