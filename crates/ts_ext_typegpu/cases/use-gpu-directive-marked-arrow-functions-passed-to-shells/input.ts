// extracted from use-gpu-directive.test.ts pin fc85681e title: marked arrow functions passed to shells
    import { tgpu } from 'typegpu';

    const shell = tgpu.fn([]);

    shell((a, b) => {
      'use gpu';
      return a + b;
    })

    shell((a, b) => {
      return a + b;
    })
  
