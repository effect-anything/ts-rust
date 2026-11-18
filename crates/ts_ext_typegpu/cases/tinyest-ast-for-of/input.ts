function f(a: number[]) {
  "use gpu";
  let s = 0;
  for (const x of a) {
    s += x;
  }
  return s;
}
