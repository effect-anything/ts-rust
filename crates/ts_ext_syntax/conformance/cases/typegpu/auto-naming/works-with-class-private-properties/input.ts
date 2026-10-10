class Foo {
  #const = tgpu.const(d.u32, 1);
  #buff;

  constructor() {
    this.#buff = root.createUniform(d.u32);
  }
}
