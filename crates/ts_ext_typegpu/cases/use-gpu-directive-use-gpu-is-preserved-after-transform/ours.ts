const fn1 = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
  'use gpu';

}, {
  v: 2,
  name: "fn1",
  ast: {
    params: [],
    body: [0, []]
  },
  externals: {}
}) && $.f)({});
const fn2 = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
  'use gpu';

  'worklet';

}, {
  v: 2,
  name: "fn2",
  ast: {
    params: [],
    body: [0, []]
  },
  externals: {}
}) && $.f)({});
const fn3 = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
  'worklet';

  'use gpu';

}, {
  v: 2,
  name: "fn3",
  ast: {
    params: [],
    body: [0, []]
  },
  externals: {}
}) && $.f)({});
console.log(fn1, fn2, fn3);
