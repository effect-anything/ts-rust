    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell(function addGPU(a, b){
      'use gpu';
      return a + b;
    })

    shell(function addCPU(a, b) {
      return a + b;
    })
  
