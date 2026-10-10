import { IF } from "./runtime.js";

export function choose(predicate: boolean): number {
  return IF(
    predicate,
    () => 3,
    () => 2,
  )();
}

export const result = choose(true);
