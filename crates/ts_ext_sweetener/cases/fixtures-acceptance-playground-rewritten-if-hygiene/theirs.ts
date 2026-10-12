import { IF as IF_1 } from "./runtime.js";

const IF = "call-site IF";

function choose(predicate: boolean): number {
  return IF_1(
    predicate,
    () => 3,
    () => 2,
  )();
}

export const hygieneResult = [IF, choose(true)];
