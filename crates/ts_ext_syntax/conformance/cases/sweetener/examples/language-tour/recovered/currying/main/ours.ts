

export function add(
        (left: number, right: number).first: (left: number, right: number).firstType
      ): ((left: number, right: number).second: (left: number, right: number).secondType) => number;
    export function add(
        (left: number, right: number).first: (left: number, right: number).firstType,
        (left: number, right: number).second: (left: number, right: number).secondType
      ): number;
    export function add(
        (left: number, right: number).first: (left: number, right: number).firstType,
        (left: number, right: number).second?: (left: number, right: number).secondType,
      ): number | (((left: number, right: number).second: (left: number, right: number).secondType) => number) {
        if ((left: number, right: number).second === undefined) {
          return ((left: number, right: number).second: (left: number, right: number).secondType): number => { return left + right; };
        }
        return left + right;
      }

export const result = [add(2, 3), add(2)(3)];
