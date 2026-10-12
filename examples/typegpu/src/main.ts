import tgpu, { d } from "typegpu";

const angle = { $: 0 };
const obj = {
  /** MOD */
  mod: (a: number, b: number): number => {
    "use gpu";
    return a % b;
  },
};

export const vertex = tgpu.vertexFn({
  in: { position: d.vec2f, color: d.vec3f },
  out: { position: d.builtin.position, color: d.vec3f },
})(({ position, color }: { position: { x: number; y: number }; color: unknown }) => {
  "use gpu";
  const turned = d.vec2f(position.x + position.y, position.y + angle.$);
  if (obj.mod(position.x, 2) === 0) {
    return { position: d.vec4f(turned, 0, 1), color };
  }
  return { position: d.vec4f(position, 0, 1), color };
});

export function frame(a: number, b: number): number {
  return a + b;
}
