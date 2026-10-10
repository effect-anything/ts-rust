const vector = <T>(...values: T[]): readonly T[] => values;
const sum = (values: readonly number[]): number =>
  values.reduce((total, value) => total + value, 0);

export const result = globalThis.Object.is(sum(vector(1, 2, 3)), 6);
