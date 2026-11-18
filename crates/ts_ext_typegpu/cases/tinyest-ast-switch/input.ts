function f(a: number) {
  "use gpu";
  switch (a) {
    case 1: return 10;
    case 2: return 20;
    default: return 0;
  }
}
