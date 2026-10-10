export function IF<T>(
  predicate: boolean,
  trueBranch: () => T,
  falseBranch: () => T,
): () => T {
  return predicate ? trueBranch : falseBranch;
}
