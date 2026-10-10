import * as d from 'typegpu/data';
import { struct } from 'typegpu/data';
const myStruct1 = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(d.struct({
a: d.u32
}), "myStruct1");
const myStruct2 = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(struct({
a: u32
}), "myStruct2");
const bait = d.i32(1);
console.log(myStruct1, myStruct2, bait);
