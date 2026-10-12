import { tgpu } from 'typegpu';
const fun = tgpu.computeFn({ workgroupSize: [1] })(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = input => {
  const x = true;
}, {
  v: 2,
  name: undefined,
  ast: {
    params: [{
      type: "i",
      name: "input"
    }],
    body: [0, [[13, "x", true]]]
  },
  externals: {}
}) && $.f)({}));
const funcWithAs = tgpu.computeFn({ workgroupSize: [1] })(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = input => {
  const x = true as boolean;
}, {
  v: 2,
  name: undefined,
  ast: {
    params: [{
      type: "i",
      name: "input"
    }],
    body: [0, [[13, "x", true]]]
  },
  externals: {}
}) && $.f)({}));
const funcWithSatisfies = tgpu.computeFn({ workgroupSize: [1] })(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = input => {
  const x = true satisfies boolean;
}, {
  v: 2,
  name: undefined,
  ast: {
    params: [{
      type: "i",
      name: "input"
    }],
    body: [0, [[13, "x", true]]]
  },
  externals: {}
}) && $.f)({}));
