const myBuffer = root.createBuffer(d.u32)
  .$usage('storage')
  .$addFlags(GPUBufferUsage.STORAGE);
const Item = d.struct({ a: d.u32 });
const myFn = tgpu.fn(
  [Item],
  Item,
) /* wgsl */\`(item: Item) -> Item { return item; }\`
  .$uses({ Item });
const myLayout = tgpu
  .bindGroupLayout({ foo: { uniform: d.vec3f } })
  .$idx(0);
