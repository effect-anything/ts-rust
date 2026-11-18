// tsc-ext: expand --stage bundle --only tsover
export const add = (a: number, b: number) => {
  "use gpu";
  return a + b;
};
