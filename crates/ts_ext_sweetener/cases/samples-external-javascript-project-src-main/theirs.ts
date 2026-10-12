
import { double } from "./plain.js";

export const answer = [21, 21];

export const piped = double(21);

/**
 * Macros expand inside a function body, not only at the top level.
 *
 * @param {number} value
 * @returns {number[]}
 */
export function scaled(value) {
  const stepped = double(value);
  return [stepped, stepped];
}

/** @param {boolean} ready */
export function describe(ready) {
  if (!(ready)) {
    return "waiting";
  }
  return "ready";
}
