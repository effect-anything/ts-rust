import { tgpu } from 'typegpu';
import * as d from 'typegpu/data';
import { struct } from 'typegpu/data';
const root = await tgpu.init();
const myBuffer = root.createBuffer(d.u32, 2).$name('int buffer');
const myStruct = struct({
a: u32
}).$name('myStruct');
console.log(myBuffer, myStruct);
