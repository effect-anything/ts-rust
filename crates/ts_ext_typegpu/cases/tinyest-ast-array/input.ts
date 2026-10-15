function f(a: number) {
  "use gpu";
  const v = [a, 2, 3];
  return v[0] + v[1] + v[2];
}
