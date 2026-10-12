import { helper as helper_1 } from "./runtime.js";

// The same spelling the first macro introduces.
const tmp = 100;

// A different `helper` from the one the macro's module binds.
function helper(value: number): string {
  return `call site helper saw ${String(value)}`;
}

export const temporary: number = ((tmp_1: number) => tmp_1 + (tmp))(1);
export const described: string = helper_1(7);
export const callSite: string = helper(7);
export const mapped: readonly number[] = ([1, 2, 3]).map((item: number) => (0));
export const untouched: number = tmp;
