// `for syntax` says this import is compile-time only. It does not appear in
// the TypeScript on the right.


const double = (values: readonly number[]): number[] =>
values.map((value) => value * 2);
const total = (values: readonly number[]): number =>
values.reduce((left, right) => left + right, 0);

// Reads left to right, instead of inside out.
export const answer: number = total(double([1, 2, 3]));
