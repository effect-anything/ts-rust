function f(a: number, b: number, c: number) {
  "use gpu";
  return (a && b) || (c ?? a);
}
