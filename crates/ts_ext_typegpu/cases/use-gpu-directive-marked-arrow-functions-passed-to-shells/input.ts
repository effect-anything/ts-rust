    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell((a, b) => {
      'use gpu';
      return a + b;
    })

    shell((a, b) => {
      return a + b;
    })
  
