const Object = "call-site Object";
const vector = <T>(...values: T[]): readonly T[] => values;
const count = (values: readonly unknown[]): number => values.length;

export const hygieneResult = [
  Object,
  globalThis.Object.is(count(vector(1, 2, 3)), 3),
];
