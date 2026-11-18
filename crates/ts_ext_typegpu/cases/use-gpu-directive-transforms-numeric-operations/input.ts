    import { tgpu, d } from 'typegpu';

    const root = await tgpu.init();
    const countMutable = root.createMutable(d.i32, 0);

    /** the main function */
    // another comment
    const main = (a, b) => {
      'use gpu';
      let c = a + b + 2;
      c += 2 * b;
      countMutable.$ += 3;
    };

    console.log(main);
  
