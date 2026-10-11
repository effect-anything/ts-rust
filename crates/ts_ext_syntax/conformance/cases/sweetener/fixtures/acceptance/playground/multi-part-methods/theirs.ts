export const result = ((
  value: number,
  minimum: number,
  maximum: number,
): boolean => value > minimum && value < maximum)(3, 1, 5);
