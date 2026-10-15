

// One line each. The union type and both constructors are generated —
// look at the TypeScript on the right.
type Result<T> = { readonly tag: "Ok"; value: T; } | { readonly tag: "Err"; message: string; }; const Ok = <T,>(value: T): Result<T> => ({ tag: "Ok", value }); const Err = <T,>(message: string): Result<T> => ({ tag: "Err", message });

export const parsed: Result<number> = Ok(42);
export const failed: Result<number> = Err("not a number");

export const describe = (result: Result<number>): string =>
({ value: result, run() { if (this.value.tag === "Ok") { const { value } = this.value; return `ok: ${value}`; } if (this.value.tag === "Err") { const { message } = this.value; return `failed: ${message}`; } const unmatched: never = this.value; throw new globalThis.Error("No match for " + globalThis.JSON.stringify(unmatched),); }, }).run();

export const good: string = describe(parsed);
export const bad: string = describe(failed);

// A recursive type: `Tree` refers to itself inside its own declaration.
type Tree<T> = { readonly tag: "Leaf"; } | { readonly tag: "Node"; left: Tree<T>; value: T; right: Tree<T>; }; const Leaf = <T,>(): Tree<T> => ({ tag: "Leaf" }); const Node = <T,>(left: Tree<T>, value: T, right: Tree<T>): Tree<T> => ({ tag: "Node", left, value, right });

export const total = (tree: Tree<number>): number =>
({ value: tree, run() { if (this.value.tag === "Leaf") { return 0; } if (this.value.tag === "Node") { const { left, value, right } = this.value; return total(left) + value + total(right); } const unmatched_1: never = this.value; throw new globalThis.Error("No match for " + globalThis.JSON.stringify(unmatched_1),); }, }).run();

export const sum: number = total(
  Node(Node(Leaf(), 1, Leaf()), 2, Node(Leaf(), 3, Leaf())),
);
