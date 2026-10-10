const value = "outer value";
const minimum = "outer minimum";
const maximum = "outer maximum";

export const hygieneResult = [
  value,
  minimum,
  maximum,
  ((value_1: number, minimum_1: number, maximum_1: number): boolean =>
    value_1 > minimum_1 && value_1 < maximum_1)(3, 1, 5),
];
