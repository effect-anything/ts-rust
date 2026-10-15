import tgpu, { d } from "typegpu";

export const mod = tgpu.fn([d.vec2f], d.vec2f)((value) => {
  "use gpu";
  return value % 2;
});

export const vertex = tgpu.vertexFn({
  in: { position: d.vec2f, color: d.vec3f },
  out: { position: d.builtin.position, color: d.vec3f },
})((input) => {
  "use gpu";
  const turned = input.position + d.vec2f(0, 1);
  if (mod(input.position).x === 0) {
    return { position: d.vec4f(turned, 0, 1), color: input.color };
  }
  return { position: d.vec4f(input.position, 0, 1), color: input.color };
});

export function frame(a: number, b: number): number {
  return a + b;
}
