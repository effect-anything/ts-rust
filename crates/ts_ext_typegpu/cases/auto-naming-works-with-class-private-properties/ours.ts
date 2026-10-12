class Foo {
  #const = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(tgpu.const(d.u32, 1), "const");
  #buff;
  constructor() {
    this.#buff = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createUniform(d.u32), "buff");
  }
}
