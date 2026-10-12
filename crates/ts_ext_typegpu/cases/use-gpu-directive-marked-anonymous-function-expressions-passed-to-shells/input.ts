    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell(function(a, b){
      'use gpu';
      return a + b;
    })

    shell(function(a, b) {
      return a + b;
    })
  
