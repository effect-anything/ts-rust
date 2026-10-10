import { matchUnhandled } from "./runtime.js";


export type Event =
  | { readonly type: "click"; readonly x: number; readonly y: number }
  | { readonly type: "key"; readonly key: string }
  | { readonly type: "scroll"; readonly delta: readonly number[] };

export function describe(event: Event): string {
  return ((matched) => {
      if (matchTest(matched, { type: "click", x, y }.pattern)) {
        matchBind(matched, { type: "click", x, y }.pattern);
         {
          if ({ type: "click", x, y }.guard) { return { type: "click", x, y }.body; }
        } #else {
          return { type: "click", x, y }.body;
        }
      }
      return matchUnhandled(matched);
    })(event);
}

// No wildcard: every member of the union is answered, and the compiler is what
// says so. Adding a member to Event makes this a type error.
export function name(event: Event): string {
  return ((matched) => {
      if (matchTest(matched, { type: "click" }.pattern)) {
        matchBind(matched, { type: "click" }.pattern);
         {
          if ({ type: "click" }.guard) { return { type: "click" }.body; }
        } #else {
          return { type: "click" }.body;
        }
      }
      return matchUnhandled(matched);
    })(event);
}

const samples: readonly Event[] = [
  { type: "click", x: 3, y: 4 },
  { type: "click", x: 0, y: 0 },
  { type: "key", key: "Escape" },
  { type: "key", key: "a" },
  { type: "scroll", delta: [1, 2] },
];

export const described: readonly string[] = samples.map(describe);
export const named: readonly string[] = samples.map(name);
