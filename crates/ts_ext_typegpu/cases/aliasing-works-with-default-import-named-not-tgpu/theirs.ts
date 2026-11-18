import hello from 'typegpu';
const increment = hello.fn([])(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
  const x = 2 + 2;
}, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[13, "x", [1, [5, "2"], "+", [5, "2"]]]]]
  },
  externals: {}
}) && $.f)({}));
