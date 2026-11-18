

const right = "outer right";

export function add(
      left: number
    ): (right: number) => number;
    export function add(
      left: number,
      right: number
    ): number;
    export function add(
      left: number,
      right?: number,
    ): number | ((right: number) => number) {
      if (right === undefined) {
        return (right: number): number => { return left + right; };
      }
      return left + right;
    }

export const hygieneResult = [right, add(2)(3)];
