const doubled = "outer doubled";
const value = "outer value";
const calculate = function (value: number) {
  const doubled = value * 2;
  return doubled + 1;
};

export const hygieneResult = [doubled, value, calculate(3)];
