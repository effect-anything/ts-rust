// extracted from use-gpu-directive.test.ts pin fc85681e title: marked named function expressions passed to shells
    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell(function addGPU(a, b){
      'use gpu';
      return a + b;
    })

    shell(function addCPU(a, b) {
      return a + b;
    })
  
