const Number = "outer Number";
const Error = "outer Error";
const NaN = 4;
export function exact(value: number): number {
  if (arguments.length !== globalThis.Number(1)) {
    throw new globalThis.Error(
      "exact" + " requires " + globalThis.Number(1) + " argument(s)",
    );
  }
  return value;
}

export const hygieneResult = [Number, Error, typeof NaN, exact(2)];
