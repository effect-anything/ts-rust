"use tsover";

export function total(a: number, b: number): number {
  const sum = a + b;
  a += b;
  return sum * a ** b;
}
