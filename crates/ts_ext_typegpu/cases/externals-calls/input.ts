import tgpu, { d } from 'typegpu';

    const fn = () => {
'use gpu';
const a = ext.comptime().x;
const b = ext.runtime().y;
    };

    const ext = {
comptime: tgpu.comptime(() => {
  return d.vec4f();
}),
runtime: () => {
  'use gpu';
  return d.vec4f();
},
    };

    console.log(fn);
