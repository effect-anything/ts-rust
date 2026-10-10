import * as d from 'typegpu/data';
import { struct } from 'typegpu/data';

const myStruct1 = d.struct({ a: d.u32 });
const myStruct2 = struct({ a: u32 });
const bait = d.i32(1);

console.log(myStruct1, myStruct2, bait);
