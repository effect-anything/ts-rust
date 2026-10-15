

export function divide(left: number, right: number): number {
  ((right !== 0) ? true : (() => {
        throw (new globalThis.Error("division by zero"));
      })());
  return left / right;
}
