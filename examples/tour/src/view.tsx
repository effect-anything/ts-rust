import tgpu, { d } from "typegpu";
import { step } from "./logic";
import { Panel } from "./panel";

const Vertex = d.struct({
  position: d.vec2f,
  color: d.vec3f,
});

export const vertex = tgpu.vertexFn({
  in: { position: d.vec2f, color: d.vec3f },
  out: { position: d.builtin.position, color: d.vec3f },
})(({ position, color }: { position: { x: number; y: number }; color: unknown }) => {
  "use gpu";
  const turned = d.vec2f(position.x + position.y, position.y);
  return { position: d.vec4f(turned, 0, 1), color };
});

export const fragment = tgpu.fragmentFn({
  in: { color: d.vec3f },
  out: d.vec4f,
})(({ color }: { color: unknown }) => {
  "use gpu";
  return d.vec4f(color, 1);
});

export function shade(n: number): number {
  "use gpu";
  return n + 1;
}

export function View(value: number) {
  const next = shade(step(value));
  return <div><Panel n={next} /></div>;
}
