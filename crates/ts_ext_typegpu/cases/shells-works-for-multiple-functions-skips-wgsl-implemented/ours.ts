      import { tgpu } from 'typegpu';

      const a = tgpu.computeFn({ workgroupSize: [1] })(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = input => {
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

      const b = tgpu.fn([])(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
        const y = 2 + 2;
      }, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[13, "y", [1, [5, "2"], "+", [5, "2"]]]]]
  },
  externals: {}
}) && $.f)({}));

      const cx = 2;
      const c = tgpu.fn([])(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => cx, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[10, "cx"]]]
  },
  externals: {
    "cx": () => cx
  }
}) && $.f)({}));

      const d = tgpu.fn([])('() {}');
