

type Event =
| { kind: "click"; at: { x: number; y: number } }
| { kind: "key"; key: string }
| { kind: "scroll"; delta: number };

// Patterns destructure and bind. `x` and `y` below are ordinary bindings the
// macro introduced from the pattern — look at the generated TypeScript.
export function describe(event: Event): string {
  return ((matched) => { if (typeof matched === "object" && matched !== null && ((matched)["kind"]) === "click" && typeof (matched)["at"] === "object" && (matched)["at"] !== null && true && true) { ; const x = ((matched)["at"])["x"];const y = ((matched)["at"])["y"]; return `click at ${String(x)},${String(y)}`; } if (typeof matched === "object" && matched !== null && ((matched)["kind"]) === "key" && true) { ; const key = (matched)["key"]; if (key === "Escape") { return "escape"; } } if (typeof matched === "object" && matched !== null && ((matched)["kind"]) === "key" && true) { ; const key = (matched)["key"]; return `key ${key}`; } return "something else"; })(event) ?? "something else";
}

export const clicked: string = describe({
    kind: "click",
    at: { x: 3, y: 4 },
  });
export const escaped: string = describe({ kind: "key", key: "Escape" });
export const typed: string = describe({ kind: "key", key: "a" });
