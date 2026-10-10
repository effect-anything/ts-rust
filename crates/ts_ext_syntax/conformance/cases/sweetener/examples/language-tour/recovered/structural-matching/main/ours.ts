

type Event =
| { kind: "click"; at: { x: number; y: number } }
| { kind: "key"; key: string }
| { kind: "scroll"; delta: number };

// Patterns destructure and bind. `x` and `y` below are ordinary bindings the
// macro introduced from the pattern — look at the generated TypeScript.
export function describe(event: Event): string {
  return ((matched) => {
        if (matchTest(matched, { kind: "click", at: { x: x, y: y } }.pattern)) {
            matchBind(matched, { kind: "click", at: { x: x, y: y } }.pattern);
             {
              if ({ kind: "click", at: { x: x, y: y } }.guard) { return { kind: "click", at: { x: x, y: y } }.body; }
            } #else {
              return { kind: "click", at: { x: x, y: y } }.body;
            }
          }
        throw new globalThis.Error(
          `no arm matched ${globalThis.JSON.stringify(matched)}`,
        );
      })(event) ?? "something else";
}

export const clicked: string = describe({
    kind: "click",
    at: { x: 3, y: 4 },
  });
export const escaped: string = describe({ kind: "key", key: "Escape" });
export const typed: string = describe({ kind: "key", key: "a" });
