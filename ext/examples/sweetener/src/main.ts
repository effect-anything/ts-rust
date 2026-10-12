import { shade } from "./view";
import { step } from "./logic.sts";
import { Panel } from "./panel.stsx";
import { total } from "./calc";

export function frame(value: number): number {
  return shade(step(total(value, 1)));
}

export function label(): string {
  return Panel.name;
}
