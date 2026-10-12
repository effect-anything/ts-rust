import { tgpu } from 'typegpu';
  import * as d from 'typegpu/data';

  const counterBuffer = root
      .createBuffer(d.vec3f, d.vec3f(0, 1, 0))
      .$usage('storage');
  const counter = counterBuffer.as('mutable');

  const increment = tgpu
      .computeFn({ in: { num: d.builtin.numWorkgroups }, workgroupSize: [1] })((input) => {
          const tmp = counter.$.x;
          counter.$.x = counter.$.y;
          counter.$.y += tmp;
          counter.$.z += d.f32(input.num.x);
      });
