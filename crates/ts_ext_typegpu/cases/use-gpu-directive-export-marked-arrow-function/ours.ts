export const add = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = (a, b) => {
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
const increment = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = n => {
  'use gpu';

  return __tsover_add(n, 1);
}, {
  v: 2,
  name: "increment",
  ast: {
    params: [{
      type: "i",
      name: "n"
    }],
    body: [0, [[10, [1, "n", "+", [5, "1"]]]]]
  },
  externals: {}
}) && $.f)({});
export { increment };
const mul = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = (a, b) => {
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
export default mul;
