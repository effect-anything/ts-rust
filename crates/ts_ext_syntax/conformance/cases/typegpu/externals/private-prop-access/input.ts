import tgpu, { d } from 'typegpu';

    const cls = new (class {
#const = tgpu.const(d.u32, 1);

fn = () => {
  'use gpu';
  const a = this.#const.$;
};
    })();

    console.log(cls);
