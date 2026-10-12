const myBuffer = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createBuffer(d.u32).$usage('storage').$addFlags(GPUBufferUsage.STORAGE), "myBuffer");
const Item = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(d.struct({
a: d.u32
}), "Item");
const myFn = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.fn([Item], Item) /* wgsl */`(item: Item) -> Item { return item; }`.$uses({
Item
}), "myFn");
const myLayout = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.bindGroupLayout({
foo: {
  uniform: d.vec3f
}
}).$idx(0), "myLayout");
