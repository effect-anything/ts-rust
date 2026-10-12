function f(a: number) {
  "use gpu";
  let s = 0;
  for (let i = 0; i < a; i++) {
    if (i === 2) { continue; }
    if (i === 5) { break; }
    s += i;
  }
  return s;
}
