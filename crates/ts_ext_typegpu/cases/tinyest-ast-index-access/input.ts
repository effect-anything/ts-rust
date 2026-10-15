function f(a: number[], i: number, o: any) {
  "use gpu";
  const x = a[i];
  const y = o["k"];
  return x + y;
}
