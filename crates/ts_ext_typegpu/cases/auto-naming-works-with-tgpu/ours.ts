      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';

      const bindGroupLayout = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.bindGroupLayout({}), "bindGroupLayout");
      const vertexLayout = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.vertexLayout(d.arrayOf(d.u32)), "vertexLayout");
      var fn = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.fn([])(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {}, {
v: 2,
name: undefined,
ast: {
params: [],
body: [0, []]
},
externals: {}
}) && $.f)({})), "fn");
      let shell = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.fn([]), "shell");
      const cst = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.const(d.u32, 1), "cst");

      console.log(bindGroupLayout, vertexLayout);
