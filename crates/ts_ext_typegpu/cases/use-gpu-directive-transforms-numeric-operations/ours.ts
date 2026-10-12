import { tgpu, d } from 'typegpu';
const root = await tgpu.init();
const countMutable = root.createMutable(d.i32, 0);
/** the main function */
// another comment
const main = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = (a, b) => {
  'use gpu';

  let c = __tsover_add(__tsover_add(a, b), 2);
  c = __tsover_add(c, __tsover_mul(2, b));
  countMutable.$ = __tsover_add(countMutable.$, 3);
}, {
  v: 2,
  name: "main",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }, {
      type: "i",
      name: "b"
    }],
    body: [0, [[12, "c", [1, [1, "a", "+", "b"], "+", [5, "2"]]], [2, "c", "+=", [1, [5, "2"], "*", "b"]], [2, [7, "countMutable", "$"], "+=", [5, "3"]]]]
  },
  externals: {
    "countMutable": () => countMutable
  }
}) && $.f)({});
console.log(main);
