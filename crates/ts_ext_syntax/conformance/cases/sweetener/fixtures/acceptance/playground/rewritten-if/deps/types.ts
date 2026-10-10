import { choose, result } from "./expected.js";

choose satisfies (predicate: boolean) => number;
result satisfies number;
