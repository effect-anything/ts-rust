import { tgpu } from 'typegpu';
import * as d from 'typegpu/data';
const myFunction = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.fn([])(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => 0, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[10, [5, "0"]]]]
  },
  externals: {}
}) && $.f)({})), "myFunction");
const myComputeFn = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.computeFn({workgroupSize:[1]})(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {}, {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, []]
  },
  externals: {}
}) && $.f)({})), "myComputeFn");
const myVertexFn = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.vertexFn({out:{ret:d.i32}})(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => ({ ret: 0 }), {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[10, [104, {
              ret: [5, "0"]
            }]]]]
  },
  externals: {}
}) && $.f)({})), "myVertexFn");
const myFragmentFn = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.fragmentFn({in:{position:d.builtin.position},out:d.vec4f,})(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => d.vec4f(), {
  v: 2,
  name: undefined,
  ast: {
    params: [],
    body: [0, [[10, [6, "d.vec4f", []]]]]
  },
  externals: {
    "d.vec4f": () => d.vec4f
  }
}) && $.f)({})), "myFragmentFn");
