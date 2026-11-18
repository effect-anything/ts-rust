"use sweetener";
import { duplicate, unless, (|>) } from "./macros.js" for syntax;
import { double } from "./plain.js";

export const answer = duplicate(21);

export const piped = 21 |> double;

/**
 * Macros expand inside a function body, not only at the top level.
 *
 * @param {number} value
 * @returns {number[]}
 */
export function scaled(value) {
  const stepped = value |> double;
  return duplicate(stepped);
}

/** @param {boolean} ready */
export function describe(ready) {
  unless (ready) {
    return "waiting";
  }
  return "ready";
}
