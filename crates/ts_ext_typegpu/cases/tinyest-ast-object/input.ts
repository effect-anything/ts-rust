function f(a: number) {
  "use gpu";
  const o = { x: a, y: 2 };
  return o.x + o.y;
}
