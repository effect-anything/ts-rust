const value = "call-site value";
const argument = 40;
const add = (left: number, right: number): number => left + right;

export const hygieneResult = {
  outer: value,
  threaded: add(2, argument),
};
