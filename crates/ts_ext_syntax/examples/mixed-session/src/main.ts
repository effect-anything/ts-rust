import { shade } from "./view";
import { step } from "./logic.sts";
import { Panel } from "./panel.stsx";

export function frame(value: number): number {
  return shade(step(value));
}

export function label(): string {
  return Panel.name;
}
