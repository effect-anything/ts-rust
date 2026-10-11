import tgpu, { d } from 'typegpu';
const cls = new (class {
  #const = tgpu.const(d.u32, 1);
  fn = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
    'use gpu';

    const a = this.#const.$;
  }, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[13, "a", [7, "this.#const", "$"]]]]
  },
  externals: {
    "this.#const": () => this.#const
  }
}) && $.f)({});
})();
console.log(cls);
