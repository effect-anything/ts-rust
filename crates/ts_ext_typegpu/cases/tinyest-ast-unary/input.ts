function f(a: number, b: boolean) {
  "use gpu";
  const x = -a;
  const y = !b;
  const z = ~a;
  const w = +a;
  return x + (y ? 1 : 0) + z + w;
}
