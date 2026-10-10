// extracted from use-gpu-directive.test.ts pin fc85681e title: marked anonymous function expressions passed to shells
    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell(function(a, b){
      'use gpu';
      return a + b;
    })

    shell(function(a, b) {
      return a + b;
    })
  
