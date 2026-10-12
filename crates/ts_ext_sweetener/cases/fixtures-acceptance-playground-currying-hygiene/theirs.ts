const right = "outer right";

export function add(left: number): (right: number) => number;
export function add(left: number, right: number): number;
export function add(
  left: number,
  right_1?: number,
): number | ((right: number) => number) {
  if (right_1 === undefined) {
    return (right_1: number): number => {
      return left + right_1;
    };
  }
  return left + right_1;
}

export const hygieneResult = [right, add(2)(3)];
