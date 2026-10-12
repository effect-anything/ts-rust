const c = 1 + 2;
switch (globalThis.YUP) {
  case 0:
    console.log(add, mul);

    /** ADD */
    // another comment
    const add = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function add(a, b) {
      'use gpu';

      return __tsover_add(__tsover_add(a, b), c);
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
        body: [0, [[10, [1, [1, "a", "+", "b"], "+", "c"]]]]
      },
      externals: {
        "c": () => c
      }
    }) && $.f)({});
    break;
  default:
    /** MUL */
    // another comment
    const mul = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function mul(a, b) {
      'use gpu';

      return __tsover_mul(__tsover_mul(a, b), c);
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
        body: [0, [[10, [1, [1, "a", "*", "b"], "*", "c"]]]]
      },
      externals: {
        "c": () => c
      }
    }) && $.f)({});
    break;
}
