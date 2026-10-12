      import { tgpu } from 'typegpu';
      import * as d from 'typegpu/data';

      const bindGroupLayout = tgpu.bindGroupLayout({});
      const vertexLayout = tgpu.vertexLayout(d.arrayOf(d.u32));
      var fn = tgpu.fn([])(() => {});
      let shell = tgpu.fn([]);
      const cst = tgpu.const(d.u32, 1);

      console.log(bindGroupLayout, vertexLayout);
    
