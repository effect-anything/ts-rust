export function exact(value: number): number {
  if (arguments.length !== globalThis.Number(1)) {
    throw new globalThis.Error(
      "exact" + " requires " + globalThis.Number(1) + " argument(s)",
    );
  }
  return value;
}

export const nanKind = "NaN";
let rejectedExtraArgument = false;
try {
  (exact as (...values: number[]) => number)(1, 2);
} catch {
  rejectedExtraArgument = true;
}

export const result = { nanKind, rejectedExtraArgument };
