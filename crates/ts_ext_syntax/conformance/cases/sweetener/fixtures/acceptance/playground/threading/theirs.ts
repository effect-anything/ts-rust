const map = <A, B>(values: readonly A[], fn: (value: A) => B): B[] =>
  values.map(fn);
const filter = <A>(
  values: readonly A[],
  predicate: (value: A) => boolean,
): A[] => values.filter(predicate);

export const threadFirst = filter(
  map([1, 2, 3], (value) => value + 1),
  (value) => value > 2,
);

const append = <A>(suffix: readonly A[], values: readonly A[]): A[] => [
  ...values,
  ...suffix,
];
const mapLast = <A, B>(fn: (value: A) => B, values: readonly A[]): B[] =>
  values.map(fn);
const filterLast = <A>(
  predicate: (value: A) => boolean,
  values: readonly A[],
): A[] => values.filter(predicate);

export const threadLast = append(
  [5],
  filterLast(
    (value) => value > 2,
    mapLast((value) => value + 1, [1, 2, 3]),
  ),
);

export const result = threadFirst;
