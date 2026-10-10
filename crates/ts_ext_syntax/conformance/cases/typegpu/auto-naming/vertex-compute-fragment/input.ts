import { tgpu } from 'typegpu';
import * as d from 'typegpu/data';

const myFunction = tgpu.fn([])(() => 0);
const myComputeFn = tgpu.computeFn({ workgroupSize: [1] })(() => {});
const myVertexFn = tgpu.vertexFn({ out: { ret: d.i32 } })(() => ({ ret: 0 }));
const myFragmentFn = tgpu.fragmentFn({
  in: { position: d.builtin.position },
  out: d.vec4f,
})(() => d.vec4f());
