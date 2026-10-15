

const map = <In, Out>(values: readonly In[], f: (value: In) => Out): Out[] =>
values.map((value) => f(value));
const keep = <T,>(values: readonly T[], f: (value: T) => boolean): T[] =>
values.filter((value) => f(value));
const sum = (values: readonly number[]): number =>
values.reduce((left, right) => left + right, 0);

// Each step takes the result of the one before it as its first argument.
export const answer: number = sum(keep(map([1, 2, 3, 4, 5], (value: number) => value * value), (value: number) => value > 4),);
