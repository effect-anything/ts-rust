/** ADD */
const addGPU = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = function addGPU(a, b) {
  'use gpu';

  // hello there
  return __tsover_add(a, b);
}, {
  v: 2,
  name: "addGPU",
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
function addCPU(a, b) {
  return a + b;
}
console.log(addGPU(1, 2), addCPU(2, 4));
