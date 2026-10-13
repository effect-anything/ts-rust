    import tgpu, { d } from 'typegpu';

    const root = await tgpu.init();
    const buffer = root.createMutable(d.vec2u);
    const fn = () => {
      'use gpu';
      const a = buffer.$.x;
    };
    console.log(fn);
