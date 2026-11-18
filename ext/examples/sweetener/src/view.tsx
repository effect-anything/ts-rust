import tgpu, { d } from "typegpu";
import { step } from "./logic.sts";
import { Panel } from "./panel.stsx";

export const vertex = tgpu.vertexFn({
  in: { position: d.vec2f, color: d.vec3f },
  out: { position: d.builtin.position, color: d.vec3f },
})((input) => {
  "use gpu";
  const turned = input.position + d.vec2f(input.position.y, 0);
  return { position: d.vec4f(turned, 0, 1), color: input.color };
});

export const fragment = tgpu.fragmentFn({
  in: { color: d.vec3f },
  out: d.vec4f,
})((input) => {
  "use gpu";
  return d.vec4f(input.color, 1);
});

export function shade(n: number): number {
  "use gpu";
  return n + 1;
}

export function View(value: number) {
  const next = shade(step(value));
  return (
    <div>
      <Panel n={next} />
    </div>
  );
}
