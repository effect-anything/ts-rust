import { add, result } from "./expected.js";

add(2) satisfies (right: number) => number;
add(2, 3) satisfies number;
result satisfies number[];
